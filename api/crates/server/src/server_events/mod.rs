//! Events of a server: a time and a place, in a voice channel or elsewhere,
//! that people sign up for, with an optional limit of seats and a waitlist.
//!
//! An event has one occurrence at a time: the next, or the one that runs. A
//! series makes the next occurrence when one ends, at the same time on the
//! clock of the event's zone, so many weeks or months after the first. Who
//! signed up belongs to the occurrence; whether each holds a seat is stored,
//! and a seat that comes free goes to the first person waiting, in the
//! transaction that freed it, with a notice.
//!
//! An event in a voice channel exists only for those who see that channel;
//! one elsewhere for every member. The counts go to everyone who sees the
//! event; what a person chose goes to that person alone. The worker
//! `server-events` reminds, starts and ends occurrences, ends a voice event
//! that has no end by who sits in its channel, and makes the next occurrence
//! of a series.
//!
//! The choices the owner made are each one constant or one branch here, so
//! that a later change of mind stays small.

mod model;
mod notices;
mod routes;
mod store;
mod worker;

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::delivery::outbox::Kind;
use crate::http::Endpoints;
use crate::values::{Id, Timestamp};

pub use model::{Repeat, State};
pub use store::EventDto;
pub use worker::{Occupancy, pass, workers};

/// How long before the start the people with a seat are reminded.
pub const REMINDER_LEAD: Duration = Duration::from_secs(15 * 60);
/// Whether people on the waitlist are reminded too. They are not: being
/// given a seat later brings a notice of its own.
pub const REMIND_THE_WAITLIST: bool = false;
/// Whether a new time or place is told to the people signed up and waiting.
pub const TELL_CHANGES: bool = true;
/// A voice event without an end ends once its channel has stayed empty this
/// long after somebody was in it; long enough for clients to come back after
/// a restart of the server, when every channel is empty for a moment.
pub const EMPTY_GRACE: Duration = Duration::from_secs(2 * 60);
/// A voice event without an end that nobody came to ends this long after its
/// start.
pub const NOBODY_GRACE: Duration = Duration::from_secs(60 * 60);
/// The longest a voice event without an end runs.
pub const LONGEST_OPEN: Duration = Duration::from_secs(12 * 60 * 60);
/// How many events a server has at most that are upcoming or running.
pub const MAX_ACTIVE: i64 = 100;
pub const MAX_NAME_UNITS: usize = 100;
pub const MAX_DESCRIPTION_UNITS: usize = 1000;
pub const MAX_PLACE_UNITS: usize = 200;
pub const MAX_SEATS: i64 = 999;
/// How far ahead an event may start, and how long ago: a little, so that a
/// form filled in at the last minute still goes.
pub const MAX_LEAD_DAYS: u64 = 365;
pub const START_SLACK: Duration = Duration::from_secs(5 * 60);
/// The longest an occurrence lasts.
pub const MAX_DURATION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/events", routes::list)
        .authorized()
        .post("/api/events", routes::create)
        .authorized()
        .json_body()
        .get("/api/events/{eventId}", routes::one)
        .authorized()
        .patch("/api/events/{eventId}", routes::edit)
        .authorized()
        .json_body()
        .delete("/api/events/{eventId}", routes::cancel)
        .authorized()
        .put(
            "/api/events/{eventId}/occurrences/{occurrenceId}/signup",
            routes::sign_up,
        )
        .authorized()
        .delete(
            "/api/events/{eventId}/occurrences/{occurrenceId}/signup",
            routes::sign_out,
        )
        .authorized()
        .get(
            "/api/events/{eventId}/occurrences/{occurrenceId}/signups",
            routes::signups,
        )
        .authorized()
}

/// Links a message to the event it announces, in the transaction that posts
/// the message.
pub(crate) fn announce_in(db: &Connection, message: Id, event: Id) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO \"ServerEventAnnouncements\" (\"MessageId\", \"EventId\") VALUES (?1, ?2)",
        params![message, event],
    )?;
    Ok(())
}

/// The cards of these announcement messages, read for `viewer`, by message.
/// A reader who does not see an event has no card for it.
pub(crate) fn cards_of(
    db: &Connection,
    messages: &[Id],
    viewer: Option<Id>,
) -> rusqlite::Result<std::collections::HashMap<Id, EventDto>> {
    store::cards(db, messages, viewer, Timestamp::now())
}

/// The server of the event a push notice is about, when the recipient still
/// sees the event.
pub(crate) fn server_for(db: &Connection, event: Id, user: Id) -> rusqlite::Result<Option<Id>> {
    let Some(found) = store::find(db, event)? else {
        return Ok(None);
    };
    Ok(store::sees(db, user, &found)?.then_some(found.server))
}

/// Whether a push notice about an event is still worth sending: the event is
/// still there for the recipient, and a reminder or a seat is about an
/// occurrence that has not started, at which the recipient still holds a
/// seat.
pub(crate) fn notice_wanted(
    db: &Connection,
    kind: Kind,
    event: Id,
    recipient: Id,
    now: Timestamp,
) -> rusqlite::Result<bool> {
    let Some(found) = store::find(db, event)? else {
        return Ok(false);
    };
    if !store::sees(db, recipient, &found)? {
        return Ok(false);
    }
    match kind {
        Kind::EventReminder | Kind::EventSeat => {
            if found.cancelled_at.is_some() || found.occurrence.starts_at <= now {
                return Ok(false);
            }
            Ok(db
                .query_row(
                    "SELECT \"Seated\" FROM \"ServerEventSignups\" WHERE \"OccurrenceId\" = ?1 AND \"UserId\" = ?2",
                    params![found.occurrence.id, recipient],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false))
        }
        _ => Ok(true),
    }
}
