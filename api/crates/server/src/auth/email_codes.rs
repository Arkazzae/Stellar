//! Codes sent by mail as a second step: six digits, valid for a few minutes,
//! few guesses, and a limit on how many an account may ask for.
//!
//! A code is sent in three steps, because a mail is never sent inside a
//! transaction: [`reserve`] writes the row, and the code to put in the mail,
//! and the caller commits; the mail is sent; then [`mark_delivered`] or
//! [`mark_failed`]. A row that was never marked delivered cannot be checked,
//! and one that failed still counts towards the cooldown and the hourly
//! limit, so that a failing mail server cannot be used to get round them.
//!
//! The code is stored sealed for the account and the purpose it was asked for.
//! A row written by the first server was protected with that server's own
//! keys and does not open here: its code always reads as wrong, it uses up
//! its guesses, and the person asks for a new one, which this module writes.

use rusqlite::{OptionalExtension, Transaction, params};

use super::policy::SecurityPolicy;
use super::tokens;
use crate::database::DbError;
use crate::instance::names;
use crate::seal::Sealer;
use crate::values::{Id, Timestamp};

/// A code that lets a person through the second step of a sign-in.
pub const LOGIN: &str = "login";
/// A code that proves a person can read their address, to add it as a step.
pub const ENROLLMENT: &str = "enrollment";

const PURPOSE: &str = "email-code";

fn aad(user: Id, purpose: &str) -> Vec<u8> {
    format!("{user}:{purpose}").into_bytes()
}

/// Which code is meant: whom it is for, what for, and where it goes.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    pub user: Id,
    /// The sign-in or enrolment that asked for it.
    pub challenge: Id,
    pub purpose: &'a str,
    /// The method a sign-in code is for; none for the code of an enrolment.
    pub method: Option<Id>,
    /// The address the person is told it goes to.
    pub address: &'a str,
}

/// What asking for a code came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Reservation {
    /// The account has no address, or not the one that was asked for.
    Unavailable,
    /// The last code was asked for too recently; the seconds to wait.
    Cooldown { retry_after_seconds: i64 },
    /// The account has asked for as many as it may this hour.
    Limit,
    /// A code was made, and is to be mailed.
    Reserved(Reserved),
}

/// A code to put in a mail.
#[derive(PartialEq, Eq)]
pub struct Reserved {
    /// The row. The mail names itself by it, so that a retry of the mail is
    /// recognised as the same one.
    pub id: Id,
    /// The address to send to, as the account has it.
    pub recipient: String,
    /// The digits.
    pub code: String,
    /// How long the mail says the code is good for.
    pub lifetime_minutes: i64,
}

impl std::fmt::Debug for Reserved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reserved")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Writes a code for the account, unless it may not have one now.
///
/// The address in the scope must be the account's own, compared without
/// regard to white space or case. A cooldown and an hourly limit count every
/// code of the account for the same purpose, whichever challenge asked for it.
pub(super) fn reserve(
    tx: &Transaction,
    sealer: &Sealer,
    scope: &Scope<'_>,
    policy: &SecurityPolicy,
    now: Timestamp,
) -> Result<Reservation, DbError> {
    let Scope {
        user,
        challenge,
        purpose,
        method,
        address: expected_address,
    } = *scope;
    let account: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT \"Email\", \"NormalizedEmail\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((Some(email), Some(normalized))) = account else {
        return Ok(Reservation::Unavailable);
    };
    let recipient = email.trim();
    let expected: String = expected_address
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if normalized.is_empty() || recipient.is_empty() || names::to_upper(&expected) != normalized {
        return Ok(Reservation::Unavailable);
    }

    let latest: Option<Timestamp> = tx.query_row(
        "SELECT MAX(\"CreatedAt\") FROM \"EmailOtpCodes\" WHERE \"UserId\" = ?1 AND \"Purpose\" = ?2",
        params![user, purpose],
        |row| row.get(0),
    )?;
    if let Some(sent_at) = latest {
        let ready = sent_at.plus(policy.email_cooldown());
        if ready > now {
            let wait = now.until(ready);
            let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            return Ok(Reservation::Cooldown {
                retry_after_seconds: seconds as i64,
            });
        }
    }
    let sent_last_hour: i64 = tx.query_row(
        "SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"UserId\" = ?1 AND \"Purpose\" = ?2 AND \"CreatedAt\" > ?3",
        params![user, purpose, now.minus(std::time::Duration::from_secs(3600))],
        |row| row.get(0),
    )?;
    if sent_last_hour >= policy.email_code_sends_per_hour_clamped() {
        return Ok(Reservation::Limit);
    }

    let code = tokens::random_digits(6);
    let id = Id::random();
    tx.execute(
        "INSERT INTO \"EmailOtpCodes\" (\"Id\", \"UserId\", \"MethodId\", \"ChallengeId\", \"Purpose\", \"EmailAddress\", \"ProtectedCode\",
             \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\", \"ConsumedAt\", \"DeliveredAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, NULL, NULL)",
        params![
            id,
            user,
            method,
            challenge,
            purpose,
            recipient,
            sealer.seal(PURPOSE, &aad(user, purpose), None, code.as_bytes()),
            policy.email_code_max_attempts_clamped(),
            now,
            now.plus(policy.email_code_lifetime()),
        ],
    )?;
    Ok(Reservation::Reserved(Reserved {
        id,
        recipient: recipient.to_string(),
        code,
        lifetime_minutes: policy.email_code_lifetime().as_secs() as i64 / 60,
    }))
}

/// The mail was handed to the mail server: from now on the code can be used.
pub(super) fn mark_delivered(tx: &Transaction, code: Id, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "UPDATE \"EmailOtpCodes\" SET \"DeliveredAt\" = ?2 WHERE \"Id\" = ?1",
        params![code, now],
    )?;
    Ok(())
}

/// The mail could not be sent: the code is spent, but it still counts towards
/// the cooldown and the hourly limit.
pub(super) fn mark_failed(tx: &Transaction, code: Id, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "UPDATE \"EmailOtpCodes\" SET \"ConsumedAt\" = ?2 WHERE \"Id\" = ?1",
        params![code, now],
    )?;
    Ok(())
}

/// A code that was right, not yet spent.
#[derive(Debug, PartialEq, Eq)]
pub struct Proof(Id);

/// Checks what the person typed against the latest code that was sent for this
/// account, challenge, purpose, method and address. `None` for a code that is
/// wrong, or has run out, or has had all its guesses; a wrong code spends a
/// guess, and the last one spends the code.
///
/// A wrong answer is an ordinary outcome and changes the row, so the caller
/// must commit the transaction after it as it would after a right one.
pub(super) fn check(
    tx: &Transaction,
    sealer: &Sealer,
    scope: &Scope<'_>,
    supplied: &str,
    now: Timestamp,
) -> Result<Option<Proof>, DbError> {
    let Scope {
        user,
        challenge,
        purpose,
        method,
        address,
    } = *scope;
    let latest: Option<(Id, String, Timestamp, i64, i64)> = tx
        .query_row(
            "SELECT \"Id\", \"ProtectedCode\", \"ExpiresAt\", \"Attempts\", \"MaxAttempts\" FROM \"EmailOtpCodes\"
             WHERE \"UserId\" = ?1 AND \"ChallengeId\" = ?2 AND \"Purpose\" = ?3 AND \"MethodId\" IS ?4 AND \"EmailAddress\" = ?5
               AND \"ConsumedAt\" IS NULL AND \"DeliveredAt\" IS NOT NULL
             ORDER BY \"CreatedAt\" DESC LIMIT 1",
            params![user, challenge, purpose, method, address],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((id, sealed, expires_at, attempts, max_attempts)) = latest else {
        return Ok(None);
    };
    if expires_at <= now || attempts >= max_attempts {
        return Ok(None);
    }
    let right = sealer
        .open(PURPOSE, &aad(user, purpose), &sealed, now)
        .is_ok_and(|code| tokens::fixed_time_equals(&code, supplied.trim().as_bytes()));
    if !right {
        tx.execute(
            "UPDATE \"EmailOtpCodes\" SET \"Attempts\" = \"Attempts\" + 1,
                 \"ConsumedAt\" = CASE WHEN \"Attempts\" + 1 >= \"MaxAttempts\" THEN ?2 ELSE \"ConsumedAt\" END
             WHERE \"Id\" = ?1 AND \"ConsumedAt\" IS NULL",
            params![id, now],
        )?;
        return Ok(None);
    }
    Ok(Some(Proof(id)))
}

/// Spends a code that was right. True for one request only.
pub(super) fn consume(tx: &Transaction, proof: &Proof, now: Timestamp) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"EmailOtpCodes\" SET \"ConsumedAt\" = ?2 WHERE \"Id\" = ?1 AND \"ConsumedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![proof.0, now],
    )?;
    Ok(changed == 1)
}

/// Spends every code the account has not used, whatever it was for.
pub fn consume_all(tx: &Transaction, user: Id, now: Timestamp) -> Result<usize, DbError> {
    Ok(tx.execute(
        "UPDATE \"EmailOtpCodes\" SET \"ConsumedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"ConsumedAt\" IS NULL",
        params![user, now],
    )?)
}

/// Spends the codes that were sent for one method.
pub fn consume_for_method(
    tx: &Transaction,
    user: Id,
    method: Id,
    now: Timestamp,
) -> Result<usize, DbError> {
    Ok(tx.execute(
        "UPDATE \"EmailOtpCodes\" SET \"ConsumedAt\" = ?3 WHERE \"UserId\" = ?1 AND \"MethodId\" = ?2 AND \"ConsumedAt\" IS NULL",
        params![user, method, now],
    )?)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::Connection;

    use super::*;
    use crate::auth::fixtures::{add_user, open};

    const ADDRESS: &str = "maya@example.test";

    fn sealer() -> Sealer {
        Sealer::from_key([7; 32])
    }

    fn scope(user: Id, challenge: Id) -> Scope<'static> {
        Scope {
            user,
            challenge,
            purpose: LOGIN,
            method: Some(Id::from_bytes([9; 16])),
            address: ADDRESS,
        }
    }

    /// A policy that lets a test ask often, unless it is the test of a limit.
    fn eager() -> SecurityPolicy {
        SecurityPolicy {
            email_code_cooldown_seconds: 0,
            email_code_sends_per_hour: 60,
            ..SecurityPolicy::default()
        }
    }

    fn reserved(
        connection: &mut Connection,
        scope: &Scope<'_>,
        policy: &SecurityPolicy,
        now: Timestamp,
    ) -> Reserved {
        let tx = connection.transaction().unwrap();
        let Reservation::Reserved(code) = reserve(&tx, &sealer(), scope, policy, now).unwrap()
        else {
            panic!("a code was expected");
        };
        mark_delivered(&tx, code.id, now).unwrap();
        tx.commit().unwrap();
        code
    }

    fn reservation(
        connection: &mut Connection,
        scope: &Scope<'_>,
        policy: &SecurityPolicy,
        now: Timestamp,
    ) -> Reservation {
        let tx = connection.transaction().unwrap();
        let outcome = reserve(&tx, &sealer(), scope, policy, now).unwrap();
        tx.commit().unwrap();
        outcome
    }

    fn checked(
        connection: &mut Connection,
        scope: &Scope<'_>,
        supplied: &str,
        now: Timestamp,
    ) -> bool {
        let tx = connection.transaction().unwrap();
        let proof = check(&tx, &sealer(), scope, supplied, now).unwrap();
        tx.commit().unwrap();
        proof.is_some()
    }

    fn attempts(connection: &Connection, code: Id) -> (i64, Option<Timestamp>) {
        connection
            .query_row(
                "SELECT \"Attempts\", \"ConsumedAt\" FROM \"EmailOtpCodes\" WHERE \"Id\" = ?1",
                [code],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    fn after(now: Timestamp, seconds: u64) -> Timestamp {
        now.plus(Duration::from_secs(seconds))
    }

    #[test]
    fn a_code_is_written_for_the_account_and_sealed_for_its_purpose() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some("  Maya@Example.test "));
        let challenge = Id::random();
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();

        let Reservation::Reserved(code) = reserve(
            &tx,
            &sealer(),
            &scope(user, challenge),
            &SecurityPolicy::default(),
            now,
        )
        .unwrap() else {
            panic!("a code was expected");
        };
        tx.commit().unwrap();

        assert_eq!(code.recipient, "Maya@Example.test");
        assert_eq!(code.code.len(), 6);
        assert!(code.code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(code.lifetime_minutes, 8);
        assert!(!format!("{code:?}").contains(&code.code));
        let column = |name: &str| -> rusqlite::types::Value {
            connection
                .query_row(
                    &format!("SELECT \"{name}\" FROM \"EmailOtpCodes\" WHERE \"Id\" = ?1"),
                    [code.id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        let text = |name: &str| match column(name) {
            rusqlite::types::Value::Text(text) => text,
            other => panic!("{name} is {other:?}"),
        };
        let number = |name: &str| match column(name) {
            rusqlite::types::Value::Integer(number) => number,
            other => panic!("{name} is {other:?}"),
        };
        let moment = |name: &str| Timestamp::parse(&text(name)).unwrap();
        let (method, stored_challenge) = (
            Id::parse(&text("MethodId")),
            Id::parse(&text("ChallengeId")).unwrap(),
        );
        let (purpose, address, sealed) =
            (text("Purpose"), text("EmailAddress"), text("ProtectedCode"));
        let (attempts, max) = (number("Attempts"), number("MaxAttempts"));
        let (created, expires) = (moment("CreatedAt"), moment("ExpiresAt"));
        let (consumed, delivered) = (column("ConsumedAt"), column("DeliveredAt"));
        assert_eq!(method, scope(user, challenge).method);
        assert_eq!(stored_challenge, challenge);
        assert_eq!(
            (purpose.as_str(), address.as_str()),
            ("login", "Maya@Example.test")
        );
        assert_eq!((attempts, max), (0, 5));
        assert_eq!((created, expires), (now, after(now, 8 * 60)));
        assert_eq!(
            (consumed, delivered),
            (rusqlite::types::Value::Null, rusqlite::types::Value::Null)
        );
        // Sealed: not the digits, and it opens for this account and purpose only.
        assert!(!sealed.contains(&code.code));
        let aad = |user: Id, purpose: &str| format!("{user}:{purpose}").into_bytes();
        assert_eq!(
            sealer()
                .open(PURPOSE, &aad(user, "login"), &sealed, now)
                .unwrap(),
            code.code.as_bytes()
        );
        assert!(
            sealer()
                .open(PURPOSE, &aad(user, "enrollment"), &sealed, now)
                .is_err()
        );
        assert!(
            sealer()
                .open(PURPOSE, &aad(Id::random(), "login"), &sealed, now)
                .is_err()
        );
        assert!(
            sealer()
                .open("totp-secret", &aad(user, "login"), &sealed, now)
                .is_err()
        );
    }

    #[test]
    fn no_code_is_written_for_an_account_without_the_address_that_was_asked_for() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let without = add_user(&connection, None);
        let now = Timestamp::now();
        let challenge = Id::random();

        for (scope, why) in [
            (
                Scope {
                    address: "someone@example.test",
                    ..scope(user, challenge)
                },
                "another address",
            ),
            (
                Scope {
                    address: "",
                    ..scope(user, challenge)
                },
                "no address",
            ),
            (scope(without, challenge), "an account without an address"),
            (scope(Id::random(), challenge), "no such account"),
        ] {
            assert_eq!(
                reservation(&mut connection, &scope, &eager(), now),
                Reservation::Unavailable,
                "{why}"
            );
        }
        // White space and case do not matter in what was asked for.
        let spaced = Scope {
            address: " MAYA @example.TEST\t",
            ..scope(user, challenge)
        };
        assert!(matches!(
            reservation(&mut connection, &spaced, &eager(), now),
            Reservation::Reserved(_)
        ));
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM \"EmailOtpCodes\"", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_second_code_waits_for_the_cooldown_and_is_told_how_long() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let policy = SecurityPolicy::default();
        let now = Timestamp::now();
        // Another challenge asks: the cooldown is the account's.
        let first = scope(user, Id::random());
        let second = scope(user, Id::random());
        reserved(&mut connection, &first, &policy, now);

        assert_eq!(
            reservation(&mut connection, &second, &policy, after(now, 10)),
            Reservation::Cooldown {
                retry_after_seconds: 35
            }
        );
        // Part of a second left counts as a whole one.
        let nearly = now.plus(Duration::from_millis(44_500));
        assert_eq!(
            reservation(&mut connection, &second, &policy, nearly),
            Reservation::Cooldown {
                retry_after_seconds: 1
            }
        );
        assert!(matches!(
            reservation(&mut connection, &second, &policy, after(now, 45)),
            Reservation::Reserved(_)
        ));
        // Another purpose has a cooldown of its own.
        let enrolling = Scope {
            purpose: ENROLLMENT,
            method: None,
            ..first
        };
        assert!(matches!(
            reservation(&mut connection, &enrolling, &policy, after(now, 46)),
            Reservation::Reserved(_)
        ));
    }

    #[test]
    fn an_account_may_have_only_so_many_codes_in_an_hour_and_a_failed_mail_counts() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let policy = SecurityPolicy {
            email_code_sends_per_hour: 3,
            ..eager()
        };
        let now = Timestamp::now();
        let scope = scope(user, Id::random());

        for n in 0..3 {
            let tx = connection.transaction().unwrap();
            let Reservation::Reserved(code) =
                reserve(&tx, &sealer(), &scope, &policy, after(now, n)).unwrap()
            else {
                panic!("code {n} was expected");
            };
            mark_failed(&tx, code.id, after(now, n)).unwrap();
            tx.commit().unwrap();
        }

        assert_eq!(
            reservation(&mut connection, &scope, &policy, after(now, 10)),
            Reservation::Limit
        );
        assert_eq!(
            reservation(&mut connection, &scope, &policy, after(now, 3599)),
            Reservation::Limit
        );
        // The hour is counted back from now, and the oldest code leaves it first.
        assert!(matches!(
            reservation(&mut connection, &scope, &policy, after(now, 3601)),
            Reservation::Reserved(_)
        ));
        let enrolling = Scope {
            purpose: ENROLLMENT,
            method: None,
            ..scope
        };
        assert!(matches!(
            reservation(&mut connection, &enrolling, &policy, after(now, 10)),
            Reservation::Reserved(_)
        ));
    }

    #[test]
    fn a_code_that_was_not_delivered_cannot_be_used_and_one_that_failed_never_can() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();

        let tx = connection.transaction().unwrap();
        let Reservation::Reserved(code) = reserve(&tx, &sealer(), &scope, &eager(), now).unwrap()
        else {
            panic!("a code was expected");
        };
        tx.commit().unwrap();
        assert!(!checked(&mut connection, &scope, &code.code, now));
        let tx = connection.transaction().unwrap();
        mark_delivered(&tx, code.id, now).unwrap();
        tx.commit().unwrap();
        assert!(checked(&mut connection, &scope, &code.code, now));

        let again = reserved(&mut connection, &scope, &eager(), now);
        let tx = connection.transaction().unwrap();
        mark_failed(&tx, again.id, now).unwrap();
        tx.commit().unwrap();
        assert!(!checked(&mut connection, &scope, &again.code, now));
    }

    #[test]
    fn a_code_is_taken_with_white_space_round_it_and_spent_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let code = reserved(&mut connection, &scope, &eager(), now);

        let tx = connection.transaction().unwrap();
        let proof = check(&tx, &sealer(), &scope, &format!(" {}\n", code.code), now)
            .unwrap()
            .unwrap();
        assert!(consume(&tx, &proof, now).unwrap());
        assert!(!consume(&tx, &proof, now).unwrap());
        assert!(
            check(&tx, &sealer(), &scope, &code.code, now)
                .unwrap()
                .is_none()
        );
        tx.commit().unwrap();
    }

    #[test]
    fn a_proof_is_not_spent_after_the_code_ran_out_in_between() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let code = reserved(&mut connection, &scope, &eager(), now);

        let tx = connection.transaction().unwrap();
        let proof = check(&tx, &sealer(), &scope, &code.code, after(now, 479))
            .unwrap()
            .unwrap();
        assert!(!consume(&tx, &proof, after(now, 480)).unwrap());
        assert!(consume(&tx, &proof, after(now, 479)).unwrap());
    }

    #[test]
    fn a_wrong_guess_spends_an_attempt_and_the_last_one_spends_the_code() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let code = reserved(&mut connection, &scope, &eager(), now);
        let wrong = if code.code == "000000" {
            "000001"
        } else {
            "000000"
        };

        for guess in 1..=4 {
            assert!(!checked(&mut connection, &scope, wrong, now));
            assert_eq!(attempts(&connection, code.id), (guess, None));
        }
        assert!(!checked(&mut connection, &scope, wrong, now));
        assert_eq!(attempts(&connection, code.id), (5, Some(now)));
        // With its guesses gone the right code is no use either.
        assert!(!checked(&mut connection, &scope, &code.code, now));
        assert_eq!(attempts(&connection, code.id), (5, Some(now)));
    }

    #[test]
    fn a_code_with_all_its_guesses_gone_is_refused_even_if_it_was_not_marked_spent() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let code = reserved(&mut connection, &scope, &eager(), now);
        connection
            .execute(
                "UPDATE \"EmailOtpCodes\" SET \"Attempts\" = \"MaxAttempts\" WHERE \"Id\" = ?1",
                [code.id],
            )
            .unwrap();

        assert!(!checked(&mut connection, &scope, &code.code, now));
        assert_eq!(attempts(&connection, code.id), (5, None));
    }

    #[test]
    fn a_code_that_ran_out_is_refused_without_costing_an_attempt() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let code = reserved(&mut connection, &scope, &eager(), now);

        assert!(!checked(
            &mut connection,
            &scope,
            &code.code,
            after(now, 480)
        ));
        assert!(!checked(&mut connection, &scope, "123456", after(now, 481)));
        assert_eq!(attempts(&connection, code.id), (0, None));
        assert!(checked(
            &mut connection,
            &scope,
            &code.code,
            after(now, 479)
        ));
    }

    #[test]
    fn a_code_is_for_the_challenge_purpose_method_and_address_it_was_sent_for() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let challenge = Id::random();
        let right = scope(user, challenge);
        let now = Timestamp::now();
        let code = reserved(&mut connection, &right, &eager(), now);

        for (other, why) in [
            (
                Scope {
                    challenge: Id::random(),
                    ..right
                },
                "another challenge",
            ),
            (
                Scope {
                    purpose: ENROLLMENT,
                    ..right
                },
                "another purpose",
            ),
            (
                Scope {
                    method: Some(Id::random()),
                    ..right
                },
                "another method",
            ),
            (
                Scope {
                    method: None,
                    ..right
                },
                "no method",
            ),
            (
                Scope {
                    address: "other@example.test",
                    ..right
                },
                "another address",
            ),
            (
                Scope {
                    address: "MAYA@example.test",
                    ..right
                },
                "the address in other letters",
            ),
            (
                Scope {
                    user: Id::random(),
                    ..right
                },
                "another account",
            ),
        ] {
            assert!(!checked(&mut connection, &other, &code.code, now), "{why}");
        }
        assert!(checked(&mut connection, &right, &code.code, now));
    }

    #[test]
    fn the_code_of_an_enrolment_belongs_to_no_method() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let enrolling = Scope {
            purpose: ENROLLMENT,
            method: None,
            ..scope(user, Id::random())
        };
        let now = Timestamp::now();
        let code = reserved(&mut connection, &enrolling, &eager(), now);

        assert!(!checked(
            &mut connection,
            &Scope {
                method: Some(Id::random()),
                ..enrolling
            },
            &code.code,
            now
        ));
        assert!(checked(&mut connection, &enrolling, &code.code, now));
    }

    #[test]
    fn only_the_latest_code_of_a_challenge_counts() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let old = reserved(&mut connection, &scope, &eager(), now);
        let new = reserved(&mut connection, &scope, &eager(), after(now, 1));
        let (older, newer) = if old.code == new.code {
            return;
        } else {
            (&old, &new)
        };

        assert!(!checked(
            &mut connection,
            &scope,
            &older.code,
            after(now, 2)
        ));
        assert!(checked(&mut connection, &scope, &newer.code, after(now, 2)));
    }

    #[test]
    fn a_code_the_first_server_protected_never_opens_and_wears_out_its_attempts() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let scope = scope(user, Id::random());
        let now = Timestamp::now();
        let code = reserved(&mut connection, &scope, &eager(), now);
        connection
            .execute(
                "UPDATE \"EmailOtpCodes\" SET \"ProtectedCode\" = 'CfDJ8Nothing-this-server-can-open' WHERE \"Id\" = ?1",
                [code.id],
            )
            .unwrap();

        assert!(!checked(&mut connection, &scope, &code.code, now));
        assert_eq!(attempts(&connection, code.id), (1, None));
        // A new code, written here, works.
        let fresh = reserved(&mut connection, &scope, &eager(), after(now, 1));
        assert!(checked(&mut connection, &scope, &fresh.code, after(now, 2)));
    }

    #[test]
    fn a_sealed_code_moved_to_another_account_or_purpose_does_not_open() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let other = add_user(&connection, Some("other@example.test"));
        let now = Timestamp::now();
        let mine = reserved(&mut connection, &scope(user, Id::random()), &eager(), now);
        let theirs_scope = Scope {
            address: "other@example.test",
            ..scope(other, Id::random())
        };
        let theirs = reserved(&mut connection, &theirs_scope, &eager(), now);
        let sealed: String = connection
            .query_row(
                "SELECT \"ProtectedCode\" FROM \"EmailOtpCodes\" WHERE \"Id\" = ?1",
                [mine.id],
                |r| r.get(0),
            )
            .unwrap();
        connection
            .execute(
                "UPDATE \"EmailOtpCodes\" SET \"ProtectedCode\" = ?2 WHERE \"Id\" = ?1",
                params![theirs.id, sealed],
            )
            .unwrap();

        // Their row now holds my sealed code, which is not sealed for them.
        assert!(!checked(&mut connection, &theirs_scope, &mine.code, now));
        // And the same code sealed for another purpose does not serve this one.
        let as_enrolment = Scope {
            purpose: ENROLLMENT,
            method: None,
            ..scope(user, Id::random())
        };
        let moved = reserved(&mut connection, &as_enrolment, &eager(), after(now, 1));
        connection
            .execute(
                "UPDATE \"EmailOtpCodes\" SET \"ProtectedCode\" = ?2 WHERE \"Id\" = ?1",
                params![moved.id, sealed],
            )
            .unwrap();
        assert!(!checked(
            &mut connection,
            &as_enrolment,
            &mine.code,
            after(now, 2)
        ));
    }

    #[test]
    fn every_code_of_an_account_or_of_a_method_can_be_spent_at_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let now = Timestamp::now();
        let method = Id::random();
        let for_method = Scope {
            method: Some(method),
            ..scope(user, Id::random())
        };
        let other = scope(user, Id::random());
        reserved(&mut connection, &for_method, &eager(), now);
        let kept = reserved(&mut connection, &other, &eager(), now);

        let tx = connection.transaction().unwrap();
        assert_eq!(consume_for_method(&tx, user, method, now).unwrap(), 1);
        assert_eq!(consume_for_method(&tx, user, method, now).unwrap(), 0);
        assert_eq!(
            consume_for_method(&tx, Id::random(), other.method.unwrap(), now).unwrap(),
            0
        );
        tx.commit().unwrap();
        assert!(checked(&mut connection, &other, &kept.code, now));
        let tx = connection.transaction().unwrap();
        assert_eq!(consume_all(&tx, user, now).unwrap(), 1);
        tx.commit().unwrap();
        assert!(!checked(&mut connection, &other, &kept.code, now));
    }

    #[test]
    fn the_cooldown_is_worked_out_from_the_policy_in_force() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let policy = SecurityPolicy {
            email_code_cooldown_seconds: 120,
            ..SecurityPolicy::default()
        };
        let now = Timestamp::now();
        let scope = scope(user, Id::random());
        reserved(&mut connection, &scope, &policy, now);

        assert_eq!(
            reservation(&mut connection, &scope, &policy, after(now, 1)),
            Reservation::Cooldown {
                retry_after_seconds: 119
            }
        );
    }
}
