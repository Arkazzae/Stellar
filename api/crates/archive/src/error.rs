use std::fmt;
use std::io;

/// Why an archive could not be written, opened or read.
#[derive(Debug)]
pub enum Error {
    /// The file or the sink failed.
    Io(io::Error),
    /// The archive is damaged, or is not a zip file at all.
    Corrupt(String),
    /// The archive is intact but uses a feature this crate does not do:
    /// encryption, another compression method, several disks.
    Unsupported(String),
    /// The archive is larger than the caller allowed.
    TooLarge(String),
    /// An entry's bytes do not match its recorded checksum.
    Checksum(String),
    /// The caller asked for something that cannot be done: an unusable entry
    /// name, an entry that is not there, a second entry opened too soon.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Corrupt(why) => write!(f, "the archive is damaged: {why}"),
            Self::Unsupported(what) => {
                write!(f, "the archive uses something not supported: {what}")
            }
            Self::TooLarge(why) => write!(f, "the archive is too large: {why}"),
            Self::Checksum(name) => {
                write!(
                    f,
                    "the archive entry '{name}' failed its checksum or length check"
                )
            }
            Self::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::from_io(error)
    }
}

impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::Io(error) => error,
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        }
    }
}

impl Error {
    /// Recovers the error an `io::Error` was made from, for the readers in
    /// this crate that can only return `io::Error`.
    pub fn from_io(error: io::Error) -> Self {
        if error.get_ref().is_some_and(|inner| inner.is::<Error>()) {
            match error.into_inner().map(|inner| inner.downcast::<Error>()) {
                Some(Ok(own)) => *own,
                Some(Err(other)) => Self::Io(io::Error::other(other)),
                None => Self::Corrupt("lost error".into()),
            }
        } else {
            Self::Io(error)
        }
    }
}
