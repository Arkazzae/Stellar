use std::fmt;

use stellarr_messages::MissingValue;

/// A message could not be built.
#[derive(Debug)]
pub enum Error {
    /// A text of the catalogue needs a value the caller did not give; a bug
    /// in the calling code.
    Text(MissingValue),
    /// The sender or the recipient is not an address.
    Address(stellarr_smtp::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Text(e) => e.fmt(f),
            Error::Address(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<MissingValue> for Error {
    fn from(e: MissingValue) -> Error {
        Error::Text(e)
    }
}

impl From<stellarr_smtp::Error> for Error {
    fn from(e: stellarr_smtp::Error) -> Error {
        Error::Address(e)
    }
}
