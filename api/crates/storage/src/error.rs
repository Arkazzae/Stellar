//! What a store can answer instead of what was asked.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    /// The key cannot name an object in any store: see [`crate::key`].
    InvalidKey(String),
    /// No object is stored under the key.
    NotFound(String),
    /// The requested range starts at or past the end of the object, or ends
    /// before it starts.
    RangeNotSatisfiable(String),
    /// An object is already stored under the key. Every key is minted fresh,
    /// so that is a mistake, and the write is refused instead of replacing it.
    AlreadyExists(String),
    /// The folder store could not do what the disk was asked, or the data
    /// being stored could not be read.
    Io(io::Error),
    /// The bucket store could not do it.
    Store { kind: StoreFault, message: String },
}

/// Why a bucket store call failed, as far as what to do next goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreFault {
    /// The store could not be reached or failed: a refused connection, a
    /// timeout, a TLS failure, a 5xx. Trying again later can help.
    Unreachable,
    /// The store asks to slow down. Trying again later can help.
    Throttled,
    /// The bucket is not there: the store is not ready yet.
    NotReady,
    /// The credentials, their rights or the clock are wrong.
    Denied,
    /// Any other refusal. The same request will be refused again.
    Rejected,
    /// The answer is not what the call expects.
    Malformed,
}

impl Error {
    pub(crate) fn store(kind: StoreFault, message: impl Into<String>) -> Self {
        Error::Store {
            kind,
            message: message.into(),
        }
    }

    /// Whether the same call may succeed if made again after a pause.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Store { kind, .. } => matches!(
                kind,
                StoreFault::Unreachable | StoreFault::Throttled | StoreFault::NotReady
            ),
            _ => false,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidKey(key) => write!(f, "'{key}' cannot name a stored object"),
            Error::NotFound(key) => write!(f, "no object is stored under '{key}'"),
            Error::RangeNotSatisfiable(key) => {
                write!(f, "the requested range lies outside the object '{key}'")
            }
            Error::AlreadyExists(key) => {
                write!(f, "an object is already stored under '{key}'")
            }
            Error::Io(error) => write!(f, "the storage failed: {error}"),
            Error::Store { message, .. } => write!(f, "the object store failed: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
