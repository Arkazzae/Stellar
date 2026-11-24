//! The rows that say "this person is owed a notification about this message".
//!
//! A route that stores a message calls [`enqueue_mentions`] (a post in a
//! channel) or [`enqueue_direct_message`] (a post in a conversation) inside the
//! transaction that stores it, after the message and its inbox entries are
//! written. Nothing is contacted here: the worker reads the rows after the
//! commit.
//!
//! Only a recipient who has opted in at this moment gets a row, and only for
//! destinations that are usable now: one row per live push subscription, or one
//! for the account's verified address. A later opt-in is never applied to
//! messages that were already sent.

use std::collections::BTreeSet;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::auth::events;
use crate::values::{Id, Timestamp};

/// What an owed notification is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mention,
    DirectMessage,
}

impl Kind {
    /// How the kind is written in the `EventKind` column.
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Mention => "mention",
            Kind::DirectMessage => "direct-message",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        match text {
            "mention" => Some(Kind::Mention),
            "direct-message" => Some(Kind::DirectMessage),
            _ => None,
        }
    }
}

/// An undelivered notice stops being useful long before it is pruned.
pub const LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const MAX_DESTINATION_KEY_UNITS: usize = 320;

/// Queues what the mentions of `recipients` in a channel message asked for.
/// `created_at` is the message's own moment; a person named twice is queued
/// once.
pub fn enqueue_mentions(
    db: &Connection,
    message: Id,
    recipients: impl IntoIterator<Item = Id>,
    created_at: Timestamp,
) -> rusqlite::Result<()> {
    let distinct: BTreeSet<Id> = recipients.into_iter().collect();
    // In the order of the people's ids, so that the rows are always written in
    // the same order for the same message.
    for recipient in distinct {
        enqueue(db, Kind::Mention, message, recipient, created_at)?;
    }
    Ok(())
}

/// Queues what a direct message asked for from `recipient`, the other person
/// of the conversation.
pub fn enqueue_direct_message(
    db: &Connection,
    message: Id,
    recipient: Id,
    created_at: Timestamp,
) -> rusqlite::Result<()> {
    enqueue(db, Kind::DirectMessage, message, recipient, created_at)
}

fn enqueue(
    db: &Connection,
    kind: Kind,
    message: Id,
    recipient: Id,
    created_at: Timestamp,
) -> rusqlite::Result<()> {
    let Some((disabled, session_version)) = db
        .query_row(
            "SELECT \"IsDisabled\", \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
            [recipient],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    if disabled {
        return Ok(());
    }
    let Some((email, push)) = db
        .query_row(
            "SELECT \"EmailNotifications\", \"PushNotifications\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
            [recipient],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    let queue = Queue {
        db,
        kind,
        message,
        recipient,
        created_at,
    };
    if email && let Some(address) = events::verified_email(db, recipient)? {
        queue.add("email", &destination_key(&address), None)?;
    }
    if !push {
        return Ok(());
    }
    // A subscription counts only while the session that made it does.
    let mut statement = db.prepare_cached(
        "SELECT p.\"Id\" FROM \"PushSubscriptions\" p
         WHERE p.\"UserId\" = ?1 AND p.\"SessionId\" IN (
             SELECT s.\"Id\" FROM \"UserSessions\" s
             WHERE s.\"UserId\" = ?1 AND s.\"RevokedAt\" IS NULL AND s.\"ExpiresAt\" > ?2
               AND s.\"AbsoluteExpiresAt\" IS NOT NULL AND s.\"AbsoluteExpiresAt\" > ?2
               AND s.\"SecurityVersion\" = ?3)
         ORDER BY p.\"Id\"",
    )?;
    let subscriptions: Vec<Id> = statement
        .query_map(params![recipient, created_at, session_version], |row| {
            row.get(0)
        })?
        .collect::<rusqlite::Result<_>>()?;
    for subscription in subscriptions {
        queue.add("push", &push_key(subscription), Some(subscription))?;
    }
    Ok(())
}

/// What makes a mail destination unique, and what the dispatcher compares
/// again: the address trimmed and in capitals.
pub fn destination_key(address: &str) -> String {
    stellarr_unicode::to_upper(address.trim_matches(char::is_whitespace))
}

/// What makes a push destination unique: the subscription's id without
/// hyphens, in lower case.
pub fn push_key(subscription: Id) -> String {
    subscription.to_string().replace('-', "")
}

struct Queue<'a> {
    db: &'a Connection,
    kind: Kind,
    message: Id,
    recipient: Id,
    created_at: Timestamp,
}

impl Queue<'_> {
    fn add(&self, channel: &str, key: &str, subscription: Option<Id>) -> rusqlite::Result<()> {
        // The key length is counted in UTF-16 code units.
        if stellarr_unicode::utf16::length(key) > MAX_DESTINATION_KEY_UNITS {
            return Ok(());
        }
        self.db
            .prepare_cached(
                "INSERT INTO \"NotificationDeliveries\" (\"Id\", \"SourceMessageId\", \"RecipientId\", \"EventKind\", \"Channel\",
                     \"DestinationKey\", \"PushSubscriptionId\", \"CreatedAt\", \"ExpiresAt\", \"Attempts\", \"NextAttemptAt\",
                     \"LastAttemptAt\", \"SentAt\", \"Status\", \"LeaseId\", \"LeaseUntil\", \"LastError\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?8, NULL, NULL, 'pending', NULL, NULL, '')",
            )?
            .execute(params![
                Id::random(),
                self.message,
                self.recipient,
                self.kind.as_str(),
                channel,
                key,
                subscription,
                self.created_at,
                self.created_at.plus(LIFETIME),
            ])?;
        Ok(())
    }
}
