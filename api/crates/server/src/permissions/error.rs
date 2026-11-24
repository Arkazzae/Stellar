use std::fmt;

/// Data that cannot be right, found while deciding: a server without a role
/// for plain members, overrides read for a place that was never loaded. It is
/// a fault of the code or of the stored rows, never of a request, so it
/// travels as a database failure and ends as a 500 that says nothing of it.
#[derive(Debug)]
pub struct DataError(String);

impl DataError {
    pub(super) fn new(what: impl Into<String>) -> DataError {
        DataError(what.into())
    }
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DataError {}

impl From<DataError> for rusqlite::Error {
    fn from(error: DataError) -> rusqlite::Error {
        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
    }
}
