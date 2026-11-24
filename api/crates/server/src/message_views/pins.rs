//! Keeping a few messages above a channel's conversation.

use axum::extract::State;
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::binding::Guid;
use super::wire::{self, MessageDto, Words};
use crate::auth::Caller;
use crate::database::DbError;
use crate::events::Event;
use crate::http::{ApiError, ApiResult, Json, Path, Problem};
use crate::permissions::authorization::{Refusal, Scope, held_at};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_MESSAGES;
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// A ceiling on pins, so that the pinned list stays a short list of things
/// worth keeping rather than a second transcript nobody reads.
const MAX_PINS_PER_CHANNEL: i64 = 50;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessages {
    messages: Vec<MessageDto>,
}

/// The attachment word every message needs for its quote.
pub fn words(state: &AppState) -> ApiResult<Words> {
    Ok(Words {
        attachment: state.messages().get(MessageKey::AttachmentPreview, &[])?,
    })
}

/// The channel's pinned messages, newest pin first. A read of its own rather
/// than a flag the transcript carries: a pin is usually on a message far above
/// the last page, and a list made of whatever the client has loaded would grow
/// as the reader scrolls.
pub async fn list(
    Path(Guid(channel)): Path<Guid>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<PinnedMessages>> {
    let words = words(&state)?;
    let user = caller.user;
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<MessageDto>>, DbError> {
            // A channel the caller does not view is one that does not exist.
            if visibility::find(db, user, server, channel, Some(ChannelKind::Text))?.is_none() {
                return Ok(None);
            }
            let mut statement = db.prepare(
                "SELECT \"Id\" FROM \"Messages\" WHERE \"ChannelId\" = ?1 AND \"PinnedAt\" IS NOT NULL
                 ORDER BY \"PinnedAt\" DESC LIMIT ?2",
            )?;
            let ids = statement
                .query_map(rusqlite::params![channel, MAX_PINS_PER_CHANNEL], |row| {
                    row.get::<_, Id>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some(wire::load(db, &ids, Some(user), &words)?))
        })
        .await?;
    found
        .map(|messages| Json(PinnedMessages { messages }))
        .ok_or_else(ApiError::not_found)
}

pub async fn pin(
    Path(Guid(message)): Path<Guid>,
    caller: Caller,
    server: ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<MessageDto>> {
    set(state, message, caller, server, true).await
}

pub async fn unpin(
    Path(Guid(message)): Path<Guid>,
    caller: Caller,
    server: ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<MessageDto>> {
    set(state, message, caller, server, false).await
}

enum Outcome {
    Missing,
    Refused(Refusal),
    /// The pinned list is as long as it may be.
    Full,
    Done {
        channel: Id,
        answer: Box<MessageDto>,
        announced: Box<MessageDto>,
    },
}

/// Pins or unpins a message, for whoever can manage messages in its channel.
/// A pin puts one message above the conversation for everybody who opens the
/// channel, so it is moderation and not the author's right, as editing is.
/// Doing either twice is not an error, and pinning a pinned message leaves its
/// pin where it is instead of moving it to the top of the list.
async fn set(
    state: AppState,
    message: Id,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    pinned: bool,
) -> ApiResult<Json<MessageDto>> {
    let words = words(&state)?;
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, DbError> {
            let found: Option<(Id, bool)> = match tx.query_row(
                "SELECT m.\"ChannelId\", m.\"PinnedAt\" IS NOT NULL FROM \"Messages\" m
                 JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\"
                 WHERE m.\"Id\" = ?1 AND c.\"ServerId\" = ?2",
                rusqlite::params![message, server],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ) {
                Ok(row) => Some(row),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(error) => return Err(error.into()),
            };
            let Some((channel, is_pinned)) = found else {
                return Ok(Outcome::Missing);
            };

            // A refusal that is a 403 and not the 404 deleting uses: everybody
            // who can read the channel already sees this message, so there is
            // nothing to conceal, only a reason to give, and it is the true
            // one. Somebody who cannot read it is answered as if it did not
            // exist.
            let held = held_at(tx, user, &Scope::channel(server, channel))?;
            if !held.views() {
                return Ok(Outcome::Refused(Refusal::Hidden));
            }
            if !held.has(MANAGE_MESSAGES) {
                return Ok(Outcome::Refused(Refusal::Forbidden));
            }

            if pinned != is_pinned {
                if pinned {
                    let count: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM \"Messages\" WHERE \"ChannelId\" = ?1 AND \"PinnedAt\" IS NOT NULL",
                        [channel],
                        |row| row.get(0),
                    )?;
                    if count >= MAX_PINS_PER_CHANNEL {
                        return Ok(Outcome::Full);
                    }
                    tx.execute(
                        "UPDATE \"Messages\" SET \"PinnedAt\" = ?2 WHERE \"Id\" = ?1",
                        rusqlite::params![message, Timestamp::now()],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE \"Messages\" SET \"PinnedAt\" = NULL WHERE \"Id\" = ?1",
                        [message],
                    )?;
                }
            }

            // The event is shared by everyone who sees the channel, so it
            // cannot say which reactions are the viewer's own.
            let answer = wire::load(tx, &[message], Some(user), &words)?.pop();
            let announced = wire::load(tx, &[message], None, &words)?.pop();
            match (answer, announced) {
                (Some(answer), Some(announced)) => Ok(Outcome::Done {
                    channel,
                    answer: Box::new(answer),
                    announced: Box::new(announced),
                }),
                _ => Ok(Outcome::Missing),
            }
        })
        .await?;

    match outcome {
        Outcome::Missing => Err(ApiError::not_found()),
        Outcome::Refused(Refusal::Forbidden) => Err(ApiError::forbidden()),
        Outcome::Refused(_) => Err(ApiError::not_found()),
        Outcome::Full => {
            let reason = state
                .messages()
                .get(MessageKey::PinLimitReached, &[("max", &MAX_PINS_PER_CHANNEL)])?;
            Err(Problem::validation(vec![("pins".to_string(), vec![reason])]).into())
        }
        Outcome::Done {
            channel,
            answer,
            announced,
        } => {
            let audience = state
                .database()
                .read(move |db| -> Result<_, DbError> {
                    Ok(visibility::audience_of(db, server, channel)?)
                })
                .await?;
            state
                .events()
                .publish(Event::new(audience, "MessageUpdated", &*announced));
            Ok(Json(*answer))
        }
    }
}
