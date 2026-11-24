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
#[cfg(test)]
mod tests;

use std::time::Duration;

use tracing::{info, warn};

use crate::background::Worker;
use crate::panel::operations::{self, Retention};
use crate::state::AppState;
use crate::values::Timestamp;

/// Whether the server is stopping, asked between the pieces of a run.
pub type Stopping<'a> = &'a (dyn Fn() -> bool + Sync);

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

/// Every hour, after a first wait at the start. A pass that fails is logged
/// and the next one tries again.
pub fn workers() -> Vec<Worker> {
    vec![Worker::new("housekeeping", |state, stop| async move {
        let settings = state.settings().maintenance;
        if !settings.enabled {
            return;
        }
        let first = Duration::from_secs(
            u64::try_from(settings.initial_delay_minutes.max(0)).unwrap_or(0) * 60,
        );
        let every =
            Duration::from_secs(u64::try_from(settings.interval_hours.max(1)).unwrap_or(1) * 3600);
        if !stop.sleep(first).await {
            return;
        }
        loop {
            match run(&state, Timestamp::now(), &|| stop.is_stopping()).await {
                // With cleanup switched off every count is zero, and saying
                // that each hour would only suggest that something had been
                // removed.
                Ok(summary) if summary.enabled => info!(
                    target: TARGET,
                    "Maintenance removed {} orphan attachments, {} backup files and {} log files; {} object deletions remain queued.",
                    summary.orphan_attachments,
                    summary.backup_files,
                    summary.log_files,
                    summary.pending_object_deletions
                ),
                Ok(_) => {}
                Err(error) => warn!(target: TARGET, "Maintenance run failed: {error}"),
            }
            if !stop.sleep(every).await {
                return;
            }
        }
    })]
}
