//! Taking people out of a voice room when they may no longer sit in it.
//!
//! A seat is given where the person may view and connect, but what they may do
//! changes while they sit: a role is taken from them, an override is written, the
//! channel is made private or moved to another section, the channel is
//! deleted, they are removed from the server, the server is deleted. Whoever
//! made such a change calls [`enforce`] once it is committed (or the two steps
//! of it, [`unseat`] and [`Unseated::announce`], where the people must be told
//! later than the seats are freed), and the people who sit where they may no
//! longer view or connect are taken out: the seat and the media session are
//! gone, the room is told who is left, and the person receives
//! `VoiceChannelKicked` with the reason `access-revoked`.
//!
//! What a person may do is asked of the permissions, once, here: what they hold
//! at the channel they sit in ([`held_at`]). There is no second copy of the
//! rules. A closed connection frees its seat by itself, so a disabled account or
//! a session that ended needs nothing from here.
//!
//! A person a moderator moved into a room they could not have joined holds that
//! seat until the next change of access on the server, which takes them out of
//! it: the move is no loss of access, but the rules they sit under are the ones
//! that count from then on.

use std::collections::HashSet;

use rusqlite::{Connection, params};
use serde::Serialize;

use super::hub::send_to_connections;
use super::rooms::broadcast_snapshot;
use crate::database::DbError;
use crate::permissions::authorization::{Scope, held_at};
use crate::permissions::privileges::CONNECT;
use crate::permissions::visibility::ChannelKind;
use crate::state::AppState;
use crate::values::Id;

/// The voice channels a server has. Read them in the transaction that deletes
/// the server or one of them, before it does, and hand them to [`unseat`]:
/// once the rows are gone nothing says which server a seat was in.
pub fn voice_channels_of(db: &Connection, server: Id) -> rusqlite::Result<Vec<Id>> {
    let mut statement =
        db.prepare("SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = ?2")?;
    let rows = statement.query_map(params![server, ChannelKind::Voice], |row| row.get(0))?;
    rows.collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelKicked {
    channel_id: Id,
    reason: &'static str,
}

/// The seats that were taken, and who is to be told.
#[derive(Debug, Default)]
pub struct Unseated {
    /// Each person, the room they sat in and the connections that sat there.
    seats: Vec<(Id, Id, Vec<String>)>,
}

impl Unseated {
    pub fn is_empty(&self) -> bool {
        self.seats.is_empty()
    }

    /// Tells each person they were taken out, and each room who is left.
    pub async fn announce(self, state: &AppState) {
        let mut rooms: Vec<Id> = Vec::new();
        for (_, room, connections) in &self.seats {
            send_to_connections(
                state,
                connections.iter().cloned(),
                "VoiceChannelKicked",
                &ChannelKicked {
                    channel_id: *room,
                    reason: "access-revoked",
                },
            );
            if !rooms.contains(room) {
                rooms.push(*room);
            }
        }
        for room in rooms {
            state.activities().sync_voice_channel(state, room).await;
            if let Err(error) = broadcast_snapshot(state, room).await {
                tracing::warn!(%error, "a room was not told that somebody was taken out of it");
            }
        }
    }
}

/// Frees the seat of everybody who sits in a voice channel of `server` and may
/// no longer view and connect there. `rooms` names the channels to look at when
/// they are known (the ones a deletion takes); without it, the server's voice
/// channels are read now. The seats and their media sessions are gone when it
/// returns; the people and the rooms are told by [`Unseated::announce`].
pub async fn unseat(state: &AppState, server: Id, rooms: Option<Vec<Id>>) -> Unseated {
    let Some(realtime) = state.realtime() else {
        return Unseated::default();
    };
    let presence = realtime.presence();
    let active: HashSet<Id> = presence.active_voice_rooms().into_iter().collect();
    if active.is_empty() {
        return Unseated::default();
    }

    let held = realtime.clone();
    let found = state
        .database()
        .read(move |db| -> Result<Vec<(Id, Id)>, DbError> {
            let candidates = match rooms {
                Some(rooms) => rooms,
                None => voice_channels_of(db, server)?,
            };
            let mut without = Vec::new();
            for room in candidates.into_iter().filter(|room| active.contains(room)) {
                for seated in held.presence().voice_users(room) {
                    let held = held_at(db, seated.user, &Scope::channel(server, room))?;
                    if !(held.views() && held.has(CONNECT)) {
                        without.push((seated.user, room));
                    }
                }
            }
            Ok(without)
        })
        .await;
    let found = match found {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(%error, "who sits where they may no longer sit was not read");
            return Unseated::default();
        }
    };

    let mut unseated = Unseated::default();
    for (user, room) in found {
        let connections = presence.voice_connections_of(user, room);
        presence.remove_user_from_voice_channels(user, &HashSet::from([room]));
        if !connections.is_empty() {
            unseated.seats.push((user, room, connections));
        }
    }
    unseated
}

/// [`unseat`] and [`Unseated::announce`] in one: what a change of access calls
/// once it is committed.
pub async fn enforce(state: &AppState, server: Id) {
    unseat(state, server, None).await.announce(state).await;
}
