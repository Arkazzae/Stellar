//! The worker of server events. Each pass looks at every event that is not
//! over: it cancels one whose voice channel is gone, reminds the people with
//! a seat, follows who sits in a voice event's channel, marks starts and
//! ends, makes the next occurrence of a series, and gives seats freed by
//! people who left the server to the waitlist. It sleeps until the next of
//! those is due, and looks every few seconds while a voice event runs.

use std::time::Duration;

use rusqlite::{Connection, params};
use stellarr_messages::Messages;
use stellarr_timezone::TimeZone;
use tracing::warn;

use super::model::{Repeat, State as Phase};
use super::notices::{self, Notice};
use super::routes::{self, Told};
use super::store::{self, Event};
use super::{EMPTY_GRACE, LONGEST_OPEN, NOBODY_GRACE, REMIND_THE_WAITLIST, REMINDER_LEAD};
use crate::background::Worker;
use crate::database::DbError;
use crate::http::ApiError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const TARGET: &str = "Stellarr.Api.ServerEvents";

/// The longest the worker sleeps: a seat freed by somebody who left the
/// server, which nothing announces here, is filled within this.
const LONGEST_SLEEP: Duration = Duration::from_secs(60);

/// How often it looks at who sits in the channel of a running voice event.
const SEAT_LOOK: Duration = Duration::from_secs(10);

/// Who sits in a voice channel. The worker reads the hub; a test says.
pub type Occupancy<'a> = &'a (dyn Fn(Id) -> usize + Sync);

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("server-events", |state, stop| async move {
        loop {
            let sitting = |channel: Id| {
                state
                    .realtime()
                    .map_or(0, |hub| hub.presence().voice_users(channel).len())
            };
            let next = match pass(&state, Timestamp::now(), &sitting).await {
                Ok(next) => next,
                Err(error) => {
                    warn!(target: TARGET, "Looking after server events failed: {error:?}");
                    None
                }
            };
            let wait = next.map_or(LONGEST_SLEEP, |at| {
                Timestamp::now().until(at).min(LONGEST_SLEEP)
            });
            tokio::select! {
                () = state.event_alarm().rung() => {}
                slept = stop.sleep(wait) => if !slept {
                    return;
                },
            }
            if stop.is_stopping() {
                return;
            }
        }
    })]
}

/// Does what is due at `now` for every event that is not over, each event in
/// a transaction of its own, and says when to look next.
pub async fn pass(state: &AppState, now: Timestamp, occupancy: Occupancy<'_>) -> Result<Option<Timestamp>, ApiError> {
    let events = state
        .database()
        .read(|db| -> Result<_, DbError> { Ok(store::unfinished(db)?) })
        .await?;
    for event in events {
        // Who sits in the channel is the hub's to say, outside the database.
        let sitting = event.voice.map(occupancy);
        let messages = *state.messages();
        let id = event.id;
        let outcome = state
            .database()
            .write(move |tx| step(tx, &messages, id, sitting, now))
            .await;
        match outcome {
            Ok(Some(told)) => routes::publish(state, told),
            Ok(None) => {}
            Err(error) => warn!(target: TARGET, "An event could not be looked after: {error:?}"),
        }
    }
    Ok(state
        .database()
        .read(move |db| -> Result<_, DbError> { Ok(next_look(db, now)?) })
        .await?)
}

/// What is due for one event at `now`, and what to tell of it.
fn step(
    tx: &Connection,
    messages: &Messages,
    id: Id,
    sitting: Option<usize>,
    now: Timestamp,
) -> Result<Option<Told>, ApiError> {
    let Some(mut event) = store::find(tx, id)? else {
        return Ok(None);
    };
    if event.cancelled_at.is_some() {
        return Ok(None);
    }
    let before = store::standings(tx, event.occurrence.id)?;

    // An event whose voice channel was deleted cannot happen there.
    if let Some(channel) = event.voice
        && event.state(now).is_active()
        && !store::channel_exists(tx, channel)?
    {
        tx.execute(
            "UPDATE \"ServerEvents\" SET \"CancelledAt\" = ?2 WHERE \"Id\" = ?1",
            params![id, now],
        )?;
        let people = store::everyone(tx, event.occurrence.id)?;
        let entries = notices::write(tx, messages, Notice::ChannelGone, &event, &people, now)?;
        store::bump(tx, id)?;
        return Ok(Some(routes::told(tx, id, &before, entries, now)?));
    }

    let mut entries = remind(tx, messages, &event, now)?;
    let mut changed = false;

    if let Some(sitting) = sitting
        && event.state(now) == Phase::Live
    {
        changed |= follow_the_channel(tx, &event, sitting, now)?;
        event = store::find(tx, id)?.ok_or_else(|| ApiError::internal("an event was not found again"))?;
    }

    let phase = event.state(now);
    if phase.name() != event.occurrence.phase {
        changed = true;
        tx.execute(
            "UPDATE \"ServerEventOccurrences\" SET \"Phase\" = ?2,
                 \"EndedAt\" = CASE WHEN ?3 THEN COALESCE(\"EndedAt\", \"EndsAt\") ELSE \"EndedAt\" END
             WHERE \"Id\" = ?1",
            params![event.occurrence.id, phase.name(), phase == Phase::Ended],
        )?;
        if phase == Phase::Ended && event.repeat != Repeat::None {
            next_occurrence(tx, &event, now)?;
        }
        event = store::find(tx, id)?.ok_or_else(|| ApiError::internal("an event was not found again"))?;
    }

    // Seats that people who left the server gave up, with their membership.
    if event.state(now).is_active() {
        let promoted = store::fill(tx, &event, now)?;
        if !promoted.is_empty() {
            changed = true;
            entries.extend(notices::write(tx, messages, Notice::Seat, &event, &promoted, now)?);
        }
    }

    if changed {
        store::bump(tx, id)?;
        return Ok(Some(routes::told(tx, id, &before, entries, now)?));
    }
    Ok((!entries.is_empty()).then(|| Told::entries(entries)))
}

/// Reminds the people with a seat once the reminder is due, each once: the
/// mark and the notice are written together. One whose moment passed while
/// the server was down still goes before the start, and is dropped after.
fn remind(
    tx: &Connection,
    messages: &Messages,
    event: &Event,
    now: Timestamp,
) -> Result<Vec<crate::inbox::Entry>, ApiError> {
    let occurrence = &event.occurrence;
    if now < occurrence.starts_at.minus(REMINDER_LEAD) || !event.state(now).is_active() {
        return Ok(Vec::new());
    }
    let due: Vec<Id> = {
        let mut statement = tx.prepare_cached(
            "SELECT \"UserId\" FROM \"ServerEventSignups\"
             WHERE \"OccurrenceId\" = ?1 AND \"RemindedAt\" IS NULL AND (\"Seated\" = 1 OR ?2)
             ORDER BY \"Number\"",
        )?;
        statement
            .query_map(params![occurrence.id, REMIND_THE_WAITLIST], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    if due.is_empty() {
        return Ok(Vec::new());
    }
    tx.execute(
        "UPDATE \"ServerEventSignups\" SET \"RemindedAt\" = ?2
         WHERE \"OccurrenceId\" = ?1 AND \"RemindedAt\" IS NULL AND (\"Seated\" = 1 OR ?3)",
        params![occurrence.id, now, REMIND_THE_WAITLIST],
    )?;
    if now >= occurrence.starts_at {
        return Ok(Vec::new());
    }
    notices::write(tx, messages, Notice::Reminder, event, &due, now)
}

/// Follows who sits in the channel of a running voice event: the moment
/// somebody is first there, the moment it empties, and for an event without
/// an end, its end. Says whether what clients are shown changed.
fn follow_the_channel(tx: &Connection, event: &Event, sitting: usize, now: Timestamp) -> rusqlite::Result<bool> {
    let occurrence = &event.occurrence;
    let mut live_since = occurrence.live_since;
    let mut empty_since = occurrence.empty_since;
    let mut ended_at = None;
    if sitting > 0 {
        live_since.get_or_insert(now);
        empty_since = None;
    } else if live_since.is_some() {
        let since = *empty_since.get_or_insert(now);
        if occurrence.ends_at.is_none() && since.until(now) >= EMPTY_GRACE {
            ended_at = Some(since);
        }
    }
    if occurrence.ends_at.is_none() {
        let start = occurrence.starts_at;
        if live_since.is_none() && now >= start.plus(NOBODY_GRACE) {
            ended_at = Some(start.plus(NOBODY_GRACE));
        }
        let cap = start.plus(LONGEST_OPEN);
        if now >= cap {
            ended_at = Some(ended_at.map_or(cap, |end| end.min(cap)));
        }
    }
    if (live_since, empty_since, ended_at) == (occurrence.live_since, occurrence.empty_since, None) {
        return Ok(false);
    }
    tx.execute(
        "UPDATE \"ServerEventOccurrences\" SET \"LiveSince\" = ?2, \"EmptySince\" = ?3, \"EndedAt\" = COALESCE(\"EndedAt\", ?4)
         WHERE \"Id\" = ?1",
        params![occurrence.id, live_since, empty_since, ended_at],
    )?;
    Ok(live_since != occurrence.live_since || ended_at.is_some())
}

/// Replaces the occurrence that ended with the next of the series that is
/// still to start, with nobody signed up. The sign-ups go with the old one.
fn next_occurrence(tx: &Connection, event: &Event, now: Timestamp) -> rusqlite::Result<()> {
    let zone = TimeZone::find(&event.zone).unwrap_or_else(|_| TimeZone::utc());
    let mut sequence = event.occurrence.sequence + 1;
    let mut start = event.repeat.start_of(&zone, event.first_starts_at, sequence);
    // Occurrences that fell while the server was down are skipped, not held.
    let mut skipped = 0;
    while start <= now && skipped < 10_000 {
        sequence += 1;
        skipped += 1;
        start = event.repeat.start_of(&zone, event.first_starts_at, sequence);
    }
    let end = event
        .duration
        .map(|seconds| start.plus(Duration::from_secs(u64::try_from(seconds).unwrap_or(0))));
    tx.execute(
        "DELETE FROM \"ServerEventOccurrences\" WHERE \"Id\" = ?1",
        [event.occurrence.id],
    )?;
    tx.execute(
        "INSERT INTO \"ServerEventOccurrences\" (\"Id\", \"EventId\", \"Sequence\", \"StartsAt\", \"EndsAt\", \"Phase\")
         VALUES (?1, ?2, ?3, ?4, ?5, 'upcoming')",
        params![Id::random(), event.id, sequence, start, end],
    )?;
    Ok(())
}

/// When the next thing is due: a reminder, a start, an end, the grace or the
/// cap of a voice event without an end, or the next look at a channel.
fn next_look(db: &Connection, now: Timestamp) -> rusqlite::Result<Option<Timestamp>> {
    let mut next: Option<Timestamp> = None;
    let mut consider = |at: Timestamp| {
        if at > now {
            next = Some(next.map_or(at, |next| next.min(at)));
        }
    };
    for event in store::unfinished(db)? {
        let occurrence = &event.occurrence;
        match event.state(now) {
            Phase::Upcoming => {
                consider(occurrence.starts_at.minus(REMINDER_LEAD));
                consider(occurrence.starts_at);
            }
            Phase::Live => {
                if let Some(end) = occurrence.ends_at {
                    consider(end);
                }
                if event.voice.is_some() {
                    consider(now.plus(SEAT_LOOK));
                    if occurrence.ends_at.is_none() {
                        consider(occurrence.starts_at.plus(NOBODY_GRACE));
                        consider(occurrence.starts_at.plus(LONGEST_OPEN));
                    }
                }
            }
            // An end not yet marked, or the next occurrence to make: soon,
            // but not at once, so that a step that keeps failing does not
            // spin.
            Phase::Ended | Phase::Cancelled => consider(now.plus(Duration::from_secs(1))),
        }
    }
    Ok(next)
}
