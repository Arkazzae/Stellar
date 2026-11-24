//! Account-global facts that the person may later be shown, and the alert mail
//! that some of them queue.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::database::DbError;
use crate::values::{Id, Timestamp};

pub const LOGIN_SUCCEEDED: &str = "login-succeeded";
pub const LOGIN_FAILED: &str = "login-failed";
pub const SESSION_REVOKED: &str = "session-revoked";

/// The kinds that send the person a mail when the account has a verified address.
const NOTIFIED: [&str; 15] = [
    "login-succeeded",
    "mfa-succeeded",
    "method-added",
    "method-removed",
    "recovery-code-used",
    "trusted-device-added",
    "trusted-device-removed",
    "session-revoked",
    "password-changed",
    "cli-recovery",
    "policy-changed",
    "desktop-approved",
    "identity-linked",
    "identity-unlinked",
    "email-changed",
];

fn truncated(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

/// Records an account-global fact. The caller commits it with its own
/// transaction. The alert, when there is a verified address, is a separate
/// durable row in the notification outbox, which the mail delivery reads; it
/// never rolls the transaction back.
pub fn record(
    tx: &Transaction,
    user: Id,
    kind: &str,
    ip_address: &str,
    device: &str,
    detail: &str,
    verified_email: Option<&str>,
) -> Result<(), DbError> {
    let now = Timestamp::now();
    let event = Id::random();
    tx.execute(
        "INSERT INTO \"SecurityEvents\" (\"Id\", \"UserId\", \"Kind\", \"Detail\", \"IpAddress\", \"Device\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![event, user, kind, truncated(detail, 500), truncated(ip_address, 64), truncated(device, 300), now],
    )?;
    if let Some(email) =
        verified_email.filter(|email| !email.is_empty() && NOTIFIED.contains(&kind))
    {
        tx.execute(
            "INSERT INTO \"SecurityNotifications\" (\"Id\", \"SecurityEventId\", \"UserId\", \"EmailAddress\", \"Attempts\",
                                                   \"CreatedAt\", \"NextAttemptAt\", \"LastAttemptAt\", \"SentAt\", \"LastError\")
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5, NULL, NULL, '')",
            params![Id::random(), event, user, email, now],
        )?;
    }
    Ok(())
}

/// The address an alert can be sent to: the one the person confirmed by mail
/// as a second step, or the one the identity provider vouched for when the
/// account was made through it. None when there is no such address.
pub fn verified_email(connection: &Connection, user: Id) -> rusqlite::Result<Option<String>> {
    let Some((normalized, email, vouched)) = connection
        .query_row(
            "SELECT \"NormalizedEmail\", \"Email\", \"ProviderVerifiedEmail\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let (Some(normalized), Some(email)) = (
        normalized.filter(|n| !n.is_empty()),
        email.map(|e| e.trim().to_string()),
    ) else {
        return Ok(None);
    };
    if email.is_empty() {
        return Ok(None);
    }
    if vouched.as_deref() == Some(normalized.as_str()) {
        return Ok(Some(email));
    }
    let confirmed: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"Type\" = 'email' AND \"ConfirmedAt\" IS NOT NULL
                        AND \"RevokedAt\" IS NULL AND \"EmailAddress\" = ?2)",
        params![user, normalized],
        |row| row.get(0),
    )?;
    Ok(confirmed.then_some(email))
}
