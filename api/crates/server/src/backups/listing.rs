//! The folder backups are kept in, and what the panel's list says of each
//! file in it.
//!
//! A backup is a plain file named `backup-<time>-<token>.db` (the database
//! alone, a SQLite file) or `.zip` (a full archive). One being written has a
//! name that starts with a dot and ends in `.partial`, so the list never
//! shows half a backup. A `.verified` file beside a backup holds the moment
//! it was last verified.

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::archive;
use super::parts;
use crate::values::Timestamp;

pub const PREFIX: &str = "backup-";
pub const DATABASE_EXTENSION: &str = ".db";
pub const ARCHIVE_EXTENSION: &str = ".zip";
pub const VERIFIED_EXTENSION: &str = ".verified";
const PARTIAL_EXTENSION: &str = ".partial";

/// The folder of the data root that holds the backups.
pub fn directory(data_root: &Path) -> PathBuf {
    std::path::absolute(data_root.join("backups")).unwrap_or_else(|_| data_root.join("backups"))
}

/// The file a backup id names, or none when the id is not a plain backup file
/// name.
pub fn resolve(directory: &Path, id: &str) -> Option<PathBuf> {
    let plain = !id.contains('/') && !id.contains('\0');
    (plain
        && id.starts_with(PREFIX)
        && (id.ends_with(DATABASE_EXTENSION) || id.ends_with(ARCHIVE_EXTENSION)))
    .then(|| directory.join(id))
}

pub fn is_archive(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|name| name.ends_with(ARCHIVE_EXTENSION))
}

/// The name a new backup gets: the moment, then a token no other backup has.
pub fn new_id(now: Timestamp, extension: &str) -> String {
    let text = now.to_db_text();
    let stamp: String = text
        .chars()
        .filter(char::is_ascii_digit)
        .take(8)
        .chain(std::iter::once('-'))
        .chain(text.chars().skip(11).filter(char::is_ascii_digit).take(6))
        .collect();
    format!("{PREFIX}{stamp}-{}{extension}", super::random_token())
}

/// A backup as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub size_bytes: i64,
    pub created_at: Timestamp,
    pub verified_at: Option<Timestamp>,
    pub parts: Vec<String>,
    pub missing_objects: i64,
}

/// One file as the list shows it; an archive says what it holds in its
/// manifest. It blocks.
pub fn describe(path: &Path) -> std::io::Result<Entry> {
    let metadata = std::fs::metadata(path)?;
    let manifest = is_archive(path)
        .then(|| archive::read_manifest(path))
        .flatten();
    let archive = is_archive(path);
    Ok(Entry {
        id: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
        created_at: created(&metadata),
        verified_at: verified_at(path),
        missing_objects: manifest
            .as_ref()
            .map_or(0, |manifest| manifest.missing_objects.len() as i64),
        parts: match manifest {
            Some(manifest) => manifest.parts,
            None if archive => Vec::new(),
            None => vec![parts::DATABASE.to_string()],
        },
    })
}

/// Every backup in the folder, the newest first. It blocks.
pub fn list(directory: &Path) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut found: Vec<Entry> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(PREFIX)
                        && (name.ends_with(DATABASE_EXTENSION) || name.ends_with(ARCHIVE_EXTENSION))
                })
                && path.is_file()
        })
        // A file removed since the folder was read is no longer a backup.
        .filter_map(|path| describe(&path).ok())
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    found
}

/// When the file was made. On Linux the first server did not read the birth
/// time the file system keeps (a file copied in kept its old date), but the
/// earlier of the last change of its content and of its entry; elsewhere the
/// birth time is used where there is one, else that same earlier moment.
pub fn created(metadata: &Metadata) -> Timestamp {
    let birth = if cfg!(target_os = "linux") {
        None
    } else {
        metadata.created().ok()
    };
    let time = birth.or_else(|| {
        let modified = metadata.modified().ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let changed = std::time::UNIX_EPOCH
                + std::time::Duration::new(
                    u64::try_from(metadata.ctime()).unwrap_or(0),
                    u32::try_from(metadata.ctime_nsec()).unwrap_or(0),
                );
            Some(modified.min(changed))
        }
        #[cfg(not(unix))]
        Some(modified)
    });
    time.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| {
            Timestamp::from_ticks(i64::try_from(elapsed.as_nanos() / 100).unwrap_or(i64::MAX))
        })
        .unwrap_or_else(Timestamp::now)
}

pub fn marker_path(backup: &Path) -> PathBuf {
    let mut name = backup.as_os_str().to_owned();
    name.push(VERIFIED_EXTENSION);
    PathBuf::from(name)
}

pub fn verified_at(backup: &Path) -> Option<Timestamp> {
    let text = std::fs::read_to_string(marker_path(backup)).ok()?;
    Timestamp::parse(text.trim())
}

pub fn mark_verified(backup: &Path, at: Timestamp) -> std::io::Result<()> {
    std::fs::write(marker_path(backup), at.to_wire_text())
}

/// Removes what a backup cut short by a stop left behind: the partial files
/// and the copies of the database SQLite may have left beside them.
pub fn remove_leftovers(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') && name[1..].contains(PARTIAL_EXTENSION) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_a_plain_backup_name_only() {
        let folder = Path::new("/data/backups");
        assert!(resolve(folder, "backup-20261124-101500-abc.zip").is_some());
        assert!(resolve(folder, "backup-20261124-101500-abc.db").is_some());
        for bad in [
            "../backup-x.zip",
            "backup-x.zip/../../y",
            "backup-a/../../x.zip",
            "backup-a\0.zip",
            "backup-a/b.db",
            "backup-x.txt",
            "other-x.zip",
            ".backup-x.zip.partial",
            "backup-x.zip.verified",
            "",
        ] {
            assert!(resolve(folder, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_new_id_names_the_moment_and_never_repeats() {
        let at = Timestamp::parse("2026-11-24 10:15:30.5").unwrap();
        let id = new_id(at, ARCHIVE_EXTENSION);
        assert!(id.starts_with("backup-20261124-101530-"), "{id}");
        assert!(id.ends_with(".zip"));
        assert_eq!(id.len(), "backup-20261124-101530-".len() + 32 + 4);
        assert_ne!(id, new_id(at, ARCHIVE_EXTENSION));
    }

    #[test]
    fn leftovers_are_the_partial_files_and_what_sqlite_left_beside_them() {
        let folder = crate::testing::TempDir::new();
        for name in [
            ".backup-1.zip.partial",
            ".snapshot-1.partial",
            ".snapshot-1.partial-wal",
            ".snapshot-1.partial-shm",
            "backup-2.zip",
            "backup-2.zip.verified",
            ".hidden",
        ] {
            std::fs::write(folder.path().join(name), b"x").unwrap();
        }
        remove_leftovers(folder.path());
        let mut left: Vec<String> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, [".hidden", "backup-2.zip", "backup-2.zip.verified"]);
    }
}
