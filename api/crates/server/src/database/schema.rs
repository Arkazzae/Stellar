//! The schema a new database starts with.
//!
//! `schema.sql` is the schema the first server creates on an empty folder,
//! dumped from a real run and kept as it is: nothing in it is renamed,
//! retyped or tidied, so that both servers can run on copies of one file.
//! There is no migration machinery. A database that already has the schema is
//! opened as it is.

use rusqlite::{Connection, TransactionBehavior};

use crate::values::Timestamp;

use super::DbError;

const SCHEMA: &str = include_str!("schema.sql");

/// The one statement of the file that SQLite refuses: the dump lists the
/// bookkeeping table of `AUTOINCREMENT`, which SQLite has already created by
/// the time it reaches it, and a name starting with `sqlite_` is reserved.
const IMPLICIT_TABLE: &str = "CREATE TABLE sqlite_sequence(name,seq);\n";

/// Work that happens once in the life of a database. A database that starts
/// empty has no data for them to change, so it only records that they have
/// run; whatever writes data later has to write what they would have.
const ONE_TIME_STEPS: [&str; 2] = ["ordinary-permissions", "view-channel"];

/// Sets the journal mode, which the database file remembers, and gives an
/// empty file its schema.
pub fn prepare(connection: &mut Connection) -> Result<(), DbError> {
    let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::Task(format!(
            "the database would not go into write-ahead logging; it answered \"{mode}\""
        )));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Decided inside the write lock, so that two processes starting on one
    // empty file do not both create the tables.
    let tables: i64 =
        transaction.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
    if tables == 0 {
        // `IMPLICIT_TABLE` is dropped because SQLite refuses to create a table
        // whose name starts with `sqlite_`, and it has made this one itself
        // by the time the statement would run.
        transaction.execute_batch(&SCHEMA.replacen(IMPLICIT_TABLE, "", 1))?;
        let now = Timestamp::now();
        for name in ONE_TIME_STEPS {
            transaction.execute(
                "INSERT OR IGNORE INTO \"OneTimeSteps\" (\"Name\", \"AppliedAt\") VALUES (?1, ?2)",
                rusqlite::params![name, now],
            )?;
        }
    }
    transaction.commit()?;
    Ok(())
}
