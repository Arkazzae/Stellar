//! The automatic backup: a full archive on a schedule, checked, and copied to
//! every destination that is switched on.
//!
//! A cycle is complete only when the archive was written, verified and
//! accepted by every destination. Intervals are measured from the last
//! success, not on a fixed time of day. What survives a restart is a small
//! file at the data root, `automatic-backup.json`: a failed copy to a
//! destination is retried with the archive that was already made, instead of
//! making more of them.
//!
//! The worker is a thin timer around [`run_once`], which takes the clock and
//! the destinations it is given, so that the schedule can be tried at any
//! moment.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{error, info};

use super::BackupFailed;
use super::destination::Destination;
use super::listing;
use super::parts::Parts;
use super::policy::{self, Schedule};
use crate::background::Worker;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const CHECKPOINT_FILE: &str = "automatic-backup.json";
const TICK: Duration = Duration::from_secs(60);

/// What the schedule remembers between runs and restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Checkpoint {
    pub last_success_at: Option<Timestamp>,
    pub next_attempt_at: Option<Timestamp>,
    pub pending_backup_id: Option<String>,
    pub server_id: Option<Id>,
    pub last_error: Option<String>,
    pub last_failure_at: Option<Timestamp>,
}

impl Checkpoint {
    /// When the next attempt is due: an interval after the last success, a
    /// retry period after the last failure, else the time recorded.
    pub fn next_attempt(&self, schedule: &Schedule) -> Option<Timestamp> {
        match (self.last_error.as_ref(), self.last_success_at) {
            (None, Some(success)) => Some(success.plus(hours(schedule.interval_hours))),
            _ => match self.last_failure_at {
                Some(failure) => Some(failure.plus(minutes(schedule.retry_minutes))),
                None => self.next_attempt_at,
            },
        }
    }
}

fn hours(count: i64) -> Duration {
    Duration::from_secs(u64::try_from(count).unwrap_or(0) * 3600)
}

fn minutes(count: i64) -> Duration {
    Duration::from_secs(u64::try_from(count).unwrap_or(0) * 60)
}

/// What the panel is told of the automatic backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub interval_hours: i64,
    pub remote_enabled: bool,
    pub last_success_at: Option<Timestamp>,
    pub next_attempt_at: Option<Timestamp>,
    pub pending_backup_id: Option<String>,
    pub last_error: Option<String>,
}

pub fn status(checkpoint: &Checkpoint, schedule: &Schedule, remote_enabled: bool) -> Status {
    Status {
        enabled: schedule.enabled,
        interval_hours: schedule.interval_hours,
        remote_enabled,
        last_success_at: checkpoint.last_success_at,
        next_attempt_at: checkpoint.next_attempt(schedule),
        pending_backup_id: checkpoint.pending_backup_id.clone(),
        last_error: checkpoint.last_error.clone(),
    }
}

/// The moment the schedule goes by.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(Timestamp::now)
}

fn checkpoint_path(state: &AppState) -> PathBuf {
    std::path::absolute(&state.settings().data_root)
        .unwrap_or_else(|_| state.settings().data_root.clone())
        .join(CHECKPOINT_FILE)
}

/// Reads the checkpoint a restart finds. One that cannot be read starts the
/// schedule over, with a note of why.
pub async fn load(state: &AppState) {
    let path = checkpoint_path(state);
    let checkpoint = match tokio::fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<Checkpoint>(&bytes) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                tracing::error!(target: "Stellarr.Api.Backups", "The automatic backup checkpoint is unreadable: {error}");
                unreadable()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Checkpoint::default(),
        Err(error) => {
            tracing::error!(target: "Stellarr.Api.Backups", "The automatic backup checkpoint cannot be read: {error}");
            unreadable()
        }
    };
    state.backups().set_checkpoint(checkpoint);
}

fn unreadable() -> Checkpoint {
    Checkpoint {
        last_error: Some(
            "The automatic backup checkpoint is unreadable; a new backup will be attempted."
                .to_string(),
        ),
        ..Checkpoint::default()
    }
}

async fn save(state: &AppState, checkpoint: &Checkpoint) -> std::io::Result<()> {
    let path = checkpoint_path(state);
    if let Some(folder) = path.parent() {
        tokio::fs::create_dir_all(folder).await?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let text = serde_json::to_vec(checkpoint).map_err(std::io::Error::other)?;
    tokio::fs::write(&temporary, text).await?;
    tokio::fs::rename(&temporary, &path).await
}

/// What one pass of the schedule did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Off, not yet due, busy, or nobody to back up for.
    Skipped,
    Completed,
    Failed,
}

/// One pass: if a backup is due, makes it (or finds the one that still has to
/// reach a destination), sends it everywhere and records the result.
pub async fn run_once(
    state: &AppState,
    destinations: &[&dyn Destination],
    clock: &Clock,
) -> Outcome {
    let schedule = match current_schedule(state).await {
        Ok(schedule) => schedule,
        Err(error) => {
            error!(target: "Stellarr.Api.Backups", "The automatic backup schedule cannot be read: {error}");
            return Outcome::Failed;
        }
    };
    if !schedule.enabled {
        return Outcome::Skipped;
    }
    let Ok(_turn) = state.backups().gate().try_lock() else {
        return Outcome::Skipped;
    };
    let now = clock();
    let checkpoint = state.backups().checkpoint();
    if checkpoint.next_attempt(&schedule).is_some_and(|due| due > now)
        || state.backups().running().is_some()
    {
        return Outcome::Skipped;
    }
    match attempt(state, destinations, clock, &schedule, checkpoint).await {
        Ok(None) => Outcome::Skipped,
        Ok(Some(done)) => {
            state.backups().set_checkpoint(done.clone());
            if let Err(error) = save(state, &done).await {
                error!(target: "Stellarr.Api.Backups", "The automatic backup checkpoint cannot be saved: {error}");
            }
            info!(target: "Stellarr.Api.Backups", "Automatic backup completed at {}", done.last_success_at.map_or_else(String::new, |at| at.to_wire_text()));
            Outcome::Completed
        }
        Err((failure, _)) if failure.cancelled => Outcome::Skipped,
        Err((failure, kept)) => {
            error!(target: "Stellarr.Api.Backups", "Automatic backup failed: {failure}");
            let at = clock();
            let failed = Checkpoint {
                last_error: Some("Automatic backup failed; inspect the server logs.".to_string()),
                next_attempt_at: Some(at.plus(minutes(schedule.retry_minutes))),
                last_failure_at: Some(at),
                ..kept
            };
            state.backups().set_checkpoint(failed.clone());
            if let Err(error) = save(state, &failed).await {
                error!(target: "Stellarr.Api.Backups", "The automatic backup checkpoint cannot be saved: {error}");
            }
            Outcome::Failed
        }
    }
}

async fn current_schedule(state: &AppState) -> Result<Schedule, crate::database::DbError> {
    let sealer = state.sealer().clone();
    let defaults = state.settings().backups;
    state
        .database()
        .read(move |connection| -> Result<Schedule, crate::database::DbError> {
            Ok(policy::current(connection, &sealer, &defaults)?.0)
        })
        .await
}

/// The error of a pass that failed, with the checkpoint as it stood when it
/// did, which is what the failure is recorded on.
type Failed = (BackupFailed, Checkpoint);

// The failure carries the checkpoint it is recorded on, which is large; it is
// built once per failed pass, so boxing it would only add a step.
#[allow(clippy::result_large_err)]
async fn attempt(
    state: &AppState,
    destinations: &[&dyn Destination],
    clock: &Clock,
    schedule: &Schedule,
    mut checkpoint: Checkpoint,
) -> Result<Option<Checkpoint>, Failed> {
    let directory = listing::directory(&state.settings().data_root);
    let pending_exists = match (&checkpoint.server_id, &checkpoint.pending_backup_id) {
        (Some(_), Some(id)) if is_backup_id(id) => directory.join(id).is_file(),
        _ => false,
    };
    if !pending_exists {
        let owner = state
            .database()
            .read(|connection| -> Result<Option<(Id, Id)>, crate::database::DbError> {
                Ok(connection
                    .query_row(
                        "SELECT \"ServerId\", \"UserId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1 LIMIT 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?)
            })
            .await
            .map_err(|error| fail(error.to_string(), &checkpoint))?;
        // Setup has not created an instance yet.
        let Some((server, user)) = owner else {
            return Ok(None);
        };
        let Some(started) = state
            .backups()
            .try_start(state, Parts::ALL, server, user, true)
        else {
            return Ok(None);
        };
        let succeeded = started.finished.await.unwrap_or(false);
        if !succeeded {
            let message = state
                .backups()
                .last_failure()
                .map_or_else(|| "Automatic backup failed.".to_string(), |f| f.message);
            let mut failure = BackupFailed::new(message);
            failure.cancelled = state.backups().is_stopping();
            return Err((failure, checkpoint));
        }
        checkpoint.pending_backup_id = Some(started.job.id.clone());
        checkpoint.server_id = Some(server);
        if let Err(error) = save(state, &checkpoint).await {
            return Err(fail(error.to_string(), &checkpoint));
        }
        state.backups().set_checkpoint(checkpoint.clone());
    }

    let path = directory.join(checkpoint.pending_backup_id.clone().unwrap_or_default());
    let server = checkpoint.server_id.unwrap_or(Id::from_bytes([0; 16]));
    // Every enabled destination has to accept the archive: a completed local
    // copy alone is not an offsite copy.
    for destination in destinations {
        if destination.enabled().await
            && let Err(error) = destination.upload(&path, server).await
        {
            return Err((error, checkpoint));
        }
    }
    let now = clock();
    Ok(Some(Checkpoint {
        last_success_at: Some(now),
        next_attempt_at: Some(now.plus(hours(schedule.interval_hours))),
        ..Checkpoint::default()
    }))
}

fn fail(message: String, checkpoint: &Checkpoint) -> Failed {
    (BackupFailed::new(message), checkpoint.clone())
}

fn is_backup_id(id: &str) -> bool {
    !id.contains('/')
        && id.starts_with(listing::PREFIX)
        && id.ends_with(listing::ARCHIVE_EXTENSION)
}

/// The timer: checks once a minute whether a backup is due, after the initial
/// delay the schedule asks for.
pub fn workers() -> Vec<Worker> {
    vec![Worker::new("automatic-backups", |state, stop| async move {
        load(&state).await;
        let clock = system_clock();
        let started = clock();
        let offsite = super::offsite::Cloudflare::new(state.clone());
        let destinations: [&dyn Destination; 1] = [&offsite];
        loop {
            let delay = current_schedule(&state)
                .await
                .map_or(0, |schedule| schedule.initial_delay_minutes);
            if clock() >= started.plus(minutes(delay)) {
                tokio::select! {
                    () = stop.stopped() => break,
                    _ = run_once(&state, &destinations, &clock) => {}
                }
            }
            if !stop.sleep(TICK).await {
                break;
            }
        }
        state.backups().stop();
    })]
}
