//! The worker that sends scheduled messages when their time comes. It sleeps
//! until the next one is due, and a request that adds or moves one rings the
//! state's alarm so that it looks again.

use std::time::Duration;

use tracing::warn;

use super::send::{self, Attempt};
use super::store::{self, Scheduled};
use super::{Failure, announce_changed};
use crate::background::Worker;
use crate::database::DbError;
use crate::http::ApiError;
use crate::messages::{Reason, Stop};
use crate::state::AppState;
use crate::values::Timestamp;

const TARGET: &str = "Stellarr.Api.ScheduledMessages";

/// The longest the worker sleeps without looking: a clock that jumps, or a
/// row that another process wrote, is noticed within this.
const LONGEST_SLEEP: Duration = Duration::from_secs(60);

/// How long a message that met a fault of the server waits before it is
/// tried again. A fault is no reason to fail the message.
const RETRY: Duration = Duration::from_secs(30);

/// How many due messages one pass takes. More than that are taken by the
/// next pass, which follows at once.
const BATCH: i64 = 50;

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("scheduled-messages", |state, stop| async move {
        loop {
            let next = match pass(&state, Timestamp::now()).await {
                Ok(next) => next,
                Err(error) => {
                    warn!(target: TARGET, "Sending scheduled messages failed: {error}");
                    None
                }
            };
            let wait = next.map_or(LONGEST_SLEEP, |at| {
                Timestamp::now().until(at).min(LONGEST_SLEEP)
            });
            tokio::select! {
                () = state.scheduled_alarm().rung() => {}
                slept = stop.sleep(wait) => if !slept {
                    return;
                },
            }
            if stop.is_stopping() {
                return;
            }
        }
    })]
}

/// Sends what is due by `now`, and says when the next pending message falls
/// due. A message that is refused is marked failed, one in slow mode waits
/// for the window to pass, and one the server failed to send is tried again.
pub async fn pass(state: &AppState, now: Timestamp) -> Result<Option<Timestamp>, DbError> {
    let due = state
        .database()
        .read(move |db| -> Result<_, DbError> { Ok(store::due(db, now, BATCH)?) })
        .await?;
    for scheduled in due {
        if let Attempt::Refused(stop) = send::attempt(state, &scheduled).await {
            settle(state, &scheduled, stop).await?;
        }
    }
    state
        .database()
        .read(|db| -> Result<_, DbError> { Ok(store::next_due(db)?) })
        .await
}

/// What a refusal leaves of a message: a later try, or a failure that its
/// author is told about.
async fn settle(state: &AppState, scheduled: &Scheduled, stop: Stop) -> Result<(), DbError> {
    let claim = scheduled.claim();
    let now = Timestamp::now();
    let failure = match stop {
        // It waits out slow mode and goes once the window has passed.
        Stop::SlowMode(seconds) => {
            let at = now.plus(Duration::from_secs(u64::try_from(seconds).unwrap_or(1)));
            return postpone(state, claim, at).await;
        }
        // It was edited or deleted while it was being sent: the next look
        // reads it as it is now.
        Stop::Refused(Reason::Withdrawn, _) => return Ok(()),
        Stop::Answer(ApiError::Internal(cause)) => {
            warn!(target: TARGET, "A scheduled message could not be sent and is tried again: {cause}");
            return postpone(state, claim, now.plus(RETRY)).await;
        }
        Stop::Refused(reason, _) => Failure::of(reason),
        Stop::Answer(_) => Failure::Other,
    };
    let (author, id) = (scheduled.author, scheduled.id);
    let failed = state
        .database()
        .write(move |tx| -> Result<_, DbError> {
            if !store::fail(tx, claim, failure, now)? {
                return Ok(None);
            }
            Ok(store::dto(tx, author, id)?)
        })
        .await?;
    if let Some(message) = failed {
        announce_changed(state, author, &message);
    }
    Ok(())
}

async fn postpone(state: &AppState, claim: store::Claim, at: Timestamp) -> Result<(), DbError> {
    state
        .database()
        .write(move |tx| -> Result<(), DbError> { Ok(store::postpone(tx, claim, at)?) })
        .await
}
