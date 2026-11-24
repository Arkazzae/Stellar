//! Account-global facts that the person may later be shown, and the alert mail
//! that some of them queue.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::policy::{FAILURE_BUDGET, FAILURE_WINDOW};
use crate::database::DbError;
use crate::values::{Id, Timestamp};

pub const LOGIN_SUCCEEDED: &str = "login-succeeded";
pub const LOGIN_FAILED: &str = "login-failed";
pub const SESSION_REVOKED: &str = "session-revoked";
pub const MFA_SUCCEEDED: &str = "mfa-succeeded";
pub const MFA_FAILED: &str = "mfa-failed";
pub const METHOD_ADDED: &str = "method-added";
pub const METHOD_REMOVED: &str = "method-removed";
pub const RECOVERY_CODES_REGENERATED: &str = "recovery-codes-regenerated";
pub const RECOVERY_CODE_USED: &str = "recovery-code-used";
pub const TRUSTED_DEVICE_ADDED: &str = "trusted-device-added";
pub const TRUSTED_DEVICE_REMOVED: &str = "trusted-device-removed";
pub const PASSWORD_CHANGED: &str = "password-changed";
pub const CLI_RECOVERY: &str = "cli-recovery";
pub const POLICY_CHANGED: &str = "policy-changed";
pub const DESKTOP_APPROVED: &str = "desktop-approved";
pub const IDENTITY_LINKED: &str = "identity-linked";
pub const IDENTITY_UNLINKED: &str = "identity-unlinked";
pub const EMAIL_CHANGED: &str = "email-changed";

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

/// [`record`] for what a request says about itself: its address, and the
/// browser's own name for itself as it was sent (cut, not replaced by a text
/// for an unknown one, since an event names what was there).
pub fn record_from(
    tx: &Transaction,
    context: &super::flow::Context<'_>,
    user: Id,
    kind: &str,
    detail: &str,
    verified_email: Option<&str>,
) -> Result<(), DbError> {
    record(
        tx,
        user,
        kind,
        context.ip_address,
        context.user_agent,
        detail,
        verified_email,
    )
}

/// Failed second-step proofs the account has collected since `since`, across
/// every challenge: what the account-wide throttle counts.
pub fn recent_mfa_failures(
    connection: &Connection,
    user: Id,
    since: Timestamp,
) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 AND \"Kind\" = ?2 AND \"CreatedAt\" >= ?3",
        params![user, MFA_FAILED, since],
        |row| row.get(0),
    )
}

/// Whether the account has spent its budget of failed proofs, so that a
/// further attempt is refused before the code is looked at.
pub fn failure_budget_spent(
    connection: &Connection,
    user: Id,
    now: Timestamp,
) -> rusqlite::Result<bool> {
    let since = now.minus(FAILURE_WINDOW);
    Ok(recent_mfa_failures(connection, user, since)? >= FAILURE_BUDGET)
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::auth::fixtures::{add_method, add_user, open};

    fn count(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn an_event_is_cut_to_the_lengths_the_column_holds_and_alerts_only_the_kinds_that_are_notified()
    {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some("Maya@Example.test"));
        let tx = connection.transaction().unwrap();

        record(
            &tx,
            user,
            MFA_SUCCEEDED,
            &"1".repeat(100),
            &"d".repeat(400),
            &"x".repeat(900),
            Some("maya@example.test"),
        )
        .unwrap();
        record(
            &tx,
            user,
            MFA_FAILED,
            "",
            "",
            "totp",
            Some("maya@example.test"),
        )
        .unwrap();
        record(&tx, user, METHOD_ADDED, "", "", "totp", None).unwrap();
        tx.commit().unwrap();

        let (detail, ip, device): (String, String, String) = connection
            .query_row(
                "SELECT \"Detail\", \"IpAddress\", \"Device\" FROM \"SecurityEvents\" WHERE \"Kind\" = 'mfa-succeeded'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((detail.len(), ip.len(), device.len()), (500, 64, 300));
        assert_eq!(count(&connection, "SecurityEvents"), 3);
        // Only the notified kind with an address queued a mail.
        assert_eq!(count(&connection, "SecurityNotifications"), 1);
    }

    #[test]
    fn only_failures_inside_the_window_count_towards_the_budget() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        for _ in 0..FAILURE_BUDGET - 1 {
            record(&tx, user, MFA_FAILED, "", "", "totp", None).unwrap();
        }
        // Other kinds, and other accounts, are not this account's failures.
        record(&tx, user, LOGIN_FAILED, "", "", "password", None).unwrap();
        record(&tx, other, MFA_FAILED, "", "", "totp", None).unwrap();
        tx.commit().unwrap();

        assert!(!failure_budget_spent(&connection, user, now).unwrap());
        let tx = connection.transaction().unwrap();
        record(&tx, user, MFA_FAILED, "", "", "totp", None).unwrap();
        tx.commit().unwrap();
        assert!(failure_budget_spent(&connection, user, now).unwrap());
        // Seen from after the window, they are forgotten.
        let later = now.plus(FAILURE_WINDOW + Duration::from_secs(60));
        assert!(!failure_budget_spent(&connection, user, later).unwrap());
    }

    #[test]
    fn an_address_is_verified_when_the_provider_vouched_for_it_or_an_email_method_confirmed_it() {
        let (_folder, connection) = open();
        let bare = add_user(&connection, Some(" maya@example.test "));
        let none = add_user(&connection, None);
        assert_eq!(verified_email(&connection, bare).unwrap(), None);
        assert_eq!(verified_email(&connection, none).unwrap(), None);

        add_method(&connection, bare, "email", Some(" MAYA@EXAMPLE.TEST "));
        assert_eq!(verified_email(&connection, bare).unwrap(), None);
        add_method(&connection, bare, "email", Some("MAYA@EXAMPLE.TEST"));
        // The address is returned as the person wrote it, trimmed.
        assert_eq!(
            verified_email(&connection, bare).unwrap().as_deref(),
            Some("maya@example.test")
        );

        let vouched = add_user(&connection, Some("v@example.test"));
        connection
            .execute(
                "UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = 'V@EXAMPLE.TEST' WHERE \"Id\" = ?1",
                [vouched],
            )
            .unwrap();
        assert_eq!(
            verified_email(&connection, vouched).unwrap().as_deref(),
            Some("v@example.test")
        );
    }
}
