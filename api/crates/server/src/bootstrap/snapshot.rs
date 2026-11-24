//! Reading everything a bootstrap holds in one snapshot of the database.
//!
//! Cost: a fixed number of statements however many channels, members or
//! messages the server has, but for the permissions and the view, which are
//! the permission model's own (a server where nobody is denied view
//! evaluates nothing). The test of the statements is in
//! `tests/api/bootstrap.rs`.

use rusqlite::{Connection, params};
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use stellarr_messages::Messages;

use super::voice;
use crate::channels::{ChannelDto, SectionDto, channels_of, sections_of};
use crate::database::DbError;
use crate::events::Presence;
use crate::http::ApiError;
use crate::instance::servers::{Server, ServerDto};
use crate::instance::users::{User, UserDto};
use crate::messages::dto::{self, MessageDto};
use crate::realtime::presence::PresenceTracker;
use crate::permissions::{authorization, privileges, visibility};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// How many messages of the first text channel come with the bootstrap.
const FIRST_PAGE_SIZE: i64 = 100;

/// What reading needs besides the database.
pub struct Context<'a> {
    pub messages: &'a Messages,
    pub presence: &'a dyn Presence,
    /// Who sits in the voice rooms; none when the server runs without the hub.
    pub seats: Option<&'a PresenceTracker>,
}

/// The markers of the read channels, as an object in the order they come.
struct Markers(Vec<(Id, Timestamp)>);

impl Serialize for Markers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (channel, at) in &self.0 {
            map.serialize_entry(channel, at)?;
        }
        map.end()
    }
}

/// The people sitting in each voice room, by room.
struct Rooms(Vec<(Id, Vec<Occupant>)>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Occupant {
    user_id: Id,
    display_name: String,
    muted: bool,
}

impl Serialize for Rooms {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (channel, occupants) in &self.0 {
            map.serialize_entry(channel, occupants)?;
        }
        map.end()
    }
}

/// The answer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    user: UserDto,
    server: ServerDto,
    servers: Vec<ServerDto>,
    channels: Vec<ChannelDto>,
    sections: Vec<SectionDto>,
    members: Vec<UserDto>,
    messages: Vec<MessageDto>,
    channel_reads: Markers,
    voice: Rooms,
    rtc: voice::Rtc,
    permissions: Vec<&'static str>,
}

/// What was read, and what is still to do before it is an answer.
pub struct Read {
    user: Id,
    server: Server,
    person: UserDto,
    servers: Vec<ServerDto>,
    channels: Vec<ChannelDto>,
    sections: Vec<SectionDto>,
    members: Vec<UserDto>,
    messages: Vec<MessageDto>,
    markers: Vec<(Id, Timestamp)>,
    /// The text channels the person sees and has never opened.
    pub missing: Vec<Id>,
    rooms: Vec<(Id, Vec<Occupant>)>,
    permissions: Vec<&'static str>,
}

fn broken(error: impl std::fmt::Display) -> ApiError {
    ApiError::internal(error)
}

pub fn read(
    db: &Connection,
    context: &Context<'_>,
    user: Id,
    server: Id,
) -> Result<Read, ApiError> {
    let person = User::find(db, user)?.ok_or_else(|| broken("the signed-in account is not there"))?;
    let home = Server::find(db, server)?.ok_or_else(|| broken("the server is not there"))?;
    let servers = {
        let mut statement = db.prepare(&format!(
            "SELECT {} FROM \"ServerMembers\" AS m INNER JOIN \"Servers\" AS s ON m.\"ServerId\" = s.\"Id\"
             WHERE m.\"UserId\" = ?1 ORDER BY m.\"JoinedAt\"",
            crate::instance::servers::COLUMNS
                .split(", ")
                .map(|column| format!("s.{column}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        statement
            .query_map([user], Server::from_row)?
            .map(|row| row.map(|server| server.dto()))
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut channels = channels_of(db, server)?;
    let mut sections = sections_of(db, server)?;
    let members = members_of(db, server)?;
    let permissions = privileges::to_names(authorization::server_wide(db, user, server)?);

    // Only what the caller sees, and from it everything else below: the read
    // markers, the first page and the voice rooms. On a server where nobody is
    // denied view that is everything, with nothing evaluated.
    if let Some(view) = visibility::of_member(db, user, server)? {
        channels.retain(|channel| view.channels.contains(&channel.id));
        sections.retain(|section| view.sections.contains(&section.id));
    }

    let text: Vec<Id> = channels.iter().filter(|c| c.is_text()).map(|c| c.id).collect();
    let (markers, missing) = markers_of(db, user, &text)?;
    let messages = match text.first() {
        Some(channel) => first_page(db, context.messages, *channel, user)?,
        None => Vec::new(),
    };

    let names: std::collections::HashMap<Id, &str> = members
        .iter()
        .map(|(id, member)| (*id, member.display_name.as_str()))
        .collect();
    let rooms = channels
        .iter()
        .filter(|channel| !channel.is_text())
        .map(|channel| {
            let occupants = voice::seats(context.seats, channel.id)
                .into_iter()
                .filter_map(|seat| {
                    names.get(&seat.user).map(|name| Occupant {
                        user_id: seat.user,
                        display_name: (*name).to_string(),
                        muted: seat.muted,
                    })
                })
                .collect();
            (channel.id, occupants)
        })
        .collect();

    Ok(Read {
        user,
        server: home,
        person: person.dto(&context.presence.presence_of(user)),
        servers,
        channels,
        sections,
        members: members
            .iter()
            .map(|(id, member)| member.dto(&context.presence.public_presence_of(*id, Some(user))))
            .collect(),
        messages,
        markers,
        missing,
        rooms,
        permissions,
    })
}

/// The members of a server by display name, as stored.
fn members_of(db: &Connection, server: Id) -> rusqlite::Result<Vec<(Id, User)>> {
    let mut statement = db.prepare(&format!(
        "SELECT {} FROM \"ServerMembers\" AS m INNER JOIN \"Users\" AS u ON m.\"UserId\" = u.\"Id\"
         WHERE m.\"ServerId\" = ?1 ORDER BY u.\"DisplayName\"",
        User::columns("u")
    ))?;
    statement
        .query_map([server], |row| User::from_row(row).map(|user| (user.id, user)))?
        .collect()
}

/// The channels with the moment they were read up to.
type Reads = Vec<(Id, Timestamp)>;

/// The markers there are for the text channels, and the channels that have
/// none.
fn markers_of(
    db: &Connection,
    user: Id,
    text: &[Id],
) -> rusqlite::Result<(Reads, Vec<Id>)> {
    if text.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let wanted = serde_json::to_string(&text.iter().map(Id::to_db_text).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string());
    let mut statement = db.prepare_cached(
        "SELECT \"ChannelId\", \"LastReadAt\" FROM \"ChannelReads\"
         WHERE \"UserId\" = ?1 AND \"ChannelId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let found: Vec<(Id, Timestamp)> = statement
        .query_map(params![user, wanted], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let missing = text
        .iter()
        .copied()
        .filter(|id| !found.iter().any(|(marked, _)| marked == id))
        .collect();
    Ok((found, missing))
}

/// Marks the channels read as of `now` where nothing is marked yet, and reads
/// what is stored for them: a marker another request stored first stands.
pub fn store_markers(
    tx: &rusqlite::Transaction<'_>,
    user: Id,
    channels: &[Id],
    now: Timestamp,
) -> Result<Vec<(Id, Timestamp)>, DbError> {
    let mut stored = Vec::with_capacity(channels.len());
    for channel in channels {
        tx.execute(
            "INSERT OR IGNORE INTO \"ChannelReads\" (\"ChannelId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)",
            params![channel, user, now],
        )?;
        let at: Timestamp = tx.query_row(
            "SELECT \"LastReadAt\" FROM \"ChannelReads\" WHERE \"ChannelId\" = ?1 AND \"UserId\" = ?2",
            params![channel, user],
            |row| row.get(0),
        )?;
        stored.push((*channel, at));
    }
    Ok(stored)
}

/// The newest hundred messages of a channel, oldest first.
fn first_page(
    db: &Connection,
    messages: &Messages,
    channel: Id,
    viewer: Id,
) -> Result<Vec<MessageDto>, DbError> {
    let mut statement = db.prepare_cached(
        "SELECT \"Id\" FROM \"Messages\" WHERE \"ChannelId\" = ?1 ORDER BY \"CreatedAt\" DESC LIMIT ?2",
    )?;
    let mut ids: Vec<Id> = statement
        .query_map(params![channel, FIRST_PAGE_SIZE], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    // Newest first as read, oldest first as answered. Messages of one moment
    // come in the reverse of the order the database listed them, which is how
    // the first server's page came out, and which of them are among the hundred
    // is the database's choice too.
    ids.reverse();
    dto::load(db, messages, &ids, Some(viewer))
}

impl Read {
    /// Adds the markers that were stored for the channels that had none.
    pub fn add_markers(&mut self, stored: Vec<(Id, Timestamp)>) {
        self.markers.extend(stored);
    }

    /// The answer, with how to take part in voice as of `now`.
    pub fn finish(self, state: &AppState, now: Timestamp) -> Result<Bootstrap, ApiError> {
        let rtc = voice::rtc(state.settings(), self.user, now);
        Ok(Bootstrap {
            user: self.person,
            server: self.server.dto(),
            servers: self.servers,
            channels: self.channels,
            sections: self.sections,
            members: self.members,
            messages: self.messages,
            channel_reads: Markers(self.markers),
            voice: Rooms(self.rooms),
            rtc,
            permissions: self.permissions,
        })
    }
}
