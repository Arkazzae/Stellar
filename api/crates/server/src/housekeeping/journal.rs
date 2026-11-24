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
pub const CHUNK: i64 = 1_000;

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
    // Which channels have entries to remove is found by reading, which keeps
    // nobody waiting; only the few rows that say so are written under the lock.
    let channels = state
        .database()
        .read(move |connection| -> Result<Vec<Id>, DbError> {
            let mut statement = connection.prepare(
                "SELECT DISTINCT \"ChannelId\" FROM \"MessageChanges\" WHERE \"CreatedAt\" < ?1",
            )?;
            Ok(statement
                .query_map([cutoff], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    if channels.is_empty() {
        return Ok(());
    }
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            for channel in channels {
                tx.execute(
                    "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
                    params![channel, Id::from_bytes([0; 16]), RESET, now],
                )?;
            }
            Ok(())
        })
        .await?;
    delete_in_chunks(
        state,
        "MessageChanges",
        "\"Sequence\"",
        "\"CreatedAt\" < ?1",
        cutoff,
        stop,
    )
    .await
}

/// Deletes the rows of `table` that `condition` (which uses `?1` for `bound`)
/// picks out, `CHUNK` at a time, each chunk in a transaction of its own.
/// Chunks go by increasing `key`, so rows that stay are not looked at again
/// and again: the work is the size of the table once, not once per chunk.
pub async fn delete_in_chunks(
    state: &AppState,
    table: &'static str,
    key: &'static str,
    condition: &'static str,
    bound: Timestamp,
    stop: Stopping<'_>,
) -> Result<(), DbError> {
    let mut after: i64 = i64::MIN;
    loop {
        if stop() {
            return Ok(());
        }
        let last = state
            .database()
            .write(move |tx| -> Result<Option<i64>, DbError> {
                let upper: Option<i64> = tx.query_row(
                    &format!(
                        "SELECT MAX(k) FROM (SELECT {key} AS k FROM \"{table}\"
                         WHERE {key} > ?2 AND {condition} ORDER BY {key} LIMIT ?3)"
                    ),
                    params![bound, after, CHUNK],
                    |row| row.get(0),
                )?;
                let Some(upper) = upper else {
                    return Ok(None);
                };
                tx.execute(
                    &format!(
                        "DELETE FROM \"{table}\" WHERE {key} > ?2 AND {key} <= ?3 AND {condition}"
                    ),
                    params![bound, after, upper],
                )?;
                Ok(Some(upper))
            })
            .await?;
        match last {
            Some(upper) => after = upper,
            None => return Ok(()),
        }
    }
}
