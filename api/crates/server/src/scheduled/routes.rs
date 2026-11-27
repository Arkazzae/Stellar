//! The routes of scheduled messages. Each answers for the caller's own
//! messages only; somebody else's is one that does not exist.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use stellarr_messages::{MessageKey, Messages, Value};

use super::send::{self, Attempt};
use super::store::{self, Filter, Scheduled, ScheduledMessageDto, Target};
use super::{MAX_LEAD_DAYS, MAX_PENDING, MIN_LEAD, announce_changed, announce_removed};
use crate::auth::Caller;
use crate::database::DbError;
use crate::direct::post as direct;
use crate::http::{ApiError, ApiResult, Json, Path, Problem, QueryValues, json_response};
use crate::messages::post::{self, Submission};
use crate::messages::{Stop, field_problem};
use crate::permissions::membership::ActiveServer;
use crate::social::conversations;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The problem codes of the refusals that are about the state of things, not
/// about a field of the request.
const LIMIT: &str = "scheduled-limit";
const ENCRYPTED: &str = "scheduled-encrypted";
const FAILED: &str = "scheduled-failed";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRequest {
    content: Option<String>,
    attachment_ids: Option<Vec<Id>>,
    reply_to_message_id: Option<Id>,
    send_at: Option<Timestamp>,
    /// Read only to be refused: a poll is not scheduled, and a body that
    /// has one must not go out as a plain message.
    poll: Option<serde_json::Value>,
}

/// A message to a conversation has no reply: direct messages have none.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRequest {
    content: Option<String>,
    attachment_ids: Option<Vec<Id>>,
    send_at: Option<Timestamp>,
    poll: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditRequest {
    content: Option<String>,
    send_at: Option<Timestamp>,
}

pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
    query: QueryValues,
) -> ApiResult<Json<Vec<ScheduledMessageDto>>> {
    let id = |name: &str| -> Result<Option<Id>, ApiError> {
        query
            .text(name)?
            .map(|text| Id::parse(&text).ok_or_else(ApiError::bad_request))
            .transpose()
    };
    let filter = match (id("channelId")?, id("conversationId")?) {
        (None, None) => Filter::All,
        (Some(channel), None) => Filter::Channel(channel),
        (None, Some(conversation)) => Filter::Conversation(conversation),
        (Some(_), Some(_)) => return Err(ApiError::bad_request()),
    };
    let author = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let rows = store::of_author(db, author, filter)?;
            Ok(store::dtos(db, rows)?)
        })
        .await?;
    Ok(Json(found))
}

pub async fn in_channel(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel): Path<Id>,
    Json(request): Json<ChannelRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let now = Timestamp::now();
    no_poll(&messages, request.poll.as_ref())?;
    let attachments = post::attachments_of(&messages, request.attachment_ids.as_deref())?;
    let content = post::content_of(&messages, request.content.as_deref(), &attachments, false)?;
    let send_at = send_time(&messages, request.send_at, now)?;
    let scheduled = Scheduled {
        id: Id::random(),
        author: caller.user,
        target: Target::Channel { server, channel },
        content,
        reply_to: request.reply_to_message_id,
        send_at,
        created_at: now,
        revision: 0,
        failure: None,
        attachments,
    };
    let made = state
        .database()
        .write(move |tx| make(tx, &messages, &scheduled))
        .await;
    Ok(answer(&state, caller.user, made, &messages, StatusCode::CREATED))
}

pub async fn in_conversation(
    State(state): State<AppState>,
    caller: Caller,
    Path(conversation): Path<Id>,
    Json(request): Json<ConversationRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let now = Timestamp::now();
    no_poll(&messages, request.poll.as_ref())?;
    let attachments = post::attachments_of(&messages, request.attachment_ids.as_deref())?;
    let content = post::content_of(&messages, request.content.as_deref(), &attachments, false)?;
    let send_at = send_time(&messages, request.send_at, now)?;
    let scheduled = Scheduled {
        id: Id::random(),
        author: caller.user,
        target: Target::Conversation(conversation),
        content,
        reply_to: None,
        send_at,
        created_at: now,
        revision: 0,
        failure: None,
        attachments,
    };
    let made = state
        .database()
        .write(move |tx| make(tx, &messages, &scheduled))
        .await;
    Ok(answer(&state, caller.user, made, &messages, StatusCode::CREATED))
}

/// Changes the text or the time of a pending message. Its files and what it
/// answers stay as they are; a failed one is not changed at all, since its
/// author sends it now or deletes it.
pub async fn edit(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
    Json(request): Json<EditRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let send_at = request
        .send_at
        .map(|at| send_time(&messages, Some(at), Timestamp::now()))
        .transpose()?;
    let author = caller.user;
    let edited = state
        .database()
        .write(move |tx| -> Result<ScheduledMessageDto, Stop> {
            let Some(mut scheduled) = store::find(tx, author, id)? else {
                return Err(Stop::not_found());
            };
            if scheduled.failure.is_some() {
                return Err(conflict(&messages, MessageKey::ScheduledFailedLocked, &[], FAILED).into());
            }
            if request.content.is_none() && send_at.is_none() {
                return reread(tx, author, id);
            }
            if let Some(content) = request.content.as_deref() {
                scheduled.content = post::content_of(&messages, Some(content), &scheduled.attachments, false)?;
            }
            if let Some(at) = send_at {
                scheduled.send_at = at;
            }
            check(tx, &messages, &scheduled)?;
            store::edit(tx, &scheduled)?;
            reread(tx, author, id)
        })
        .await;
    Ok(answer(&state, author, edited, &messages, StatusCode::OK))
}

/// Removes a message, pending or failed, with the files that nothing else
/// holds: they were uploaded for it.
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let author = caller.user;
    let removed = state
        .database()
        .write(move |tx| -> Result<Option<Vec<Id>>, DbError> {
            let Some(scheduled) = store::find(tx, author, id)? else {
                return Ok(None);
            };
            store::remove(tx, id)?;
            Ok(Some(scheduled.attachments))
        })
        .await?;
    let Some(attachments) = removed else {
        return Err(ApiError::not_found());
    };
    announce_removed(&state, author, id, false);
    for attachment in attachments {
        crate::uploads::retire_if_unreferenced(
            &state,
            Some(format!("/assets/{attachment}")),
            Some(author),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a message now, pending or failed, and answers as the post route
/// does. A refusal leaves the message as it was.
pub async fn send_now(
    State(state): State<AppState>,
    caller: Caller,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let author = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok(match store::find(db, author, id)? {
                Some(scheduled) => Ok(scheduled),
                // Sent before this asked, by the worker or another tab.
                None => Err(send::already_sent(db, &messages, author, id)?),
            })
        })
        .await?;
    let scheduled = match found {
        Ok(scheduled) => scheduled,
        Err(Some(message)) => return Ok(message.respond(StatusCode::OK)),
        Err(None) => return Err(ApiError::not_found()),
    };
    Ok(match send::attempt(&state, &scheduled).await {
        Attempt::Sent(message) => message.respond(StatusCode::CREATED),
        Attempt::Replayed(message) => message.respond(StatusCode::OK),
        Attempt::Refused(stop) => stop.into_response(&messages),
    })
}

/// Checks a new message where it is going, and writes it.
pub(super) fn make(
    tx: &rusqlite::Transaction,
    messages: &Messages,
    scheduled: &Scheduled,
) -> Result<ScheduledMessageDto, Stop> {
    check(tx, messages, scheduled)?;
    if store::pending_of(tx, scheduled.author)? >= MAX_PENDING {
        return Err(conflict(messages, MessageKey::ScheduledLimit, &[("max", &MAX_PENDING)], LIMIT).into());
    }
    store::insert(tx, scheduled)?;
    reread(tx, scheduled.author, scheduled.id)
}

/// Whether the message could be posted where it goes, as the post path would
/// decide it now, slow mode aside. An encrypted conversation takes no
/// scheduled message, since the server cannot encrypt one for its author.
fn check(tx: &rusqlite::Connection, messages: &Messages, scheduled: &Scheduled) -> Result<(), Stop> {
    match scheduled.target {
        Target::Channel { server, channel } => post::admissible(
            tx,
            messages,
            server,
            &Submission {
                channel,
                author: scheduled.author,
                content: scheduled.content.clone(),
                attachments: scheduled.attachments.clone(),
                reply_to: scheduled.reply_to,
                client_id: None,
                scheduled: None,
                poll: None,
                event: None,
            },
        ),
        Target::Conversation(conversation) => {
            let peer = direct::reach(tx, conversation, scheduled.author)?;
            let found = conversations::find(tx, conversation)?.ok_or_else(Stop::not_found)?;
            if found.encryption_state == "On" {
                return Err(conflict(messages, MessageKey::ScheduledEncrypted, &[], ENCRYPTED).into());
            }
            let submission = direct::Submission::plain(
                conversation,
                scheduled.author,
                scheduled.content.clone(),
                scheduled.attachments.clone(),
                None,
                None,
            );
            direct::admit(tx, messages, &submission, peer)?;
            Ok(())
        }
    }
}

fn reread(tx: &rusqlite::Connection, author: Id, id: Id) -> Result<ScheduledMessageDto, Stop> {
    store::dto(tx, author, id)?
        .ok_or_else(|| ApiError::internal("a scheduled message was not found again").into())
}

/// The answer to a write: the message, which the author's other tabs are told
/// of and the worker looks at again, or the refusal.
fn answer(
    state: &AppState,
    author: Id,
    outcome: Result<ScheduledMessageDto, Stop>,
    messages: &Messages,
    status: StatusCode,
) -> Response {
    match outcome {
        Ok(message) => {
            announce_changed(state, author, &message);
            state.scheduled_alarm().ring();
            json_response(status, &message)
        }
        Err(stop) => stop.into_response(messages),
    }
}

/// Refuses a poll in a message to be scheduled: polls are posted when they
/// start, and are not scheduled.
fn no_poll(messages: &Messages, poll: Option<&serde_json::Value>) -> Result<(), ApiError> {
    match poll {
        None => Ok(()),
        Some(_) => Err(field_problem("poll", messages, MessageKey::PollNotScheduled, &[])),
    }
}

/// The time a message is for, refused when there is none, or when it is less
/// than a minute or more than thirty days away.
fn send_time(messages: &Messages, send_at: Option<Timestamp>, now: Timestamp) -> Result<Timestamp, ApiError> {
    let latest = now.plus(Duration::from_secs(MAX_LEAD_DAYS * 86_400));
    match send_at {
        Some(at) if at >= now.plus(MIN_LEAD) && at <= latest => Ok(at),
        _ => Err(field_problem(
            "sendAt",
            messages,
            MessageKey::ScheduledSendAtRange,
            &[("days", &MAX_LEAD_DAYS)],
        )),
    }
}

/// A refusal of the state of things, with its title in the instance's
/// language and its code last.
fn conflict(messages: &Messages, title: MessageKey, values: &[(&str, Value<'_>)], code: &str) -> ApiError {
    match messages.get(title, values) {
        Ok(text) => Problem::new(StatusCode::CONFLICT).title(text).with("code", code).into(),
        Err(error) => error.into(),
    }
}
