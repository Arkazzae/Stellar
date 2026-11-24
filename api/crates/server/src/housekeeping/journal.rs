//! The journal of message changes that clients replay to close the gap left by
//! a dropped realtime connection.

use rusqlite::params;

use super::Stopping;
use crate::database::DbError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The kind of an entry that tells a client its cursor points into a range
/// that is gone, so it reloads the channel instead of replaying.
const RESET: i64 = 2;

/// How many entries one transaction removes.
pub const CHUNK: i64 = 5_000;

/// Deletes the journal's entries older than `days`. A reset entry per affected
/// channel stays behind, so a client whose cursor points into the removed
/// range is told to reload instead of silently missing changes.
///
/// The resets are written first, in one short transaction, and the old
/// entries then go a few thousand at a time. A run cut short between the two
/// leaves the channels with a reset each and entries still to remove, and the
/// next run writes one more reset for them, which does no harm.
pub async fn compact(
    state: &AppState,
    now: Timestamp,
    days: i64,
    stop: Stopping<'_>,
) -> Result<(), DbError> {
    let cutoff = super::cutoff_days(now, days);
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\")
                 SELECT DISTINCT \"ChannelId\", ?1, ?2, ?3 FROM \"MessageChanges\" WHERE \"CreatedAt\" < ?4",
                params![Id::from_bytes([0; 16]), RESET, now, cutoff],
            )?;
            Ok(())
        })
        .await?;
    loop {
        let removed = state
            .database()
            .write(move |tx| -> Result<usize, DbError> {
                Ok(tx.execute(
                    "DELETE FROM \"MessageChanges\" WHERE \"Sequence\" IN
                       (SELECT \"Sequence\" FROM \"MessageChanges\" WHERE \"CreatedAt\" < ?1 LIMIT ?2)",
                    params![cutoff, CHUNK],
                )?)
            })
            .await?;
        if (removed as i64) < CHUNK || stop() {
            return Ok(());
        }
    }
}
