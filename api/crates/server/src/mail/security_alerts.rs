//! The queue of security alerts: a row in `SecurityNotifications` for each
//! event that happened to an account with an address it has proved to be its
//! own, and the worker that sends them.
//!
//! The account's request that caused the event never waits on the mail and
//! never fails because of it: it wrote the row, and this worker looks at the
//! queue every thirty seconds.
//!
//! A batch is the next 25 rows that are due, have not been sent and have had
//! fewer than 20 tries. With alerts switched off, or mail off or not
//! configured, nothing is attempted and the batch is put an hour off. A try
//! counts when it fails or succeeds; a failure keeps what went wrong (cut to
//! 500 characters) and waits five minutes for the first, ten for the second
//! and so on up to an hour. After twenty tries the row is left as it is.
//!
//! The message is built in the instance's language and time zone. A row's
//! outcome is written as soon as it is known, so a server that is stopped in
//! the middle of a batch loses no more than the message it was sending.

use std::time::Duration;

use rusqlite::params;
use stellarr_mail::{SecurityEvent, security_alert};
use stellarr_timezone::TimeZone;
use tracing::{debug, warn};

use super::delivery::Outcome;
use super::setup::{self, MailSetup, Mailing};
use crate::background::Worker;
use crate::database::DbError;
use crate::instance::access;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const TARGET: &str = "Stellarr.Api.Mail";
/// How long the worker waits between two looks at the queue.
pub const INTERVAL: Duration = Duration::from_secs(30);
/// Rows looked at in one batch.
pub const BATCH: usize = 25;
/// A row that has been tried this often is left alone.
pub const MAX_ATTEMPTS: i64 = 20;
const RETRY_STEP_MINUTES: i64 = 5;
const RETRY_CAP_MINUTES: i64 = 60;
/// Where a batch goes when alerts or mail are off.
const OFF_DELAY: Duration = Duration::from_secs(60 * 60);
/// The most of an error a row keeps, counted in UTF-16 units as the clients
/// and the first server count.
const MAX_ERROR_UNITS: usize = 500;

pub fn workers() -> Vec<Worker> {
    vec![Worker::new(
        "security-notifications",
        |state, stop| async move {
            loop {
                // A look that fails is logged, and the next one tries again.
                let result = run(&state, || stop.is_stopping(), Timestamp::now()).await;
                if let Err(error) = result {
                    warn!(target: TARGET, "The security notification queue could not be processed: {error}");
                }
                if !stop.sleep(INTERVAL).await {
                    break;
                }
            }
        },
    )]
}

struct Pending {
    id: Id,
    user: Id,
    address: String,
    attempts: i64,
    event: Id,
    kind: String,
    ip_address: String,
    device: String,
    happened_at: Timestamp,
    display_name: String,
}

/// What a look at the queue found.
enum Plan {
    /// Nothing is due.
    Idle,
    /// Alerts or mail are off, or not set up: put the rows off.
    Postpone(Vec<Id>),
    Send {
        rows: Vec<Pending>,
        mailing: Box<Mailing>,
        zone: TimeZone,
    },
}

/// Looks at the queue as of `now` and sends what is due. Returns how many
/// alerts the provider took.
pub async fn dispatch(state: &AppState, now: Timestamp) -> Result<usize, DbError> {
    run(state, || false, now).await
}

/// [`dispatch`], which asks `stopping` between two messages.
pub(crate) async fn run(
    state: &AppState,
    stopping: impl Fn() -> bool,
    now: Timestamp,
) -> Result<usize, DbError> {
    let reading = state.clone();
    let plan = state
        .database()
        .read(move |db| -> Result<Plan, DbError> {
            let mut statement = db.prepare(
                "SELECT n.\"Id\", n.\"UserId\", n.\"EmailAddress\", n.\"Attempts\", e.\"Id\", e.\"Kind\", e.\"IpAddress\",
                        e.\"Device\", e.\"CreatedAt\", u.\"DisplayName\"
                 FROM \"SecurityNotifications\" n
                 JOIN \"SecurityEvents\" e ON e.\"Id\" = n.\"SecurityEventId\"
                 JOIN \"Users\" u ON u.\"Id\" = n.\"UserId\"
                 WHERE n.\"SentAt\" IS NULL AND n.\"NextAttemptAt\" <= ?1 AND n.\"Attempts\" < ?2
                 ORDER BY n.\"NextAttemptAt\", n.\"CreatedAt\", n.\"Id\"
                 LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![now, MAX_ATTEMPTS, BATCH as i64], |row| {
                    Ok(Pending {
                        id: row.get(0)?,
                        user: row.get(1)?,
                        address: row.get(2)?,
                        attempts: row.get(3)?,
                        event: row.get(4)?,
                        kind: row.get(5)?,
                        ip_address: row.get(6)?,
                        device: row.get(7)?,
                        happened_at: row.get(8)?,
                        display_name: row.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                return Ok(Plan::Idle);
            }
            let postponed = |rows: Vec<Pending>| Plan::Postpone(rows.into_iter().map(|r| r.id).collect());
            // Alerts have their own switch; without a saved row of settings
            // there is none to switch on.
            if !setup::switches(db)?.alerts || access::instance_server(db)?.is_none() {
                return Ok(postponed(rows));
            }
            let settings = reading.settings();
            let MailSetup::Ready(mailing) = setup::find(db, settings, reading.sealer())? else {
                return Ok(postponed(rows));
            };
            let zone = instance_zone(&mailing.time_zone);
            Ok(Plan::Send { rows, mailing, zone })
        })
        .await?;

    match plan {
        Plan::Idle => Ok(0),
        Plan::Postpone(ids) => {
            let later = now.plus(OFF_DELAY);
            state
                .database()
                .write(move |tx| -> Result<(), DbError> {
                    for id in ids {
                        tx.execute(
                            "UPDATE \"SecurityNotifications\" SET \"NextAttemptAt\" = ?1 WHERE \"Id\" = ?2",
                            params![later, id],
                        )?;
                    }
                    Ok(())
                })
                .await?;
            Ok(0)
        }
        Plan::Send {
            rows,
            mailing,
            zone,
        } => {
            let mut sent = 0;
            for row in rows {
                if stopping() {
                    break;
                }
                let outcome = send_one(state, &mailing, &zone, &row).await;
                if outcome.is_sent() {
                    sent += 1;
                } else {
                    // The account's id and nothing of the message or of what
                    // the provider said: that can hold an address.
                    warn!(target: TARGET, "A security notification for account {} could not be sent.", row.user);
                }
                record(state, &row, &outcome, now).await?;
            }
            Ok(sent)
        }
    }
}

async fn send_one(state: &AppState, mailing: &Mailing, zone: &TimeZone, row: &Pending) -> Outcome {
    let event = SecurityEvent {
        id: u128::from_be_bytes(*row.event.as_bytes()),
        kind: &row.kind,
        ip_address: &row.ip_address,
        device: &row.device,
        created_at: row.happened_at.unix_seconds(),
    };
    let built = security_alert(
        &mailing.sender,
        &row.address,
        &row.display_name,
        &mailing.instance_name,
        &mailing.public_url,
        &event,
        zone,
        state.messages(),
    );
    match built {
        Ok(message) => state.mail().send(&message, &mailing.transport).await,
        Err(error) => Outcome::Refused {
            reason: error.to_string(),
        },
    }
}

/// Writes down what became of one row.
async fn record(
    state: &AppState,
    row: &Pending,
    outcome: &Outcome,
    now: Timestamp,
) -> Result<(), DbError> {
    let attempts = row.attempts + 1;
    let id = row.id;
    let (sent_at, error, next): (Option<Timestamp>, String, Option<Timestamp>) = match outcome {
        Outcome::Sent { .. } => (Some(now), String::new(), None),
        failed => {
            let reason = failed.reason().unwrap_or("the mail is not set up");
            let minutes = (RETRY_STEP_MINUTES * attempts).min(RETRY_CAP_MINUTES);
            (
                None,
                cut(reason),
                Some(now.plus(Duration::from_secs(minutes as u64 * 60))),
            )
        }
    };
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "UPDATE \"SecurityNotifications\"
                 SET \"Attempts\" = ?1, \"LastAttemptAt\" = ?2, \"SentAt\" = ?3, \"LastError\" = ?4,
                     \"NextAttemptAt\" = COALESCE(?5, \"NextAttemptAt\")
                 WHERE \"Id\" = ?6",
                params![attempts, now, sent_at, error, next, id],
            )?;
            Ok(())
        })
        .await?;
    debug!(target: TARGET, "A security notification was recorded as {}.", if sent_at.is_some() { "sent" } else { "failed" });
    Ok(())
}

/// The instance's zone, or UTC when it has none or this host does not know
/// its name.
fn instance_zone(id: &str) -> TimeZone {
    let id = id.trim();
    if id.is_empty() {
        return TimeZone::utc();
    }
    TimeZone::find(id).unwrap_or_else(|_| TimeZone::utc())
}

/// The first 500 UTF-16 units of `text`, never half a character.
pub(crate) fn cut(text: &str) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|letter| {
            units += letter.len_utf16();
            units <= MAX_ERROR_UNITS
        })
        .collect()
}
