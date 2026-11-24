//! Runs one archive backup at a time, outside the request that asked for it:
//! with every object of the store it can take minutes. The archive is written
//! under a name the list ignores and renamed once complete, so the list never
//! shows half a backup.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags};
use stellarr_messages::MessageKey;
use stellarr_storage::StoredObjectInfo;
use tokio::sync::oneshot;
use tracing::error;

use super::archive::{self, Sources, utf16_order};
use super::listing::{self, ARCHIVE_EXTENSION};
use super::parts::Parts;
use super::progress::Progress;
use super::{BackupFailed, random_token, snapshot, space};
use crate::secrets::{self, Source};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// Room kept free beyond the estimate, for the snapshot and the zip's own
/// records.
const SPACE_MARGIN: f64 = 1.1;

/// A backup being written.
pub struct Job {
    pub id: String,
    pub parts: Parts,
    pub started_at: Timestamp,
    pub progress: Progress,
}

/// Why the last backup did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub id: String,
    pub message: String,
    pub at: Timestamp,
}

/// A job that was started, and the way to wait for how it ended.
pub struct Started {
    pub job: Arc<Job>,
    /// True when the backup was written and recorded.
    pub finished: oneshot::Receiver<bool>,
}

#[derive(Default)]
struct Inner {
    running: Option<Arc<Job>>,
    last_failure: Option<Failure>,
}

#[derive(Default)]
pub struct Jobs {
    inner: Mutex<Inner>,
    stopping: Arc<AtomicBool>,
    checkpoint: Mutex<super::schedule::Checkpoint>,
    gate: tokio::sync::Mutex<()>,
}

impl Jobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn running(&self) -> Option<Arc<Job>> {
        self.lock().running.clone()
    }

    pub fn last_failure(&self) -> Option<Failure> {
        self.lock().last_failure.clone()
    }

    /// What the automatic backup remembers.
    pub fn checkpoint(&self) -> super::schedule::Checkpoint {
        self.checkpoint.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_checkpoint(&self, checkpoint: super::schedule::Checkpoint) {
        *self.checkpoint.lock().unwrap_or_else(|p| p.into_inner()) = checkpoint;
    }

    /// One pass of the automatic backup at a time.
    pub fn gate(&self) -> &tokio::sync::Mutex<()> {
        &self.gate
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }

    /// Tells a running backup that the server is stopping, so that it ends
    /// instead of holding the shutdown up.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }

    /// The backup that was asked for, or none while another one runs.
    pub fn try_start(
        &self,
        state: &AppState,
        parts: Parts,
        server: Id,
        actor: Id,
        automatic: bool,
    ) -> Option<Started> {
        let job = {
            let mut inner = self.lock();
            if inner.running.is_some() {
                return None;
            }
            let now = Timestamp::now();
            let job = Arc::new(Job {
                id: listing::new_id(now, ARCHIVE_EXTENSION),
                parts,
                started_at: now,
                progress: Progress::new(0),
            });
            inner.running = Some(job.clone());
            job
        };
        let (done, finished) = oneshot::channel();
        let state = state.clone();
        let running = job.clone();
        tokio::spawn(async move {
            let outcome = run(&state, running.clone(), server, actor, automatic).await;
            let succeeded = outcome.is_ok();
            {
                let mut inner = state.backups().lock();
                match outcome {
                    Ok(()) => inner.last_failure = None,
                    Err(failure) if failure.cancelled => {}
                    Err(failure) => {
                        error!(target: "Stellarr.Api.Backups", "Backup {} failed: {failure}", running.id);
                        inner.last_failure = Some(Failure {
                            id: running.id.clone(),
                            message: failure.message().to_string(),
                            at: Timestamp::now(),
                        });
                    }
                }
                inner.running = None;
            }
            let _ = done.send(succeeded);
        });
        Some(Started { job, finished })
    }
}

async fn run(
    state: &AppState,
    job: Arc<Job>,
    server: Id,
    actor: Id,
    automatic: bool,
) -> Result<(), BackupFailed> {
    let directory = listing::directory(&state.settings().data_root);
    let partial = directory.join(format!(".{}.partial", job.id));
    let mut snapshot_path: Option<PathBuf> = None;
    let outcome = write_and_record(state, &job, server, actor, automatic, &directory, &partial, &mut snapshot_path).await;
    if outcome.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    if let Some(path) = snapshot_path {
        snapshot::remove_with_side_files(&path);
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn write_and_record(
    state: &AppState,
    job: &Arc<Job>,
    server: Id,
    actor: Id,
    automatic: bool,
    directory: &Path,
    partial: &Path,
    snapshot_path: &mut Option<PathBuf>,
) -> Result<(), BackupFailed> {
    let settings = state.settings();
    tokio::fs::create_dir_all(directory).await?;
    let database = state.database().path().to_path_buf();
    let keys_folder = settings.data_root.join("keys");
    let data_root = std::path::absolute(&settings.data_root).unwrap_or(settings.data_root.clone());

    // An upload publishes its object before committing its row. Taking the
    // snapshot first makes every upload named by it eligible for the later
    // listing, even while people continue sending files.
    if job.parts.has(Parts::DATABASE) {
        ensure_space(
            state,
            directory,
            estimate(Parts::DATABASE, &database, &[], &keys_folder, &data_root).await?,
        )
        .await?;
        let path = directory.join(format!(".snapshot-{}.partial", random_token()));
        *snapshot_path = Some(path.clone());
        let source = database.clone();
        tokio::task::spawn_blocking(move || snapshot::copy_database(&source, &path))
            .await
            .map_err(|error| BackupFailed::new(error.to_string()))??;
    }

    let mut objects: Vec<StoredObjectInfo> = Vec::new();
    if job.parts.has(Parts::OBJECTS) {
        objects = list_objects(state, snapshot_path.as_deref())
            .await
            .map_err(|reason| {
                BackupFailed::new(
                    state
                        .messages()
                        .get(MessageKey::CockpitBackupStorageUnreadable, &[("reason", &reason)])
                        .unwrap_or(reason),
                )
            })?;
    }
    job.progress.set_total(objects.len());
    ensure_space(
        state,
        directory,
        estimate(job.parts, &database, &objects, &keys_folder, &data_root).await?,
    )
    .await?;

    let instance_name = state
        .database()
        .read(move |connection| -> Result<String, crate::database::DbError> {
            Ok(connection.query_row(
                "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| row.get(0),
            )?)
        })
        .await
        .map_err(|error| BackupFailed::new(error.to_string()))?;
    let environment_secrets = {
        let environment = settings.secrets.clone();
        let sealer = state.sealer().clone();
        state
            .database()
            .read(move |connection| -> Result<Vec<String>, crate::database::DbError> {
                let resolved = secrets::read(connection, &environment, &sealer)?;
                Ok(secrets::ALL
                    .iter()
                    .filter(|definition| resolved.get(definition).source == Source::Environment)
                    .map(|definition| definition.variable().to_string())
                    .collect())
            })
            .await
            .map_err(|error| BackupFailed::new(error.to_string()))?
    };

    let sources = Sources {
        database,
        store: state.storage_handle(),
        objects,
        keys_folder,
        instance_name,
        environment_secrets,
        work_folder: directory.to_path_buf(),
        data_root,
        database_snapshot: snapshot_path.clone(),
    };
    let wanted = job.parts;
    let target = partial.to_path_buf();
    let runtime = tokio::runtime::Handle::current();
    let cancel = state.backups().stopping.clone();
    let running = job.clone();
    let manifest = tokio::task::spawn_blocking({
        let target = target.clone();
        move || archive::write(&target, wanted, &sources, &running.progress, &cancel, &runtime)
    })
    .await
    .map_err(|error| BackupFailed::new(error.to_string()))??;

    let final_path = directory.join(&job.id);
    if automatic {
        if !manifest.missing_objects.is_empty() {
            return Err(BackupFailed::new("Automatic backup contains missing objects."));
        }
        let (checked, folder) = (target.clone(), directory.to_path_buf());
        let verdict = tokio::task::spawn_blocking(move || archive::verify(&checked, &folder))
            .await
            .map_err(|error| BackupFailed::new(error.to_string()))?;
        if verdict != "ok" {
            return Err(BackupFailed::new(verdict));
        }
    }
    tokio::fs::rename(&target, &final_path).await?;
    if automatic {
        tokio::fs::write(
            listing::marker_path(&final_path),
            Timestamp::now().to_wire_text(),
        )
        .await?;
    }

    let sentence = state
        .messages()
        .get(MessageKey::AuditBackupCreated, &[("id", &job.id)])
        .map_err(|error| BackupFailed::new(error.to_string()))?;
    let sentence = if automatic {
        format!("[automatic] {sentence}")
    } else {
        sentence
    };
    state
        .database()
        .write(move |transaction| -> Result<(), crate::database::DbError> {
            crate::panel::record(transaction, server, actor, sentence)?;
            Ok(())
        })
        .await
        .map_err(|error| BackupFailed::new(error.to_string()))?;
    Ok(())
}

/// Every object of the store and every key the snapshot of the database
/// refers to, in key order. Deletions can run after the snapshot too, so a
/// key the snapshot names is kept even when it disappeared before the
/// listing: its absence is then reported, and an automatic backup fails safely.
async fn list_objects(
    state: &AppState,
    snapshot: Option<&Path>,
) -> Result<Vec<StoredObjectInfo>, String> {
    let mut objects: std::collections::HashMap<String, StoredObjectInfo> =
        std::collections::HashMap::new();
    let mut listing = state.storage().list();
    while let Some(item) = listing.next().await.map_err(|error| error.to_string())? {
        objects.insert(item.key.clone(), item);
    }
    if let Some(path) = snapshot {
        let path = path.to_path_buf();
        let referred = tokio::task::spawn_blocking(move || referred_by(&path))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        for (key, size) in referred {
            objects
                .entry(key.clone())
                .or_insert(StoredObjectInfo { key, size });
        }
    }
    let mut all: Vec<StoredObjectInfo> = objects.into_values().collect();
    all.sort_by(|left, right| utf16_order(&left.key, &right.key));
    Ok(all)
}

fn referred_by(snapshot: &Path) -> rusqlite::Result<Vec<(String, u64)>> {
    let connection = Connection::open_with_flags(
        snapshot,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut statement = connection.prepare(
        "SELECT \"ObjectKey\", \"SizeBytes\" FROM \"ImageAttachments\"
         UNION ALL
         SELECT \"PosterObjectKey\", coalesce(\"PosterSizeBytes\", 0) FROM \"ImageAttachments\" WHERE \"PosterObjectKey\" IS NOT NULL
         UNION ALL
         SELECT \"PreviewObjectKey\", coalesce(\"PreviewSizeBytes\", 0) FROM \"ImageAttachments\" WHERE \"PreviewObjectKey\" IS NOT NULL
         UNION ALL
         SELECT \"ObjectKey\", \"SizeBytes\" FROM \"ServerEmoji\"",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
        ))
    })?;
    rows.collect()
}

/// About how many bytes the archive will take, before the zip compresses the
/// database.
async fn estimate(
    parts: Parts,
    database: &Path,
    objects: &[StoredObjectInfo],
    keys_folder: &Path,
    data_root: &Path,
) -> Result<u64, BackupFailed> {
    let (database, keys_folder, data_root) = (
        database.to_path_buf(),
        keys_folder.to_path_buf(),
        data_root.to_path_buf(),
    );
    let objects: u64 = objects.iter().map(|object| object.size).sum();
    tokio::task::spawn_blocking(move || -> Result<u64, BackupFailed> {
        let size_of = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
        let mut bytes = 0u64;
        if parts.has(Parts::DATABASE) {
            let mut wal = database.as_os_str().to_owned();
            wal.push("-wal");
            // The snapshot and its copy in the archive are both on this disk
            // for a moment.
            bytes += 2 * (size_of(&database) + size_of(Path::new(&wal)));
        }
        if parts.has(Parts::OBJECTS) {
            bytes += objects;
        }
        if parts.has(Parts::KEYS)
            && let Ok(entries) = std::fs::read_dir(&keys_folder)
        {
            bytes += entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| size_of(&entry.path()))
                .sum::<u64>();
        }
        if parts.has(Parts::INSTANCE) {
            bytes += archive::instance_files(&data_root)?
                .iter()
                .map(|(path, _)| size_of(path))
                .sum::<u64>();
        }
        Ok(bytes)
    })
    .await
    .map_err(|error| BackupFailed::new(error.to_string()))?
}

async fn ensure_space(state: &AppState, directory: &Path, estimate: u64) -> Result<(), BackupFailed> {
    let folder = directory.to_path_buf();
    let free = tokio::task::spawn_blocking(move || space::free_bytes(&folder))
        .await
        .ok()
        .flatten();
    let Some(free) = free else {
        return Ok(());
    };
    let needed = (estimate as f64 * SPACE_MARGIN) as u64;
    if needed > free {
        let message = state
            .messages()
            .get(
                MessageKey::CockpitBackupNoSpace,
                &[("needed", &megabytes(needed)), ("free", &megabytes(free))],
            )
            .map_err(|error| BackupFailed::new(error.to_string()))?;
        return Err(BackupFailed::new(message));
    }
    Ok(())
}

/// `1.5 MB`: one decimal where there is one to show.
fn megabytes(bytes: u64) -> String {
    let value = (bytes as f64 / 1024.0 / 1024.0 * 10.0).round() / 10.0;
    if value.fract() == 0.0 {
        format!("{value:.0} MB")
    } else {
        format!("{value:.1} MB")
    }
}
