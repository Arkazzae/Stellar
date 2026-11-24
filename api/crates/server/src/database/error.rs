use std::fmt;

use rusqlite::ffi;

/// A failure of the database, or of getting a connection to it.
#[derive(Debug)]
pub enum DbError {
    Sqlite(rusqlite::Error),
    /// The thread that ran the work stopped before it finished.
    Task(String),
}

impl DbError {
    /// Whether a row with the same key or the same unique value already
    /// exists, which is how a lost race shows up.
    pub fn is_unique_violation(&self) -> bool {
        self.extended_code().is_some_and(|code| {
            code == ffi::SQLITE_CONSTRAINT_PRIMARYKEY || code == ffi::SQLITE_CONSTRAINT_UNIQUE
        })
    }

    /// Whether a row points at one that is not there, or one that something
    /// points at was removed.
    pub fn is_foreign_key_violation(&self) -> bool {
        self.extended_code() == Some(ffi::SQLITE_CONSTRAINT_FOREIGNKEY)
    }

    /// SQLite's extended result code, when SQLite refused the statement.
    pub fn extended_code(&self) -> Option<i32> {
        match self {
            DbError::Sqlite(rusqlite::Error::SqliteFailure(error, _)) => Some(error.extended_code),
            _ => None,
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Sqlite(error) => write!(f, "database: {error}"),
            DbError::Task(reason) => write!(f, "database: {reason}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> DbError {
        DbError::Sqlite(error)
    }
}

/// So that a closure that only runs statements can return `rusqlite::Result`
/// and need no error type of its own: the failure to get a connection or to
/// run the work comes back in the same type as a failed statement.
impl From<DbError> for rusqlite::Error {
    fn from(error: DbError) -> rusqlite::Error {
        match error {
            DbError::Sqlite(error) => error,
            // The variant that carries any error: it is not about a conversion.
            DbError::Task(reason) => rusqlite::Error::ToSqlConversionFailure(reason.into()),
        }
    }
}
