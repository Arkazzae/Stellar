//! Putting a full backup archive in place of an instance's data, while no
//! process of that instance runs. These are the calls the operator's
//! command-line tool makes: it stages an archive while the instance runs, and
//! the container's next start applies it before any service reads the data
//! volume (the entrypoint runs `backup apply`). While the instance runs, its
//! database, keys and secrets are open, and replacing them under it would mix
//! two instances.
//!
//! The archive is checked before anything changes: its manifest, every entry
//! name, the database's integrity and every entry's bytes. Its contents are
//! then unpacked beside the live folders, on the same volume, and only once
//! all of them are written does each live folder make way for its new one.
//! The folders it replaces are moved whole into `restore/previous-...`, the
//! database's `-wal` and `-shm` with them, so the restore can be undone by
//! moving them back.
//!
//! An archive may come from anywhere, so nothing in it is trusted: a name that
//! could reach outside the folder it belongs to, a link, a name twice, a size
//! that the data does not keep, a database that is not one, all refuse the
//! whole archive and change nothing. Files are only ever created, never
//! opened over, in a folder this restore has just made.
//!
//! Only an archive of all four parts is restored: a database without the keys
//! it was written with cannot read the secrets stored in it, and objects
//! without their rows serve nobody.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use stellarr_archive::{Entry, ZipArchive};
use stellarr_storage::key;

use super::archive::{
    self, DATABASE_ENTRY, INSTANCE_FOLDER, INSTANCE_FOLDERS, KEYS_FOLDER, MANIFEST_ENTRY,
    OBJECTS_FOLDER,
};
use super::manifest::{self, CURRENT_VERSION, MAX_MANIFEST_BYTES, Manifest};
use super::parts::Parts;
use super::snapshot::SQLITE_HEADER;
use super::{random_token, space};
use crate::values::Timestamp;

/// The folder of the data volume that holds staged archives and replaced data.
pub const FOLDER_NAME: &str = "restore";
/// The archive the next start of the container restores.
pub const STAGED_ARCHIVE_NAME: &str = "staged.zip";
/// What the last restore did or why it refused, for a person to read.
pub const LAST_RESULT_NAME: &str = "last-restore.json";
/// Left by a restore that failed part-way and could not undo itself; the
/// container does not start while it exists.
pub const BROKEN_MARKER_NAME: &str = "broken";

const DATABASE_FOLDER: &str = "db";
const DATABASE_FILE: &str = "stellarr.db";
const KEYS_FOLDER_NAME: &str = "keys";
const OBJECTS_FOLDER_NAME: &str = "files";
/// The folders a restore leaves alone: `logs/`, `backups/` and the checkpoint
/// of the automatic backup are not in an archive and are not touched.
const PRIVATE_FOLDERS: [&str; 3] = [KEYS_FOLDER_NAME, "secrets", "tls"];

/// Why an archive was not restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    /// The archive cannot be restored. Nothing on the data volume was changed.
    Refused(String),
    /// A restore that failed part-way and could not put the previous data
    /// back. The data volume needs a person before the instance starts again.
    Broken(String),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::Refused(message) | RestoreError::Broken(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for RestoreError {}

fn refused(message: impl Into<String>) -> RestoreError {
    RestoreError::Refused(message.into())
}

/// What a restore replaced, and where the data it replaced went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreResult {
    pub manifest: Manifest,
    pub previous_data_directory: PathBuf,
}

/// Every folder of the data volume a restore replaces, in the order it
/// replaces them.
pub fn replaced_folders() -> Vec<&'static str> {
    let mut folders = vec![DATABASE_FOLDER, OBJECTS_FOLDER_NAME, KEYS_FOLDER_NAME];
    folders.extend(INSTANCE_FOLDERS);
    folders
}

/// Where the archive the next start restores sits, for a data root.
pub fn staged_archive_path(data_root: &Path) -> PathBuf {
    data_root.join(FOLDER_NAME).join(STAGED_ARCHIVE_NAME)
}

/// Checks that the file is a full backup this release can restore, reading
/// every byte of it. `work_folder` holds the copy of the database that is
/// opened for the check, and is made if it is not there. It blocks.
pub fn check(archive_path: &Path, work_folder: &Path) -> Result<Manifest, RestoreError> {
    if is_database_file(archive_path).map_err(|error| refused(format!("The file cannot be read: {error}")))? {
        return Err(refused(
            "This is a database-only backup: a single SQLite file, not an archive. Only a full backup is restored as a whole; restore this one by hand, as docs/admin/backups.md describes.",
        ));
    }
    let unreadable = |why: String| refused(format!("The file is not a readable backup archive: {why}"));
    let file = File::open(archive_path).map_err(|error| unreadable(error.to_string()))?;
    let mut zip = ZipArchive::with_limits(file, archive::limits())
        .map_err(|error| unreadable(error.to_string()))?;
    let manifest = read_manifest(&mut zip)?;
    check_entry_names(&zip)?;
    drop(zip);

    std::fs::create_dir_all(work_folder)
        .map_err(|error| refused(format!("The work folder cannot be made: {error}")))?;
    let verdict = archive::verify(archive_path, work_folder);
    if verdict != "ok" {
        return Err(refused(format!("The archive failed its check: {verdict}")));
    }
    Ok(manifest)
}

/// Replaces the instance's database, files, keys and instance folders under
/// `data_root` with the contents of the archive. It blocks.
pub fn apply(
    archive_path: &Path,
    data_root: &Path,
    now: Timestamp,
) -> Result<RestoreResult, RestoreError> {
    apply_with(archive_path, data_root, now, &|from, to| {
        std::fs::rename(from, to)
    })
}

/// [`apply`] with the way a folder is moved given, so that a move that fails
/// can be tried.
pub fn apply_with(
    archive_path: &Path,
    data_root: &Path,
    now: Timestamp,
    mover: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<RestoreResult, RestoreError> {
    let data_root = std::path::absolute(data_root)
        .map_err(|error| refused(format!("The data root cannot be resolved: {error}")))?;
    let restore_folder = data_root.join(FOLDER_NAME);
    clear_leftovers(&data_root);
    let manifest = check(archive_path, &restore_folder)?;
    check_free_space(archive_path, &data_root)?;

    let unpacked = restore_folder.join(format!(".unpacking-{}", random_token()));
    if let Err(error) = unpack(archive_path, &unpacked) {
        let _ = std::fs::remove_dir_all(&unpacked);
        return Err(refused(format!("The archive could not be unpacked: {error}")));
    }

    let previous = new_previous_directory(&restore_folder, now);
    let outcome = swap(&data_root, &unpacked, &previous, mover);
    if let Err(RestoreError::Refused(_)) = &outcome {
        let _ = std::fs::remove_dir_all(&unpacked);
    }
    outcome?;
    let _ = std::fs::remove_dir_all(&unpacked);
    Ok(RestoreResult {
        manifest,
        previous_data_directory: previous,
    })
}

/// Removes what a staging or a restore cut short by a stop left in the
/// restore folder: a half-copied archive, the database copied out for its
/// check, a half-unpacked archive. None of them is data the instance reads,
/// and left there they would count against the next restore's space.
pub fn clear_leftovers(data_root: &Path) {
    let folder = data_root.join(FOLDER_NAME);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let path = entry.path();
        if name.starts_with(".unpacking-") {
            let _ = std::fs::remove_dir_all(&path);
        } else if name.starts_with(".verify-")
            || (name.starts_with(&format!("{STAGED_ARCHIVE_NAME}.")) && name.ends_with(".partial"))
        {
            super::snapshot::remove_with_side_files(&path);
        }
    }
}

/// Records what the last restore did, beside the archives, for a person to
/// read.
pub fn write_result(data_root: &Path, result: &impl Serialize) -> std::io::Result<()> {
    let folder = data_root.join(FOLDER_NAME);
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(LAST_RESULT_NAME);
    let pending = folder.join(format!("{LAST_RESULT_NAME}.partial"));
    let text = serde_json::to_vec_pretty(result).map_err(std::io::Error::other)?;
    std::fs::write(&pending, text)?;
    std::fs::rename(&pending, &path)
}

/// [`write_result`] for text that is already JSON.
pub fn write_result_text(data_root: &Path, text: &str) -> std::io::Result<()> {
    let folder = data_root.join(FOLDER_NAME);
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(LAST_RESULT_NAME);
    let pending = folder.join(format!("{LAST_RESULT_NAME}.partial"));
    std::fs::write(&pending, text)?;
    std::fs::rename(&pending, &path)
}

/// Stages a checked archive for the next start: copied, not moved (the archive
/// may be one the panel lists, and the copy lives on the data volume, which a
/// recreated container keeps), under a name nothing reads until the copy is
/// whole. The caller has checked the archive and asked the operator.
pub fn stage(archive_path: &Path, data_root: &Path) -> Result<PathBuf, RestoreError> {
    let staged = staged_archive_path(data_root);
    let folder = data_root.join(FOLDER_NAME);
    std::fs::create_dir_all(&folder).map_err(|error| refused(error.to_string()))?;
    clear_leftovers(data_root);
    let size = std::fs::metadata(archive_path)
        .map_err(|error| refused(error.to_string()))?
        .len();
    match space::free_bytes(&folder) {
        None => return Err(refused(space::unknown("staging the archive", size))),
        Some(available) if available < size => {
            return Err(refused(format!(
                "Staging the archive needs {size} bytes on the data volume, and {available} are free. Nothing was staged."
            )));
        }
        Some(_) => {}
    }
    let partial = folder.join(format!("{STAGED_ARCHIVE_NAME}.{}.partial", random_token()));
    let copied = (|| -> std::io::Result<()> {
        let mut source = File::open(archive_path)?;
        let mut target = OpenOptions::new().write(true).create_new(true).open(&partial)?;
        std::io::copy(&mut source, &mut target)?;
        target.sync_all()?;
        std::fs::rename(&partial, &staged)
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&partial);
        return Err(refused(error.to_string()));
    }
    Ok(staged)
}

/// Drops the staged archive, so that the next start keeps the current data.
/// False when nothing was staged.
pub fn cancel(data_root: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(staged_archive_path(data_root)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Keeps a staged archive that was refused beside the others, named for when,
/// and returns where it went. The data was left as it was.
pub fn keep_refused(data_root: &Path, now: Timestamp) -> std::io::Result<PathBuf> {
    keep_staged(data_root, "refused", now)
}

/// Marks the volume as one a restore left half done: the container refuses to
/// start while the marker exists. The staged archive is kept as `broken-...`.
pub fn mark_broken(data_root: &Path, reason: &str, now: Timestamp) -> std::io::Result<PathBuf> {
    let folder = data_root.join(FOLDER_NAME);
    std::fs::create_dir_all(&folder)?;
    let marker = folder.join(BROKEN_MARKER_NAME);
    std::fs::write(&marker, reason)?;
    keep_staged(data_root, "broken", now)?;
    Ok(marker)
}

fn keep_staged(data_root: &Path, kind: &str, now: Timestamp) -> std::io::Result<PathBuf> {
    let kept = data_root
        .join(FOLDER_NAME)
        .join(format!("{kind}-{}.zip", stamp(now)));
    std::fs::rename(staged_archive_path(data_root), &kept)?;
    Ok(kept)
}

/// `yyyyMMdd-HHmmss` of a moment.
fn stamp(now: Timestamp) -> String {
    let text = now.to_db_text();
    let date: String = text.chars().take(10).filter(char::is_ascii_digit).collect();
    let time: String = text.chars().skip(11).take(8).filter(char::is_ascii_digit).collect();
    format!("{date}-{time}")
}

/// Whether the file starts as every SQLite database does, as the panel's
/// database-only backups are.
fn is_database_file(path: &Path) -> std::io::Result<bool> {
    let mut start = [0u8; 16];
    let mut file = File::open(path)?;
    let mut read = 0;
    while read < start.len() {
        match file.read(&mut start[read..])? {
            0 => break,
            more => read += more,
        }
    }
    Ok(read == start.len() && &start == SQLITE_HEADER)
}

fn read_manifest(zip: &mut ZipArchive<File>) -> Result<Manifest, RestoreError> {
    let unreadable =
        |why: String| refused(format!("The file is not a readable backup archive: {why}"));
    let Some(at) = zip.find(MANIFEST_ENTRY.as_bytes()) else {
        return Err(refused(
            "The archive has no manifest, so it is not a Stellarr backup.",
        ));
    };
    let bytes = zip
        .read_to_vec(at, MAX_MANIFEST_BYTES)
        .map_err(|error| unreadable(error.to_string()))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
    if manifest.format != manifest::FORMAT {
        return Err(refused("The archive's manifest is not a Stellarr backup's."));
    }
    if manifest.version > CURRENT_VERSION {
        return Err(refused(format!(
            "The archive was written in backup format {}; this release reads up to {CURRENT_VERSION}. Restore it with the release that made it, or a newer one.",
            manifest.version
        )));
    }
    let missing: Vec<&str> = Parts::ALL
        .names()
        .into_iter()
        .filter(|name| !manifest.has_part(name))
        .collect();
    if !missing.is_empty() {
        return Err(refused(format!(
            "The archive lacks {}. Only a full backup is restored as a whole; restore a partial one by hand, as docs/admin/backups.md describes.",
            missing.join(", ")
        )));
    }
    Ok(manifest)
}

/// Every entry has to land inside the folder it belongs to. A name that could
/// reach elsewhere is refused rather than skipped, and so is a link.
fn check_entry_names(zip: &ZipArchive<File>) -> Result<(), RestoreError> {
    let mut seen: HashSet<&[u8]> = HashSet::with_capacity(zip.entries().len());
    for entry in zip.entries() {
        let shown = entry.name_lossy();
        if !seen.insert(entry.name_bytes()) {
            return Err(refused(format!("The archive holds '{shown}' twice.")));
        }
        if entry.is_symlink() || !is_plain_file(entry) {
            return Err(refused(format!(
                "The archive holds '{shown}', which is a link or another kind of file, not a file."
            )));
        }
        let placed = entry.name().and_then(target_of);
        if placed.is_none() {
            return Err(refused(format!(
                "The archive holds '{shown}', which no part of a backup puts there."
            )));
        }
    }
    Ok(())
}

/// A file as the entry says it, or one that does not say what it is.
fn is_plain_file(entry: &Entry) -> bool {
    match entry.unix_mode() {
        Some(mode) => matches!(mode & 0o170_000, 0 | 0o100_000),
        None => true,
    }
}

/// Where an entry unpacks, relative to the data root: empty for the manifest,
/// which stays in the archive, and none for a name that belongs nowhere.
fn target_of(name: &str) -> Option<PathBuf> {
    if name == MANIFEST_ENTRY {
        return Some(PathBuf::new());
    }
    if name == DATABASE_ENTRY {
        return Some(Path::new(DATABASE_FOLDER).join(DATABASE_FILE));
    }
    if let Some(object) = name.strip_prefix(OBJECTS_FOLDER) {
        return (key::is_safe(object) && !object.starts_with('.'))
            .then(|| Path::new(OBJECTS_FOLDER_NAME).join(object));
    }
    if let Some(file) = name.strip_prefix(KEYS_FOLDER) {
        return (key::is_safe(file) && !file.contains('/'))
            .then(|| Path::new(KEYS_FOLDER_NAME).join(file));
    }
    if let Some(relative) = name.strip_prefix(INSTANCE_FOLDER) {
        let folder = relative.split('/').next().unwrap_or("");
        return (key::is_safe(relative)
            && relative.len() > folder.len()
            && INSTANCE_FOLDERS.contains(&folder))
        .then(|| PathBuf::from(relative));
    }
    None
}

/// The unpacked archive takes about its own size again; the folders it
/// replaces are moved, not copied, so they cost nothing more.
fn check_free_space(archive_path: &Path, data_root: &Path) -> Result<(), RestoreError> {
    let file = File::open(archive_path).map_err(|error| refused(error.to_string()))?;
    let zip = ZipArchive::with_limits(file, archive::limits())
        .map_err(|error| refused(error.to_string()))?;
    let needed = zip.total_uncompressed_size();
    match space::free_bytes(data_root) {
        None => Err(refused(space::unknown("unpacking the archive", needed))),
        Some(available) if available < needed => Err(refused(format!(
            "Unpacking the archive needs {needed} bytes on the data volume, and {available} are free."
        ))),
        Some(_) => Ok(()),
    }
}

fn unpack(archive_path: &Path, unpacked: &Path) -> Result<(), stellarr_archive::Error> {
    for folder in replaced_folders() {
        std::fs::create_dir_all(unpacked.join(folder))?;
    }
    let file = File::open(archive_path)?;
    let mut zip = ZipArchive::with_limits(file, archive::limits())?;
    for at in 0..zip.entries().len() {
        let name = zip.entries()[at].name().map(str::to_string);
        let relative = name.as_deref().and_then(target_of).unwrap_or_default();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let path = unpacked.join(&relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let private = relative
            .components()
            .next()
            .and_then(|first| first.as_os_str().to_str())
            .is_some_and(|folder| PRIVATE_FOLDERS.contains(&folder));
        let mut target = create_file(&path, private)?;
        zip.copy_to(at, &mut target)?;
        target.flush()?;
        target.sync_all()?;
    }
    Ok(())
}

/// A new file; a zip entry carries no file mode that is to be trusted, so what
/// the container creates as its owner's alone stays its owner's alone.
fn create_file(path: &Path, private: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let _ = private;
    options.open(path)
}

/// Moves each live folder into `previous` and its unpacked replacement into
/// its place. Renames within one volume either happen or do not, so a failure
/// leaves a known list of moves to undo.
fn swap(
    data_root: &Path,
    unpacked: &Path,
    previous: &Path,
    mover: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), RestoreError> {
    std::fs::create_dir_all(previous).map_err(|error| refused(error.to_string()))?;
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    let attempt = (|| -> std::io::Result<()> {
        for folder in replaced_folders() {
            let live = data_root.join(folder);
            if live.is_dir() {
                let aside = previous.join(folder);
                mover(&live, &aside)?;
                done.push((live.clone(), aside));
            }
            let fresh = unpacked.join(folder);
            mover(&fresh, &live)?;
            done.push((fresh, live));
        }
        Ok(())
    })();
    let Err(failure) = attempt else {
        return Ok(());
    };
    while let Some((from, to)) = done.pop() {
        if let Err(undo) = mover(&to, &from) {
            return Err(RestoreError::Broken(format!(
                "Putting the archive in place failed ({failure}), and so did putting the previous data back ({undo}). The previous folders are in {}; the archive's are in {}.",
                previous.display(),
                unpacked.display()
            )));
        }
    }
    let _ = std::fs::remove_dir_all(previous);
    Err(refused(format!(
        "The archive could not be put in place: {failure}"
    )))
}

/// A new folder for the data a restore replaces, named for when it was
/// replaced.
fn new_previous_directory(restore_folder: &Path, now: Timestamp) -> PathBuf {
    let name = format!("previous-{}", stamp(now));
    let mut path = restore_folder.join(&name);
    let mut attempt = 2;
    while path.exists() {
        path = restore_folder.join(format!("{name}-{attempt}"));
        attempt += 1;
    }
    path
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
