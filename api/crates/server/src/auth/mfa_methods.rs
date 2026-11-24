//! The second steps an account has, as rows of `MfaMethods`: authenticator
//! apps, codes sent by mail and passkeys. This module keeps the rows and the
//! authenticator app's secret; checking a code against them is `mfa`.
//!
//! The secret of an authenticator app is sealed for the account and the method
//! it belongs to, so a copy of the column moved to another row opens as
//! nothing. A value sealed by the first server's data protection cannot be
//! opened here: such a method answers every code as wrong, and its owner signs
//! in with one of their other steps or a recovery code, and sets the app up
//! again.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::email_codes;
use super::totp;
use crate::database::DbError;
use crate::seal::Sealer;
use crate::values::{Id, Timestamp};

pub const TOTP: &str = "totp";
pub const EMAIL: &str = "email";
pub const PASSKEY: &str = "passkey";

/// The longest name a person may give a method.
pub const MAX_NAME_LENGTH: usize = 64;

const TOTP_PURPOSE: &str = "totp-secret";

fn totp_aad(user: Id, method: Id) -> Vec<u8> {
    format!("{user}:{method}").into_bytes()
}

/// A method row.
#[derive(Clone)]
pub struct Method {
    pub id: Id,
    pub user: Id,
    pub kind: String,
    pub name: String,
    pub confirmed_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
    /// The address an e-mail method sends to, in capitals.
    pub email_address: Option<String>,
    pub last_totp_step: Option<i64>,
    sealed_secret: Option<String>,
}

/// What the method is, never what protects it.
impl std::fmt::Debug for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Method")
            .field("id", &self.id)
            .field("user", &self.user)
            .field("kind", &self.kind)
            .field("confirmed_at", &self.confirmed_at)
            .field("revoked_at", &self.revoked_at)
            .finish_non_exhaustive()
    }
}

const COLUMNS: &str = "\"Id\", \"UserId\", \"Type\", \"Name\", \"ConfirmedAt\", \"RevokedAt\", \"EmailAddress\", \"LastTotpStep\", \"ProtectedSecret\"";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Method> {
    Ok(Method {
        id: row.get(0)?,
        user: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        confirmed_at: row.get(4)?,
        revoked_at: row.get(5)?,
        email_address: row.get(6)?,
        last_totp_step: row.get(7)?,
        sealed_secret: row.get(8)?,
    })
}

/// A method of the account that is confirmed and not revoked, of any type.
pub fn find_active(
    connection: &Connection,
    user: Id,
    method: Id,
) -> rusqlite::Result<Option<Method>> {
    connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM \"MfaMethods\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL"
            ),
            params![method, user],
            from_row,
        )
        .optional()
}

/// A method of the account that is not revoked and has not been confirmed yet.
pub fn find_pending(
    connection: &Connection,
    user: Id,
    method: Id,
    kind: &str,
) -> rusqlite::Result<Option<Method>> {
    connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM \"MfaMethods\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"Type\" = ?3 AND \"ConfirmedAt\" IS NULL AND \"RevokedAt\" IS NULL"
            ),
            params![method, user, kind],
            from_row,
        )
        .optional()
}

/// How many other methods of the account are confirmed and in use.
pub fn count_active_besides(
    connection: &Connection,
    user: Id,
    method: Id,
) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"Id\" <> ?2 AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL",
        params![user, method],
        |row| row.get(0),
    )
}

/// Begins setting up an authenticator app: a method that is not confirmed
/// yet, with a new secret sealed for it. The secret comes back to be shown to
/// the person, once.
pub fn start_totp(
    tx: &Transaction,
    sealer: &Sealer,
    user: Id,
    name: &str,
    now: Timestamp,
) -> Result<(Id, String), DbError> {
    let id = Id::random();
    let secret = totp::new_secret();
    let sealed = sealer.seal(TOTP_PURPOSE, &totp_aad(user, id), None, secret.as_bytes());
    tx.execute(
        "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"LastUsedAt\",
             \"ProtectedSecret\", \"EmailAddress\", \"LastTotpStep\", \"RevokedAt\")
         VALUES (?1, ?2, 'totp', ?3, ?4, NULL, NULL, ?5, NULL, NULL, NULL)",
        params![id, user, name, now, sealed],
    )?;
    Ok((id, secret))
}

/// The step the code is for, if the method is an authenticator app whose
/// secret opens and the code is one of its codes for now or the steps next to
/// it. The step is not spent.
pub fn totp_step_of(sealer: &Sealer, method: &Method, code: &str, now: Timestamp) -> Option<i64> {
    if method.kind != TOTP {
        return None;
    }
    let sealed = method.sealed_secret.as_deref()?;
    let secret = sealer
        .open(TOTP_PURPOSE, &totp_aad(method.user, method.id), sealed, now)
        .ok()?;
    totp::verify(std::str::from_utf8(&secret).ok()?, code, now)
}

/// Spends a step of an authenticator app: false when it, or a later one, was
/// spent before, so a code is never accepted twice, and when the method was
/// revoked or (unless `allow_pending`) was never confirmed.
pub fn spend_totp_step(
    tx: &Transaction,
    user: Id,
    method: Id,
    step: i64,
    now: Timestamp,
    allow_pending: bool,
) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"MfaMethods\" SET \"LastTotpStep\" = ?3, \"LastUsedAt\" = ?4
         WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL AND (\"LastTotpStep\" IS NULL OR \"LastTotpStep\" < ?3)
           AND (?5 OR \"ConfirmedAt\" IS NOT NULL)",
        params![method, user, step, now, allow_pending],
    )?;
    Ok(changed == 1)
}

/// Finishes setting up an authenticator app with the first code that was
/// right: confirms the method, names it, and spends the step. False when the
/// method is not a pending one of the account, or the step was spent.
pub fn confirm_totp(
    tx: &Transaction,
    user: Id,
    method: Id,
    step: i64,
    name: &str,
    now: Timestamp,
) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"MfaMethods\" SET \"ConfirmedAt\" = ?4, \"Name\" = ?5, \"LastTotpStep\" = ?3, \"LastUsedAt\" = ?4
         WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"Type\" = 'totp' AND \"ConfirmedAt\" IS NULL AND \"RevokedAt\" IS NULL
           AND (\"LastTotpStep\" IS NULL OR \"LastTotpStep\" < ?3)",
        params![method, user, step, now, name],
    )?;
    Ok(changed == 1)
}

/// Adds the mail address of the account as a second step, which the person
/// has just shown they can read. False, and nothing is added, when the account
/// already has one for the address.
pub fn add_email(
    tx: &Transaction,
    user: Id,
    normalized_address: &str,
    name: &str,
    now: Timestamp,
) -> Result<bool, DbError> {
    let existing: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"Type\" = 'email' AND \"EmailAddress\" = ?2 AND \"RevokedAt\" IS NULL)",
        params![user, normalized_address],
        |row| row.get(0),
    )?;
    if existing {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"LastUsedAt\",
             \"ProtectedSecret\", \"EmailAddress\", \"LastTotpStep\", \"RevokedAt\")
         VALUES (?1, ?2, 'email', ?3, ?4, ?4, ?4, NULL, ?5, NULL, NULL)",
        params![Id::random(), user, name, now, normalized_address],
    )?;
    Ok(true)
}

/// Whether the account has an e-mail method for the address, in use or being
/// set up.
pub fn has_email(
    connection: &Connection,
    user: Id,
    normalized_address: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"Type\" = 'email' AND \"EmailAddress\" = ?2 AND \"RevokedAt\" IS NULL)",
        params![user, normalized_address],
        |row| row.get(0),
    )
}

/// Takes a method back. False when the account has no such method in use.
pub fn revoke(tx: &Transaction, user: Id, method: Id, now: Timestamp) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?3 WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL",
        params![method, user, now],
    )?;
    Ok(changed == 1)
}

/// The mail method whose address is no longer the account's: it is taken back,
/// and the codes that were sent for it stop working. Only done when the
/// method really is stale (checked here, inside the transaction that decides
/// it), and never for a method of another account.
pub fn revoke_if_address_changed(
    tx: &Transaction,
    user: Id,
    method: Id,
    now: Timestamp,
) -> Result<bool, DbError> {
    let current: Option<(Option<String>, Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT m.\"EmailAddress\", u.\"NormalizedEmail\", u.\"Email\"
             FROM \"MfaMethods\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\" WHERE m.\"Id\" = ?1 AND m.\"UserId\" = ?2",
            params![method, user],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let stale = current.map(|(bound, normalized, email)| {
        bound != normalized || email.is_none_or(|email| email.trim().is_empty())
    });
    if stale != Some(true) {
        return Ok(false);
    }
    revoke(tx, user, method, now)?;
    email_codes::consume_for_method(tx, user, method, now)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::auth::fixtures::{add_method, add_pending_method, add_user, open};

    fn sealer() -> Sealer {
        Sealer::from_key([3; 32])
    }

    fn pending_totp(connection: &mut Connection, user: Id) -> (Id, String) {
        let tx = connection.transaction().unwrap();
        let started = start_totp(&tx, &sealer(), user, "Phone", Timestamp::now()).unwrap();
        tx.commit().unwrap();
        started
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds).unwrap()
    }

    fn pending(connection: &Connection, user: Id, id: Id) -> Method {
        find_pending(connection, user, id, TOTP).unwrap().unwrap()
    }

    #[test]
    fn an_authenticator_app_starts_as_a_pending_method_with_a_sealed_secret() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);

        let (id, secret) = pending_totp(&mut connection, user);

        assert_eq!(secret.len(), 32);
        let method = pending(&connection, user, id);
        assert_eq!(
            (method.kind.as_str(), method.name.as_str()),
            ("totp", "Phone")
        );
        assert_eq!(method.confirmed_at, None);
        let stored: String = connection
            .query_row(
                "SELECT \"ProtectedSecret\" FROM \"MfaMethods\" WHERE \"Id\" = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!stored.contains(&secret));
        assert_eq!(
            sealer()
                .open(TOTP_PURPOSE, &totp_aad(user, id), &stored, Timestamp::now())
                .unwrap(),
            secret.as_bytes()
        );
        assert!(!format!("{method:?}").contains(&stored));
        // A pending method is not one in use, and not another account's.
        assert!(find_active(&connection, user, id).unwrap().is_none());
        assert!(
            find_pending(&connection, add_user(&connection, None), id, TOTP)
                .unwrap()
                .is_none()
        );
        assert!(
            find_pending(&connection, user, id, EMAIL)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_code_is_read_against_the_secret_sealed_for_that_very_method() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (id, secret) = pending_totp(&mut connection, user);
        let (other, _) = pending_totp(&mut connection, user);
        let now = at(1_800_000_000);
        let code = totp::code_at(&secret, now).unwrap();
        let method = pending(&connection, user, id);

        assert_eq!(
            totp_step_of(&sealer(), &method, &code, now),
            Some(1_800_000_000 / 30)
        );
        assert_eq!(
            totp_step_of(&sealer(), &method, "000000", now).filter(|_| code != "000000"),
            None
        );
        // The secret of one method put in the row of another does not open there.
        let moved = Method {
            id: other,
            ..method.clone()
        };
        assert_eq!(totp_step_of(&sealer(), &moved, &code, now), None);
        let theirs = Method {
            user: Id::random(),
            ..method.clone()
        };
        assert_eq!(totp_step_of(&sealer(), &theirs, &code, now), None);
        // A method that is not an authenticator app, or has no secret, or a
        // secret that another server sealed, has no codes.
        assert_eq!(
            totp_step_of(
                &sealer(),
                &Method {
                    kind: EMAIL.into(),
                    ..method.clone()
                },
                &code,
                now
            ),
            None
        );
        assert_eq!(
            totp_step_of(
                &sealer(),
                &Method {
                    sealed_secret: None,
                    ..method.clone()
                },
                &code,
                now
            ),
            None
        );
        let foreign = Method {
            sealed_secret: Some("CfDJ8-protected-by-the-first-server".into()),
            ..method.clone()
        };
        assert_eq!(totp_step_of(&sealer(), &foreign, &code, now), None);
        assert_eq!(
            totp_step_of(&Sealer::from_key([4; 32]), &method, &code, now),
            None
        );
    }

    #[test]
    fn a_step_is_spent_once_and_never_again_or_before_one_already_spent() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let method = add_method(&connection, user, TOTP, None);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();

        assert!(spend_totp_step(&tx, user, method, 100, now, false).unwrap());
        assert!(!spend_totp_step(&tx, user, method, 100, now, false).unwrap());
        assert!(!spend_totp_step(&tx, user, method, 99, now, false).unwrap());
        assert!(spend_totp_step(&tx, user, method, 101, now, false).unwrap());
        let (step, used): (i64, Option<Timestamp>) = tx
            .query_row(
                "SELECT \"LastTotpStep\", \"LastUsedAt\" FROM \"MfaMethods\" WHERE \"Id\" = ?1",
                [method],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((step, used), (101, Some(now)));
    }

    #[test]
    fn a_step_is_not_spent_on_a_method_that_is_revoked_pending_or_another_accounts() {
        let (_folder, connection) = open();
        let mut connection = connection;
        let user = add_user(&connection, None);
        let revoked = add_method(&connection, user, TOTP, None);
        let waiting = add_pending_method(&connection, user, TOTP);
        let theirs = add_method(&connection, add_user(&connection, None), TOTP, None);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        assert!(revoke(&tx, user, revoked, now).unwrap());

        assert!(!spend_totp_step(&tx, user, revoked, 5, now, false).unwrap());
        assert!(!spend_totp_step(&tx, user, waiting, 5, now, false).unwrap());
        assert!(!spend_totp_step(&tx, user, theirs, 5, now, false).unwrap());
        // Setting up is the one time an unconfirmed method takes a step.
        assert!(spend_totp_step(&tx, user, waiting, 5, now, true).unwrap());
        assert!(!spend_totp_step(&tx, user, revoked, 6, now, true).unwrap());
    }

    #[test]
    fn the_first_right_code_confirms_a_pending_method_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (id, _) = pending_totp(&mut connection, user);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();

        assert!(!confirm_totp(&tx, Id::random(), id, 7, "Mine", now).unwrap());
        assert!(confirm_totp(&tx, user, id, 7, "Mine", now).unwrap());
        assert!(!confirm_totp(&tx, user, id, 8, "Mine", now).unwrap());
        tx.commit().unwrap();

        let method = find_active(&connection, user, id).unwrap().unwrap();
        assert_eq!(
            (method.name.as_str(), method.last_totp_step),
            ("Mine", Some(7))
        );
        assert_eq!(method.confirmed_at, Some(now));
    }

    #[test]
    fn a_pending_method_taken_back_is_not_confirmed() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (id, _) = pending_totp(&mut connection, user);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        assert!(revoke(&tx, user, id, now).unwrap());
        assert!(!revoke(&tx, user, id, now).unwrap());

        assert!(!confirm_totp(&tx, user, id, 7, "Mine", now).unwrap());
    }

    #[test]
    fn an_address_is_added_as_a_step_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some("maya@example.test"));
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();

        assert!(add_email(&tx, user, "MAYA@EXAMPLE.TEST", "Mail", now).unwrap());
        assert!(!add_email(&tx, user, "MAYA@EXAMPLE.TEST", "Again", now).unwrap());
        tx.commit().unwrap();

        assert!(has_email(&connection, user, "MAYA@EXAMPLE.TEST").unwrap());
        assert!(!has_email(&connection, user, "OTHER@EXAMPLE.TEST").unwrap());
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"UserId\" = ?1",
                [user],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        // Taken back, the address can be added again.
        let id: Id = connection
            .query_row(
                "SELECT \"Id\" FROM \"MfaMethods\" WHERE \"UserId\" = ?1",
                [user],
                |r| r.get(0),
            )
            .unwrap();
        let tx = connection.transaction().unwrap();
        assert!(revoke(&tx, user, id, now).unwrap());
        assert!(add_email(&tx, user, "MAYA@EXAMPLE.TEST", "Mail", now).unwrap());
    }

    #[test]
    fn methods_in_use_are_counted_apart_from_the_one_asked_about() {
        let (_folder, connection) = open();
        let user = add_user(&connection, None);
        let first = add_method(&connection, user, TOTP, None);
        let second = add_method(&connection, user, EMAIL, Some("A@B.C"));
        add_pending_method(&connection, user, TOTP);
        add_method(&connection, add_user(&connection, None), TOTP, None);

        assert_eq!(count_active_besides(&connection, user, first).unwrap(), 1);
        assert_eq!(count_active_besides(&connection, user, second).unwrap(), 1);
        assert_eq!(
            count_active_besides(&connection, user, Id::random()).unwrap(),
            2
        );
    }

    #[test]
    fn a_mail_method_whose_address_is_no_longer_the_accounts_is_taken_back_with_its_codes() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some("maya@example.test"));
        let bound = add_method(&connection, user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
        let now = Timestamp::now();
        connection
            .execute(
                "INSERT INTO \"EmailOtpCodes\" VALUES (?1, ?2, ?3, ?4, 'login', 'a', 'x', 0, 5, ?5, ?5, NULL, ?5)",
                params![Id::random(), user, bound, Id::random(), now],
            )
            .unwrap();
        let tx = connection.transaction().unwrap();

        // Still the address: nothing happens.
        assert!(!revoke_if_address_changed(&tx, user, bound, now).unwrap());
        assert!(!revoke_if_address_changed(&tx, Id::random(), bound, now).unwrap());
        tx.execute("UPDATE \"Users\" SET \"Email\" = 'new@example.test', \"NormalizedEmail\" = 'NEW@EXAMPLE.TEST' WHERE \"Id\" = ?1", [user]).unwrap();
        // Another account's method is not touched by this one's request.
        assert!(!revoke_if_address_changed(&tx, add_user(&tx, None), bound, now).unwrap());
        assert!(revoke_if_address_changed(&tx, user, bound, now).unwrap());
        tx.commit().unwrap();

        assert!(find_active(&connection, user, bound).unwrap().is_none());
        let unspent: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"ConsumedAt\" IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unspent, 0);
    }

    #[test]
    fn a_mail_method_of_an_account_that_lost_its_address_is_stale_too() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some("maya@example.test"));
        let bound = add_method(&connection, user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
        let now = Timestamp::now().plus(Duration::from_secs(1));
        let tx = connection.transaction().unwrap();
        tx.execute(
            "UPDATE \"Users\" SET \"Email\" = '   ' WHERE \"Id\" = ?1",
            [user],
        )
        .unwrap();

        assert!(revoke_if_address_changed(&tx, user, bound, now).unwrap());
    }
}
