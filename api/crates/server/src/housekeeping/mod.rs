//! The housekeeping that keeps an instance from growing without bound. Once
//! an hour it compacts the message journal, removes uploads nobody uses,
//! removes from the object store what was queued for removal, prunes old
//! backups and logs, and deletes the short-lived records of signing in once
//! they are well past use.
//!
//! A run never holds the one writer for long: each piece is its own short
//! transaction (a delete is done a thousand rows at a time, one orphan or
//! one queued object at a time), so requests that write wait for a moment and
//! not for the length of the run. A run looks at whether the server is
//! stopping between pieces.
//!
//! What a run does not touch: anything the operator switched automatic cleanup
//! off for. Compacting the journal and emptying the deletion queue are
//! correctness and not retention, so they run whatever the operator chose.

mod expired;
mod files;
mod journal;
mod orphans;
mod queue;
pub(crate) use queue::enqueue as queue_deletion;
#[cfg(test)]
mod tests;

use std::time::Duration;

use tracing::{info, warn};

use crate::background::{Stop, Worker};
use crate::panel::operations::{self, Retention};
use crate::state::AppState;
use crate::values::Timestamp;

/// Whether the server is stopping, asked between the pieces of a run.
pub type Stopping<'a> = &'a (dyn Fn() -> bool + Sync);

/// How long a run steps aside between two transactions. A request that waits
/// for the write lock asks again after a growing pause, so transactions that
/// follow one another with no gap can keep it out for a second or more even
/// when each is short; a few milliseconds is enough for it to get in.
const PAUSE: Duration = Duration::from_millis(5);

/// A transaction of a run: `Database::write`, with the time the call took
/// noted when the tests ask how long a run holds the writer.
pub(crate) async fn write<T, E, F>(state: &AppState, work: F) -> Result<T, E>
where
    F: FnOnce(&rusqlite::Transaction) -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<crate::database::DbError> + From<rusqlite::Error> + Send + 'static,
{
    #[cfg(test)]
    let began = std::time::Instant::now();
    let outcome = state.database().write(work).await;
    #[cfg(test)]
    LONGEST_WRITE_MICROS.fetch_max(
        u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX),
        std::sync::atomic::Ordering::Relaxed,
    );
    outcome
}

/// The longest any call of [`write`] took, in microseconds.
#[cfg(test)]
pub(crate) static LONGEST_WRITE_MICROS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) async fn step_aside() {
    tokio::time::sleep(PAUSE).await;
}

const TARGET: &str = "Stellarr.Api.Maintenance";

/// What one run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// False when the operator switched automatic cleanup off.
    pub enabled: bool,
    pub orphan_attachments: usize,
    pub backup_files: usize,
    pub log_files: usize,
    /// Objects still waiting in the deletion queue after the run.
    pub pending_object_deletions: i64,
}

/// One pass of the housekeeping, with every age measured against `now`.
pub async fn run(
    state: &AppState,
    now: Timestamp,
    stop: Stopping<'_>,
) -> Result<Summary, crate::database::DbError> {
    let settings = state.settings().maintenance;
    let batch = settings.batch();
    journal::compact(state, now, settings.journal_days(), stop).await?;

    let retention = state
        .database()
        .read(
            |connection| -> Result<Retention, crate::database::DbError> {
                Ok(operations::newest(connection)?)
            },
        )
        .await?;
    if !retention.automatic_cleanup {
        // Explicit deletes and failed upload rollbacks are correctness, not
        // retention, so they are carried out whatever the owner chose.
        queue::drain(state, batch, stop).await;
        return Ok(Summary {
            enabled: false,
            orphan_attachments: 0,
            backup_files: 0,
            log_files: 0,
            pending_object_deletions: queue::pending(state).await?,
        });
    }

    let orphan_attachments = orphans::sweep(
        state,
        cutoff(now, retention.orphan_object_days),
        batch,
        stop,
    )
    .await?;
    queue::drain(state, batch, stop).await;

    let root = state.settings().data_root.clone();
    let backup_files = files::prune(
        root.join("backups"),
        cutoff(now, retention.backup_days),
        "backup",
    )
    .await;
    let log_files = files::prune(root.join("logs"), cutoff(now, retention.log_days), "log").await;

    expired::delete(state, now, stop).await?;

    Ok(Summary {
        enabled: true,
        orphan_attachments,
        backup_files,
        log_files,
        pending_object_deletions: queue::pending(state).await?,
    })
}

/// How far back a retention reaches: its days, kept between 1 and 3650.
pub(crate) fn cutoff(now: Timestamp, retention_days: i64) -> Timestamp {
    cutoff_days(now, retention_days.clamp(1, 3650))
}

/// `days` before `now`, as they are.
pub(crate) fn cutoff_days(now: Timestamp, days: i64) -> Timestamp {
    now.minus(Duration::from_secs(
        u64::try_from(days).unwrap_or(0) * 86_400,
    ))
}

/// The wait before the first run and the time between runs: the settings,
/// the first not below nothing and the second not below an hour.
pub(crate) fn intervals(settings: &crate::settings::MaintenanceSettings) -> (Duration, Duration) {
    let first =
        Duration::from_secs(u64::try_from(settings.initial_delay_minutes.max(0)).unwrap_or(0) * 60);
    let every =
        Duration::from_secs(u64::try_from(settings.interval_hours.max(1)).unwrap_or(1) * 3600);
    (first, every)
}

/// Calls `pass` after `first` and then every `every`, until the server stops. A
/// pass that fails is logged and the schedule goes on.
pub(crate) async fn on_schedule<F, Fut>(first: Duration, every: Duration, stop: &Stop, mut pass: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if !stop.sleep(first).await {
        return;
    }
    loop {
        if let Err(error) = pass().await {
            warn!(target: TARGET, "Maintenance run failed: {error}");
        }
        if !stop.sleep(every).await {
            return;
        }
    }
}

/// Every hour, after a first wait at the start. A pass that fails is logged
/// and the next one tries again.
pub fn workers() -> Vec<Worker> {
    vec![Worker::new("housekeeping", |state, stop| async move {
        let settings = state.settings().maintenance;
        if !settings.enabled {
            return;
        }
        let (first, every) = intervals(&settings);
        let ending = stop.clone();
        on_schedule(first, every, &stop, || async {
            match run(&state, Timestamp::now(), &|| ending.is_stopping()).await {
                // With cleanup switched off every count is zero, and saying
                // that each hour would only suggest that something had been
                // removed.
                Ok(summary) if summary.enabled => {
                    info!(
                        target: TARGET,
                        "Maintenance removed {} orphan attachments, {} backup files and {} log files; {} object deletions remain queued.",
                        summary.orphan_attachments,
                        summary.backup_files,
                        summary.log_files,
                        summary.pending_object_deletions
                    );
                    Ok(())
                }
                Ok(_) => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        })
        .await;
    })]
}
