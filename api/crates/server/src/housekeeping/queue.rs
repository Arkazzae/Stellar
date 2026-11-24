//! The queue of objects to remove from the store. Whatever deletes a row that
//! owned an object (an upload, a preview, a picture, a server) puts the
//! object's key here in the same transaction, so that neither a request that
//! is cut short nor a store that cannot be reached leaves an object behind.

use tracing::warn;

use super::Stopping;
use super::TARGET;
use crate::database::DbError;
use crate::state::AppState;

/// Deletes the oldest queued objects, up to `batch`. One that cannot be
/// deleted stays queued, to be asked again by the next run.
pub async fn drain(state: &AppState, batch: usize, stop: Stopping<'_>) {
    let keys = state
        .database()
        .read(move |connection| -> Result<Vec<String>, DbError> {
            let mut statement = connection.prepare(
                "SELECT \"ObjectKey\" FROM \"ObjectDeletions\" ORDER BY \"QueuedAt\" LIMIT ?1",
            )?;
            Ok(statement
                .query_map([batch as i64], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await;
    let keys = match keys {
        Ok(keys) => keys,
        Err(error) => {
            warn!(target: TARGET, "The deletion queue cannot be read: {error}");
            return;
        }
    };
    for key in keys {
        if stop() {
            return;
        }
        // The object is gone, or never was, before the row is: a crash in
        // between asks the store again, and deleting what is not there is no
        // error.
        if let Err(error) = state.storage().delete(&key).await {
            warn!(target: TARGET, "Deleting stored object {key} failed; it remains queued for retry: {error}");
            continue;
        }
        let forgotten = key.clone();
        let removed = state
            .database()
            .write(move |tx| -> Result<(), DbError> {
                tx.execute(
                    "DELETE FROM \"ObjectDeletions\" WHERE \"ObjectKey\" = ?1",
                    [forgotten],
                )?;
                Ok(())
            })
            .await;
        if let Err(error) = removed {
            warn!(target: TARGET, "A deleted object {key} is still queued: {error}");
        }
    }
}

/// How many objects wait in the queue.
pub async fn pending(state: &AppState) -> Result<i64, DbError> {
    state
        .database()
        .read(|connection| -> Result<i64, DbError> {
            Ok(
                connection.query_row("SELECT COUNT(*) FROM \"ObjectDeletions\"", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .await
}
