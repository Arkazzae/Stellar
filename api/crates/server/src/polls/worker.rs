//! The worker that ends polls when their time is up. It sleeps until the next
//! one ends, and a new poll rings the state's alarm so that it looks again.
//! A poll whose time passed while the server was down is ended at its first
//! look, at the time it was due; readers have taken it for ended all along.

use std::time::Duration;

use rusqlite::params;
use tracing::warn;

use super::read;
use crate::background::Worker;
use crate::database::DbError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const TARGET: &str = "Stellarr.Api.Polls";

/// The longest the worker sleeps without looking: a clock that jumps, or a
/// poll that another process wrote, is noticed within this.
const LONGEST_SLEEP: Duration = Duration::from_secs(60);

/// How many polls one pass ends. More than that are ended by the next pass,
/// which follows at once.
const BATCH: i64 = 100;

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("poll-ends", |state, stop| async move {
        loop {
            let next = match pass(&state, Timestamp::now()).await {
                Ok(next) => next,
                Err(error) => {
                    warn!(target: TARGET, "Ending polls failed: {error}");
                    None
                }
            };
            let wait = next.map_or(LONGEST_SLEEP, |at| {
                Timestamp::now().until(at).min(LONGEST_SLEEP)
            });
            tokio::select! {
                () = state.poll_alarm().rung() => {}
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

/// Ends the polls whose time is up by `now`, each at the time it was due, and
/// tells whoever sees their channels. Says when the next running poll ends.
pub async fn pass(state: &AppState, now: Timestamp) -> Result<Option<Timestamp>, DbError> {
    let due: Vec<(Id, Id, Id)> = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            let mut statement = db.prepare_cached(
                "SELECT p.\"MessageId\", m.\"ChannelId\", c.\"ServerId\" FROM \"Polls\" p
                 JOIN \"Messages\" m ON m.\"Id\" = p.\"MessageId\"
                 JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\"
                 WHERE p.\"EndedAt\" IS NULL AND p.\"EndsAt\" <= ?1
                 ORDER BY p.\"EndsAt\", p.\"MessageId\" LIMIT ?2",
            )?;
            Ok(statement
                .query_map(params![now, BATCH], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    for (message, channel, server) in due {
        // An early end that came first leaves nothing to do here.
        let ended = state
            .database()
            .write(move |tx| -> Result<_, DbError> {
                let ended = tx.execute(
                    "UPDATE \"Polls\" SET \"EndedAt\" = \"EndsAt\" WHERE \"MessageId\" = ?1 AND \"EndedAt\" IS NULL",
                    [message],
                )?;
                if ended == 0 {
                    return Ok(None);
                }
                let sequence = super::journal(tx, message)?;
                Ok(read::one(tx, message, None)?.map(|poll| (sequence, poll)))
            })
            .await?;
        if let Some((sequence, poll)) = ended
            && let Err(error) =
                super::routes::announce(state, server, message, channel, sequence, &poll).await
        {
            warn!(target: TARGET, "A poll ended but its end was not announced: {error:?}");
        }
    }
    state
        .database()
        .read(|db| -> Result<_, DbError> {
            Ok(db.query_row(
                "SELECT MIN(\"EndsAt\") FROM \"Polls\" WHERE \"EndedAt\" IS NULL",
                [],
                |row| row.get::<_, Option<Timestamp>>(0),
            )?)
        })
        .await
}
