//! The rows of events, their occurrences and sign-ups; who sees an event; the
//! seats and the waitlist; and what an event looks like as JSON.

use std::collections::HashMap;

use rusqlite::{Connection, Row, params};
use serde::Serialize;

use super::model::{Repeat, State};
use crate::events::Audience;
use crate::instance::users::UserDto;
use crate::permissions::visibility;
use crate::values::{Id, Timestamp, json_ids};

/// An event as stored, with the occurrence it has now.
#[derive(Debug, Clone)]
pub struct Event {
    pub id: Id,
    pub server: Id,
    pub name: String,
    pub description: Option<String>,
    pub voice: Option<Id>,
    pub place_text: Option<String>,
    pub zone: String,
    pub first_starts_at: Timestamp,
    pub duration: Option<i64>,
    pub repeat: Repeat,
    pub seats: Option<i64>,
    pub created_by: Id,
    pub cancelled_at: Option<Timestamp>,
    pub revision: i64,
    pub occurrence: Occurrence,
    /// The first message that announces it, if one does.
    pub announcement: Option<Id>,
}

#[derive(Debug, Clone)]
pub struct Occurrence {
    pub id: Id,
    pub sequence: i64,
    pub starts_at: Timestamp,
    pub ends_at: Option<Timestamp>,
    pub live_since: Option<Timestamp>,
    pub empty_since: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    /// The state clients were last told of.
    pub phase: String,
}

impl Event {
    pub fn state(&self, now: Timestamp) -> State {
        let occurrence = &self.occurrence;
        State::at(
            self.cancelled_at.is_some(),
            occurrence.starts_at,
            occurrence.ends_at,
            occurrence.ended_at,
            now,
        )
    }
}

const COLUMNS: &str = "e.\"Id\", e.\"ServerId\", e.\"Name\", e.\"Description\", e.\"VoiceChannelId\", e.\"PlaceText\",
    e.\"TimeZone\", e.\"FirstStartsAt\", e.\"DurationSeconds\", e.\"Repeat\", e.\"Seats\", e.\"CreatedById\", e.\"CancelledAt\",
    e.\"Revision\", o.\"Id\", o.\"Sequence\", o.\"StartsAt\", o.\"EndsAt\", o.\"LiveSince\", o.\"EmptySince\", o.\"EndedAt\",
    o.\"Phase\", (SELECT MIN(a.\"MessageId\") FROM \"ServerEventAnnouncements\" a WHERE a.\"EventId\" = e.\"Id\")";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Event> {
    let repeat: String = row.get(9)?;
    Ok(Event {
        id: row.get(0)?,
        server: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        voice: row.get(4)?,
        place_text: row.get(5)?,
        zone: row.get(6)?,
        first_starts_at: row.get(7)?,
        duration: row.get(8)?,
        repeat: Repeat::parse(&repeat).unwrap_or(Repeat::None),
        seats: row.get(10)?,
        created_by: row.get(11)?,
        cancelled_at: row.get(12)?,
        revision: row.get(13)?,
        occurrence: Occurrence {
            id: row.get(14)?,
            sequence: row.get(15)?,
            starts_at: row.get(16)?,
            ends_at: row.get(17)?,
            live_since: row.get(18)?,
            empty_since: row.get(19)?,
            ended_at: row.get(20)?,
            phase: row.get(21)?,
        },
        announcement: row.get(22)?,
    })
}

fn select(
    db: &Connection,
    condition: &str,
    values: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<Event>> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM \"ServerEvents\" e JOIN \"ServerEventOccurrences\" o ON o.\"EventId\" = e.\"Id\"
         WHERE {condition} ORDER BY o.\"StartsAt\", e.\"Id\""
    ))?;
    statement.query_map(values, from_row)?.collect()
}

pub fn find(db: &Connection, id: Id) -> rusqlite::Result<Option<Event>> {
    Ok(select(db, "e.\"Id\" = ?1", &[&id])?.into_iter().next())
}

/// The condition of an occurrence that is upcoming or running at `?2`.
const ACTIVE: &str = "e.\"CancelledAt\" IS NULL AND o.\"EndedAt\" IS NULL AND (o.\"EndsAt\" IS NULL OR o.\"EndsAt\" > ?2)";

/// The server's events that are upcoming or running, the soonest first.
pub fn active_of(db: &Connection, server: Id, now: Timestamp) -> rusqlite::Result<Vec<Event>> {
    select(db, &format!("e.\"ServerId\" = ?1 AND {ACTIVE}"), &[&server, &now])
}

pub fn count_active(db: &Connection, server: Id, now: Timestamp) -> rusqlite::Result<i64> {
    db.query_row(
        &format!(
            "SELECT COUNT(*) FROM \"ServerEvents\" e JOIN \"ServerEventOccurrences\" o ON o.\"EventId\" = e.\"Id\"
             WHERE e.\"ServerId\" = ?1 AND {ACTIVE}"
        ),
        params![server, now],
        |row| row.get(0),
    )
}

/// The events the worker still has something to do for: not cancelled, and
/// not over for good.
pub fn unfinished(db: &Connection) -> rusqlite::Result<Vec<Event>> {
    select(
        db,
        "e.\"CancelledAt\" IS NULL AND (o.\"Phase\" <> 'ended' OR e.\"Repeat\" <> 'none')",
        &[],
    )
}

pub fn is_member(db: &Connection, server: Id, user: Id) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                        WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2 AND u.\"IsDisabled\" = 0)",
        params![server, user],
        |row| row.get(0),
    )
}

pub fn channel_exists(db: &Connection, channel: Id) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"Channels\" WHERE \"Id\" = ?1)",
        [channel],
        |row| row.get(0),
    )
}

/// Whether the person sees the event: a member of its server who sees its
/// voice channel, or any member for an event elsewhere. Once its voice
/// channel is gone, nobody can say who saw it, so only those who knew of it
/// do: its creator and the people signed up or waiting.
pub fn sees(db: &Connection, user: Id, event: &Event) -> rusqlite::Result<bool> {
    if !is_member(db, event.server, user)? {
        return Ok(false);
    }
    match event.voice {
        Some(channel) if channel_exists(db, channel)? => visibility::views(db, user, channel),
        Some(_) => Ok(knew_of(db, event)?.contains(&user)),
        None => Ok(true),
    }
}

/// The creator and everybody signed up or waiting.
fn knew_of(db: &Connection, event: &Event) -> rusqlite::Result<Vec<Id>> {
    let mut people = everyone(db, event.occurrence.id)?;
    if !people.contains(&event.created_by) {
        people.push(event.created_by);
    }
    Ok(people)
}

/// Who is told of the event: those who see its voice channel, the whole
/// server for an event elsewhere, and those who knew of it once its channel
/// is gone.
pub fn audience(db: &Connection, event: &Event) -> rusqlite::Result<Audience> {
    match event.voice {
        Some(channel) if channel_exists(db, channel)? => {
            visibility::audience_of(db, event.server, channel)
        }
        Some(_) => Ok(Audience::Users(knew_of(db, event)?)),
        None => Ok(Audience::Server(event.server)),
    }
}

/// Whether a card in `text` reveals nothing to its readers about an event in
/// `voice`: everybody who reads the text channel sees the voice channel.
pub fn card_fits(db: &Connection, server: Id, voice: Id, text: Id) -> rusqlite::Result<bool> {
    let Some(seeing) = visibility::viewers(db, server, voice)? else {
        return Ok(true);
    };
    Ok(visibility::viewers(db, server, text)?.is_some_and(|readers| readers.is_subset(&seeing)))
}

/// The channels whose messages announce the event.
pub fn announced_in(db: &Connection, event: Id) -> rusqlite::Result<Vec<Id>> {
    let mut statement = db.prepare_cached(
        "SELECT DISTINCT m.\"ChannelId\" FROM \"ServerEventAnnouncements\" a JOIN \"Messages\" m ON m.\"Id\" = a.\"MessageId\"
         WHERE a.\"EventId\" = ?1",
    )?;
    statement.query_map([event], |row| row.get(0))?.collect()
}

/// Counts the change of an event, and writes it in the journal of the
/// channels that announce it, so that a client that missed the event reads
/// the card again when it catches up. Says the new revision.
pub fn bump(db: &Connection, event: Id) -> rusqlite::Result<i64> {
    db.execute(
        "UPDATE \"ServerEvents\" SET \"Revision\" = \"Revision\" + 1 WHERE \"Id\" = ?1",
        [event],
    )?;
    db.execute(
        "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\")
         SELECT m.\"ChannelId\", m.\"Id\", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
         FROM \"Messages\" m JOIN \"ServerEventAnnouncements\" a ON a.\"MessageId\" = m.\"Id\" WHERE a.\"EventId\" = ?1",
        [event],
    )?;
    db.query_row(
        "SELECT \"Revision\" FROM \"ServerEvents\" WHERE \"Id\" = ?1",
        [event],
        |row| row.get(0),
    )
}

// ---- seats and the waitlist

/// What one person is at an occurrence: holding a seat, or waiting at a
/// place counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Seated,
    Waiting(i64),
}

/// Everybody signed up for an occurrence, and where each stands.
pub fn standings(db: &Connection, occurrence: Id) -> rusqlite::Result<HashMap<Id, Standing>> {
    let mut statement = db.prepare_cached(
        "SELECT \"UserId\", \"Seated\" FROM \"ServerEventSignups\" WHERE \"OccurrenceId\" = ?1 ORDER BY \"Number\"",
    )?;
    let rows = statement
        .query_map([occurrence], |row| Ok((row.get::<_, Id>(0)?, row.get::<_, bool>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut place = 0;
    Ok(rows
        .into_iter()
        .map(|(user, seated)| {
            if seated {
                (user, Standing::Seated)
            } else {
                place += 1;
                (user, Standing::Waiting(place))
            }
        })
        .collect())
}

fn seated_count(db: &Connection, occurrence: Id) -> rusqlite::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM \"ServerEventSignups\" WHERE \"OccurrenceId\" = ?1 AND \"Seated\" = 1",
        [occurrence],
        |row| row.get(0),
    )
}

/// Signs the person up: a seat while one is free and nobody waits for it,
/// the waitlist otherwise, so that nobody passes those already waiting. The
/// caller gives free seats to the waitlist first. Somebody who signs up when
/// the reminder is due or past is not reminded: they have just seen the
/// event. False when they were signed up already.
pub fn sign_up(db: &Connection, event: &Event, user: Id, now: Timestamp) -> rusqlite::Result<bool> {
    let occurrence = event.occurrence.id;
    let waiting: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ServerEventSignups\" WHERE \"OccurrenceId\" = ?1 AND \"Seated\" = 0)",
        [occurrence],
        |row| row.get(0),
    )?;
    let seated = match event.seats {
        None => true,
        Some(seats) => !waiting && seated_count(db, occurrence)? < seats,
    };
    let reminded = (now >= event.occurrence.starts_at.minus(super::REMINDER_LEAD)).then_some(now);
    let added = db.execute(
        "INSERT INTO \"ServerEventSignups\" (\"OccurrenceId\", \"ServerId\", \"UserId\", \"Seated\", \"SignedUpAt\", \"RemindedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
        params![occurrence, event.server, user, seated, now, reminded],
    )?;
    Ok(added > 0)
}

/// Takes the person's sign-up back. False when there was none.
pub fn sign_out(db: &Connection, event: &Event, user: Id) -> rusqlite::Result<bool> {
    Ok(db.execute(
        "DELETE FROM \"ServerEventSignups\" WHERE \"OccurrenceId\" = ?1 AND \"UserId\" = ?2",
        params![event.occurrence.id, user],
    )? > 0)
}

/// Gives the seats that are free to the waitlist, in the order people signed
/// up, and says who got one. With no limit, everybody waiting does. Somebody
/// given a seat when the reminder is due or past is not reminded as well:
/// the notice of the seat tells them.
pub fn fill(db: &Connection, event: &Event, now: Timestamp) -> rusqlite::Result<Vec<Id>> {
    let occurrence = event.occurrence.id;
    let free = match event.seats {
        None => i64::MAX,
        Some(seats) => (seats - seated_count(db, occurrence)?).max(0),
    };
    if free == 0 {
        return Ok(Vec::new());
    }
    let promoted: Vec<(i64, Id)> = {
        let mut statement = db.prepare_cached(
            "SELECT \"Number\", \"UserId\" FROM \"ServerEventSignups\"
             WHERE \"OccurrenceId\" = ?1 AND \"Seated\" = 0 ORDER BY \"Number\" LIMIT ?2",
        )?;
        statement
            .query_map(params![occurrence, free], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let reminded = (now >= event.occurrence.starts_at.minus(super::REMINDER_LEAD)).then_some(now);
    for (number, _) in &promoted {
        db.execute(
            "UPDATE \"ServerEventSignups\" SET \"Seated\" = 1, \"RemindedAt\" = COALESCE(\"RemindedAt\", ?2) WHERE \"Number\" = ?1",
            params![number, reminded],
        )?;
    }
    Ok(promoted.into_iter().map(|(_, user)| user).collect())
}

/// Everybody signed up or waiting.
pub fn everyone(db: &Connection, occurrence: Id) -> rusqlite::Result<Vec<Id>> {
    let mut statement = db.prepare_cached(
        "SELECT \"UserId\" FROM \"ServerEventSignups\" WHERE \"OccurrenceId\" = ?1 ORDER BY \"Number\"",
    )?;
    statement.query_map([occurrence], |row| row.get(0))?.collect()
}

// ---- JSON

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PlaceDto {
    Voice { channel_id: Id },
    Elsewhere { text: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDto {
    pub id: Id,
    pub server_id: Id,
    pub name: String,
    pub description: Option<String>,
    pub place: PlaceDto,
    pub time_zone: String,
    pub repeat: &'static str,
    pub seats: Option<i64>,
    pub created_by: Id,
    pub announce_message_id: Option<Id>,
    pub occurrence_id: Id,
    pub starts_at: Timestamp,
    pub ends_at: Option<Timestamp>,
    pub state: &'static str,
    pub live_since: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    /// The seats taken.
    pub signed_up: i64,
    /// The people on the waitlist.
    pub waiting: i64,
    /// The first three with a seat, in the order they signed up.
    pub faces: Vec<UserDto>,
    /// `none`, `signed` or `waiting` for the reader; null when the event was
    /// read for many, whose own standing it cannot say.
    pub mine: Option<&'static str>,
    pub my_position: Option<i64>,
    pub revision: i64,
}

/// The JSON of these events for `viewer`, in their order. Four statements
/// however many there are: the counts, the faces, and the viewer's own
/// sign-ups with their places in line.
pub fn dtos(
    db: &Connection,
    events: &[Event],
    viewer: Option<Id>,
    now: Timestamp,
) -> rusqlite::Result<Vec<EventDto>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let occurrences: Vec<Id> = events.iter().map(|event| event.occurrence.id).collect();
    let wanted = json_ids(&occurrences);
    let mut counts: HashMap<Id, (i64, i64)> = HashMap::new();
    {
        let mut statement = db.prepare_cached(
            "SELECT \"OccurrenceId\", SUM(\"Seated\"), SUM(1 - \"Seated\") FROM \"ServerEventSignups\"
             WHERE \"OccurrenceId\" IN (SELECT value FROM json_each(?1)) GROUP BY \"OccurrenceId\"",
        )?;
        for row in statement.query_map([&wanted], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        })? {
            let (occurrence, seated, waiting) = row?;
            counts.insert(occurrence, (seated, waiting));
        }
    }
    let mut faces: HashMap<Id, Vec<UserDto>> = HashMap::new();
    {
        let mut statement = db.prepare_cached(
            "SELECT s.\"OccurrenceId\", u.\"Id\", u.\"Username\", u.\"DisplayName\", u.\"AvatarColor\", u.\"AvatarUrl\",
                    u.\"BannerUrl\", u.\"CustomStatus\", u.\"Bio\", u.\"Pronouns\", u.\"Origin\"
             FROM (SELECT \"OccurrenceId\", \"UserId\",
                          ROW_NUMBER() OVER (PARTITION BY \"OccurrenceId\" ORDER BY \"Number\") AS \"Place\"
                   FROM \"ServerEventSignups\"
                   WHERE \"Seated\" = 1 AND \"OccurrenceId\" IN (SELECT value FROM json_each(?1))) s
             JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
             WHERE s.\"Place\" <= 3 ORDER BY s.\"OccurrenceId\", s.\"Place\"",
        )?;
        for row in statement.query_map([&wanted], |row| {
            let id: Id = row.get(1)?;
            Ok((
                row.get::<_, Id>(0)?,
                UserDto {
                    id: id.to_string(),
                    username: row.get(2)?,
                    display_name: row.get(3)?,
                    avatar_color: row.get(4)?,
                    // As beside a message: where somebody really is is the
                    // roster's business.
                    presence: "online".to_string(),
                    avatar_url: row.get(5)?,
                    banner_url: row.get(6)?,
                    custom_status: row.get(7)?,
                    bio: row.get(8)?,
                    pronouns: row.get(9)?,
                    origin: row.get(10)?,
                },
            ))
        })? {
            let (occurrence, face) = row?;
            faces.entry(occurrence).or_default().push(face);
        }
    }
    let mut mine: HashMap<Id, Standing> = HashMap::new();
    if let Some(viewer) = viewer {
        let mut statement = db.prepare_cached(
            "SELECT s.\"OccurrenceId\", s.\"Seated\",
                    (SELECT COUNT(*) FROM \"ServerEventSignups\" w
                     WHERE w.\"OccurrenceId\" = s.\"OccurrenceId\" AND w.\"Seated\" = 0 AND w.\"Number\" <= s.\"Number\")
             FROM \"ServerEventSignups\" s
             WHERE s.\"UserId\" = ?2 AND s.\"OccurrenceId\" IN (SELECT value FROM json_each(?1))",
        )?;
        for row in statement.query_map(params![wanted, viewer], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, bool>(1)?, row.get::<_, i64>(2)?))
        })? {
            let (occurrence, seated, place) = row?;
            mine.insert(
                occurrence,
                if seated {
                    Standing::Seated
                } else {
                    Standing::Waiting(place)
                },
            );
        }
    }
    Ok(events
        .iter()
        .map(|event| {
            let occurrence = &event.occurrence;
            let state = event.state(now);
            let (signed_up, waiting) = counts.get(&occurrence.id).copied().unwrap_or((0, 0));
            let (standing, position) = match (viewer, mine.get(&occurrence.id)) {
                (None, _) => (None, None),
                (Some(_), None) => (Some("none"), None),
                (Some(_), Some(Standing::Seated)) => (Some("signed"), None),
                (Some(_), Some(Standing::Waiting(place))) => (Some("waiting"), Some(*place)),
            };
            EventDto {
                id: event.id,
                server_id: event.server,
                name: event.name.clone(),
                description: event.description.clone(),
                place: match (event.voice, &event.place_text) {
                    (Some(channel), _) => PlaceDto::Voice { channel_id: channel },
                    (None, text) => PlaceDto::Elsewhere {
                        text: text.clone().unwrap_or_default(),
                    },
                },
                time_zone: event.zone.clone(),
                repeat: event.repeat.name(),
                seats: event.seats,
                created_by: event.created_by,
                announce_message_id: event.announcement,
                occurrence_id: occurrence.id,
                starts_at: occurrence.starts_at,
                ends_at: occurrence.ends_at,
                state: state.name(),
                live_since: match event.voice {
                    Some(_) => occurrence.live_since,
                    None => (state != State::Upcoming).then_some(occurrence.starts_at),
                },
                ended_at: match state {
                    State::Ended => occurrence.ended_at.or(occurrence.ends_at),
                    _ => None,
                },
                signed_up,
                waiting,
                faces: faces.remove(&occurrence.id).unwrap_or_default(),
                mine: standing,
                my_position: position,
                revision: event.revision,
            }
        })
        .collect())
}

/// The cards of these announcement messages for `viewer`, by message.
pub fn cards(
    db: &Connection,
    messages: &[Id],
    viewer: Option<Id>,
    now: Timestamp,
) -> rusqlite::Result<HashMap<Id, EventDto>> {
    if messages.is_empty() {
        return Ok(HashMap::new());
    }
    let links: Vec<(Id, Id)> = {
        let mut statement = db.prepare_cached(
            "SELECT \"MessageId\", \"EventId\" FROM \"ServerEventAnnouncements\"
             WHERE \"MessageId\" IN (SELECT value FROM json_each(?1))",
        )?;
        statement
            .query_map([json_ids(messages)], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let ids: Vec<Id> = links.iter().map(|(_, event)| *event).collect();
    let mut events = select(
        db,
        "e.\"Id\" IN (SELECT value FROM json_each(?1))",
        &[&json_ids(&ids)],
    )?;
    if let Some(viewer) = viewer {
        let mut seen = Vec::with_capacity(events.len());
        for event in events {
            if sees(db, viewer, &event)? {
                seen.push(event);
            }
        }
        events = seen;
    }
    let by_event: HashMap<Id, EventDto> = dtos(db, &events, viewer, now)?
        .into_iter()
        .map(|dto| (dto.id, dto))
        .collect();
    Ok(links
        .into_iter()
        .filter_map(|(message, event)| by_event.get(&event).map(|dto| (message, dto.clone())))
        .collect())
}

/// One event for `viewer`.
pub fn dto(db: &Connection, event: &Event, viewer: Option<Id>, now: Timestamp) -> rusqlite::Result<EventDto> {
    let mut found = dtos(db, std::slice::from_ref(event), viewer, now)?;
    Ok(found.remove(0))
}

/// Who signed up for an occurrence, those with a seat first and then the
/// waitlist, each in the order they signed up, a page at a time after
/// `after`: whether the last one had a seat, and their number.
pub fn signups(
    db: &Connection,
    occurrence: Id,
    after: Option<(bool, i64)>,
    limit: i64,
) -> rusqlite::Result<Vec<(i64, bool, i64, UserDto)>> {
    let mut statement = db.prepare_cached(
        "SELECT s.\"Number\", s.\"Seated\",
                (SELECT COUNT(*) FROM \"ServerEventSignups\" w
                 WHERE w.\"OccurrenceId\" = s.\"OccurrenceId\" AND w.\"Seated\" = 0 AND w.\"Number\" <= s.\"Number\"),
                u.\"Id\", u.\"Username\", u.\"DisplayName\", u.\"AvatarColor\", u.\"AvatarUrl\",
                u.\"BannerUrl\", u.\"CustomStatus\", u.\"Bio\", u.\"Pronouns\", u.\"Origin\"
         FROM \"ServerEventSignups\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
         WHERE s.\"OccurrenceId\" = ?1 AND (?2 IS NULL OR (1 - s.\"Seated\", s.\"Number\") > (1 - ?2, ?3))
         ORDER BY 1 - s.\"Seated\", s.\"Number\" LIMIT ?4",
    )?;
    statement
        .query_map(
            params![occurrence, after.map(|(seated, _)| seated), after.map(|(_, number)| number), limit],
            |row| {
                let id: Id = row.get(3)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, i64>(2)?,
                    UserDto {
                        id: id.to_string(),
                        username: row.get(4)?,
                        display_name: row.get(5)?,
                        avatar_color: row.get(6)?,
                        presence: "online".to_string(),
                        avatar_url: row.get(7)?,
                        banner_url: row.get(8)?,
                        custom_status: row.get(9)?,
                        bio: row.get(10)?,
                        pronouns: row.get(11)?,
                        origin: row.get(12)?,
                    },
                ))
            },
        )?
        .collect()
}
