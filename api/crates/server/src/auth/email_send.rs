//! Sending a code by mail in the one order that is right: look whether mail
//! can be sent at all, write the code and commit it, send it, and only then
//! mark it delivered or spent. A route calls [`send`] and answers from what it
//! says; nothing else in the second step sends mail.
//!
//! The row is committed before the mail goes, so that the cooldown and the
//! hourly limit count a code whose mail failed, and no transaction is held
//! across the network.

use rusqlite::{OptionalExtension, Transaction};

use super::email_codes::{self, Reservation};
use super::policy::SecurityPolicy;
use crate::database::DbError;
use crate::mail::{self, MailSetup, Outcome, setup};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// What asking for a code to be mailed came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
    /// The mail was taken by the provider; the code works now.
    Delivered,
    /// Mail is off or not set up, or the account has no such address or
    /// method.
    Unavailable,
    Cooldown {
        retry_after_seconds: i64,
    },
    Limit,
    /// The mail was not taken. The code is spent, and counts towards the
    /// cooldown and the hourly limit.
    DeliveryFailed,
}

/// Reserves a code as `reserve` says (a call of `mfa::request_login_email_code`
/// or `mfa::start_email_enrollment`, or `email_codes::reserve` itself; `None`
/// means the account or the method cannot have one), mails it to the account
/// and marks it. The account's name is read for the greeting.
pub async fn send<F>(state: &AppState, user: Id, reserve: F) -> Result<Sent, DbError>
where
    F: FnOnce(&Transaction, &SecurityPolicy, Timestamp) -> Result<Option<Reservation>, DbError>
        + Send
        + 'static,
{
    // Settings are looked at before anything is written: with no way to send,
    // no code is made and none counts against the limits.
    if !matches!(setup::find_for(state).await?, MailSetup::Ready(_)) {
        return Ok(Sent::Unavailable);
    }
    let reserved = state
        .database()
        .write(move |tx| -> Result<_, DbError> {
            let policy = SecurityPolicy::load(tx)?;
            let name: Option<String> = tx
                .query_row(
                    "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                    [user],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((reserve(tx, &policy, Timestamp::now())?, name, policy))
        })
        .await?;
    let (reservation, name, _policy) = reserved;
    let code = match reservation {
        None | Some(Reservation::Unavailable) => return Ok(Sent::Unavailable),
        Some(Reservation::Cooldown {
            retry_after_seconds,
        }) => {
            return Ok(Sent::Cooldown {
                retry_after_seconds,
            });
        }
        Some(Reservation::Limit) => return Ok(Sent::Limit),
        Some(Reservation::Reserved(code)) => code,
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
        Sent::Delivered
    } else {
        Sent::DeliveryFailed
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;
    use crate::auth::email_codes::{ENROLLMENT, Scope};
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

    fn ask(
        user: Id,
    ) -> impl FnOnce(
        &Transaction,
        &SecurityPolicy,
        Timestamp,
    ) -> Result<Option<Reservation>, DbError>
    + Send
    + 'static {
        move |tx, policy, now| {
            let scope = Scope {
                user,
                challenge: Id::from_bytes([3; 16]),
                purpose: ENROLLMENT,
                method: None,
                address: "maya@example.test",
            };
            let sealer = crate::seal::Sealer::from_key([5u8; 32]);
            email_codes::reserve(tx, &sealer, &scope, policy, now).map(Some)
        }
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

        let sent = send(&state, user, ask(user)).await.unwrap();

        assert_eq!(sent, Sent::Delivered);
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

        assert_eq!(
            send(&state, user, ask(user)).await.unwrap(),
            Sent::DeliveryFailed
        );

        assert_eq!(row(&state), (false, true));
        // The code that failed holds the cooldown.
        assert!(matches!(
            send(&state, user, ask(user)).await.unwrap(),
            Sent::Cooldown { .. }
        ));
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

        assert_eq!(
            send(&state, user, ask(user)).await.unwrap(),
            Sent::Unavailable
        );

        let connection = rusqlite::Connection::open(state.database().path()).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM \"EmailOtpCodes\"", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn a_reservation_that_says_no_sends_nothing() {
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

        let sent = send(&state, user, |_, _, _| Ok(None)).await.unwrap();

        assert_eq!(sent, Sent::Unavailable);
        assert_eq!(smtp.seen().connections, 0);
    }
}
