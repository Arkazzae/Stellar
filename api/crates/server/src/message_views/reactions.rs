//! Reacting to a message with an emoji, and taking the reaction back.
//!
//! Doing either twice is not an error: the answer and the event say how
//! things stand now, so a client that lost a response can simply ask again.

use axum::extract::State;
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::binding::Guid;
use super::text;
use crate::auth::Caller;
use crate::database::DbError;
use crate::events::Event;
use crate::http::{ApiError, ApiResult, Json, Path, Problem};
use crate::permissions::authorization::{Held, Refusal, Scope, held_at};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::ADD_REACTIONS;
use crate::permissions::visibility;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The longest emoji, in UTF-16 units. A flag or a family is several.
const MAX_EMOJI_UNITS: usize = 64;

/// What a reaction's change is told as, to the one who made it and to
/// everybody who sees the channel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionChanged {
    pub message_id: Id,
    pub channel_id: Id,
    pub emoji: String,
    /// The total for this emoji after the change.
    pub count: i64,
    pub user_id: Id,
    /// False when the reaction was taken back.
    pub added: bool,
}

pub async fn add(
    Path((Guid(message), emoji)): Path<(Guid, String)>,
    caller: Caller,
    server: ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<ReactionChanged>> {
    set(state, message, emoji, caller, server, true).await
}

pub async fn remove(
    Path((Guid(message), emoji)): Path<(Guid, String)>,
    caller: Caller,
    server: ActiveServer,
    State(state): State<AppState>,
) -> ApiResult<Json<ReactionChanged>> {
    set(state, message, emoji, caller, server, false).await
}

/// What the checks in front of a change found.
enum Check {
    /// No such message on this server.
    Missing,
    Refused(Refusal),
    Allowed(Id),
}

async fn set(
    state: AppState,
    message: Id,
    emoji: String,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    added: bool,
) -> ApiResult<Json<ReactionChanged>> {
    let emoji = text::trim(&emoji).to_string();
    let length = text::units(&emoji);
    if !(1..=MAX_EMOJI_UNITS).contains(&length) || emoji.chars().any(text::is_white_space) {
        let reason = state
            .messages()
            .get(MessageKey::ReactionInvalid, &[("max", &MAX_EMOJI_UNITS)])?;
        return Err(Problem::validation(vec![("emoji".to_string(), vec![reason])]).into());
    }

    let user = caller.user;
    // Adding is admitted in the message's channel; taking one's own reaction
    // back needs no permission, so that losing the right to react leaves no
    // reaction stuck. Either needs the channel to be seen: one that is not is
    // answered as one that does not exist, since the answer says how many
    // reacted.
    let checked = state
        .database()
        .read(move |db| -> Result<Check, DbError> {
            let channel: Option<Id> = optional(db.query_row(
                "SELECT m.\"ChannelId\" FROM \"Messages\" m
                 JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\"
                 WHERE m.\"Id\" = ?1 AND c.\"ServerId\" = ?2",
                rusqlite::params![message, server],
                |row| row.get(0),
            ))?;
            let Some(channel) = channel else {
                return Ok(Check::Missing);
            };
            let verdict = if added {
                held_at(db, user, &Scope::channel(server, channel))?.require(ADD_REACTIONS)
            } else if visibility::views(db, user, channel)? {
                Ok(Held(0))
            } else {
                Err(Refusal::Hidden)
            };
            Ok(match verdict {
                Ok(_) => Check::Allowed(channel),
                Err(refusal) => Check::Refused(refusal),
            })
        })
        .await?;
    let channel = match checked {
        Check::Missing => return Err(ApiError::not_found()),
        Check::Refused(refusal) => {
            return Err(refusal.into_error(
                state.messages(),
                MessageKey::AddReactionsDenied,
                ApiError::not_found(),
            )?);
        }
        Check::Allowed(channel) => channel,
    };

    let reacted = emoji.clone();
    let count = state
        .database()
        .write(move |tx| -> Result<i64, DbError> {
            if added {
                // The key is the whole reaction, so asking twice, or twice at
                // once, leaves one row without either request failing.
                tx.execute(
                    "INSERT INTO \"MessageReactions\" (\"MessageId\", \"UserId\", \"Emoji\", \"CreatedAt\")
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT DO NOTHING",
                    rusqlite::params![message, user, reacted, Timestamp::now()],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM \"MessageReactions\" WHERE \"MessageId\" = ?1 AND \"UserId\" = ?2 AND \"Emoji\" = ?3",
                    rusqlite::params![message, user, reacted],
                )?;
            }
            Ok(tx.query_row(
                "SELECT COUNT(*) FROM \"MessageReactions\" WHERE \"MessageId\" = ?1 AND \"Emoji\" = ?2",
                rusqlite::params![message, reacted],
                |row| row.get(0),
            )?)
        })
        .await?;

    let changed = ReactionChanged {
        message_id: message,
        channel_id: channel,
        emoji,
        count,
        user_id: user,
        added,
    };
    let audience = state
        .database()
        .read(move |db| -> Result<_, DbError> { Ok(visibility::audience_of(db, server, channel)?) })
        .await?;
    state
        .events()
        .publish(Event::new(audience, "MessageReactionChanged", &changed));
    Ok(Json(changed))
}

fn optional<T>(found: rusqlite::Result<T>) -> rusqlite::Result<Option<T>> {
    match found {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error),
    }
}
