//! The SQLite database.
//!
//! SQLite is a blocking library, so every statement runs on a thread of the
//! blocking pool and never on one that serves requests. Handlers do not hold
//! a connection: they hand a closure to [`Database::read`] or
//! [`Database::write`], which lends one for the length of the closure.
//!
//! ```ignore
//! let user = state.database().read(move |db| {
//!     db.query_row("SELECT ...", [&id], |row| row.get::<_, String>(0)).optional()
//! }).await?;
//!
//! let created = state.database().write(move |tx| {
//!     tx.execute("INSERT ...", params)?;
//!     Ok(new_id)
//! }).await?;
//! ```
//!
//! - A read sees one snapshot of the database for its whole length.
//! - A write is one transaction that takes the write lock when it starts
//!   (`BEGIN IMMEDIATE`) and so waits its turn behind other writers instead of
//!   failing when it tries to upgrade a read. It commits when the closure
//!   returns `Ok` and rolls back when it returns `Err`.
//! - A change is committed before the event that announces it is published,
//!   and the event is sent after `write` has returned.

mod error;
mod gate;
mod schema;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use tokio::sync::Semaphore;

use crate::settings::DatabaseSettings;

pub use error::DbError;
pub use gate::MutationGate;

/// One of the sixteen connections is reserved for the writer.
const MAX_READERS: usize = 15;
const WRITER_FLAGS: OpenFlags =
    OpenFlags::SQLITE_OPEN_READ_WRITE.union(OpenFlags::SQLITE_OPEN_CREATE);

#[derive(Clone)]
pub struct Database {
    readers: Arc<Pool>,
    writer: Arc<Pool>,
}

struct Pool {
    settings: DatabaseSettings,
    flags: OpenFlags,
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

impl Database {
    /// Opens the database, creating the file and its schema when there is
    /// none. It blocks, so it is called before the server starts or from a
    /// blocking thread.
    pub fn open(settings: &DatabaseSettings) -> Result<Database, DbError> {
        let mut connection = connect(settings, WRITER_FLAGS)?;
        schema::prepare(&mut connection)?;
        Ok(Database {
            readers: Pool::new(
                settings,
                OpenFlags::SQLITE_OPEN_READ_ONLY,
                MAX_READERS,
                vec![],
            ),
            writer: Pool::new(settings, WRITER_FLAGS, 1, vec![connection]),
        })
    }

    /// Where the database file is.
    pub fn path(&self) -> &Path {
        &self.writer.settings.path
    }

    /// Runs `work` on a connection, inside one snapshot of the database. A
    /// statement in it that would change something fails.
    pub async fn read<T, E, F>(&self, work: F) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<DbError> + From<rusqlite::Error> + Send + 'static,
    {
        self.readers
            .run(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
                let result = work(&transaction);
                // A read changes nothing; ending the transaction either way only
                // releases the snapshot.
                drop(transaction);
                result
            })
            .await
    }

    /// Runs `work` in one transaction that holds the write lock. Returning
    /// `Err` rolls everything back.
    pub async fn write<T, E, F>(&self, work: F) -> Result<T, E>
    where
        F: FnOnce(&Transaction) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<DbError> + From<rusqlite::Error> + Send + 'static,
    {
        self.writer
            .run(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let value = work(&transaction)?;
                transaction.commit()?;
                Ok(value)
            })
            .await
    }

    /// Takes the write lock with short polls for work that must compete with
    /// an external writer committing back-to-back transactions.
    pub(crate) async fn write_with_lock_retry<F>(&self, work: F) -> Result<(), DbError>
    where
        F: FnOnce(&Transaction) -> rusqlite::Result<()> + Send + 'static,
    {
        let timeout = self.writer.settings.busy_timeout;
        self.writer
            .run(move |connection| {
                connection.busy_timeout(Duration::ZERO)?;
                let outcome = (|| {
                    let transaction = retry_lock(timeout, || {
                        Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
                    })?;
                    work(&transaction)?;
                    transaction.commit()
                })();
                let restored = connection.busy_timeout(timeout);
                outcome?;
                restored?;
                Ok(())
            })
            .await
    }

    /// Like [`Database::write`], but a unique value that another request took
    /// after this one had checked that it was free comes back as `None`
    /// instead of an error. Nothing is kept of the attempt.
    pub async fn try_write<T, F>(&self, work: F) -> Result<Option<T>, DbError>
    where
        F: FnOnce(&Transaction) -> Result<T, DbError> + Send + 'static,
        T: Send + 'static,
    {
        match self.write(work).await {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is_unique_violation() => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Pool {
    fn new(
        settings: &DatabaseSettings,
        flags: OpenFlags,
        size: usize,
        idle: Vec<Connection>,
    ) -> Arc<Self> {
        Arc::new(Self {
            settings: settings.clone(),
            flags,
            idle: Mutex::new(idle),
            permits: Arc::new(Semaphore::new(size)),
        })
    }

    async fn run<T, E, F>(self: &Arc<Self>, work: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
    {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| DbError::Task("the database is closed".to_string()))?;
        let pool = Arc::clone(self);
        let joined = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let taken = pool
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop();
            let mut connection = match taken {
                Some(connection) => connection,
                None => connect(&pool.settings, pool.flags).map_err(E::from)?,
            };
            if pool.flags.contains(OpenFlags::SQLITE_OPEN_READ_WRITE) {
                for (setting, value) in [("foreign_keys", true), ("query_only", false)] {
                    connection
                        .pragma_update(None, setting, value)
                        .map_err(DbError::from)?;
                }
            }
            let outcome = work(&mut connection);
            // A connection that was in the middle of a transaction when the
            // closure panicked never gets here, so only sound ones are kept.
            pool.idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(connection);
            outcome
        })
        .await;
        match joined {
            Ok(outcome) => outcome,
            Err(error) => Err(E::from(DbError::Task(error.to_string()))),
        }
    }
}

/// Retries SQLite contention after releasing the failed attempt's resources.
/// Callers disable SQLite's own busy wait so the retries share one budget.
pub(crate) fn retry_lock<T>(
    timeout: Duration,
    mut attempt: impl FnMut() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    const POLL: Duration = Duration::from_millis(1);
    let started = Instant::now();
    loop {
        match attempt() {
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                std::thread::sleep(POLL.min(timeout.saturating_sub(started.elapsed())));
                if started.elapsed() >= timeout {
                    return Err(error);
                }
            }
            result => return result,
        }
    }
}

/// A connection the way every connection to the database is opened: foreign
/// keys enforced, and a wait for another connection's lock instead of an
/// immediate refusal.
///
/// SQLite keeps `foreign_keys` per connection, so it is set here on each one,
/// not once at start. The schema depends on it: deletes cascade. A shared
/// cache is not used: with write-ahead logging a reader goes on from the last
/// commit while another connection writes, and a shared cache would make it
/// wait.
fn connect(settings: &DatabaseSettings, flags: OpenFlags) -> Result<Connection, DbError> {
    let connection =
        Connection::open_with_flags(&settings.path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    connection.busy_timeout(settings.busy_timeout)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    if flags.contains(OpenFlags::SQLITE_OPEN_READ_ONLY) {
        connection.pragma_update(None, "query_only", true)?;
    }
    Ok(connection)
}

#[cfg(test)]
mod tests;
