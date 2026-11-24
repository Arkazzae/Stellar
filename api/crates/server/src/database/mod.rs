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

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use tokio::sync::Semaphore;

use crate::settings::DatabaseSettings;

pub use error::DbError;
pub use gate::MutationGate;

/// Connections open at the same time, at most. SQLite lets many readers and
/// one writer work together; more connections than this only queue behind
/// the writer.
const MAX_CONNECTIONS: usize = 16;

/// A handle to the database. Cloning it shares the connections.
#[derive(Clone)]
pub struct Database {
    inner: Arc<Inner>,
}

struct Inner {
    settings: DatabaseSettings,
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

impl Database {
    /// Opens the database, creating the file and its schema when there is
    /// none. It blocks, so it is called before the server starts or from a
    /// blocking thread.
    pub fn open(settings: &DatabaseSettings) -> Result<Database, DbError> {
        let mut connection = connect(settings)?;
        schema::prepare(&mut connection)?;
        Ok(Database {
            inner: Arc::new(Inner {
                settings: settings.clone(),
                idle: Mutex::new(vec![connection]),
                permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            }),
        })
    }

    /// Where the database file is.
    pub fn path(&self) -> &Path {
        &self.inner.settings.path
    }

    /// Runs `work` on a connection, inside one snapshot of the database.
    pub async fn read<T, E, F>(&self, work: F) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<DbError> + From<rusqlite::Error> + Send + 'static,
    {
        self.run(move |connection| {
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
        self.run(move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = work(&transaction)?;
            transaction.commit()?;
            Ok(value)
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

    async fn run<T, E, F>(&self, work: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
    {
        let permit = Arc::clone(&self.inner.permits)
            .acquire_owned()
            .await
            .map_err(|_| DbError::Task("the database is closed".to_string()))?;
        let inner = Arc::clone(&self.inner);
        let joined = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let taken = inner
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop();
            let mut connection = match taken {
                Some(connection) => connection,
                None => connect(&inner.settings).map_err(E::from)?,
            };
            // A statement that switched it off on this connection must not
            // leave it off for the next one that borrows it.
            connection
                .pragma_update(None, "foreign_keys", true)
                .map_err(|e| E::from(DbError::from(e)))?;
            let outcome = work(&mut connection);
            // A connection that was in the middle of a transaction when the
            // closure panicked never gets here, so only sound ones are kept.
            inner
                .idle
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

/// A connection the way every connection to the database is opened: foreign
/// keys enforced, and a wait for another connection's lock instead of an
/// immediate refusal.
///
/// SQLite keeps `foreign_keys` per connection, so it is set here on each one,
/// not once at start. The schema depends on it: deletes cascade. A shared
/// cache is not used: with write-ahead logging a reader goes on from the last
/// commit while another connection writes, and a shared cache would make it
/// wait.
fn connect(settings: &DatabaseSettings) -> Result<Connection, DbError> {
    let connection = Connection::open_with_flags(
        &settings.path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(settings.busy_timeout)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

#[cfg(test)]
mod tests;
