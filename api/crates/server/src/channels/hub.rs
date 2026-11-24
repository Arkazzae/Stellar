//! The two calls a client makes about a text channel: `JoinChannel` and
//! `SendTyping`.
//!
//! Neither joins a group. A channel's events reach whoever sees it when they
//! are sent, so a group joined here would only go stale when somebody lost
//! view.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use stellarr_hub::{HubError, Outbound};

use crate::database::DbError;
use crate::permissions::authorization::{Scope, held_at};
use crate::permissions::privileges::SEND_MESSAGES;
use crate::permissions::visibility::{self, ChannelKind};
use crate::realtime::names::{follower_group, parse_guid, server_group};
use crate::realtime::throttle::Policy;
use crate::realtime::{Context, Method, Registration, method::arguments};
use crate::values::{Id, Timestamp};

/// How long a typing notice counts for.
const TYPING_LIFETIME: Duration = Duration::from_secs(6);

pub fn registration() -> Registration {
    Registration::new()
        .method(Method::new("JoinChannel", join_channel))
        .method(Method::new("SendTyping", send_typing))
}

/// The id a call names, or the sentence for one that is not an id.
fn channel_id(argument: Value) -> Result<Id, HubError> {
    let text = arguments::text(argument)?;
    text.as_deref()
        .and_then(parse_guid)
        .ok_or_else(|| HubError::visible("Invalid channelId."))
}

fn unavailable(error: DbError) -> HubError {
    HubError::internal(error.to_string())
}

/// Confirms that the account may follow one text channel, which it may when it
/// sees it.
async fn join_channel(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    let [channel] = arguments::exactly(arguments)?;
    let channel = channel_id(channel)?;
    let user = cx.user();
    let sees = cx
        .state()
        .database()
        .read(move |db| -> Result<bool, DbError> {
            let Some(place) = visibility::place(db, channel, None, Some(ChannelKind::Text), Some(user))?
            else {
                return Ok(false);
            };
            Ok(visibility::find(db, user, place.server, channel, Some(ChannelKind::Text))?.is_some())
        })
        .await
        .map_err(unavailable)?;
    if !sees {
        return Err(HubError::visible("Unknown channel."));
    }
    Ok(Value::Null)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Typing {
    channel_id: Id,
    user_id: Id,
    display_name: String,
    expires_at: Timestamp,
}

/// What one typing notice is addressed to.
struct Notice {
    server: Id,
    typing: Typing,
    /// None when everybody on the server sees the channel.
    viewers: Option<Vec<Id>>,
}

/// Tells the others who see a text channel that this account is typing in it.
/// Typing needs what sending does: seeing the channel, which somebody who
/// does not is answered as for no channel at all, and the permission to send
/// there.
async fn send_typing(cx: Context, arguments: Vec<Value>) -> Result<Value, HubError> {
    // One notice reaches the channel's whole audience and costs a few
    // lookups, so the cadence is decided here rather than trusted to every
    // client.
    if !cx
        .realtime()
        .throttle()
        .try_acquire(cx.connection_id(), Policy::Typing)
    {
        return Err(HubError::visible("Typing is too fast."));
    }
    let [channel] = arguments::exactly(arguments)?;
    let channel = channel_id(channel)?;
    let user = cx.user();
    let notice = cx
        .state()
        .database()
        .read(move |db| -> Result<Result<Notice, &'static str>, DbError> {
            let server: Option<Id> = rusqlite::OptionalExtension::optional(db.query_row(
                "SELECT \"ServerId\" FROM \"Channels\" WHERE \"Id\" = ?1 AND \"Kind\" = ?2",
                rusqlite::params![channel, ChannelKind::Text],
                |row| row.get(0),
            ))?;
            let held = match server {
                Some(server) => held_at(db, user, &Scope::channel(server, channel))?,
                None => crate::permissions::authorization::Held(0),
            };
            let Some(server) = server.filter(|_| held.views()) else {
                return Ok(Err("Unknown channel."));
            };
            if !held.has(SEND_MESSAGES) {
                return Ok(Err("Not allowed to send messages here."));
            }
            let display_name: String = db.query_row(
                "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| row.get(0),
            )?;
            let viewers = visibility::viewers(db, server, channel)?.map(|seeing| {
                let mut users: Vec<Id> = seeing.into_iter().collect();
                users.sort();
                users
            });
            Ok(Ok(Notice {
                server,
                typing: Typing {
                    channel_id: channel,
                    user_id: user,
                    display_name,
                    expires_at: Timestamp::now().plus(TYPING_LIFETIME),
                },
                viewers,
            }))
        })
        .await
        .map_err(unavailable)?;
    let notice = notice.map_err(HubError::visible)?;
    let message = Outbound::single("TypingChanged", &notice.typing)
        .map_err(|error| HubError::internal(error.to_string()))?;
    let hub = cx.hub();
    let me = cx.connection_id();
    match notice.viewers {
        // Everybody else on the server.
        None => {
            hub.send_to_group_except(&server_group(notice.server), &[me], &message);
        }
        // Everybody else who sees it, and this account's other tabs, as the
        // server's group without this connection would have reached.
        Some(viewers) => {
            hub.send_to_groups(
                viewers
                    .iter()
                    .filter(|viewer| **viewer != user)
                    .map(|viewer| follower_group(notice.server, *viewer)),
                &message,
            );
            hub.send_to_group_except(&follower_group(notice.server, user), &[me], &message);
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typing_notice_says_its_members_in_the_order_clients_read_them() {
        let notice = Typing {
            channel_id: Id::from_bytes([1; 16]),
            user_id: Id::from_bytes([2; 16]),
            display_name: "Maya".to_string(),
            expires_at: Timestamp::from_unix_seconds(1_800_000_000).unwrap(),
        };

        assert_eq!(
            serde_json::to_string(&notice).unwrap(),
            "{\"channelId\":\"01010101-0101-0101-0101-010101010101\",\"userId\":\"02020202-0202-0202-0202-020202020202\",\"displayName\":\"Maya\",\"expiresAt\":\"2027-01-15T08:00:00Z\"}"
        );
    }
}
