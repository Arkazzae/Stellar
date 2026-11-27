//! Posting in a conversation, plain or encrypted.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::{MessageKey, Messages};

use super::encryption::{self, Payload};
use crate::encoding::optional_octets;
use serde::Deserialize;
use super::{peer_in, peer_takes_messages_from, set_read};
use crate::auth::Caller;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::delivery::outbox;
use crate::messages::{MAX_CODE_CONTENT_UNITS, Reason, Stop, content_limit, field_problem, trimmed, units};
use crate::social::conversations::{self, DirectMessage};
use crate::social::is_blocked;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

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
const MAX_CIPHERTEXT_BYTES: usize = MAX_CODE_CONTENT_UNITS * 6 + 128 * 1024;

/// What a sender asked to post, after trimming and de-duplication.
pub(crate) struct Submission {
    conversation: Id,
    author: Id,
    content: String,
    attachments: Vec<Id>,
    encrypted: bool,
    client_id: Option<Id>,
    payload: Option<Payload>,
}

/// What the request said, read.
#[derive(Deserialize)]
pub struct Request {
    content: Option<String>,
    #[serde(rename = "attachmentIds")]
    attachments: Option<Vec<Id>>,
    #[serde(rename = "clientMessageId")]
    client_id: Option<Id>,
    encrypted: Option<RequestedEncryption>,
}

#[derive(Deserialize)]
pub struct RequestedEncryption {
    #[serde(rename = "epochId")]
    epoch: Option<Id>,
    #[serde(default, deserialize_with = "optional_octets")]
    iv: Option<Vec<u8>>,
    #[serde(default, deserialize_with = "optional_octets")]
    ciphertext: Option<Vec<u8>>,
}

pub(crate) enum Posted {
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

/// The other person of the conversation, when the author is one of its two
/// and neither has blocked the other.
pub(crate) fn reach(connection: &Connection, conversation: Id, author: Id) -> Result<Id, Stop> {
    let Some(peer) = peer_in(connection, conversation, author)? else {
        return Err(Stop::refused(Reason::Gone, ApiError::not_found()));
    };
    if is_blocked(connection, author, peer)? {
        return Err(Stop::refused(Reason::NoAccess, ApiError::forbidden()));
    }
    Ok(peer)
}

/// What else a message must pass before it goes in: its uploads are the
/// author's and of its kind, and the peer takes messages from the author.
pub(crate) fn admit(
    connection: &Connection,
    messages: &Messages,
    submission: &Submission,
    peer: Id,
) -> Result<Vec<(Id, bool)>, Stop> {
    let owned = own_uploads(connection, submission.author, &submission.attachments)?;
    if owned.len() != submission.attachments.len() {
        return Err(field_problem("attachmentIds", messages, MessageKey::AttachmentsNotFound, &[]).into());
    }
    if owned.iter().any(|(_, is_encrypted)| *is_encrypted != submission.encrypted) {
        return Err(encryption::attachment_kind(messages).into());
    }
    if !peer_takes_messages_from(connection, submission.author, peer)? {
        return Err(Stop::refused(Reason::NoAccess, ApiError::forbidden()));
    }
    Ok(owned)
}

/// Stores the message to `peer`, in the transaction it is given, with its
/// attachments, the author's read marker and the peer's delivery.
pub(crate) fn store(
    tx: &Connection,
    messages: &Messages,
    submission: &Submission,
    peer: Id,
) -> Result<Posted, Stop> {
    // A retry of a send that went through is answered before anything
    // that could have changed since: the attachments it used, the
    // peer's settings.
    if let Some(id) = submission.client_id {
        match produced(tx, submission, id)? {
            Some(true) => return Ok(Posted::Replayed(load(tx, id)?)),
            Some(false) => return Err(ApiError::conflict().into()),
            None => {}
        }
    }

    let owned = admit(tx, messages, submission, peer)?;

    let now = Timestamp::now();
    let stored = conversations::find(tx, submission.conversation)?
        .ok_or_else(|| Stop::refused(Reason::Gone, ApiError::not_found()))?;
    // Plain text is refused only by a conversation that is encrypted, which a
    // sender other than a person at the keyboard needs told apart.
    encryption::refusal(tx, messages, &stored, submission.payload.as_ref(), now).map_err(
        |error| match (&submission.payload, error) {
            (None, error @ ApiError::Problem(_)) => Stop::refused(Reason::Encrypted, error),
            (_, error) => Stop::Answer(error),
        },
    )?;

    let id = submission.client_id.unwrap_or_else(Id::random);
    let payload = submission.payload.as_ref();
    tx.execute(
        "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\",
             \"EncryptedEpochId\", \"EncryptedIv\", \"EncryptedCiphertext\", \"SystemKind\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
        params![
            id,
            submission.conversation,
            submission.author,
            submission.content,
            now,
            payload.map(|p| p.epoch),
            payload.map(|p| p.iv.clone()),
            payload.map(|p| p.ciphertext.clone()),
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
    set_read(tx, submission.conversation, submission.author, now)?;
    outbox::enqueue_direct_message(tx, id, peer, now)?;
    Ok(Posted::Stored { message: load(tx, id)?, peer })
}

/// Tells both people of a stored message, once it is committed. The message
/// itself is delivered, not just a hint to reload: the open transcript
/// appends it at once while the rail refreshes its unread count.
pub(crate) fn announce(state: &AppState, author: Id, peer: Id, message: &DirectMessage) {
    state.events().publish(Event::new(
        Audience::Users(vec![author, peer]),
        "DirectMessageCreated",
        message,
    ));
    crate::social::announce(state, &[author, peer]);
}

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Path(conversation): Path<Id>,
    Json(request): Json<Request>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let me = caller.user;
    let posted = state
        .database()
        .write(move |tx| -> Result<Posted, Stop> {
            let peer = reach(tx, conversation, me)?;

            let encrypted = request.encrypted.is_some();
            let most = if encrypted { MAX_ENCRYPTED_UPLOADS } else { MAX_ATTACHMENTS };
            if request.attachments.as_ref().is_some_and(|ids| ids.len() > most) {
                return Err(field_problem("attachmentIds", &messages, MessageKey::AttachmentLimit, &[("max", &most)]).into());
            }
            if request.client_id.is_some_and(|id| id == Id::from_bytes([0; 16])) {
                return Err(field_problem("clientMessageId", &messages, MessageKey::MessageIdInvalid, &[]).into());
            }

            let mut attachments: Vec<Id> = Vec::new();
            for id in request.attachments.iter().flatten() {
                if !attachments.contains(id) {
                    attachments.push(*id);
                }
            }
            let mut submission = Submission {
                conversation,
                author: me,
                content: trimmed(request.content.as_deref().unwrap_or("")).to_string(),
                attachments,
                encrypted,
                client_id: request.client_id,
                payload: None,
            };
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
                            submission.payload = Some(Payload { epoch, iv: iv.clone(), ciphertext: ciphertext.clone() });
                            true
                        }
                        _ => false,
                    };
                if !valid {
                    return Err(field_problem("encrypted", &messages, MessageKey::EncryptionMessageInvalid, &[]).into());
                }
            } else {
                let length = units(&submission.content);
                let max = content_limit(&submission.content, length);
                if (submission.content.is_empty() && submission.attachments.is_empty()) || length > max {
                    return Err(field_problem("content", &messages, MessageKey::MessageContentInvalid, &[("max", &max)]).into());
                }
            }

            store(tx, &messages, &submission, peer)
        })
        .await;

    match posted {
        Ok(Posted::Replayed(message)) => Ok(json_response(StatusCode::OK, &message)),
        Ok(Posted::Stored { message, peer }) => {
            announce(&state, me, peer, &message);
            let mut response = json_response(StatusCode::CREATED, &message);
            if let Ok(location) = HeaderValue::from_str(&format!(
                "/api/social/conversations/{conversation}/messages/{}",
                message.id()
            )) {
                response.headers_mut().insert(header::LOCATION, location);
            }
            Ok(response.into_response())
        }
        Err(stop) => Ok(stop.into_response(&messages)),
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
