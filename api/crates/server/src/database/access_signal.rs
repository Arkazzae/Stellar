//! Tells the API's realtime watcher that a committed transaction changed
//! realtime access, and only then.
//!
//! Triggers in the schema journal every change of sessions, accounts, roles,
//! overrides, membership and channels in `RealtimeChanges`. A connection that
//! registers an [`AccessSignal`] also gets a connection-local trigger that
//! notes the first journal row a transaction inserts; message writes and
//! last-seen touches never reach it. [`AccessTransaction::commit`] then wakes
//! the watcher when a journal row survived:
//!
//! - the API's own writer wakes it in memory, once the transaction committed;
//! - a writer in another process (the CLI) writes a fresh token to
//!   `<database>.access-wake` while it still holds the write lock. The watcher
//!   reads that small file while sockets are open, and reads the journal
//!   under the write lock, so it sees the writer's final commit or rollback,
//!   even when the writer dies during COMMIT. Without the file no API has
//!   started on this database, and nothing is written.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, Transaction};
use tokio::sync::watch;

use super::DbError;
use crate::values::Id;

/// One writer's note of its first journal row, and how it wakes the watcher.
/// Clones share both, so they belong to one serialized writer.
#[derive(Clone)]
pub struct AccessSignal {
    first: Arc<AtomicI64>,
    wake: Wake,
}

#[derive(Clone)]
enum Wake {
    /// The API's own writer: the watcher of the same process listens here.
    Local(Arc<watch::Sender<()>>),
    /// A writer in another process: the token file beside the database.
    File(PathBuf),
}

/// The token file other processes write for the database at `database`.
pub fn wake_file(database: &Path) -> PathBuf {
    database.with_added_extension("access-wake")
}

/// Creates the token file when it is missing. The API only reads it, so one
/// that another user owns (`docker exec` runs the CLI as root) still works.
/// The CLI never creates it.
pub(crate) fn prepare_wake_file(database: &Path) -> io::Result<()> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(wake_file(database))
    {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        created => created.map(drop),
    }
}

impl AccessSignal {
    /// For the API's own writer connections.
    pub(crate) fn local() -> Self {
        Self {
            first: Arc::new(AtomicI64::new(i64::MAX)),
            wake: Wake::Local(Arc::new(watch::channel(()).0)),
        }
    }

    /// For a writer in another process, such as the CLI, on the database at
    /// `database`.
    pub fn external(database: &Path) -> Self {
        Self {
            first: Arc::new(AtomicI64::new(i64::MAX)),
            wake: Wake::File(wake_file(database)),
        }
    }

    /// What the API's watcher waits on; `None` for another process.
    pub(crate) fn subscribe(&self) -> Option<watch::Receiver<()>> {
        match &self.wake {
            Wake::Local(sender) => Some(sender.subscribe()),
            Wake::File(_) => None,
        }
    }

    /// Installs the trigger that notes journal rows on `db`, a connection of
    /// this writer.
    pub fn register(&self, db: &Connection) -> rusqlite::Result<()> {
        let first = self.first.clone();
        db.create_scalar_function(
            "stellarr_access_pending",
            1,
            FunctionFlags::SQLITE_UTF8,
            move |ctx| {
                first.fetch_min(ctx.get::<i64>(0)?, Ordering::Relaxed);
                Ok(0i64)
            },
        )?;
        db.execute_batch(
            "CREATE TEMP TRIGGER SignalRealtimeChange AFTER INSERT ON main.RealtimeChanges
             BEGIN SELECT stellarr_access_pending(NEW.Seq); END",
        )
    }
}

/// A write transaction that wakes the watcher when it commits an access
/// change.
pub struct AccessTransaction<'a> {
    tx: Transaction<'a>,
    signal: Option<&'a AccessSignal>,
}

impl<'a> AccessTransaction<'a> {
    pub fn new(tx: Transaction<'a>, signal: Option<&'a AccessSignal>) -> Self {
        if let Some(signal) = signal {
            signal.first.store(i64::MAX, Ordering::Relaxed);
        }
        Self { tx, signal }
    }

    /// Commits. A token that cannot be written rolls the change back, so an
    /// operator is never told of a change the API would not apply.
    pub fn commit(self) -> Result<(), DbError> {
        let signal = match self.signal {
            Some(signal) if self.changed_access(signal)? => Some(signal),
            _ => None,
        };
        if let Some(AccessSignal {
            wake: Wake::File(path),
            ..
        }) = signal
        {
            write_token(path).map_err(|e| {
                DbError::Task(format!(
                    "cannot signal access changes at {}: {e}",
                    path.display()
                ))
            })?;
        }
        self.tx.commit()?;
        if let Some(AccessSignal {
            wake: Wake::Local(sender),
            ..
        }) = signal
        {
            sender.send_replace(());
        }
        Ok(())
    }

    /// Whether a journal row of this transaction survived: a statement or a
    /// savepoint may have rolled back the one the trigger noted.
    fn changed_access(&self, signal: &AccessSignal) -> rusqlite::Result<bool> {
        let first = signal.first.load(Ordering::Relaxed);
        if first == i64::MAX {
            return Ok(false);
        }
        self.tx
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM RealtimeChanges WHERE Seq >= ?1)")?
            .query_row([first], |row| row.get(0))
    }
}

/// Overwrites the token in place. No file means no API to wake.
fn write_token(path: &Path) -> io::Result<()> {
    match OpenOptions::new().write(true).open(path) {
        Ok(mut file) => file.write_all(Id::random().to_string().as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl<'a> Deref for AccessTransaction<'a> {
    type Target = Transaction<'a>;
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
