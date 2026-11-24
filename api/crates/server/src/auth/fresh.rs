//! A second step passed a moment ago, for the changes that could hand an
//! account, or the whole instance, to somebody else.
//!
//! An account that has a second step must have passed it, in this very
//! session, within [`FRESH_SECOND_STEP_WINDOW`]. A session that is only signed
//! in, or got in on a remembered device, does not count: a remembered device
//! is never recorded as the session having passed a second step. An account
//! with no second step has nothing to prove and is always fresh.
//!
//! A route asks for it with `Endpoints::fresh_second_step()`; a handler that
//! asks only sometimes, or in another way, calls the functions here and
//! answers with [`refusal`].

use rusqlite::{Connection, OptionalExtension, params};
use stellarr_messages::{MessageKey, Messages, MissingValue};

use super::policy::{FRESH_SECOND_STEP_WINDOW, has_confirmed_method};
use crate::http::Problem;
use crate::instance::access;
use crate::instance::users::User;
use crate::values::{Id, Timestamp};

/// The `code` of the problem a refused change is answered with, which the
/// clients answer by asking for the proof.
pub const CODE: &str = "fresh-mfa-required";

/// Whether the session may make a sensitive change now.
pub fn has_fresh_second_step(
    connection: &Connection,
    user: Id,
    session: Option<Id>,
    now: Timestamp,
) -> rusqlite::Result<bool> {
    if !has_confirmed_method(connection, user)? {
        return Ok(true);
    }
    let Some(session) = session else {
        return Ok(false);
    };
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL
                        AND \"MfaCompletedAt\" IS NOT NULL AND \"MfaCompletedAt\" >= ?3)",
        params![session, user, now.minus(FRESH_SECOND_STEP_WINDOW)],
        |row| row.get(0),
    )
}

/// [`has_fresh_second_step`] for a change on one server: only the instance's
/// own server, whose roles reach the operator's panel, asks for it. Other
/// servers keep their ordinary role management.
pub fn has_fresh_second_step_for_server(
    connection: &Connection,
    user: Id,
    session: Option<Id>,
    server: Id,
    now: Timestamp,
) -> rusqlite::Result<bool> {
    if access::instance_server(connection)? != Some(server) {
        return Ok(true);
    }
    has_fresh_second_step(connection, user, session, now)
}

/// The 403 that refuses a change for want of a fresh proof.
pub fn refusal(messages: &Messages) -> Result<Problem, MissingValue> {
    Ok(Problem::new(axum::http::StatusCode::FORBIDDEN)
        .title(messages.get(MessageKey::FreshSecondStepRequired, &[])?)
        .with("code", CODE))
}

/// Whether a session may change the account's security settings (add or take
/// away a second step, a password, a way in).
///
/// A disabled account, one waiting for an operator's recovery, and a session
/// that is gone, ended, out of date with the account or not the account's
/// may not. An account with a second step needs it passed in the last few
/// minutes. One without needs a sign-in in the last few minutes, or its
/// password: `password_proves` is called only then, since checking a password
/// costs time and a stolen session must not be a way to guess one at an
/// account that has a second step.
pub fn can_change_security(
    connection: &Connection,
    user: &User,
    session: Option<Id>,
    password_proves: impl FnOnce() -> bool,
    now: Timestamp,
) -> rusqlite::Result<bool> {
    if user.is_disabled || user.security_recovery_required {
        return Ok(false);
    }
    let Some(session) = session else {
        return Ok(false);
    };
    let since = now.minus(FRESH_SECOND_STEP_WINDOW);
    let current: Option<(Timestamp, Option<Timestamp>)> = connection
        .query_row(
            "SELECT COALESCE(\"AuthenticatedAt\", \"CreatedAt\"), \"MfaCompletedAt\" FROM \"UserSessions\"
             WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?3
               AND \"AbsoluteExpiresAt\" > ?3 AND \"SecurityVersion\" = ?4",
            params![session, user.id, now, user.session_version],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((signed_in_at, second_step_at)) = current else {
        return Ok(false);
    };
    if has_confirmed_method(connection, user.id)? {
        return Ok(second_step_at.is_some_and(|at| at >= since));
    }
    Ok(signed_in_at >= since || password_proves())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stellarr_messages::Language;

    use super::*;
    use crate::auth::fixtures::{add_instance_server, add_method, add_session, add_user, open};

    fn ago(now: Timestamp, seconds: u64) -> Timestamp {
        now.minus(Duration::from_secs(seconds))
    }

    #[test]
    fn an_account_without_a_second_step_is_always_fresh() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let session = add_session(&connection, user, ago(now, 86_400), None);

        assert!(has_fresh_second_step(&connection, user, Some(session), now).unwrap());
        assert!(has_fresh_second_step(&connection, user, None, now).unwrap());
    }

    #[test]
    fn with_a_second_step_only_one_passed_in_this_session_in_the_last_five_minutes_is_fresh() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        add_method(&connection, user, "totp", None);
        let now = Timestamp::now();
        let fresh = add_session(&connection, user, ago(now, 3600), Some(ago(now, 299)));
        let edge = add_session(&connection, user, ago(now, 3600), Some(ago(now, 300)));
        let stale = add_session(&connection, user, ago(now, 3600), Some(ago(now, 301)));
        let signed_in_only = add_session(&connection, user, now, None);
        let other = add_user(&connection, None);
        let theirs = add_session(&connection, other, now, Some(now));

        assert!(has_fresh_second_step(&connection, user, Some(fresh), now).unwrap());
        assert!(has_fresh_second_step(&connection, user, Some(edge), now).unwrap());
        assert!(!has_fresh_second_step(&connection, user, Some(stale), now).unwrap());
        assert!(!has_fresh_second_step(&connection, user, Some(signed_in_only), now).unwrap());
        assert!(!has_fresh_second_step(&connection, user, None, now).unwrap());
        assert!(!has_fresh_second_step(&connection, user, Some(theirs), now).unwrap());
        assert!(!has_fresh_second_step(&connection, user, Some(Id::random()), now).unwrap());
        connection
            .execute(
                "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1 WHERE \"Id\" = ?2",
                params![now, fresh],
            )
            .unwrap();
        assert!(!has_fresh_second_step(&connection, user, Some(fresh), now).unwrap());
    }

    #[test]
    fn a_method_taken_back_or_not_confirmed_is_not_one_to_ask_for() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let method = add_method(&connection, user, "totp", None);
        let now = Timestamp::now();
        let session = add_session(&connection, user, ago(now, 3600), None);
        assert!(!has_fresh_second_step(&connection, user, Some(session), now).unwrap());

        connection
            .execute("UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?1", [now])
            .unwrap();
        assert!(has_fresh_second_step(&connection, user, Some(session), now).unwrap());
        connection.execute("UPDATE \"MfaMethods\" SET \"RevokedAt\" = NULL, \"ConfirmedAt\" = NULL WHERE \"Id\" = ?1", [method]).unwrap();
        assert!(has_fresh_second_step(&connection, user, Some(session), now).unwrap());
    }

    #[test]
    fn only_the_instance_server_asks() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        add_method(&connection, user, "totp", None);
        let now = Timestamp::now();
        let session = add_session(&connection, user, ago(now, 3600), None);
        let instance = add_instance_server(&connection);

        assert!(
            !has_fresh_second_step_for_server(&connection, user, Some(session), instance, now)
                .unwrap()
        );
        assert!(
            has_fresh_second_step_for_server(&connection, user, Some(session), Id::random(), now)
                .unwrap()
        );
    }

    #[test]
    fn the_refusal_is_a_403_with_the_code_the_clients_look_for() {
        let english =
            serde_json::to_string(&refusal(&Messages::new(Language::English)).unwrap()).unwrap();
        let polish =
            serde_json::to_string(&refusal(&Messages::new(Language::Polish)).unwrap()).unwrap();

        assert!(english.starts_with(
            "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.4\",\"title\":\"This needs a second step from the last 5 minutes. Confirm it and try again.\",\"status\":403,\"code\":\"fresh-mfa-required\",\"traceId\""
        ), "{english}");
        assert!(
            polish.contains("Ta operacja wymaga drugiego kroku sprzed najwyżej 5 minut."),
            "{polish}"
        );
    }

    fn account(connection: &Connection, user: Id) -> User {
        User::find(connection, user).unwrap().unwrap()
    }

    #[test]
    fn security_settings_of_an_account_with_a_second_step_need_it_passed_lately_and_never_the_password()
     {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        add_method(&connection, user, "totp", None);
        let now = Timestamp::now();
        let fresh = add_session(&connection, user, ago(now, 60), Some(ago(now, 60)));
        let signed_in_lately = add_session(&connection, user, ago(now, 10), None);
        let stale = add_session(&connection, user, ago(now, 3600), Some(ago(now, 3600)));
        let user = account(&connection, user);
        let never = || panic!("a password is not asked about when there is a second step");

        assert!(can_change_security(&connection, &user, Some(fresh), never, now).unwrap());
        assert!(
            !can_change_security(&connection, &user, Some(signed_in_lately), never, now).unwrap()
        );
        assert!(!can_change_security(&connection, &user, Some(stale), never, now).unwrap());
        assert!(!can_change_security(&connection, &user, None, never, now).unwrap());
    }

    #[test]
    fn security_settings_of_an_account_without_one_need_a_recent_sign_in_or_the_password() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let recent = add_session(&connection, user, ago(now, 120), None);
        let old = add_session(&connection, user, ago(now, 3600), None);
        let user = account(&connection, user);

        assert!(can_change_security(&connection, &user, Some(recent), || false, now).unwrap());
        assert!(!can_change_security(&connection, &user, Some(old), || false, now).unwrap());
        assert!(can_change_security(&connection, &user, Some(old), || true, now).unwrap());
        // Without a session of the account there is nothing to be fresh in.
        assert!(
            !can_change_security(&connection, &user, Some(Id::random()), || true, now).unwrap()
        );
    }

    #[test]
    fn a_session_that_is_not_in_good_standing_changes_nothing() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let session = add_session(&connection, user, ago(now, 10), None);
        let ok = |connection: &Connection| {
            can_change_security(
                connection,
                &account(connection, user),
                Some(session),
                || true,
                now,
            )
            .unwrap()
        };
        assert!(ok(&connection));

        for (statement, why) in [
            ("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1", "revoked"),
            ("UPDATE \"UserSessions\" SET \"ExpiresAt\" = ?1", "ended"),
            (
                "UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = ?1",
                "past its absolute end",
            ),
            (
                "UPDATE \"UserSessions\" SET \"SecurityVersion\" = 0 WHERE ?1 IS NOT NULL",
                "of an older version of the account",
            ),
            (
                "UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE ?1 IS NOT NULL",
                "of a disabled account",
            ),
            (
                "UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1 WHERE ?1 IS NOT NULL",
                "of an account waiting for recovery",
            ),
        ] {
            let copy = connection
                .execute("SAVEPOINT s", [])
                .and_then(|_| connection.execute(statement, [now]));
            copy.unwrap();
            assert!(!ok(&connection), "{why}");
            connection.execute("ROLLBACK TO s", []).unwrap();
            connection.execute("RELEASE s", []).unwrap();
        }
        assert!(ok(&connection));
    }
}
