//! One scheduled message sent through the post path: by the worker when its
//! time comes, and by its author who sends it now. Both go the way a post a
//! person makes goes, the batched writer for a channel included, and tell
//! the same people about it.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::Messages;
use tracing::warn;

use super::store::{self, Scheduled, Target};
use crate::database::DbError;
use crate::direct::post as direct;
use crate::http::{ApiError, json_response};
use crate::messages::dto::{self, MessageDto};
use crate::messages::post::{self, Posted, Submission};
use crate::messages::{Reason, Stop};
use crate::permissions::visibility;
use crate::social::conversations::{self, DirectMessage};
use crate::state::AppState;
use crate::values::Id;

/// The message a send posted.
pub enum Message {
    Channel(Box<MessageDto>),
    Conversation {
        conversation: Id,
        message: Box<DirectMessage>,
    },
}

impl Message {
    /// The answer the post route gives with this message, and the address of
    /// the message when it was made now.
    pub fn respond(&self, status: StatusCode) -> Response {
        let (mut response, location) = match self {
            Message::Channel(message) => (
                json_response(status, &**message),
                format!("/api/channels/{}/messages/{}", message.channel_id, message.id),
            ),
            Message::Conversation {
                conversation,
                message,
            } => (
                json_response(status, &**message),
                format!(
                    "/api/social/conversations/{conversation}/messages/{}",
                    message.id()
                ),
            ),
        };
        if status == StatusCode::CREATED
            && let Ok(location) = HeaderValue::from_str(&location)
        {
            response.headers_mut().insert(header::LOCATION, location);
        }
        response
    }
}

/// How a send ended.
pub enum Attempt {
    /// Posted now, and everyone it concerns told.
    Sent(Message),
    /// It had gone out before; it is out of the queue now as well.
    Replayed(Message),
    /// The post path refused it, and nothing was posted.
    Refused(Stop),
}

/// Sends the message as it was read. Whatever posts it takes it out of the
/// queue in the same transaction, so a second sender, or a restart, finds it
/// gone or meets the stored message under its id.
pub async fn attempt(state: &AppState, scheduled: &Scheduled) -> Attempt {
    let outcome = match scheduled.target {
        Target::Channel { channel, .. } => to_channel(state, scheduled, channel).await,
        Target::Conversation(conversation) => {
            to_conversation(state, scheduled, conversation).await
        }
    };
    match outcome {
        Ok(attempt) => {
            super::announce_removed(state, scheduled.author, scheduled.id, true);
            attempt
        }
        Err(stop) => Attempt::Refused(stop),
    }
}

/// The message a scheduled one became, for its author who sends it again
/// once it is out of the queue: a retry of a send that went through, which
/// the post routes answer with the message as stored. None when there is
/// none of theirs under that id, or they no longer see where it is.
pub fn already_sent(
    db: &Connection,
    messages: &Messages,
    author: Id,
    id: Id,
) -> Result<Option<Message>, DbError> {
    let channel: Option<Id> = db
        .query_row(
            "SELECT \"ChannelId\" FROM \"Messages\" WHERE \"Id\" = ?1 AND \"AuthorId\" = ?2 AND \"IsSystem\" = 0",
            params![id, author],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(channel) = channel {
        if !visibility::views(db, author, channel)? {
            return Ok(None);
        }
        return Ok(dto::load(db, messages, &[id], Some(author))?
            .pop()
            .map(|message| Message::Channel(Box::new(message))));
    }
    let conversation: Option<Id> = db
        .query_row(
            "SELECT \"ConversationId\" FROM \"DirectMessages\" WHERE \"Id\" = ?1 AND \"AuthorId\" = ?2 AND \"SystemKind\" IS NULL",
            params![id, author],
            |row| row.get(0),
        )
        .optional()?;
    let Some(conversation) = conversation else {
        return Ok(None);
    };
    if crate::direct::peer_in(db, conversation, author)?.is_none() {
        return Ok(None);
    }
    Ok(conversations::message(db, id)?.map(|(message, _)| Message::Conversation {
        conversation,
        message: Box::new(message),
    }))
}

async fn to_channel(state: &AppState, scheduled: &Scheduled, channel: Id) -> Result<Attempt, Stop> {
    let author = scheduled.author;
    // The server the channel is on now; a channel that is gone has none.
    let (server, may_post) = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok((store::server_of(db, channel)?, store::author_may_post(db, author)?))
        })
        .await?;
    if !may_post {
        return Err(Stop::refused(Reason::NoAccess, ApiError::forbidden()));
    }
    let Some(server) = server else {
        return Err(Stop::refused(Reason::Gone, ApiError::not_found()));
    };
    let submission = Submission {
        channel,
        author,
        content: scheduled.content.clone(),
        attachments: scheduled.attachments.clone(),
        reply_to: scheduled.reply_to,
        client_id: Some(scheduled.id),
        scheduled: Some(scheduled.claim()),
    };
    let posted = state
        .post_queue()
        .submit(state.database(), *state.messages(), server, submission)
        .await?;
    match posted {
        Posted::Stored(stored) => {
            // The message is out whatever happens to the news of it.
            if let Err(error) = post::announce(state, server, author, &stored).await {
                warn!("a scheduled message was posted but not announced: {error:?}");
            }
            Ok(Attempt::Sent(Message::Channel(Box::new(stored.message))))
        }
        Posted::Replayed(message) => Ok(Attempt::Replayed(Message::Channel(message))),
    }
}

async fn to_conversation(
    state: &AppState,
    scheduled: &Scheduled,
    conversation: Id,
) -> Result<Attempt, Stop> {
    let messages = *state.messages();
    let author = scheduled.author;
    let submission = direct::Submission::plain(
        conversation,
        author,
        scheduled.content.clone(),
        scheduled.attachments.clone(),
        Some(scheduled.id),
        Some(scheduled.claim()),
    );
    let posted = state
        .database()
        .write(move |tx| -> Result<direct::Posted, Stop> {
            if !store::author_may_post(tx, author)? {
                return Err(Stop::refused(Reason::NoAccess, ApiError::forbidden()));
            }
            let peer = direct::reach(tx, conversation, author)?;
            direct::store(tx, &messages, &submission, peer)
        })
        .await?;
    match posted {
        direct::Posted::Stored { message, peer } => {
            direct::announce(state, author, peer, &message);
            Ok(Attempt::Sent(Message::Conversation {
                conversation,
                message: Box::new(message),
            }))
        }
        direct::Posted::Replayed(message) => Ok(Attempt::Replayed(Message::Conversation {
            conversation,
            message: Box::new(message),
        })),
    }
}
