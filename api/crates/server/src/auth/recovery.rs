//! An operator's recovery of an account that lost its second step, shared by
//! the command line and the operator's panel, and the credential it hands
//! over.
//!
//! In one transaction the recovery revokes every second step, remembered
//! device, session, approved desktop handoff, unused recovery code and earlier
//! credential, moves the account to a new security version, marks it as
//! waiting for recovery, and issues one credential that can only start the
//! setup of a new second step. The links to an identity provider stay. While
//! the account waits, no way of signing in gives it a session: every one of
//! them ends in the same refusal (see `flow`), until the setup of a new step
//! completes ([`finish_enrollment`]).

use rusqlite::{OptionalExtension, Transaction, params};

use super::events;
use super::policy::SecurityPolicy;
use super::recovery_codes;
use super::tokens;
use crate::database::DbError;
use crate::values::{Id, Timestamp};

/// What a recovery ended, and the one credential it issued.
#[derive(Clone, PartialEq, Eq)]
pub struct Recovered {
    pub methods_revoked: usize,
    pub trusted_devices_revoked: usize,
    pub sessions_revoked: usize,
    pub handoffs_invalidated: usize,
    /// What the operator hands to the person. It is stored only as a digest.
    pub credential: String,
    pub expires_at: Timestamp,
    pub session_version: i64,
}

/// What was ended, not the credential.
impl std::fmt::Debug for Recovered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recovered")
            .field("methods_revoked", &self.methods_revoked)
            .field("trusted_devices_revoked", &self.trusted_devices_revoked)
            .field("sessions_revoked", &self.sessions_revoked)
            .field("handoffs_invalidated", &self.handoffs_invalidated)
            .field("expires_at", &self.expires_at)
            .field("session_version", &self.session_version)
            .finish_non_exhaustive()
    }
}

/// Recovers the account. `audit` writes the operator's own record of it, in
/// the same transaction. None when there is no such account or it is disabled.
pub fn recover(
    tx: &Transaction,
    user: Id,
    detail: &str,
    policy: &SecurityPolicy,
    now: Timestamp,
    audit: impl FnOnce(&Transaction) -> Result<(), DbError>,
) -> Result<Option<Recovered>, DbError> {
    let found: Option<(bool, i64)> = tx
        .query_row(
            "SELECT \"IsDisabled\", \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((false, version)) = found else {
        return Ok(None);
    };
    // Read before the methods go, since a mail method is what verifies most
    // addresses.
    let alert_recipient = events::verified_email(tx, user)?;
    let methods = tx.execute(
        "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
        params![user, now],
    )?;
    let devices = super::trusted_devices::revoke_all(tx, user, now)?;
    let sessions: i64 = tx.query_row(
        "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![user, now],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
        params![user, now],
    )?;
    let handoffs = tx.execute(
        "UPDATE \"DesktopBrowserLogins\" SET \"RedeemedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"ApprovedAt\" IS NOT NULL AND \"RedeemedAt\" IS NULL",
        params![user, now],
    )?;
    recovery_codes::spend_all(tx, user, now)?;
    tx.execute(
        "UPDATE \"CliRecoveryCredentials\" SET \"UsedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL",
        params![user, now],
    )?;

    let session_version = version + 1;
    tx.execute(
        "UPDATE \"Users\" SET \"SessionVersion\" = ?2, \"SecurityRecoveryRequired\" = 1 WHERE \"Id\" = ?1",
        params![user, session_version],
    )?;
    let credential = tokens::random(32);
    let expires_at = now.plus(policy.cli_recovery_lifetime());
    tx.execute(
        "INSERT INTO \"CliRecoveryCredentials\" (\"Id\", \"UserId\", \"TokenHash\", \"Reason\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"UsedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            Id::random(),
            user,
            tokens::hash(&credential),
            detail,
            session_version,
            now,
            expires_at
        ],
    )?;
    events::record(
        tx,
        user,
        events::CLI_RECOVERY,
        "",
        "",
        &format!("operator recovery: {detail}"),
        alert_recipient.as_deref(),
    )?;
    audit(tx)?;
    Ok(Some(Recovered {
        methods_revoked: methods,
        trusted_devices_revoked: devices,
        sessions_revoked: sessions as usize,
        handoffs_invalidated: handoffs,
        credential,
        expires_at,
        session_version,
    }))
}

/// A credential, and the account it is for, as they stand.
struct Redeeming {
    id: Id,
    user: Id,
    credential_version: i64,
    disabled: bool,
    waiting: bool,
    account_version: i64,
}

/// The shortest and the longest credential that is looked up at all, in UTF-16
/// units as the first server counted them.
const CREDENTIAL_LENGTH: std::ops::RangeInclusive<usize> = 16..=512;

/// Exchanges the credential an operator handed over, once, for the right to set
/// up a new second step: the account it is for. It must be of an account that
/// is not disabled and is waiting for recovery, and of the security version the
/// account is at: a later change of the account makes it void. It must also be
/// unused and not run out, which the claim that spends it decides in the same
/// statement, so that two requests cannot both have it. Anything else is
/// nothing, said the same way.
pub(super) fn redeem(
    tx: &Transaction,
    credential: &str,
    now: Timestamp,
) -> Result<Option<Id>, DbError> {
    let credential = credential.trim();
    if !CREDENTIAL_LENGTH.contains(&credential.encode_utf16().count()) {
        return Ok(None);
    }
    // Found by the digest of a 256-bit secret, which nobody can choose a
    // prefix of.
    let row = tx
        .query_row(
            "SELECT c.\"Id\", c.\"UserId\", c.\"SecurityVersion\", u.\"IsDisabled\", u.\"SecurityRecoveryRequired\", u.\"SessionVersion\"
             FROM \"CliRecoveryCredentials\" c JOIN \"Users\" u ON u.\"Id\" = c.\"UserId\" WHERE c.\"TokenHash\" = ?1",
            params![tokens::hash(credential)],
            |row| {
                Ok(Redeeming {
                    id: row.get(0)?,
                    user: row.get(1)?,
                    credential_version: row.get(2)?,
                    disabled: row.get(3)?,
                    waiting: row.get(4)?,
                    account_version: row.get(5)?,
                })
            },
        )
        .optional()?;
    let Some(found) = row else {
        return Ok(None);
    };
    if found.disabled || !found.waiting || found.credential_version != found.account_version {
        return Ok(None);
    }
    let (id, user) = (found.id, found.user);
    let claimed = tx.execute(
        "UPDATE \"CliRecoveryCredentials\" SET \"UsedAt\" = ?2 WHERE \"Id\" = ?1 AND \"UsedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![id, now],
    )?;
    Ok((claimed == 1).then_some(user))
}

/// What the completion of a setup does to the account: a new security version,
/// which ends every session that is not the one about to be made, and, for the
/// setup that follows a recovery, the end of the wait. The new version.
pub(super) fn finish_enrollment(
    tx: &Transaction,
    user: Id,
    after_recovery: bool,
) -> Result<i64, DbError> {
    tx.execute(
        "UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1,
             \"SecurityRecoveryRequired\" = CASE WHEN ?2 THEN 0 ELSE \"SecurityRecoveryRequired\" END WHERE \"Id\" = ?1",
        params![user, after_recovery],
    )?;
    Ok(tx.query_row(
        "SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
        [user],
        |row| row.get(0),
    )?)
}

/// Moves the account to a new security version, which ends every session but
/// `keep_session` (kept, at the new version; none keeps nothing) and forgets
/// every remembered device. The new version. What a removal of a second step
/// does.
pub(super) fn end_other_sessions(
    tx: &Transaction,
    user: Id,
    keep_session: Option<Id>,
    now: Timestamp,
) -> Result<i64, DbError> {
    tx.execute(
        "UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1 WHERE \"Id\" = ?1",
        [user],
    )?;
    let version: i64 = tx.query_row(
        "SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
        [user],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?3 WHERE \"UserId\" = ?1 AND (?2 IS NULL OR \"Id\" <> ?2) AND \"RevokedAt\" IS NULL",
        params![user, keep_session, now],
    )?;
    super::trusted_devices::revoke_all(tx, user, now)?;
    tx.execute(
        "UPDATE \"UserSessions\" SET \"SecurityVersion\" = ?2 WHERE \"Id\" = ?1",
        params![keep_session, version],
    )?;
    Ok(version)
}

/// What a change of the account's password does, which the route that changes
/// it must call in the same transaction: what [`end_other_sessions`] does, and
/// every desktop handoff that was approved but not used ends too. A change
/// made from a session keeps that session; a reset by mail has none to keep
/// and ends them all. The new version.
pub fn credentials_changed(
    tx: &Transaction,
    user: Id,
    keep_session: Option<Id>,
    now: Timestamp,
) -> Result<i64, DbError> {
    let version = end_other_sessions(tx, user, keep_session, now)?;
    tx.execute(
        "UPDATE \"DesktopBrowserLogins\" SET \"RedeemedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"ApprovedAt\" IS NOT NULL AND \"RedeemedAt\" IS NULL",
        params![user, now],
    )?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::Connection;

    use super::*;
    use crate::auth::fixtures::{add_method, add_session, add_user, events_of, open};

    fn count(connection: &Connection, sql: &str) -> i64 {
        connection.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// An account with everything a recovery ends.
    fn account(connection: &mut Connection) -> Id {
        let user = add_user(connection, Some("maya@example.test"));
        let now = Timestamp::now();
        add_method(connection, user, "totp", None);
        add_method(connection, user, "email", Some("MAYA@EXAMPLE.TEST"));
        add_session(connection, user, now, None);
        add_session(connection, user, now, None);
        let tx = connection.transaction().unwrap();
        recovery_codes::regenerate(&tx, user, now).unwrap();
        crate::auth::trusted_devices::revoke_all(&tx, user, now).unwrap();
        tx.execute(
            "INSERT INTO \"TrustedDevices\" VALUES (?1, ?2, 'h', 'n', 'd', 'i', ?3, ?4, NULL, NULL)",
            params![Id::random(), user, now, now.plus(Duration::from_secs(3600))],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\", \"RedeemedAt\")
             VALUES (?1, 'h', ?2, 1, ?3, ?3, ?3, NULL), (?4, 'h', ?2, 1, ?3, ?3, NULL, NULL)",
            params![Id::random(), user, now, Id::random()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO \"CliRecoveryCredentials\" VALUES (?1, ?2, 'old', 'earlier', 1, ?3, ?4, NULL)",
            params![Id::random(), user, now, now.plus(Duration::from_secs(600))],
        )
        .unwrap();
        tx.commit().unwrap();
        user
    }

    fn recovered(connection: &mut Connection, user: Id, now: Timestamp) -> Recovered {
        let tx = connection.transaction().unwrap();
        let done = recover(
            &tx,
            user,
            "lost phone",
            &SecurityPolicy::default(),
            now,
            |tx| {
                tx.execute(
                    "INSERT INTO \"OperatorAuditEntries\" VALUES (?1, 'cli', 'recover', ?2, ?3)",
                    params![Id::random(), user, now],
                )?;
                Ok(())
            },
        )
        .unwrap()
        .expect("the account is recovered");
        tx.commit().unwrap();
        done
    }

    #[test]
    fn a_recovery_ends_everything_and_leaves_one_credential() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);
        let now = Timestamp::now().plus(Duration::from_secs(5));

        let done = recovered(&mut connection, user, now);

        assert_eq!(
            (
                done.methods_revoked,
                done.trusted_devices_revoked,
                done.sessions_revoked,
                done.handoffs_invalidated
            ),
            (2, 1, 2, 1)
        );
        assert_eq!(done.session_version, 2);
        assert_eq!(done.expires_at, now.plus(Duration::from_secs(600)));
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"RevokedAt\" IS NULL"
            ),
            0
        );
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL"
            ),
            0
        );
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"
            ),
            0
        );
        assert_eq!(recovery_codes::remaining(&connection, user).unwrap(), 0);
        // The handoff that was approved is spent; the one nobody approved is left alone.
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NOT NULL"
            ),
            1
        );
        // The earlier credential is spent, and one new one is stored as a digest.
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"CliRecoveryCredentials\" WHERE \"UsedAt\" IS NULL"
            ),
            1
        );
        let stored: String = connection
            .query_row(
                "SELECT \"TokenHash\" FROM \"CliRecoveryCredentials\" WHERE \"UsedAt\" IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, tokens::hash(&done.credential));
        let (version, waiting): (i64, bool) = connection
            .query_row("SELECT \"SessionVersion\", \"SecurityRecoveryRequired\" FROM \"Users\" WHERE \"Id\" = ?1", [user], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((version, waiting), (2, true));
        assert_eq!(
            events_of(&connection, user, "cli-recovery"),
            vec!["operator recovery: lost phone"]
        );
        assert_eq!(
            count(&connection, "SELECT COUNT(*) FROM \"OperatorAuditEntries\""),
            1
        );
        assert!(!format!("{done:?}").contains(&done.credential));
    }

    #[test]
    fn the_alert_goes_to_the_address_that_was_verified_before_the_methods_went() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);

        recovered(&mut connection, user, Timestamp::now());

        let to: String = connection
            .query_row(
                "SELECT \"EmailAddress\" FROM \"SecurityNotifications\" WHERE \"UserId\" = ?1",
                [user],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(to, "maya@example.test");
    }

    #[test]
    fn a_disabled_or_missing_account_is_not_recovered_and_nothing_is_written() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);
        connection
            .execute("UPDATE \"Users\" SET \"IsDisabled\" = 1", [])
            .unwrap();
        let tx = connection.transaction().unwrap();

        assert_eq!(
            recover(
                &tx,
                user,
                "x",
                &SecurityPolicy::default(),
                Timestamp::now(),
                |_| panic!("no audit for nothing")
            )
            .unwrap(),
            None
        );
        assert_eq!(
            recover(
                &tx,
                Id::random(),
                "x",
                &SecurityPolicy::default(),
                Timestamp::now(),
                |_| panic!("no audit for nothing")
            )
            .unwrap(),
            None
        );
        assert_eq!(
            count(
                &tx,
                "SELECT COUNT(*) FROM \"CliRecoveryCredentials\" WHERE \"UsedAt\" IS NULL"
            ),
            1
        );
        assert_eq!(count(&tx, "SELECT COUNT(*) FROM \"SecurityEvents\""), 0);
    }

    #[test]
    fn a_recovery_that_fails_to_audit_leaves_nothing_behind_once_rolled_back() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);
        let tx = connection.transaction().unwrap();

        let outcome = recover(
            &tx,
            user,
            "x",
            &SecurityPolicy::default(),
            Timestamp::now(),
            |_| Err(DbError::Task("audit".into())),
        );
        assert!(outcome.is_err());
        drop(tx);

        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"RevokedAt\" IS NULL"
            ),
            2
        );
        assert_eq!(
            count(
                &connection,
                "SELECT \"SecurityRecoveryRequired\" FROM \"Users\""
            ),
            0
        );
    }

    #[test]
    fn the_credential_is_redeemed_once_by_the_account_that_waits_for_it() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);
        let now = Timestamp::now();
        let done = recovered(&mut connection, user, now);
        let tx = connection.transaction().unwrap();

        assert_eq!(
            redeem(&tx, &format!("  {}\n", done.credential), now).unwrap(),
            Some(user)
        );
        assert_eq!(redeem(&tx, &done.credential, now).unwrap(), None);
        // The earlier credential, spent by the recovery, opens nothing either.
        assert_eq!(redeem(&tx, "old", now).unwrap(), None);
    }

    #[test]
    fn a_credential_is_void_when_it_is_short_long_wrong_run_out_or_the_account_moved_on() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);
        let now = Timestamp::now();
        let done = recovered(&mut connection, user, now);
        let tx = connection.transaction().unwrap();

        assert_eq!(redeem(&tx, "", now).unwrap(), None);
        assert_eq!(redeem(&tx, &done.credential[..15], now).unwrap(), None);
        assert_eq!(redeem(&tx, &"a".repeat(513), now).unwrap(), None);
        assert_eq!(redeem(&tx, &"a".repeat(43), now).unwrap(), None);
        assert_eq!(
            redeem(&tx, &done.credential, done.expires_at).unwrap(),
            None
        );
        // A change of the account since, a new version, makes it void.
        tx.execute("UPDATE \"Users\" SET \"SessionVersion\" = 9", [])
            .unwrap();
        assert_eq!(redeem(&tx, &done.credential, now).unwrap(), None);
        tx.execute(
            "UPDATE \"Users\" SET \"SessionVersion\" = 2, \"IsDisabled\" = 1",
            [],
        )
        .unwrap();
        assert_eq!(redeem(&tx, &done.credential, now).unwrap(), None);
        // And an account that no longer waits has nothing to redeem it for.
        tx.execute(
            "UPDATE \"Users\" SET \"IsDisabled\" = 0, \"SecurityRecoveryRequired\" = 0",
            [],
        )
        .unwrap();
        assert_eq!(redeem(&tx, &done.credential, now).unwrap(), None);
        tx.execute("UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1", [])
            .unwrap();
        assert_eq!(redeem(&tx, &done.credential, now).unwrap(), Some(user));
    }

    #[test]
    fn finishing_a_setup_moves_the_version_and_ends_the_wait_only_after_a_recovery() {
        let (_folder, mut connection) = open();
        let user = account(&mut connection);
        let done = recovered(&mut connection, user, Timestamp::now());
        let tx = connection.transaction().unwrap();

        assert_eq!(
            finish_enrollment(&tx, user, false).unwrap(),
            done.session_version + 1
        );
        assert_eq!(
            count(&tx, "SELECT \"SecurityRecoveryRequired\" FROM \"Users\""),
            1
        );
        assert_eq!(
            finish_enrollment(&tx, user, true).unwrap(),
            done.session_version + 2
        );
        assert_eq!(
            count(&tx, "SELECT \"SecurityRecoveryRequired\" FROM \"Users\""),
            0
        );
    }
}
