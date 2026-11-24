//! A backup as one zip file: `manifest.json`, `database/stellarr.db`,
//! `objects/...` under their own keys, `keys/...` and `instance/...` as the
//! folders sit on the data volume. Objects are stored without compression, as
//! they mostly are compressed media already.
//!
//! Entries are written in this order: the database, the objects in key order,
//! the key files, the instance files and the manifest last, which is what an
//! archive of the first server looks like and what its verifier expects to
//! find.

use std::cmp::Ordering;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as Atomic};

use stellarr_archive::{EntryOptions, Limits, ZipArchive, ZipWriter};
use stellarr_deflate::Level;
use stellarr_storage::{ObjectStorage, StoredObjectInfo, key};
use tokio::runtime::Handle;

use super::manifest::{self, MAX_MANIFEST_BYTES, Manifest};
use super::parts::{self, Parts};
use super::progress::Progress;
use super::snapshot;
use super::{BackupFailed, space};
use crate::values::Timestamp;

pub const MANIFEST_ENTRY: &str = "manifest.json";
pub const DATABASE_ENTRY: &str = "database/stellarr.db";
pub const OBJECTS_FOLDER: &str = "objects/";
pub const KEYS_FOLDER: &str = "keys/";
pub const INSTANCE_FOLDER: &str = "instance/";

/// The folders of the data volume that make up the instance files. The
/// database, the keys, the uploaded files, the logs and the backups are
/// elsewhere in the archive or not wanted in it.
pub const INSTANCE_FOLDERS: [&str; 3] = ["secrets", "tls", "releases"];

/// Where a backup reads from.
pub struct Sources {
    pub database: PathBuf,
    pub store: Arc<dyn ObjectStorage>,
    pub objects: Vec<StoredObjectInfo>,
    pub keys_folder: PathBuf,
    pub instance_name: String,
    pub environment_secrets: Vec<String>,
    /// A folder on the volume the archive is written to, for the copy of the
    /// database that is taken first.
    pub work_folder: PathBuf,
    pub data_root: PathBuf,
    /// A copy of the database that was taken already; the archive uses it
    /// instead of taking one.
    pub database_snapshot: Option<PathBuf>,
}

/// The order the first server sorted names in: by UTF-16 code unit.
pub fn utf16_order(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

/// Writes the archive to `path`, the database first, so that the objects it
/// names are taken after it. An object that cannot be read is listed in the
/// manifest instead of failing the backup; when none can be read, the store
/// itself is the problem and the backup fails.
///
/// It blocks: run it on a blocking thread, with the handle of the runtime
/// that the object store works on.
pub fn write(
    path: &Path,
    wanted: Parts,
    sources: &Sources,
    progress: &Progress,
    cancel: &AtomicBool,
    runtime: &Handle,
) -> Result<Manifest, BackupFailed> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut zip = ZipWriter::new(BufWriter::with_capacity(256 * 1024, file));
    let now = Timestamp::now().unix_seconds();
    let mut database_bytes = 0u64;
    let mut object_count = 0i64;
    let mut object_bytes = 0u64;
    let mut missing = Vec::new();
    let mut key_files = 0i64;
    let mut instance_count = 0i64;

    if wanted.has(Parts::DATABASE) {
        progress.set_phase(parts::DATABASE);
        let own;
        let snapshot_path = match &sources.database_snapshot {
            Some(path) => path.as_path(),
            None => {
                own = sources
                    .work_folder
                    .join(format!(".snapshot-{}.partial", super::random_token()));
                snapshot::copy_database(&sources.database, &own)?;
                own.as_path()
            }
        };
        let outcome = (|| {
            database_bytes = std::fs::metadata(snapshot_path)?.len();
            add_file(&mut zip, snapshot_path, DATABASE_ENTRY, false, cancel)
        })();
        if sources.database_snapshot.is_none() {
            snapshot::remove_with_side_files(snapshot_path);
        }
        outcome?;
    }

    if wanted.has(Parts::OBJECTS) {
        progress.set_phase(parts::OBJECTS);
        for item in &sources.objects {
            check_cancelled(cancel)?;
            let stored = if key::is_safe(&item.key) {
                runtime.block_on(sources.store.get(&item.key, None)).ok()
            } else {
                None
            };
            let Some(mut stored) = stored else {
                missing.push(item.key.clone());
                progress.object_done();
                continue;
            };
            let mut entry = zip.start_entry(
                &format!("{OBJECTS_FOLDER}{}", item.key),
                EntryOptions::stored().modified(now),
            )?;
            while let Some(chunk) = runtime
                .block_on(stored.body.chunk())
                .map_err(|error| BackupFailed::new(error.to_string()))?
            {
                entry.write_all(&chunk)?;
                check_cancelled(cancel)?;
            }
            entry.finish()?;
            object_bytes += stored.total_length;
            object_count += 1;
            progress.object_done();
        }
        if !sources.objects.is_empty() && object_count == 0 {
            return Err(BackupFailed::new(
                "No object could be read from the object store.",
            ));
        }
    }

    if wanted.has(Parts::KEYS) {
        progress.set_phase(parts::KEYS);
        for (path, name) in key_files_of(&sources.keys_folder)? {
            add_file(&mut zip, &path, &format!("{KEYS_FOLDER}{name}"), true, cancel)?;
            key_files += 1;
        }
    }

    if wanted.has(Parts::INSTANCE) {
        progress.set_phase(parts::INSTANCE);
        for (path, name) in instance_files(&sources.data_root)? {
            let private = INSTANCE_FOLDERS[..2]
                .iter()
                .any(|folder| name.strip_prefix(INSTANCE_FOLDER).is_some_and(|rest| rest.starts_with(&format!("{folder}/"))));
            add_file(&mut zip, &path, &name, private, cancel)?;
            instance_count += 1;
        }
    }

    let manifest = Manifest {
        format: manifest::FORMAT.to_string(),
        version: manifest::CURRENT_VERSION,
        created_at: Timestamp::now(),
        instance_name: sources.instance_name.clone(),
        parts: wanted.names().into_iter().map(str::to_string).collect(),
        database_bytes: i64::try_from(database_bytes).unwrap_or(i64::MAX),
        object_count,
        object_bytes: i64::try_from(object_bytes).unwrap_or(i64::MAX),
        missing_objects: missing,
        key_files,
        environment_secrets: sources.environment_secrets.clone(),
        instance_files: instance_count,
    };
    zip.add_bytes(
        MANIFEST_ENTRY,
        EntryOptions::deflated(Level::Default).modified(now),
        &manifest.to_json(),
    )?;
    let mut out = zip.finish()?;
    out.flush()?;
    out.into_inner()
        .map_err(|error| BackupFailed::new(error.to_string()))?
        .sync_all()?;
    Ok(manifest)
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), BackupFailed> {
    if cancel.load(Atomic::Relaxed) {
        Err(BackupFailed::cancelled())
    } else {
        Ok(())
    }
}

/// Adds a file of the volume. `private` marks one only its owner reads.
fn add_file<W: Write>(
    zip: &mut ZipWriter<W>,
    path: &Path,
    name: &str,
    private: bool,
    cancel: &AtomicBool,
) -> Result<(), BackupFailed> {
    check_cancelled(cancel)?;
    let mut source = File::open(path)?;
    let modified = source
        .metadata()?
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    let options = EntryOptions::deflated(Level::Default)
        .modified(modified)
        .unix_mode(if private { 0o100_600 } else { 0o100_644 });
    zip.add_reader(name, options, &mut source)?;
    Ok(())
}

/// The files directly in the keys folder, as `(path, name)`, in name order.
fn key_files_of(folder: &Path) -> Result<Vec<(PathBuf, String)>, BackupFailed> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            found.push((entry.path(), file_name_text(&entry.path())?));
        }
    }
    found.sort_by(|left, right| utf16_order(&left.1, &right.1));
    Ok(found)
}

fn file_name_text(path: &Path) -> Result<String, BackupFailed> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            BackupFailed::new(format!(
                "The file name of {} is not text, so it cannot be put in an archive.",
                path.display()
            ))
        })
}

/// Every file of the instance folders, with the entry it goes to, in a stable
/// order: folder by folder, each by the whole path as text.
pub fn instance_files(data_root: &Path) -> Result<Vec<(PathBuf, String)>, BackupFailed> {
    let mut found = Vec::new();
    for folder in INSTANCE_FOLDERS {
        let root = data_root.join(folder);
        let mut files = Vec::new();
        walk(&root, &mut files)?;
        let mut named = Vec::new();
        for path in files {
            let relative = path.strip_prefix(data_root).unwrap_or(&path);
            let parts = relative
                .components()
                .map(|part| {
                    part.as_os_str().to_str().map(str::to_string).ok_or_else(|| {
                        BackupFailed::new(format!(
                            "The file name of {} is not text, so it cannot be put in an archive.",
                            path.display()
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            named.push((path, format!("{INSTANCE_FOLDER}{}", parts.join("/"))));
        }
        named.sort_by(|left, right| utf16_order(&left.1, &right.1));
        found.extend(named);
    }
    Ok(found)
}

/// Regular files below `folder`; a link is neither followed nor listed.
fn walk(folder: &Path, files: &mut Vec<PathBuf>) -> Result<(), BackupFailed> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(&entry.path(), files)?;
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

/// The limits an archive is read with: entries may be large, but there are
/// not millions of them in a backup, and the manifest is never big.
pub fn limits() -> Limits {
    Limits {
        max_entries: 2_000_000,
        max_directory_bytes: 256 * 1024 * 1024,
        ..Limits::default()
    }
}

/// The manifest of an archive, or none when the file is not one.
pub fn read_manifest(path: &Path) -> Option<Manifest> {
    let mut archive = ZipArchive::with_limits(File::open(path).ok()?, limits()).ok()?;
    manifest_of(&mut archive)
}

pub(crate) fn manifest_of<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Option<Manifest> {
    let at = archive.find(MANIFEST_ENTRY.as_bytes())?;
    let bytes = archive.read_to_vec(at, MAX_MANIFEST_BYTES).ok()?;
    Manifest::read(&bytes)
}

/// `"ok"`, or what is wrong with the archive. Every byte of every entry is
/// read and its length and CRC-32 checked, and the database in it is opened
/// and checked by SQLite, in a copy made in `work_folder`.
///
/// It blocks: run it on a blocking thread.
pub fn verify(path: &Path, work_folder: &Path) -> String {
    match verify_inner(path, work_folder) {
        Ok(verdict) => verdict,
        Err(message) => message,
    }
}

fn verify_inner(path: &Path, work_folder: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::with_limits(file, limits()).map_err(verdict_of)?;
    let Some(manifest) = manifest_of(&mut archive) else {
        return Ok("The archive has no readable manifest.".to_string());
    };

    if manifest.has_part(parts::DATABASE) {
        let Some(at) = archive.find(DATABASE_ENTRY.as_bytes()) else {
            return Ok("The database is missing from the archive.".to_string());
        };
        let size = archive.entries()[at].uncompressed_size();
        match space::free_bytes(work_folder) {
            None => return Err(space::unknown("checking the database", size)),
            Some(free) if size > free => {
                return Err(format!(
                    "Checking the database needs {size} bytes on the volume, and {free} are free."
                ));
            }
            Some(_) => {}
        }
        std::fs::create_dir_all(work_folder).map_err(|error| error.to_string())?;
        let copy = work_folder.join(format!(
            ".verify-{}.partial",
            super::random_token()
        ));
        let copied = copy_entry(&mut archive, at, &copy);
        let verdict = copied.map(|()| snapshot::check_integrity(&copy));
        snapshot::remove_with_side_files(&copy);
        let verdict = verdict.map_err(|error| error.to_string())?;
        if !verdict.eq_ignore_ascii_case("ok") {
            return Ok(verdict);
        }
    }

    let count = |prefix: &str| {
        archive
            .entries()
            .iter()
            .filter(|entry| entry.name_bytes().starts_with(prefix.as_bytes()))
            .count() as i64
    };
    for (prefix, found, named, what) in [
        (OBJECTS_FOLDER, count(OBJECTS_FOLDER), manifest.object_count, "objects"),
        (KEYS_FOLDER, count(KEYS_FOLDER), manifest.key_files, "key files"),
        (INSTANCE_FOLDER, count(INSTANCE_FOLDER), manifest.instance_files, "instance files"),
    ] {
        let _ = prefix;
        if found != named {
            return Ok(format!(
                "The archive holds {found} {what}; its manifest names {named}."
            ));
        }
    }

    archive.verify_all().map_err(verdict_of)?;
    Ok("ok".to_string())
}

/// What is said of an archive that failed: the checksum failure in the words
/// the first server used, the rest as the archive library says it.
fn verdict_of(error: stellarr_archive::Error) -> String {
    match error {
        stellarr_archive::Error::Checksum(name) => {
            format!("The archive entry '{name}' failed its checksum or length check.")
        }
        stellarr_archive::Error::Corrupt(why) if why.contains("end-of-archive record") => {
            "End of Central Directory record could not be found.".to_string()
        }
        stellarr_archive::Error::Corrupt(why) if why.contains("too short") => {
            "Central Directory corrupt.".to_string()
        }
        other => other.to_string(),
    }
}

/// Writes an entry to a new file, never more than it declared.
fn copy_entry(
    archive: &mut ZipArchive<File>,
    at: usize,
    to: &Path,
) -> Result<(), stellarr_archive::Error> {
    let mut target = OpenOptions::new().write(true).create_new(true).open(to)?;
    archive.copy_to(at, &mut target)?;
    target.flush()?;
    Ok(())
}
