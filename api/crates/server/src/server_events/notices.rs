//! What the people of an event are told outside the card: an entry in their
//! inbox, which links to the event, and a push notification. Both are
//! written in the transaction that makes the change, as a mention's are,
//! and the push goes out once it has committed; never by mail.

use rusqlite::{Connection, params};
use stellarr_messages::{MessageKey, Messages};

use super::store::Event;
use crate::delivery::outbox::{self, Kind};
use crate::http::ApiError;
use crate::inbox::{self, Entry, NewEntry};
use crate::values::{Id, Timestamp};

/// The inbox entries of events are notices of the system, under this package.
const PACKAGE_ID: &str = "stellarr.events";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The occurrence starts soon.
    Reminder,
    /// A seat came free and is theirs.
    Seat,
    /// The event was cancelled by whoever manages it.
    Cancelled,
    /// The event was cancelled because its voice channel was deleted.
    ChannelGone,
    /// The start, the end or the place changed.
    Changed,
}

impl Notice {
    fn kind(self) -> Kind {
        match self {
            Notice::Reminder => Kind::EventReminder,
            Notice::Seat => Kind::EventSeat,
            Notice::Cancelled | Notice::ChannelGone => Kind::EventCancelled,
            Notice::Changed => Kind::EventChanged,
        }
    }

    fn texts(self) -> (MessageKey, MessageKey) {
        match self {
            Notice::Reminder => (MessageKey::EventReminderTitle, MessageKey::EventReminderBody),
            Notice::Seat => (MessageKey::EventSeatTitle, MessageKey::EventSeatBody),
            Notice::Cancelled => (MessageKey::EventCancelledTitle, MessageKey::EventCancelledBody),
            Notice::ChannelGone => (MessageKey::EventCancelledTitle, MessageKey::EventChannelGoneBody),
            Notice::Changed => (MessageKey::EventChangedTitle, MessageKey::EventChangedBody),
        }
    }

    fn module(self) -> &'static str {
        match self {
            Notice::Reminder => "reminder",
            Notice::Seat => "seat",
            Notice::Cancelled | Notice::ChannelGone => "cancelled",
            Notice::Changed => "changed",
        }
    }
}

/// Tells these people of the event, and says the inbox entries to announce
/// once the transaction has committed. A push about the occurrence is of no
/// use once it has started; one about a cancel is kept as long as any.
pub fn write(
    db: &Connection,
    messages: &Messages,
    notice: Notice,
    event: &Event,
    people: &[Id],
    now: Timestamp,
) -> Result<Vec<Entry>, ApiError> {
    if people.is_empty() {
        return Ok(Vec::new());
    }
    let (title, body) = notice.texts();
    let title = messages.get(title, &[("name", &event.name)])?;
    let body = messages.get(body, &[("name", &event.name)])?;
    let expires = match notice {
        Notice::Cancelled | Notice::ChannelGone => now.plus(outbox::LIFETIME),
        _ => event.occurrence.starts_at,
    };
    let mut entries = Vec::with_capacity(people.len());
    for person in people {
        let entry = inbox::add(
            db,
            &NewEntry {
                server: event.server,
                user: *person,
                source: "system".to_string(),
                source_package_id: PACKAGE_ID.to_string(),
                source_package_name: "Stellarr".to_string(),
                source_module_id: notice.module().to_string(),
                source_module_name: event.name.clone(),
                title: title.clone(),
                body: body.clone(),
                tone: "info".to_string(),
                target_channel: None,
                target_invite_code: None,
                created_at: now,
            },
        )?;
        db.execute(
            "INSERT INTO \"ServerEventNotices\" (\"NotificationId\", \"EventId\") VALUES (?1, ?2)",
            params![entry.id, event.id],
        )?;
        outbox::enqueue_event(db, notice.kind(), event.id, *person, now, expires)?;
        entries.push(entry);
    }
    Ok(entries)
}
