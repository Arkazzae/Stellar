//! Sending a code by mail in the one order that is right: look whether mail
//! can be sent at all, write the code and commit it, send it, and only then
//! mark it delivered or spent. A route calls [`send`] and answers from what it
//! says; nothing else in the second step sends mail.
//!
//! The row is committed before the mail goes, so that the cooldown and the
//! hourly limit count a code whose mail failed, and no transaction is held
//! across the network.

use rusqlite::{OptionalExtension, Transaction};

use super::ceremony::{Ask, Own, open};
use super::email_codes::{self, Reservation};
use super::flow::ChallengeStatus;
use super::mfa::{self, Reserve};
use super::policy::SecurityPolicy;
use crate::database::DbError;
use crate::mail::{self, MailSetup, Outcome, setup};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// What asking for a code to be mailed came to.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
    /// The mail was taken by the provider; the code works now. `address` is
    /// the one it was mailed to, as the account has it, for the sentence the
    /// client is shown; `retry_after_seconds` is how long before another may
    /// be asked for.
    Delivered {
        address: String,
        retry_after_seconds: i64,
    },
    /// The method cannot be used, or the account has no address to mail, or
    /// has the step already (a 409 that says the method is not available).
    MethodUnavailable,
    /// Mail is off or not set up (a 409 that says mail is not available).
    MailUnavailable,
    Cooldown {
        retry_after_seconds: i64,
    },
    Limit,
    /// The mail was not taken. The code is spent, and counts towards the
    /// cooldown and the hourly limit.
    DeliveryFailed,
    /// The challenge cannot be used.
    Invalid(ChallengeStatus),
}

/// What the reserving step found.
pub(super) enum Attempt {
    Reserved {
        user: Id,
        reservation: Reservation,
        cooldown_seconds: i64,
    },
    MethodUnavailable,
    MailUnavailable,
    Invalid(ChallengeStatus),
}

fn cooldown_seconds(policy: &SecurityPolicy) -> i64 {
    i64::try_from(policy.email_cooldown().as_secs()).unwrap_or(i64::MAX)
}

fn attempt_of(user: Id, policy: &SecurityPolicy, reserve: Reserve) -> Attempt {
    match reserve {
        Reserve::MethodUnavailable => Attempt::MethodUnavailable,
        Reserve::MailUnavailable => Attempt::MailUnavailable,
        Reserve::Reservation(reservation) => Attempt::Reserved {
            user,
            reservation,
            cooldown_seconds: cooldown_seconds(policy),
        },
    }
}

async fn mail_ready(state: &AppState) -> Result<bool, DbError> {
    Ok(matches!(setup::find_for(state).await?, MailSetup::Ready(_)))
}

/// Mails the code of a sign-in's second step for one of the account's mail
/// methods. The challenge is looked at first, then the method, then whether
/// mail can be sent, as the first server did; nothing is written until all
/// three are in order.
pub(super) async fn login_code(
    state: &AppState,
    ask: &Ask<'_>,
    method: Option<&str>,
) -> Result<Sent, DbError> {
    let own = Own::of(ask);
    let method = method.and_then(Id::parse);
    let sealer_state = state.clone();
    let ready = mail_ready(state).await?;
    send_with(state, move |tx, policy, now| {
        let challenge = match open(tx, &own, mfa::answers_with_a_code, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(Attempt::Invalid(status)),
        };
        let Some(method) = method else {
            return Ok(Attempt::MethodUnavailable);
        };
        let reserve = mfa::request_login_email_code(
            tx,
            sealer_state.sealer(),
            &challenge,
            method,
            ready,
            policy,
            now,
        )?;
        Ok(attempt_of(challenge.user, policy, reserve))
    })
    .await
}

/// Mails the code that proves the account's address, to add it as a second
/// step.
pub(super) async fn enrollment_code(state: &AppState, ask: &Ask<'_>) -> Result<Sent, DbError> {
    enrollment_code_own(state, Own::of(ask)).await
}

async fn enrollment_code_own(state: &AppState, own: Own) -> Result<Sent, DbError> {
    let sealer_state = state.clone();
    let ready = mail_ready(state).await?;
    send_with(state, move |tx, policy, now| {
        let challenge = match open(tx, &own, mfa::enrols, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(Attempt::Invalid(status)),
        };
        let reserve =
            mfa::start_email_enrollment(tx, sealer_state.sealer(), &challenge, ready, policy, now)?;
        Ok(attempt_of(challenge.user, policy, reserve))
    })
    .await
}

/// Reserves the code as `reserve` says (in the transaction it runs in, which
/// also reads the challenge), commits, mails it and marks the row.
pub(super) async fn send_with<F>(state: &AppState, reserve: F) -> Result<Sent, DbError>
where
    F: FnOnce(&Transaction, &SecurityPolicy, Timestamp) -> Result<Attempt, DbError>
        + Send
        + 'static,
{
    let reserved = state
        .database()
        .write(move |tx| -> Result<_, DbError> {
            let policy = SecurityPolicy::load(tx)?;
            let attempt = reserve(tx, &policy, Timestamp::now())?;
            let name: Option<String> = match &attempt {
                Attempt::Reserved { user, .. } => tx
                    .query_row(
                        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                        [*user],
                        |row| row.get(0),
                    )
                    .optional()?,
                _ => None,
            };
            Ok((attempt, name))
        })
        .await?;
    let (attempt, name) = reserved;
    let (reservation, cooldown) = match attempt {
        Attempt::Invalid(status) => return Ok(Sent::Invalid(status)),
        Attempt::MethodUnavailable => return Ok(Sent::MethodUnavailable),
        Attempt::MailUnavailable => return Ok(Sent::MailUnavailable),
        Attempt::Reserved {
            reservation,
            cooldown_seconds,
            ..
        } => (reservation, cooldown_seconds),
    };
    let code = match reservation {
        Reservation::Unavailable => return Ok(Sent::MailUnavailable),
        Reservation::Cooldown {
            retry_after_seconds,
        } => {
            return Ok(Sent::Cooldown {
                retry_after_seconds,
            });
        }
        Reservation::Limit => return Ok(Sent::Limit),
        Reservation::Reserved(code) => code,
    };
    let minutes = u32::try_from(code.lifetime_minutes).unwrap_or(1);
    let id = u128::from_be_bytes(*code.id.as_bytes());
    let outcome = mail::deliver(state, |mailing| {
        stellarr_mail::email_code(
            &mailing.sender,
            &code.recipient,
            name.as_deref().unwrap_or(""),
            &mailing.instance_name,
            &mailing.public_url,
            &code.code,
            minutes,
            id,
            state.messages(),
        )
    })
    .await?;
    let delivered = matches!(outcome, Outcome::Sent { .. });
    let code_id = code.id;
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            if delivered {
                email_codes::mark_delivered(tx, code_id, Timestamp::now())
            } else {
                email_codes::mark_failed(tx, code_id, Timestamp::now())
            }
        })
        .await?;
    Ok(if delivered {
        Sent::Delivered {
            address: code.recipient,
            retry_after_seconds: cooldown,
        }
    } else {
        Sent::DeliveryFailed
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;
    use crate::auth::email_codes::ENROLLMENT;
    use crate::mail::tests::support::{Behaviour, FakeSmtp, Saved, found, instance};

    async fn account(state: &AppState) -> Id {
        let user = Id::random();
        state
            .database()
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\",
                         \"NormalizedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                     VALUES (?1, 'maya', 'MAYA', 'Maya', 'x', 'maya@example.test', 'MAYA@EXAMPLE.TEST', 0, 1, 0, 'c', ?2)",
                    params![user, Timestamp::now()],
                )
            })
            .await
            .unwrap();
        user
    }

    /// A challenge of an enrolment of the account, in the state's database.
    fn challenge_of(state: &AppState, user: Id) -> Own {
        let connection = rusqlite::Connection::open(state.database().path()).unwrap();
        let challenge = crate::auth::fixtures::add_challenge(&connection, user, ENROLLMENT);
        let preauth = crate::auth::fixtures::preauth_of(&challenge);
        let context = crate::auth::flow::Context {
            ip_address: "203.0.113.9",
            user_agent: "test",
            https: true,
            trusted_device_cookie: None,
            passkeys_available: false,
        };
        Own::of(&Ask {
            context: &context,
            preauth: Some(&preauth),
            challenge_id: &challenge.id.to_string(),
        })
    }

    async fn ask(state: &AppState, user: Id) -> Sent {
        enrollment_code_own(state, challenge_of(state, user))
            .await
            .unwrap()
    }

    fn row(state: &AppState) -> (bool, bool) {
        let connection = rusqlite::Connection::open(state.database().path()).unwrap();
        connection
            .query_row(
                "SELECT \"DeliveredAt\" IS NOT NULL, \"ConsumedAt\" IS NOT NULL FROM \"EmailOtpCodes\"",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn a_code_is_written_mailed_and_then_marked_delivered() {
        let smtp = FakeSmtp::start(Behaviour::default()).await;
        let (_folder, state) = instance(&[]);
        found(
            &state,
            Saved {
                port: smtp.port,
                ..Saved::default()
            },
        )
        .await;
        let user = account(&state).await;

        let sent = ask(&state, user).await;

        assert!(
            matches!(&sent, Sent::Delivered { address, .. } if address == "maya@example.test"),
            "{sent:?}"
        );
        assert_eq!(row(&state), (true, false));
        let mails = smtp.mails();
        assert_eq!(mails.len(), 1);
        assert!(mails[0].data.contains("Subject:"), "{}", mails[0].data);
        assert!(
            mails[0]
                .commands
                .iter()
                .any(|c| c.contains("maya@example.test"))
        );
        // The digits the person is told are the ones that were written.
        let six = mails[0]
            .data
            .lines()
            .map(str::trim)
            .find(|l| l.len() == 6 && l.bytes().all(|b| b.is_ascii_digit()));
        assert!(six.is_some(), "{}", mails[0].data);
    }

    #[tokio::test]
    async fn a_mail_the_server_refuses_leaves_a_spent_code_that_still_counts() {
        let smtp = FakeSmtp::start(Behaviour {
            recipient_reply: "550 no such user",
            ..Behaviour::default()
        })
        .await;
        let (_folder, state) = instance(&[]);
        found(
            &state,
            Saved {
                port: smtp.port,
                ..Saved::default()
            },
        )
        .await;
        let user = account(&state).await;

        assert_eq!(ask(&state, user).await, Sent::DeliveryFailed);

        assert_eq!(row(&state), (false, true));
        // The code that failed holds the cooldown.
        assert!(matches!(ask(&state, user).await, Sent::Cooldown { .. }));
    }

    #[tokio::test]
    async fn with_mail_off_no_code_is_written() {
        let (_folder, state) = instance(&[]);
        found(
            &state,
            Saved {
                mailing: false,
                ..Saved::default()
            },
        )
        .await;
        let user = account(&state).await;

        assert_eq!(ask(&state, user).await, Sent::MailUnavailable);

        let connection = rusqlite::Connection::open(state.database().path()).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM \"EmailOtpCodes\"", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn a_challenge_that_cannot_be_used_sends_nothing() {
        let smtp = FakeSmtp::start(Behaviour::default()).await;
        let (_folder, state) = instance(&[]);
        found(
            &state,
            Saved {
                port: smtp.port,
                ..Saved::default()
            },
        )
        .await;
        let user = account(&state).await;
        let mut own = challenge_of(&state, user);
        own.break_secret();

        let sent = enrollment_code_own(&state, own).await.unwrap();

        assert!(matches!(sent, Sent::Invalid(_)), "{sent:?}");
        assert_eq!(smtp.seen().connections, 0);
    }
}
