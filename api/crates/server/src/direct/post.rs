//! Posting in a conversation, plain or encrypted.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::MessageKey;

use super::encryption::{self, Payload};
use crate::http::lenient::{Malformed, Members, Octets};
use super::{peer_in, peer_takes_messages_from, set_read};
use crate::auth::Caller;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::delivery::outbox;
use crate::messages::{MAX_CONTENT_UNITS, field_problem, trimmed, units};
use crate::social::conversations::{self, DirectMessage};
use crate::social::{announce, is_blocked};
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

const MAX_ATTACHMENTS: usize = 10;

/// An encrypted video brings its poster, and an encrypted image its preview,
/// as an upload of its own, since the server cannot make one from bytes it
/// cannot read.
const MAX_ENCRYPTED_UPLOADS: usize = 2 * MAX_ATTACHMENTS;

/// The nonce of every AES-GCM encryption.
const IV_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
/// The longest text escaped as JSON in the worst case, and the descriptions of
/// its attachments.
const MAX_CIPHERTEXT_BYTES: usize = 256 * 1024;

/// What a sender asked to post, after trimming and de-duplication.
struct Submission {
    conversation: Id,
    author: Id,
    content: String,
    attachments: Vec<Id>,
    encrypted: bool,
}

/// What the request said, read.
struct Request {
    content: Option<String>,
    attachments: Option<Vec<Id>>,
    client_id: Option<Id>,
    encrypted: Option<RequestedEncryption>,
}

struct RequestedEncryption {
    epoch: Option<Id>,
    iv: Option<Vec<u8>>,
    ciphertext: Option<Vec<u8>>,
}

fn read(body: &Members) -> Result<Request, Malformed> {
    let encrypted = match body.get::<Members>("encrypted")? {
        None => None,
        Some(inner) => Some(RequestedEncryption {
            epoch: inner.get::<Id>("epochId")?,
            iv: inner.get::<Octets>("iv")?.map(|octets| octets.0),
            ciphertext: inner.get::<Octets>("ciphertext")?.map(|octets| octets.0),
        }),
    };
    Ok(Request {
        content: body.get::<String>("content")?,
        attachments: body.get::<Vec<Id>>("attachmentIds")?,
        client_id: body.get::<Id>("clientMessageId")?,
        encrypted,
    })
}

enum Posted {
    /// A retry of a send that had gone through.
    Replayed(DirectMessage),
    Stored { message: DirectMessage, peer: Id },
}

/// Author, conversation, content, epoch and timeline kind of a stored message.
type Stored = (Id, Id, String, Option<Id>, Option<String>);

/// Whether the stored message is what this submission would have produced. An
/// encrypted message is the same send when the id the client picked, the
/// author, the conversation and the uploads match: every encrypted message has
/// empty content, and a browser that retries may encrypt again, with a new
/// nonce or a newer epoch.
fn produced(connection: &Connection, submission: &Submission, id: Id) -> rusqlite::Result<Option<bool>> {
    let existing: Option<Stored> = connection
        .query_row(
            "SELECT \"AuthorId\", \"ConversationId\", \"Content\", \"EncryptedEpochId\", \"SystemKind\"
             FROM \"DirectMessages\" WHERE \"Id\" = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((author, conversation, content, epoch, kind)) = existing else {
        return Ok(None);
    };
    let mut attached: Vec<Id> = {
        let mut statement = connection
            .prepare("SELECT \"AttachmentId\" FROM \"DirectMessageAttachments\" WHERE \"DirectMessageId\" = ?1")?;
        statement
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut asked = submission.attachments.clone();
    attached.sort();
    asked.sort();
    let same_kind = if submission.encrypted {
        epoch.is_some()
    } else {
        epoch.is_none() && content == submission.content
    };
    Ok(Some(
        author == submission.author
            && conversation == submission.conversation
            && kind.is_none()
            && same_kind
            && attached == asked,
    ))
}

fn load(connection: &Connection, id: Id) -> Result<DirectMessage, ApiError> {
    conversations::message(connection, id)?
        .map(|(message, _)| message)
        .ok_or_else(|| ApiError::internal("a stored message was not found again"))
}

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Path(Guid(conversation)): Path<Guid>,
    Json(body): Json<Members>,
) -> ApiResult<Response> {
    let request = read(&body)?;
    let messages = *state.messages();
    let me = caller.user;
    let posted = state
        .database()
        .write(move |tx| -> Result<Posted, ApiError> {
            let Some(peer) = peer_in(tx, conversation, me)? else {
                return Err(ApiError::not_found());
            };
            if is_blocked(tx, me, peer)? {
                return Err(ApiError::forbidden());
            }

            let encrypted = request.encrypted.is_some();
            let most = if encrypted { MAX_ENCRYPTED_UPLOADS } else { MAX_ATTACHMENTS };
            if request.attachments.as_ref().is_some_and(|ids| ids.len() > most) {
                return Err(field_problem("attachmentIds", &messages, MessageKey::AttachmentLimit, &[("max", &most)]));
            }
            if request.client_id.is_some_and(|id| id == Id::from_bytes([0; 16])) {
                return Err(field_problem("clientMessageId", &messages, MessageKey::MessageIdInvalid, &[]));
            }

            let mut attachments: Vec<Id> = Vec::new();
            for id in request.attachments.iter().flatten() {
                if !attachments.contains(id) {
                    attachments.push(*id);
                }
            }
            let submission = Submission {
                conversation,
                author: me,
                content: trimmed(request.content.as_deref().unwrap_or("")).to_string(),
                attachments,
                encrypted,
            };
            let mut payload = None;
            if let Some(sent) = &request.encrypted {
                // The text is in the ciphertext, which the server cannot check
                // against the limit; it only bounds the size.
                let valid = request.client_id.is_some()
                    && request.content.as_deref().is_none_or(str::is_empty)
                    && match (sent.epoch, &sent.iv, &sent.ciphertext) {
                        (Some(epoch), Some(iv), Some(ciphertext))
                            if iv.len() == IV_BYTES
                                && ciphertext.len() > TAG_BYTES
                                && ciphertext.len() <= MAX_CIPHERTEXT_BYTES =>
                        {
                            payload = Some(Payload { epoch, iv: iv.clone(), ciphertext: ciphertext.clone() });
                            true
                        }
                        _ => false,
                    };
                if !valid {
                    return Err(field_problem("encrypted", &messages, MessageKey::EncryptionMessageInvalid, &[]));
                }
            } else if (submission.content.is_empty() && submission.attachments.is_empty())
                || units(&submission.content) > MAX_CONTENT_UNITS
            {
                return Err(field_problem(
                    "content",
                    &messages,
                    MessageKey::MessageContentInvalid,
                    &[("max", &MAX_CONTENT_UNITS)],
                ));
            }

            // A retry of a send that went through is answered before anything
            // that could have changed since: the attachments it used, the
            // peer's settings.
            if let Some(id) = request.client_id {
                match produced(tx, &submission, id)? {
                    Some(true) => return Ok(Posted::Replayed(load(tx, id)?)),
                    Some(false) => return Err(ApiError::conflict()),
                    None => {}
                }
            }

            let owned = own_uploads(tx, me, &submission.attachments)?;
            if owned.len() != submission.attachments.len() {
                return Err(field_problem("attachmentIds", &messages, MessageKey::AttachmentsNotFound, &[]));
            }
            if owned.iter().any(|(_, is_encrypted)| *is_encrypted != encrypted) {
                return Err(encryption::attachment_kind(&messages));
            }
            if !peer_takes_messages_from(tx, me, peer)? {
                return Err(ApiError::forbidden());
            }

            let now = Timestamp::now();
            let stored = conversations::find(tx, conversation)?
                .ok_or_else(ApiError::not_found)?;
            encryption::refusal(tx, &messages, &stored, payload.as_ref(), now)?;

            let id = request.client_id.unwrap_or_else(Id::random);
            tx.execute(
                "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\",
                     \"EncryptedEpochId\", \"EncryptedIv\", \"EncryptedCiphertext\", \"SystemKind\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    id,
                    conversation,
                    me,
                    submission.content,
                    now,
                    payload.as_ref().map(|p| p.epoch),
                    payload.as_ref().map(|p| p.iv.clone()),
                    payload.as_ref().map(|p| p.ciphertext.clone()),
                ],
            )?;
            for (attachment, _) in &owned {
                tx.execute(
                    "INSERT INTO \"DirectMessageAttachments\" (\"DirectMessageId\", \"AttachmentId\") VALUES (?1, ?2)",
                    params![id, attachment],
                )?;
            }
            // Whoever sent a message has read everything before it. Only the
            // peer is a recipient of its deliveries, which commit with the
            // message; a retry never reaches this point.
            set_read(tx, conversation, me, now)?;
            outbox::enqueue_direct_message(tx, id, peer, now)?;
            Ok(Posted::Stored { message: load(tx, id)?, peer })
        })
        .await?;

    match posted {
        Posted::Replayed(message) => Ok(json_response(StatusCode::OK, &message)),
        Posted::Stored { message, peer } => {
            // The message itself is delivered, not just a hint to reload: the
            // open transcript appends it at once while the rail refreshes its
            // unread count.
            state.events().publish(Event::new(
                Audience::Users(vec![me, peer]),
                "DirectMessageCreated",
                &message,
            ));
            announce(&state, &[me, peer]);
            let mut response = json_response(StatusCode::CREATED, &message);
            if let Ok(location) = HeaderValue::from_str(&format!(
                "/api/social/conversations/{conversation}/messages/{}",
                message.id()
            )) {
                response.headers_mut().insert(header::LOCATION, location);
            }
            Ok(response.into_response())
        }
    }
}

/// The uploads among `ids` that this account made, with whether each is
/// encrypted, in the order of their ids.
fn own_uploads(connection: &Connection, owner: Id, ids: &[Id]) -> rusqlite::Result<Vec<(Id, bool)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = serde_json::to_string(&ids.iter().map(Id::to_db_text).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string());
    let mut statement = connection.prepare_cached(
        "SELECT \"Id\", \"IsEncrypted\" FROM \"ImageAttachments\"
         WHERE \"Id\" IN (SELECT value FROM json_each(?1)) AND \"UploadedById\" = ?2 ORDER BY \"Id\"",
    )?;
    statement
        .query_map(params![wanted, owner], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}
