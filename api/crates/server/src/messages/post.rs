//! Posting to a text channel.
pub(crate) mod batch;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use stellarr_messages::{MessageKey, Messages};

use super::dto::{self, ChannelReadDto, MessageDto};
use super::mentions::{self, Mentioning};
use super::{Reason, Stop, content_limit, field_problem, trimmed, units};
use crate::auth::Caller;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::inbox::{self, Entry};
use crate::link_previews::PreviewRequest;
use crate::permissions::authorization::{Held, held_in_channel};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{ATTACH_FILES, MANAGE_CHANNELS, SEND_MESSAGES};
use crate::permissions::visibility::{self, ChannelKind, ChannelPlace};
use crate::scheduled::Claim;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

pub(crate) const MAX_ATTACHMENTS: usize = 10;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageRequest {
    content: Option<String>,
    attachment_ids: Option<Vec<Id>>,
    reply_to_message_id: Option<Id>,
    client_message_id: Option<Id>,
    poll: Option<crate::polls::PollRequest>,
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
    /// The scheduled message this is, which leaves the queue with it.
    pub scheduled: Option<Claim>,
    /// The poll the message carries, checked against its limits.
    pub poll: Option<crate::polls::Draft>,
    /// The server event the message announces, whose card it shows.
    pub event: Option<Id>,
}

/// How a post ended.
#[derive(Debug)]
pub enum Posted {
    /// A retry of a send that had gone through: the message as stored.
    Replayed(Box<MessageDto>),
    Stored(Box<Stored>),
}

/// A message that was stored, and what its storing adds. A message sent from
/// the queue of scheduled ones leaves the author's read marker where it was:
/// they were not there to read what came before it.
#[derive(Debug)]
pub struct Stored {
    pub message: MessageDto,
    pub read: Option<ChannelReadDto>,
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
    let (place, held) = locate(db, server, submission)?;

    // A retry of a send that went through must not wait behind slow mode or
    // be refused for the attachments it already used, so it is recognised
    // first.
    if let Some(replay) = replay(db, messages, submission)? {
        // A scheduled message that went out before leaves the queue with
        // this retry.
        if let (Ok(_), Some(claim)) = (&replay, submission.scheduled) {
            claim.settle(db)?;
        }
        return replay;
    }

    let now = Timestamp::now();
    let attachments = admit(db, messages, &place, held, submission, now)?;

    // A scheduled message leaves the queue in the transaction that posts it,
    // so that a restart cannot post it twice; one edited or deleted since it
    // was read is not posted at all.
    if let Some(claim) = submission.scheduled
        && !claim.take(db)?
    {
        return Err(Stop::refused(Reason::Withdrawn, ApiError::conflict()));
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
    if let Some(poll) = &submission.poll {
        crate::polls::insert(db, id, poll, now)?;
    }
    if let Some(event) = submission.event {
        crate::server_events::announce_in(db, id, event)?;
    }

    // Whoever sent a message has read everything before it, unless it was sent
    // from the queue while they were away; and nobody is told about a message
    // that was not stored.
    let read = match submission.scheduled {
        None => Some(advance_read(db, channel, author, now)?),
        Some(_) => None,
    };
    let author_name: String = crate::instance::users::User::display_name(db, author)?;
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

/// Whether the submission could be posted now, as [`post`] decides it, with
/// nothing written: what scheduling a message checks when it is made or
/// changed. Slow mode is the one refusal left out, since a scheduled message
/// waits it out when its time comes.
pub fn admissible(
    db: &Connection,
    messages: &Messages,
    server: Id,
    submission: &Submission,
) -> Result<(), Stop> {
    let (place, held) = locate(db, server, submission)?;
    match admit(db, messages, &place, held, submission, Timestamp::now()) {
        Ok(_) | Err(Stop::SlowMode(_)) => Ok(()),
        Err(stop) => Err(stop),
    }
}

/// The text channel the submission is for, and what its author holds there.
/// Without view it is nothing, and the channel is answered as one that does
/// not exist, before anything else, a retry included: a retry would answer
/// with the message as it is stored. The rights are read once for every
/// decision that follows.
fn locate(
    db: &Connection,
    server: Id,
    submission: &Submission,
) -> Result<(ChannelPlace, Held), Stop> {
    let place = visibility::place(
        db,
        submission.channel,
        Some(server),
        Some(ChannelKind::Text),
        None,
    )?
    .ok_or_else(|| Stop::refused(Reason::Gone, ApiError::not_found()))?;
    // What the sender holds in the channel, read once for every decision
    // that follows, from the place already found rather than by finding the
    // channel again.
    let held = held_in_channel(db, submission.author, &place)?;
    if !held.views() {
        return Err(Stop::refused(Reason::NoAccess, ApiError::not_found()));
    }
    Ok((place, held))
}

/// Every check of a post after the replay, slow mode last so that its wait
/// is only given to a post that would otherwise go in: the attachments it
/// may carry, each with whether it is a voice message.
fn admit(
    db: &Connection,
    messages: &Messages,
    place: &ChannelPlace,
    held: Held,
    submission: &Submission,
    now: Timestamp,
) -> Result<Vec<(Id, bool)>, Stop> {
    let Submission {
        channel, author, ..
    } = *submission;

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
            return Err(Stop::refused(
                Reason::ReplyGone,
                field_problem(
                    "replyToMessageId",
                    messages,
                    MessageKey::ReplyTargetInvalid,
                    &[],
                ),
            ));
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
    Ok(attachments)
}

/// The refusal of a permission the sender lacks, which names it.
fn deny(messages: &Messages, held: Held, permission: i64, reason: MessageKey) -> Result<(), Stop> {
    match held.require(permission) {
        Ok(_) => Ok(()),
        Err(refusal) => Err(Stop::refused(
            Reason::NoAccess,
            refusal.into_error(messages, reason, ApiError::not_found())?,
        )),
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

/// The attachments a client named, each once, refused when there are more
/// than a message may carry. Checked before anything is read.
pub(crate) fn attachments_of(messages: &Messages, ids: Option<&[Id]>) -> Result<Vec<Id>, ApiError> {
    if ids.is_some_and(|ids| ids.len() > MAX_ATTACHMENTS) {
        return Err(field_problem(
            "attachmentIds",
            messages,
            MessageKey::AttachmentLimit,
            &[("max", &MAX_ATTACHMENTS)],
        ));
    }
    let mut attachments: Vec<Id> = Vec::new();
    for id in ids.into_iter().flatten() {
        if !attachments.contains(id) {
            attachments.push(*id);
        }
    }
    Ok(attachments)
}

/// The text a client sent, trimmed, refused when it is empty with nothing
/// attached and no poll, or longer than a message may be.
pub(crate) fn content_of(
    messages: &Messages,
    content: Option<&str>,
    attachments: &[Id],
    with_poll: bool,
) -> Result<String, ApiError> {
    let content = trimmed(content.unwrap_or("")).to_string();
    let length = units(&content);
    let max = content_limit(&content, length);
    if (content.is_empty() && attachments.is_empty() && !with_poll) || length > max {
        return Err(field_problem(
            "content",
            messages,
            MessageKey::MessageContentInvalid,
            &[("max", &max)],
        ));
    }
    Ok(content)
}

/// Tells everyone a stored message concerns about it, once it is committed:
/// whoever sees the channel, the author's other tabs when their read marker
/// moved, the people it mentions, and the worker that unfurls its links.
pub(crate) async fn announce(
    state: &AppState,
    server: Id,
    author: Id,
    stored: &Stored,
) -> Result<(), ApiError> {
    let channel = stored.message.channel_id;
    let audience = state
        .database()
        .read(move |db| -> Result<Audience, ApiError> {
            Ok(visibility::audience_of(db, server, channel)?)
        })
        .await?;
    let events = state.events();
    if stored.message.poll.is_some() || stored.message.event.is_some() {
        // Everyone is told of a poll or an event card alike, so the event
        // cannot say what is the reader's own; the author's answer can.
        let mut shared = stored.message.clone();
        if let Some(poll) = shared.poll.as_mut() {
            poll.mine = None;
        }
        if let Some(card) = shared.event.as_mut() {
            card.mine = None;
            card.my_position = None;
        }
        events.publish(Event::new(audience, "MessageCreated", &shared));
    } else {
        events.publish(Event::new(audience, "MessageCreated", &stored.message));
    }
    if let Some(read) = &stored.read {
        events.publish(Event::new(
            Audience::Users(vec![author]),
            "ChannelReadChanged",
            read,
        ));
    }
    for entry in &stored.mentions {
        inbox::announce(events, entry);
    }
    // The worker that ends polls looks again at when the next one ends.
    if stored.message.poll.is_some() {
        state.poll_alarm().ring();
    }

    // Last: unfurling talks to hosts we do not control, and no send waits for
    // one of them.
    state.previews().enqueue(PreviewRequest {
        channel,
        message: stored.message.id,
    });
    Ok(())
}

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel): Path<Id>,
    Json(request): Json<CreateMessageRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let attachments = attachments_of(&messages, request.attachment_ids.as_deref())?;
    if request.client_message_id == Some(Id::from_bytes([0; 16])) {
        return Err(field_problem(
            "clientMessageId",
            &messages,
            MessageKey::MessageIdInvalid,
            &[],
        ));
    }
    let poll = request
        .poll
        .as_ref()
        .map(|poll| poll.draft(&messages))
        .transpose()?;
    // A poll is a message of its own kind, which carries no files.
    if poll.is_some() && !attachments.is_empty() {
        return Err(field_problem(
            "attachmentIds",
            &messages,
            MessageKey::PollAttachments,
            &[],
        ));
    }
    let content = content_of(
        &messages,
        request.content.as_deref(),
        &attachments,
        poll.is_some(),
    )?;
    let submission = Submission {
        channel,
        author: caller.user,
        content,
        attachments,
        reply_to: request.reply_to_message_id,
        client_id: request.client_message_id,
        scheduled: None,
        poll,
        event: None,
    };

    let outcome = state
        .post_queue()
        .submit(state.database(), messages, server, submission)
        .await;
    let stored = match outcome {
        Ok(Posted::Replayed(message)) => return Ok(json_response(StatusCode::OK, &*message)),
        Ok(Posted::Stored(stored)) => *stored,
        Err(stop) => return Ok(stop.into_response(&messages)),
    };

    // The message is stored by now, so a client that hangs up must not keep
    // the others from hearing about it (the handler runs to its end).
    announce(&state, server, caller.user, &stored).await?;

    let mut response = json_response(StatusCode::CREATED, &stored.message);
    if let Ok(location) = HeaderValue::from_str(&format!(
        "/api/channels/{channel}/messages/{}",
        stored.message.id
    )) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response.into_response())
}
