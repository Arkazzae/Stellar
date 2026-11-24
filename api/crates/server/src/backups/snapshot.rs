//! Copying a database that is being written to, and checking a copy.
//!
//! A file copy of a live SQLite database is not a backup: it can hold half a
//! transaction, or pages from two moments. SQLite's online backup copies the
//! whole database in one step under one read lock, so what comes out is a
//! state the database really had, whatever is being written meanwhile. (A
//! step of a few pages at a time would start over each time a writer
//! committed, and never finish under a steady writer.)

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags, ffi};

use super::BackupFailed;
use crate::database::retry_lock;

/// The time allowed for acquiring the locks needed to copy the database.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Copies the database at `source` into a new file at `destination`. A copy
/// that fails is removed: a half-written file would otherwise sit in the list
/// as a backup. The destination must not exist.
pub fn copy_database(source: &Path, destination: &Path) -> Result<(), BackupFailed> {
    // Only remove the destination after we have created and own it.
    match super::create_private(destination) {
        Ok(file) => drop(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(BackupFailed::new(format!(
                "{} already exists",
                destination.display()
            )));
        }
        Err(error) => return Err(error.into()),
    }
    let outcome = copy(source, destination);
    if outcome.is_err() {
        remove_with_side_files(destination);
    }
    outcome
}

fn copy(source: &Path, destination: &Path) -> Result<(), BackupFailed> {
    let result = retry_lock(BUSY_TIMEOUT, || {
        let from = Connection::open_with_flags(
            source,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        from.busy_timeout(Duration::ZERO)?;
        let mut to = Connection::open_with_flags(
            destination,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        to.busy_timeout(Duration::ZERO)?;
        // SQLite rolls back the destination when an unfinished backup is
        // dropped. The next attempt starts with a new backup handle.
        let backup = Backup::new(&from, &mut to)?;
        let code = match backup.step(-1)? {
            StepResult::Busy => ffi::SQLITE_BUSY,
            StepResult::Locked => ffi::SQLITE_LOCKED,
            step => return Ok(step),
        };
        Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None))
    })?;
    if matches!(result, StepResult::Done) {
        Ok(())
    } else {
        Err(BackupFailed::new(
            "SQLite did not complete the database snapshot.",
        ))
    }
}

/// Removes a database file and what SQLite leaves beside a copy it opened.
pub fn remove_with_side_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

/// `"ok"`, or what SQLite found wrong with the file, or why it could not be
/// read as a database at all. Never leaves a side file behind.
pub fn check_integrity(path: &Path) -> String {
    let checked = (|| -> rusqlite::Result<String> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
    })();
    let verdict = match checked {
        Ok(verdict) => verdict,
        Err(error) => sqlite_message(&error),
    };
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
    verdict
}

/// Which of `tables` the database at `path` does not have. A file that cannot
/// be read as a database has none of them.
pub fn missing_tables(path: &Path, tables: &[&str]) -> Vec<String> {
    let found = (|| -> rusqlite::Result<Vec<String>> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut statement =
            connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
        let names = statement.query_map([], |row| row.get::<_, String>(0))?;
        names.collect()
    })()
    .unwrap_or_default();
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
    tables
        .iter()
        .filter(|table| !found.iter().any(|name| name == *table))
        .map(|table| (*table).to_string())
        .collect()
}

/// Whether the bytes begin as every SQLite database does.
pub const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// A failure of SQLite in the words the first server used for it,
/// `SQLite Error 26: 'file is not a database'.`
fn sqlite_message(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(failure, message) => format!(
            "SQLite Error {}: '{}'.",
            failure.extended_code & 0xff,
            message.clone().unwrap_or_else(|| failure.to_string())
        ),
        other => other.to_string(),
    }
}
