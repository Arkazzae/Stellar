//! Backups of the instance: making an archive, checking and listing them,
//! the automatic backup on a schedule, where backups are kept, restoring one,
//! and the administration panel's routes for all of it.
//!
//! An archive is a file an operator keeps, so its layout is not this
//! server's alone: one the first server made reads, verifies and restores
//! here, and the other way round.

use std::fmt;

pub mod archive;
pub mod destination;
pub mod jobs;
pub mod listing;
pub mod manifest;
pub mod offsite;
pub mod parts;
pub mod policy;
pub mod schedule;
pub mod progress;
pub mod restore;
mod routes;
pub mod snapshot;
pub mod space;

pub use jobs::Jobs;
pub use parts::Parts;

/// A failure the operator should read as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFailed {
    message: String,
    /// The server was stopping, which is no failure of the backup.
    pub cancelled: bool,
}

impl BackupFailed {
    pub fn new(message: impl Into<String>) -> BackupFailed {
        BackupFailed {
            message: message.into(),
            cancelled: false,
        }
    }

    pub fn cancelled() -> BackupFailed {
        BackupFailed {
            message: "The backup was cancelled.".to_string(),
            cancelled: true,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BackupFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BackupFailed {}

impl From<std::io::Error> for BackupFailed {
    fn from(error: std::io::Error) -> Self {
        BackupFailed::new(error.to_string())
    }
}

impl From<rusqlite::Error> for BackupFailed {
    fn from(error: rusqlite::Error) -> Self {
        BackupFailed::new(error.to_string())
    }
}

impl From<stellarr_archive::Error> for BackupFailed {
    fn from(error: stellarr_archive::Error) -> Self {
        BackupFailed::new(error.to_string())
    }
}

/// A new file that only its owner may read or write. A backup, a copy of the
/// database, a staged archive and everything unpacked from one holds every
/// secret of the instance, so they are made so and never opened up and shut
/// again.
pub fn create_private(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// A folder that only its owner may enter.
pub fn make_private(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

/// 32 lower-case hex digits that nobody can guess, for names of files and
/// backups.
pub fn random_token() -> String {
    let mut text = String::with_capacity(32);
    for byte in crate::values::Id::random().as_bytes() {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

pub use routes::routes;

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod schedule_tests;

#[cfg(test)]
mod archive_tests;
