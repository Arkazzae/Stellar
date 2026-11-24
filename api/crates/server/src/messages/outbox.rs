//! The external deliveries a mention asks for: rows like the inbox entries,
//! written in the message's own transaction. Nothing is contacted here; the
//! dispatcher that sends them reads the rows after the commit.
//!
//! Only a recipient who has opted in at this moment gets a row, and only for
//! destinations that are usable now: one row per live push subscription, or
//! one for the account's verified address. A later opt-in is never applied to
//! messages that were already sent.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::values::{Id, Timestamp};

/// An activity notice stops being useful long before it is pruned.
const LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const MAX_DESTINATION_KEY: usize = 320;

const MENTION: &str = "mention";
const DIRECT_MESSAGE: &str = "direct-message";

/// Queues what a mention of `recipient` in the message asked for.
pub fn enqueue_mention(
    db: &Connection,
    message: Id,
    recipient: Id,
    created_at: Timestamp,
) -> rusqlite::Result<()> {
    enqueue(db, MENTION, message, recipient, created_at)
}

/// Queues what a direct message to `recipient` asked for.
pub fn enqueue_direct_message(
    db: &Connection,
    message: Id,
    recipient: Id,
    created_at: Timestamp,
) -> rusqlite::Result<()> {
    enqueue(db, DIRECT_MESSAGE, message, recipient, created_at)
}

fn enqueue(
    db: &Connection,
    kind: &'static str,
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
    if email && let Some(address) = verified_email(db, recipient)? {
        queue.add("email", &stellarr_unicode::to_upper(trim(&address)), None)?;
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
        // The key is the id without hyphens, in lower case.
        let key = subscription.to_string().replace('-', "");
        queue.add("push", &key, Some(subscription))?;
    }
    Ok(())
}

/// The address an account may be mailed at: the one it has when a confirmed
/// e-mail method matches it, or the one its sign-in provider vouched for.
fn verified_email(db: &Connection, user: Id) -> rusqlite::Result<Option<String>> {
    let Some((email, normalized, provider)) = db
        .query_row(
            "SELECT \"Email\", \"NormalizedEmail\", \"ProviderVerifiedEmail\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some(normalized) = normalized.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let Some(address) = email
        .map(|text| trim(&text).to_string())
        .filter(|text| !text.is_empty())
    else {
        return Ok(None);
    };
    if provider.as_deref() == Some(normalized.as_str()) {
        return Ok(Some(address));
    }
    let confirmed: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"Type\" = 'email'
             AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL AND \"EmailAddress\" = ?2)",
        params![user, normalized],
        |row| row.get(0),
    )?;
    Ok(confirmed.then_some(address))
}

/// Trims what the first server's `Trim()` trims.
fn trim(text: &str) -> &str {
    text.trim_matches(char::is_whitespace)
}

struct Queue<'a> {
    db: &'a Connection,
    kind: &'static str,
    message: Id,
    recipient: Id,
    created_at: Timestamp,
}

impl Queue<'_> {
    fn add(&self, channel: &str, key: &str, subscription: Option<Id>) -> rusqlite::Result<()> {
        // The key is counted in UTF-16 units, as the first server counted it.
        if key.encode_utf16().count() > MAX_DESTINATION_KEY {
            return Ok(());
        }
        self.db.prepare_cached(
            "INSERT INTO \"NotificationDeliveries\" (\"Id\", \"SourceMessageId\", \"RecipientId\", \"EventKind\", \"Channel\",
                 \"DestinationKey\", \"PushSubscriptionId\", \"CreatedAt\", \"ExpiresAt\", \"Attempts\", \"NextAttemptAt\",
                 \"LastAttemptAt\", \"SentAt\", \"Status\", \"LeaseId\", \"LeaseUntil\", \"LastError\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?8, NULL, NULL, 'pending', NULL, NULL, '')",
        )?
        .execute(params![
            Id::random(),
            self.message,
            self.recipient,
            self.kind,
            channel,
            key,
            subscription,
            self.created_at,
            self.created_at.plus(LIFETIME),
        ])?;
        Ok(())
    }
}
