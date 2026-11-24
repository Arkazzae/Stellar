//! Posting to a text channel.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use stellarr_messages::{MessageKey, Messages};

use super::dto::{self, ChannelReadDto, MessageDto};
use super::mentions::{self, Mentioning};
use super::{MAX_CONTENT_UNITS, Stop, field_problem, trimmed, units};
use crate::auth::Caller;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::inbox::{self, Entry};
use crate::link_previews::PreviewRequest;
use crate::permissions::authorization::{self, Held, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{ATTACH_FILES, MANAGE_CHANNELS, SEND_MESSAGES};
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const MAX_ATTACHMENTS: usize = 10;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageRequest {
    content: Option<String>,
    attachment_ids: Option<Vec<Id>>,
    reply_to_message_id: Option<Id>,
    client_message_id: Option<Id>,
}

/// What a sender asked to post, after trimming and de-duplication.
#[derive(Debug, Clone)]
pub struct Submission {
    pub channel: Id,
    pub author: Id,
    pub content: String,
    pub attachments: Vec<Id>,
    pub reply_to: Option<Id>,
    pub client_id: Option<Id>,
}

/// How a post ended.
#[derive(Debug)]
pub enum Posted {
    /// A retry of a send that had gone through: the message as stored.
    Replayed(Box<MessageDto>),
    Stored(Box<Stored>),
}

/// A message that was stored, and what its storing adds.
#[derive(Debug)]
pub struct Stored {
    pub message: MessageDto,
    pub read: ChannelReadDto,
    pub mentions: Vec<Entry>,
}

/// Everything a post asks of the database, in the transaction it is given:
/// the admission, the checks that follow, the message with its attachments,
/// the author's read marker, and the inbox entries and deliveries of the
/// mentions.
pub fn post(
    db: &Connection,
    messages: &Messages,
    server: Id,
    submission: &Submission,
) -> Result<Posted, Stop> {
    let Submission {
        channel, author, ..
    } = *submission;
    let place = visibility::place(db, channel, Some(server), Some(ChannelKind::Text), None)?
        .ok_or_else(Stop::not_found)?;

    // What the sender holds in the channel, read once for every decision
    // below. Without view it is nothing, and the channel is answered as one
    // that does not exist, before anything else, a retry included: a retry
    // would answer with the message as it is stored.
    let held = authorization::held_at(db, author, &Scope::channel(server, channel))?;
    if !held.views() {
        return Err(Stop::not_found());
    }

    // A retry of a send that went through must not wait behind slow mode or
    // be refused for the attachments it already used, so it is recognised
    // first.
    if let Some(replay) = replay(db, messages, submission)? {
        return replay;
    }

    // The one place posting is admitted, after the replay because that posts
    // nothing. It is decided in the channel, so its overrides count.
    deny(
        messages,
        held,
        SEND_MESSAGES,
        MessageKey::SendMessagesDenied,
    )?;
    if !submission.attachments.is_empty() {
        deny(messages, held, ATTACH_FILES, MessageKey::AttachFilesDenied)?;
    }

    if let Some(target) = submission.reply_to {
        let found: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM \"Messages\" WHERE \"Id\" = ?1 AND \"ChannelId\" = ?2)",
            params![target, channel],
            |row| row.get(0),
        )?;
        if !found {
            return Err(field_problem(
                "replyToMessageId",
                messages,
                MessageKey::ReplyTargetInvalid,
                &[],
            )
            .into());
        }
    }

    let attachments = own_attachments(db, author, &submission.attachments)?;
    if attachments.len() != submission.attachments.len() {
        return Err(field_problem(
            "attachmentIds",
            messages,
            MessageKey::AttachmentsNotFound,
            &[],
        )
        .into());
    }

    let now = Timestamp::now();
    if place.slow_mode_seconds > 0 && !held.has(MANAGE_CHANNELS) {
        let last: Option<Timestamp> = db
            .query_row(
                "SELECT \"LastPostedAt\" FROM \"ChannelPostTimes\" WHERE \"ChannelId\" = ?1 AND \"UserId\" = ?2",
                params![channel, author],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(wait) = last.and_then(|last| slow_mode_wait(last, place.slow_mode_seconds, now))
        {
            return Err(Stop::SlowMode(wait));
        }
    }

    // A recording is a message of its own: no text and nothing beside it.
    if attachments.iter().any(|(_, voice)| *voice)
        && (attachments.len() > 1 || !submission.content.is_empty())
    {
        return Err(field_problem(
            "attachmentIds",
            messages,
            MessageKey::VoiceMessageAlone,
            &[],
        )
        .into());
    }

    let id = submission.client_id.unwrap_or_else(Id::random);
    db.prepare_cached(
        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"EditedAt\",
             \"PinnedAt\", \"ReplyToMessageId\")
         VALUES (?1, ?2, ?3, 0, ?4, ?5, NULL, NULL, ?6)",
    )?
    .execute(params![
        id,
        channel,
        author,
        submission.content,
        now,
        submission.reply_to
    ])?;
    for (attachment, _) in &attachments {
        db.prepare_cached(
            "INSERT INTO \"MessageAttachments\" (\"MessageId\", \"AttachmentId\") VALUES (?1, ?2)",
        )?
        .execute(params![id, attachment])?;
    }

    // Whoever sent a message has read everything before it, and nobody is told
    // about a message that was not stored.
    let read = advance_read(db, channel, author, now)?;
    let author_name: String = db.query_row(
        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
        [author],
        |row| row.get(0),
    )?;
    let mentioned = mentions::notify(
        db,
        &Mentioning {
            server,
            channel,
            author,
            author_name: &author_name,
            content: &submission.content,
            created_at: now,
        },
    )?;
    // External deliveries are rows like the inbox entries, written before the
    // commit. No provider is contacted here; the dispatcher does that after it
    // can see the stored message.
    crate::delivery::outbox::enqueue_mentions(db, id, mentioned.iter().map(|n| n.user), now)?;

    let message = dto::load(db, messages, &[id], Some(author))?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal("a stored message was not found again"))?;
    Ok(Posted::Stored(Box::new(Stored {
        message,
        read,
        mentions: mentioned,
    })))
}

/// The refusal of a permission the sender lacks, which names it.
fn deny(messages: &Messages, held: Held, permission: i64, reason: MessageKey) -> Result<(), Stop> {
    match held.require(permission) {
        Ok(_) => Ok(()),
        Err(refusal) => Err(refusal
            .into_error(messages, reason, ApiError::not_found())?
            .into()),
    }
}

/// The answer to a send whose client id is already taken: the stored message
/// when it is the same send again, a conflict when it is not. None when
/// nothing is stored under the id.
fn replay(
    db: &Connection,
    messages: &Messages,
    submission: &Submission,
) -> Result<Option<Result<Posted, Stop>>, Stop> {
    let Some(client_id) = submission.client_id else {
        return Ok(None);
    };
    // The id is taken wherever it is stored, so a message of another channel
    // answers as a conflict.
    let existing: Option<(Id, Id, String, Option<Id>)> = db
        .query_row(
            "SELECT \"AuthorId\", \"ChannelId\", \"Content\", \"ReplyToMessageId\" FROM \"Messages\" WHERE \"Id\" = ?1",
            [client_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((author, channel, content, reply_to)) = existing else {
        return Ok(None);
    };
    let mut attached: Vec<Id> = {
        let mut statement = db.prepare_cached(
            "SELECT \"AttachmentId\" FROM \"MessageAttachments\" WHERE \"MessageId\" = ?1",
        )?;
        statement
            .query_map([client_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut asked = submission.attachments.clone();
    attached.sort();
    asked.sort();
    let same = author == submission.author
        && channel == submission.channel
        && content == submission.content
        && reply_to == submission.reply_to
        && attached == asked;
    if !same {
        return Ok(Some(Err(Stop::Answer(ApiError::conflict()))));
    }
    let message = dto::load(db, messages, &[client_id], Some(submission.author))?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::internal("a stored message was not found again"))?;
    Ok(Some(Ok(Posted::Replayed(Box::new(message)))))
}

/// The attachments among `ids` that this account uploaded and that are not
/// encrypted, with whether each is a voice message, in the order of their ids.
fn own_attachments(db: &Connection, owner: Id, ids: &[Id]) -> rusqlite::Result<Vec<(Id, bool)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = serde_json::to_string(&ids.iter().map(Id::to_db_text).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string());
    let mut statement = db.prepare_cached(
        "SELECT \"Id\", \"IsVoiceMessage\" FROM \"ImageAttachments\"
         WHERE \"Id\" IN (SELECT value FROM json_each(?1)) AND \"UploadedById\" = ?2 AND \"IsEncrypted\" = 0
         ORDER BY \"Id\"",
    )?;
    statement
        .query_map(params![wanted, owner], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// How many whole seconds the author still has to wait in a slow-mode
/// channel, or none when they may post now.
fn slow_mode_wait(last: Timestamp, seconds: i64, now: Timestamp) -> Option<i64> {
    let may_post_at = last.plus(std::time::Duration::from_secs(u64::try_from(seconds).ok()?));
    if may_post_at <= now {
        return None;
    }
    let ticks = may_post_at.ticks() - now.ticks();
    Some(((ticks + 9_999_999) / 10_000_000).max(1))
}

/// Moves the marker forward to `read_at`, and leaves it where it is when it is
/// already later: a slow request must not win against a newer one, and the
/// database decides which is newer.
pub fn advance_read(
    db: &Connection,
    channel: Id,
    user: Id,
    read_at: Timestamp,
) -> rusqlite::Result<ChannelReadDto> {
    db.prepare_cached(
        "INSERT INTO \"ChannelReads\" (\"ChannelId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)
         ON CONFLICT (\"ChannelId\", \"UserId\") DO UPDATE
         SET \"LastReadAt\" = MAX(\"ChannelReads\".\"LastReadAt\", excluded.\"LastReadAt\")",
    )?
    .execute(params![channel, user, read_at])?;
    Ok(ChannelReadDto {
        channel_id: channel,
        last_read_at: read_at,
    })
}

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel): Path<Id>,
    Json(request): Json<CreateMessageRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    if request
        .attachment_ids
        .as_ref()
        .is_some_and(|ids| ids.len() > MAX_ATTACHMENTS)
    {
        return Err(field_problem(
            "attachmentIds",
            &messages,
            MessageKey::AttachmentLimit,
            &[("max", &MAX_ATTACHMENTS)],
        ));
    }
    if request.client_message_id == Some(Id::from_bytes([0; 16])) {
        return Err(field_problem(
            "clientMessageId",
            &messages,
            MessageKey::MessageIdInvalid,
            &[],
        ));
    }

    let mut attachments: Vec<Id> = Vec::new();
    for id in request.attachment_ids.iter().flatten() {
        if !attachments.contains(id) {
            attachments.push(*id);
        }
    }
    let submission = Submission {
        channel,
        author: caller.user,
        content: trimmed(request.content.as_deref().unwrap_or("")).to_string(),
        attachments,
        reply_to: request.reply_to_message_id,
        client_id: request.client_message_id,
    };
    if (submission.content.is_empty() && submission.attachments.is_empty())
        || units(&submission.content) > MAX_CONTENT_UNITS
    {
        return Err(field_problem(
            "content",
            &messages,
            MessageKey::MessageContentInvalid,
            &[("max", &MAX_CONTENT_UNITS)],
        ));
    }

    let outcome = state
        .database()
        .write(move |tx| post(tx, &messages, server, &submission))
        .await;
    let stored = match outcome {
        Ok(Posted::Replayed(message)) => return Ok(json_response(StatusCode::OK, &*message)),
        Ok(Posted::Stored(stored)) => *stored,
        Err(stop) => return Ok(stop.into_response(&messages)),
    };

    // The message is stored by now, so a client that hangs up must not keep
    // the others from hearing about it (the handler runs to its end).
    let audience = state
        .database()
        .read(move |db| -> Result<Audience, ApiError> {
            Ok(visibility::audience_of(db, server, channel)?)
        })
        .await?;
    let events = state.events();
    events.publish(Event::new(audience, "MessageCreated", &stored.message));
    events.publish(Event::new(
        Audience::Users(vec![caller.user]),
        "ChannelReadChanged",
        &stored.read,
    ));
    for entry in &stored.mentions {
        inbox::announce(events, entry);
    }

    // Last: unfurling talks to hosts we do not control, and no send waits for
    // one of them.
    state.previews().enqueue(PreviewRequest {
        channel,
        message: stored.message.id,
    });

    let mut response = json_response(StatusCode::CREATED, &stored.message);
    if let Ok(location) = HeaderValue::from_str(&format!(
        "/api/channels/{channel}/messages/{}",
        stored.message.id
    )) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response.into_response())
}
