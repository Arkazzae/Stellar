//! What a restore ends. The data of an instance is replaced by a backup's, and
//! what the backup holds is only what stood when it was made: a session that
//! was ended since, a browser that was forgotten, a mailed link that was used,
//! all stand again in the copy. Nothing that can let somebody in, or past a
//! step, may stand on the strength of a row made before the archive.

use rusqlite::{Transaction, params};

use crate::database::DbError;
use crate::values::Timestamp;

/// Ends, in the transaction given, everything of the account-security tables
/// that could let somebody in or past a step on a row that was made before the
/// copy. For a restore only; it reaches the whole instance, so no route calls
/// it.
///
/// Ended, with the time given where a row keeps one:
/// - every running session, and every remembered browser (the second step is
///   asked again);
/// - every mailed code, every password-reset link, every mail-change link and
///   every recovery credential an operator printed, that was not yet used (a
///   link used after the archive was made would otherwise be good again);
/// - the period of every confirmed authenticator app that is now running, so a
///   code shown since cannot be shown to it again.
///
/// Removed, since they only mean something half-way: open sign-in challenges,
/// passkey sign-in ceremonies, desktop sign-ins with their approvals, and the
/// identities an identity provider vouched for that were waiting to be linked.
///
/// Left as the archive holds them, because they are the account's own
/// credentials, as the password is: recovery codes, passkeys, the methods of
/// the second step. `SecurityVersion` is not raised: every row it would end
/// is ended above, directly.
pub(crate) fn end_what_a_copy_may_bring_back(
    tx: &Transaction,
    now: Timestamp,
) -> Result<(), DbError> {
    for statement in [
        "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1 WHERE \"RevokedAt\" IS NULL",
        "UPDATE \"TrustedDevices\" SET \"RevokedAt\" = ?1 WHERE \"RevokedAt\" IS NULL",
        "UPDATE \"EmailOtpCodes\" SET \"ConsumedAt\" = ?1 WHERE \"ConsumedAt\" IS NULL",
        "UPDATE \"PasswordResetTokens\" SET \"UsedAt\" = ?1 WHERE \"UsedAt\" IS NULL",
        "UPDATE \"EmailChangeRequests\" SET \"UsedAt\" = ?1 WHERE \"UsedAt\" IS NULL",
        "UPDATE \"CliRecoveryCredentials\" SET \"UsedAt\" = ?1 WHERE \"UsedAt\" IS NULL",
    ] {
        tx.execute(statement, params![now])?;
    }
    tx.execute(
        "UPDATE \"MfaMethods\" SET \"LastTotpStep\" = ?1
         WHERE \"Type\" = 'totp' AND \"ConfirmedAt\" IS NOT NULL
           AND (\"LastTotpStep\" IS NULL OR \"LastTotpStep\" < ?1)",
        params![now.unix_seconds() / 30],
    )?;
    for statement in [
        "DELETE FROM \"AuthChallenges\"",
        "DELETE FROM \"PasskeySignIns\"",
        "DELETE FROM \"DesktopBrowserLogins\"",
        "DELETE FROM \"PendingExternalIdentities\"",
    ] {
        tx.execute(statement, [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::fixtures::{add_challenge, add_method, add_session, add_user, open};

    fn run(connection: &mut rusqlite::Connection, now: Timestamp) {
        let tx = connection.transaction().unwrap();
        end_what_a_copy_may_bring_back(&tx, now).unwrap();
        tx.commit().unwrap();
    }

    fn count(connection: &rusqlite::Connection, sql: &str) -> i64 {
        connection.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// One row of every table the call reaches, all still good.
    fn seed(connection: &rusqlite::Connection, now: Timestamp) {
        let user = add_user(connection, Some("a@example.test"));
        let later = now.plus(std::time::Duration::from_secs(86_400));
        add_session(connection, user, now, None);
        add_challenge(connection, user, "mfa");
        let id = || crate::values::Id::random();
        let inserts: [(&str, &str); 7] = [
            (
                "TrustedDevices",
                "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, ?2, 'h', 'n', 'd', 'ip', ?3, ?4)",
            ),
            (
                "EmailOtpCodes",
                "INSERT INTO \"EmailOtpCodes\" (\"Id\", \"UserId\", \"Purpose\", \"EmailAddress\", \"ProtectedCode\", \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, ?2, 'p', 'a@example.test', 'c', 0, 5, ?3, ?4)",
            ),
            (
                "PasswordResetTokens",
                "INSERT INTO \"PasswordResetTokens\" (\"Id\", \"UserId\", \"TokenHash\", \"CreatedAt\", \"ExpiresAt\") VALUES (?1, ?2, 'h', ?3, ?4)",
            ),
            (
                "EmailChangeRequests",
                "INSERT INTO \"EmailChangeRequests\" (\"Id\", \"UserId\", \"NewEmail\", \"NormalizedNewEmail\", \"TokenHash\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, ?2, 'b@example.test', 'B@EXAMPLE.TEST', 'h', ?3, ?4)",
            ),
            (
                "CliRecoveryCredentials",
                "INSERT INTO \"CliRecoveryCredentials\" (\"Id\", \"UserId\", \"TokenHash\", \"Reason\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, ?2, 'h', 'r', 1, ?3, ?4)",
            ),
            (
                "DesktopBrowserLogins",
                "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, 'h', ?2, 1, ?3, ?4)",
            ),
            (
                "PasskeySignIns",
                "INSERT INTO \"PasskeySignIns\" (\"Id\", \"SecretHash\", \"Options\", \"CreatedAt\", \"ExpiresAt\") VALUES (?1, 'h', '{}', ?3, ?4)",
            ),
        ];
        for (table, sql) in inserts {
            let user_param = if sql.contains("?2") { Some(user) } else { None };
            match user_param {
                Some(user) => connection.execute(sql, params![id(), user, now, later]),
                None => connection.execute(
                    &sql.replace("?3", "?2").replace("?4", "?3"),
                    params![id(), now, later],
                ),
            }
            .unwrap_or_else(|error| panic!("{table}: {error}"));
        }
        connection
            .execute(
                "INSERT INTO \"PendingExternalIdentities\" (\"Id\", \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"EmailVerified\", \"Amr\", \"ShortLived\", \"CreatedAt\", \"ExpiresAt\")
                 VALUES (?1, 'h', 'p', 'i', 's', 0, '[]', 0, ?2, ?3)",
                params![id(), now, later],
            )
            .unwrap();
    }

    #[test]
    fn nothing_that_could_let_somebody_in_stands_after_a_copy_is_put_back() {
        let (_folder, mut connection) = open();
        let now = Timestamp::now();
        seed(&connection, now);
        run(&mut connection, now);

        for (what, sql) in [
            (
                "a running session",
                "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL",
            ),
            (
                "a remembered browser",
                "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL",
            ),
            (
                "a mailed code",
                "SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"ConsumedAt\" IS NULL",
            ),
            (
                "a reset link",
                "SELECT COUNT(*) FROM \"PasswordResetTokens\" WHERE \"UsedAt\" IS NULL",
            ),
            (
                "a mail-change link",
                "SELECT COUNT(*) FROM \"EmailChangeRequests\" WHERE \"UsedAt\" IS NULL",
            ),
            (
                "a recovery credential",
                "SELECT COUNT(*) FROM \"CliRecoveryCredentials\" WHERE \"UsedAt\" IS NULL",
            ),
            ("a challenge", "SELECT COUNT(*) FROM \"AuthChallenges\""),
            (
                "a passkey ceremony",
                "SELECT COUNT(*) FROM \"PasskeySignIns\"",
            ),
            (
                "a desktop sign-in",
                "SELECT COUNT(*) FROM \"DesktopBrowserLogins\"",
            ),
            (
                "a pending identity",
                "SELECT COUNT(*) FROM \"PendingExternalIdentities\"",
            ),
        ] {
            assert_eq!(count(&connection, sql), 0, "{what}");
        }
        // The rows that were ended are kept, with the time of the restore.
        assert_eq!(
            count(&connection, "SELECT COUNT(*) FROM \"UserSessions\""),
            1
        );
        assert_eq!(
            count(&connection, "SELECT COUNT(*) FROM \"TrustedDevices\""),
            1
        );
    }

    #[test]
    fn rows_that_were_ended_before_keep_their_time_and_the_accounts_own_credentials_stand() {
        let (_folder, mut connection) = open();
        // Start two seconds before a TOTP period boundary.
        let now = Timestamp::from_unix_seconds(1_800_000_028).unwrap();
        let earlier = now.minus(std::time::Duration::from_secs(3600));
        let user = add_user(&connection, None);
        let ended = add_session(&connection, user, now, None);
        connection
            .execute(
                "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
                params![ended, earlier],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"RecoveryCodes\" (\"Id\", \"UserId\", \"BatchId\", \"CodeHash\", \"CreatedAt\")
                 VALUES (?1, ?2, ?1, 'h', ?3)",
                params![crate::values::Id::random(), user, now],
            )
            .unwrap();
        let app = add_method(&connection, user, "totp", None);
        let mail = add_method(&connection, user, "email", Some("a@example.test"));
        connection
            .execute(
                "UPDATE \"MfaMethods\" SET \"LastTotpStep\" = 5 WHERE \"Id\" = ?1",
                [app],
            )
            .unwrap();
        run(&mut connection, now);

        let revoked: Option<Timestamp> = connection
            .query_row(
                "SELECT \"RevokedAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
                [ended],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revoked, Some(earlier));
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UsedAt\" IS NULL"
            ),
            1
        );
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"RevokedAt\" IS NULL"
            ),
            2
        );
        // An app's period moves up to this one; a mail method has none.
        fn step(connection: &rusqlite::Connection, id: crate::values::Id) -> Option<i64> {
            connection
                .query_row(
                    "SELECT \"LastTotpStep\" FROM \"MfaMethods\" WHERE \"Id\" = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap()
        }
        assert_eq!(step(&connection, app), Some(now.unix_seconds() / 30));
        assert_eq!(step(&connection, mail), None);
        // Restoring again in the same period leaves the step alone.
        run(&mut connection, now.plus(std::time::Duration::from_secs(1)));
        assert_eq!(step(&connection, app), Some(now.unix_seconds() / 30));
        let next = now.plus(std::time::Duration::from_secs(2));
        run(&mut connection, next);
        assert_eq!(step(&connection, app), Some(next.unix_seconds() / 30));
    }
}
