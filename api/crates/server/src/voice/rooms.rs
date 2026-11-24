//! Who may use a voice room, and telling everybody who should know who is in
//! it.
//!
//! A room id is either a voice channel of a server the caller belongs to or a
//! direct conversation between two people. A direct call is refused when either
//! person blocked the other. Otherwise it is open to a caller whose peer is
//! already seated, to one whose peer takes direct messages from anybody, and
//! to friends.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::permissions::authorization::{Held, Scope, held_at};
use crate::permissions::privileges;
use crate::permissions::visibility::{self, ChannelKind};
use crate::realtime::presence::PresenceTracker;
use crate::state::AppState;
use crate::values::Id;

/// Whether the caller may use the room at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub allowed: bool,
    /// How many people the room takes; 0 for no limit.
    pub user_limit: i64,
    /// The server of a voice channel; none for a direct call.
    pub server: Option<Id>,
}

const NO_ACCESS: Access = Access {
    allowed: false,
    user_limit: 0,
    server: None,
};

/// Whether `user` may use `room`.
pub fn access(
    db: &Connection,
    presence: &PresenceTracker,
    room: Id,
    user: Id,
) -> rusqlite::Result<Access> {
    let channel: Option<(i64, Id)> = db
        .query_row(
            "SELECT c.\"UserLimit\", c.\"ServerId\" FROM \"Channels\" c
             WHERE c.\"Id\" = ?1 AND c.\"Kind\" = ?2
               AND EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = c.\"ServerId\" AND m.\"UserId\" = ?3)",
            params![room, ChannelKind::Voice, user],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((user_limit, server)) = channel {
        return Ok(Access {
            allowed: true,
            user_limit,
            server: Some(server),
        });
    }

    let conversation: Option<(Id, Id)> = db
        .query_row(
            "SELECT \"UserAId\", \"UserBId\" FROM \"DirectConversations\"
             WHERE \"Id\" = ?1 AND (\"UserAId\" = ?2 OR \"UserBId\" = ?2) LIMIT 1",
            params![room, user],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match conversation {
        Some((a, b)) if direct_call_allowed(db, presence, room, user, if a == user { b } else { a })? => {
            Ok(Access {
                allowed: true,
                user_limit: 0,
                server: None,
            })
        }
        _ => Ok(NO_ACCESS),
    }
}

/// What `user` holds in the room: everything in a direct call, and in a
/// channel what the channel's overrides leave them.
pub fn held_in(db: &Connection, access: &Access, room: Id, user: Id) -> rusqlite::Result<Held> {
    match access.server {
        Some(server) => held_at(db, user, &Scope::channel(server, room)),
        None => Ok(Held(privileges::ALL)),
    }
}

fn direct_call_allowed(
    db: &Connection,
    presence: &PresenceTracker,
    conversation: Id,
    user: Id,
    peer: Id,
) -> rusqlite::Result<bool> {
    let blocked: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"UserBlocks\"
            WHERE (\"BlockerId\" = ?1 AND \"BlockedId\" = ?2) OR (\"BlockerId\" = ?2 AND \"BlockedId\" = ?1))",
        params![user, peer],
        |row| row.get(0),
    )?;
    if blocked {
        return Ok(false);
    }

    // Answering a call that is already ringing must not depend on the
    // answerer's own direct-message preference.
    if presence
        .voice_users(conversation)
        .iter()
        .any(|seated| seated.user == peer)
    {
        return Ok(true);
    }

    let takes_messages: Option<bool> = db
        .query_row(
            "SELECT \"AllowDirectMessages\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
            [peer],
            |row| row.get(0),
        )
        .optional()?;
    if takes_messages.unwrap_or(true) {
        return Ok(true);
    }

    let (first, second) = if user < peer { (user, peer) } else { (peer, user) };
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"Friendships\" WHERE \"UserAId\" = ?1 AND \"UserBId\" = ?2)",
        params![first, second],
        |row| row.get(0),
    )
}

/// The two people of a direct conversation, when `user` is one of them.
pub fn conversation_of(
    db: &Connection,
    conversation: Id,
    user: Id,
) -> rusqlite::Result<Option<(Id, Id)>> {
    db.query_row(
        "SELECT \"UserAId\", \"UserBId\" FROM \"DirectConversations\"
         WHERE \"Id\" = ?1 AND (\"UserAId\" = ?2 OR \"UserBId\" = ?2) LIMIT 1",
        params![conversation, user],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Occupant {
    user_id: Id,
    display_name: String,
    muted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Occupancy {
    channel_id: Id,
    sequence: i64,
    participants: Vec<Occupant>,
}

/// Where a room's occupancy goes, worked out from the database.
enum Reach {
    Conversation(Id, Id),
    Channel(Id, Option<Vec<Id>>),
    Nobody,
}

/// Tells everybody who should see a room its current occupants. The hub and
/// the routes that change who sits where share it.
///
/// A server's voice channel shows who is talking to everybody who sees it,
/// which is how people know before they join, and to whoever sits in it, who
/// may have been moved there without seeing it. A direct call is as private as
/// its transcript and only reaches its two owners.
pub async fn broadcast_snapshot(state: &AppState, room: Id) -> Result<(), DbError> {
    let Some(realtime) = state.realtime() else {
        return Ok(());
    };
    // Taken before the lookups below, which finish in no fixed order: the
    // sequence has to follow the order in which seats changed.
    let snapshot = realtime.presence().voice_snapshot(room);
    let seated: Vec<Id> = snapshot.participants.iter().map(|p| p.user).collect();

    let looked_up = seated.clone();
    let (names, reach) = state
        .database()
        .read(move |db| -> Result<(HashMap<Id, String>, Reach), DbError> {
            let mut names = HashMap::new();
            for user in &looked_up {
                let name: Option<String> = db
                    .query_row(
                        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                        [user],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(name) = name {
                    names.insert(*user, name);
                }
            }

            let conversation: Option<(Id, Id)> = db
                .query_row(
                    "SELECT \"UserAId\", \"UserBId\" FROM \"DirectConversations\" WHERE \"Id\" = ?1",
                    [room],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((a, b)) = conversation {
                return Ok((names, Reach::Conversation(a, b)));
            }

            let server: Option<Id> = db
                .query_row(
                    "SELECT \"ServerId\" FROM \"Channels\" WHERE \"Id\" = ?1",
                    [room],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(server) = server else {
                return Ok((names, Reach::Nobody));
            };
            let viewers = visibility::viewers(db, server, room)?.map(|seeing| {
                let mut people: Vec<Id> = seeing.into_iter().collect();
                people.sort();
                people
            });
            Ok((names, Reach::Channel(server, viewers)))
        })
        .await?;

    let occupancy = Occupancy {
        channel_id: room,
        sequence: snapshot.sequence,
        participants: snapshot
            .participants
            .iter()
            .filter_map(|participant| {
                names.get(&participant.user).map(|name| Occupant {
                    user_id: participant.user,
                    display_name: name.clone(),
                    muted: participant.muted,
                })
            })
            .collect(),
    };
    let audience = match reach {
        Reach::Conversation(a, b) => Audience::Users(vec![a, b]),
        Reach::Channel(server, None) => Audience::Server(server),
        Reach::Channel(server, Some(mut people)) => {
            for person in seated {
                if !people.contains(&person) {
                    people.push(person);
                }
            }
            Audience::Followers {
                server,
                users: people,
            }
        }
        Reach::Nobody => return Ok(()),
    };
    state
        .events()
        .publish(Event::new(audience, "VoiceParticipantChanged", &occupancy));
    Ok(())
}

/// A caller that was offline, or reconnecting, while a call was started needs
/// to hear about it. Only direct rooms are restored: the bootstrap already
/// carries the occupancy of voice channels.
pub async fn restore_direct_calls(state: &AppState, user: Id) -> Result<(), DbError> {
    let Some(realtime) = state.realtime() else {
        return Ok(());
    };
    let active = realtime.presence().active_voice_rooms();
    if active.is_empty() {
        return Ok(());
    }
    let own: Vec<Id> = state
        .database()
        .read(move |db| -> Result<Vec<Id>, DbError> {
            let mut own = Vec::new();
            for room in active {
                if conversation_of(db, room, user)?.is_some() {
                    own.push(room);
                }
            }
            Ok(own)
        })
        .await?;
    for room in own {
        broadcast_snapshot(state, room).await?;
    }
    Ok(())
}
