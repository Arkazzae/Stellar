//! `backup verify | restore | cancel | apply`: checking an archive and staging
//! it for the container's next start, which puts it in place before any
//! service reads the data volume.
//!
//! This is the commands' own side: the arguments, what is asked, what is
//! printed and which status ends it. Reading an archive and replacing the data
//! folder are the backups library's ([`RestoreLibrary`]), which this module
//! only calls: a second restore written here would be a second place for the
//! rules of one to go wrong.

use std::io::Write;
use std::path::{Path, PathBuf};

use stellarr_api::values::Timestamp;

use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::json::{Json, Zone, moment};
use crate::runtime::Runtime;
use crate::status::probe_api;

/// The folder the panel and the automatic backups write archives to.
const LOCAL_BACKUPS_FOLDER: &str = "backups";

/// What an archive says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub created_at: Timestamp,
    pub instance_name: String,
    pub database_bytes: i64,
    pub object_count: i64,
    pub object_bytes: i64,
    pub missing_objects: Vec<String>,
    pub key_files: i64,
    pub instance_files: i64,
    pub environment_secrets: Vec<String>,
}

/// Why an archive was not restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    /// The archive cannot be restored; the data was not touched.
    Refused(String),
    /// The restore stopped halfway: the data folder is in neither state.
    Broken(String),
}

/// What was put in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub manifest: Manifest,
    pub previous_data_directory: String,
}

/// The library a restore is made with: the checks of an archive, and the
/// replacement of the data folder by one. The names are the first tool's
/// `BackupRestore` members.
pub trait RestoreLibrary {
    /// Reads the archive to its end, checks every entry and the database in
    /// it, and returns its manifest; `work` is a folder to make copies in.
    fn check(&self, archive: &Path, work: &Path) -> Result<Manifest, RestoreError>;
    /// Where a staged archive waits: `<data root>/restore/staged.zip`.
    fn staged_archive_path(&self, data_root: &Path) -> PathBuf;
    /// The folder under the data root that holds the staged archive and what a
    /// restore leaves: `restore`.
    fn folder_name(&self) -> &'static str;
    /// The file whose presence stops the container from starting after a
    /// restore broke halfway.
    fn broken_marker_name(&self) -> &'static str;
    /// Removes what an earlier restore left in the folder.
    fn clear_leftovers(&self, data_root: &Path) -> std::io::Result<()>;
    /// Replaces the data with the staged archive.
    fn apply(
        &self,
        staged: &Path,
        data_root: &Path,
        now: Timestamp,
    ) -> Result<Applied, RestoreError>;
    /// Writes what happened, for `restore/last-restore.json`.
    fn write_result(&self, data_root: &Path, outcome: &Json) -> std::io::Result<()>;
}

/// What stands in until the backups library is on this branch: every command
/// that needs it says so, and none touches the data. It is replaced where the
/// runtime is made (`Runtime::create`), with nothing else to change.
pub struct NotYetBuilt;

impl NotYetBuilt {
    fn error() -> RestoreError {
        RestoreError::Refused(
            "this build has no backups library yet; see stellarr_cli::backup::NotYetBuilt"
                .to_string(),
        )
    }
}

impl RestoreLibrary for NotYetBuilt {
    fn check(&self, _: &Path, _: &Path) -> Result<Manifest, RestoreError> {
        Err(NotYetBuilt::error())
    }

    fn staged_archive_path(&self, data_root: &Path) -> PathBuf {
        data_root.join("restore").join("staged.zip")
    }

    fn folder_name(&self) -> &'static str {
        "restore"
    }

    fn broken_marker_name(&self) -> &'static str {
        "restore-broken"
    }

    fn clear_leftovers(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }

    fn apply(&self, _: &Path, _: &Path, _: Timestamp) -> Result<Applied, RestoreError> {
        Err(NotYetBuilt::error())
    }

    fn write_result(&self, _: &Path, _: &Json) -> std::io::Result<()> {
        Ok(())
    }
}

/// `{moment:u}`: `2026-11-14 03:00:00Z`.
fn universal(at: Timestamp) -> String {
    let text = at.to_wire_text();
    format!("{}Z", text[..19].replace('T', " "))
}

fn summary(path: &str, manifest: &Manifest) -> Json {
    Json::object([
        ("archive", Json::text(path)),
        ("createdAt", moment(manifest.created_at, Zone::Utc)),
        ("instanceName", Json::text(&manifest.instance_name)),
        ("databaseBytes", manifest.database_bytes.into()),
        ("objectCount", manifest.object_count.into()),
        ("objectBytes", manifest.object_bytes.into()),
        ("missingObjects", manifest.missing_objects.len().into()),
        ("keyFiles", manifest.key_files.into()),
        ("instanceFiles", manifest.instance_files.into()),
        (
            "environmentSecrets",
            Json::array(manifest.environment_secrets.iter().cloned().map(Json::from)),
        ),
    ])
}

fn warnings(manifest: &Manifest) -> Vec<String> {
    let mut warnings = Vec::new();
    if !manifest.missing_objects.is_empty() {
        warnings.push(format!(
            "{} uploaded files could not be read when the backup was made and are not in it.",
            manifest.missing_objects.len()
        ));
    }
    if !manifest.environment_secrets.is_empty() {
        warnings.push(format!(
            "These secrets came from the environment and are not in the archive; keep them in .env: {}.",
            manifest.environment_secrets.join(", ")
        ));
    }
    warnings
}

/// A path as given, or the name of an archive in the instance's own backups folder.
fn resolve_archive(runtime: &Runtime, archive: &str) -> CliOutcome<String> {
    if Path::new(archive).is_file() {
        return Ok(crate::runtime::full_path(archive));
    }
    let folder = Path::new(&runtime.data_root).join(LOCAL_BACKUPS_FOLDER);
    let local = folder.join(archive);
    let plain_name = Path::new(archive)
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(archive));
    if plain_name && local.is_file() {
        return Ok(local.display().to_string());
    }
    Err(CliError::new(
        "backup_not_found",
        format!(
            "No archive exists at {archive}, nor under that name in {}.",
            folder.display()
        ),
        exit::NOT_FOUND,
    ))
}

/// The database is copied out of the archive to be checked; the system temp
/// folder keeps that copy off the data volume.
fn check(runtime: &Runtime, path: &str) -> CliOutcome<Manifest> {
    let work = std::env::temp_dir().join(format!(
        "stellarr-backup-check-{}",
        stellarr_api::values::Id::random()
    ));
    let checked = runtime.restore.check(Path::new(path), &work);
    let _ = std::fs::remove_dir_all(&work);
    checked.map_err(|error| match error {
        RestoreError::Refused(message) | RestoreError::Broken(message) => {
            CliError::new("backup_refused", message, exit::CONFLICT)
        }
    })
}

fn describe(manifest: &Manifest) -> Vec<String> {
    vec![
        format!("Database: {} bytes.", manifest.database_bytes),
        format!(
            "Uploaded files: {} ({} bytes).",
            manifest.object_count, manifest.object_bytes
        ),
        format!(
            "Key files: {}. Instance files: {}.",
            manifest.key_files, manifest.instance_files
        ),
    ]
}

pub fn verify(runtime: &mut Runtime, archive: &str) -> CliOutcome {
    let path = resolve_archive(runtime, archive)?;
    let manifest = check(runtime, &path)?;
    let mut lines = vec![format!(
        "{path} is a full backup of {} from {}, and this release can restore it.",
        manifest.instance_name,
        universal(manifest.created_at)
    )];
    lines.extend(describe(&manifest));
    Ok(
        CliResult::new("backup.verify", summary(&path, &manifest), lines)
            .warnings(warnings(&manifest)),
    )
}

pub fn restore(runtime: &mut Runtime, archive: &str) -> CliOutcome {
    let path = resolve_archive(runtime, archive)?;
    let manifest = check(runtime, &path)?;
    let data_root = PathBuf::from(&runtime.data_root);
    let staged = runtime.restore.staged_archive_path(&data_root);
    let replacing = staged.is_file();
    runtime.confirm(&format!(
        "At the container's next start, the database, uploaded files, keys, secrets, TLS files and desktop releases under {} will be replaced by the backup of {} from {}. Everything written since then is lost. The current data is kept in {}/previous-<time>.{}",
        runtime.data_root,
        manifest.instance_name,
        universal(manifest.created_at),
        data_root.join(runtime.restore.folder_name()).display(),
        if replacing { " This replaces the backup staged before." } else { "" }
    ))?;
    let folder = staged.parent().unwrap_or(&data_root).to_path_buf();
    std::fs::create_dir_all(&folder)?;
    runtime.restore.clear_leftovers(&data_root)?;
    let size = std::fs::metadata(&path)?.len();
    enough_space(size, crate::status::free_bytes(&folder))?;
    // Copied, not moved: the archive may be one the panel lists, and the copy
    // lives on the data volume, which a recreated container keeps.
    let partial = PathBuf::from(format!(
        "{}.{}.partial",
        staged.display(),
        stellarr_api::values::Id::random()
            .to_string()
            .replace('-', "")
    ));
    let copied = (|| -> std::io::Result<()> {
        let mut source = std::fs::File::open(&path)?;
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        std::io::copy(&mut source, &mut target)?;
        target.flush()?;
        target.sync_all()?;
        std::fs::rename(&partial, &staged)
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&partial);
        return Err(error.into());
    }
    let data = Json::object([
        ("staged", Json::text(staged.display().to_string())),
        ("backup", summary(&path, &manifest)),
    ]);
    Ok(CliResult::new(
        "backup.restore",
        data,
        vec![
            format!(
                "Staged the backup of {} from {}.",
                manifest.instance_name,
                universal(manifest.created_at)
            ),
            "Restart the container to restore it, for example: docker compose restart stellarr"
                .to_string(),
            "Until then, 'stellarr backup cancel' drops it.".to_string(),
        ],
    )
    .warnings(warnings(&manifest))
    .changed())
}

pub fn cancel(runtime: &mut Runtime) -> CliOutcome {
    let staged = runtime
        .restore
        .staged_archive_path(Path::new(&runtime.data_root));
    if !staged.is_file() {
        return Err(nothing_staged(&staged));
    }
    runtime.confirm(&format!(
        "Will drop the staged backup {}; the next start keeps the current data.",
        staged.display()
    ))?;
    std::fs::remove_file(&staged)?;
    Ok(CliResult::new(
        "backup.cancel",
        Json::object([("staged", Json::text(staged.display().to_string()))]),
        vec!["Dropped the staged backup. The next start keeps the current data.".to_string()],
    )
    .changed())
}

/// Puts the staged backup in place. The container's entrypoint runs this
/// before it starts any service; it refuses while the API answers.
pub fn apply(runtime: &mut Runtime) -> CliOutcome {
    let data_root = PathBuf::from(&runtime.data_root);
    let staged = runtime.restore.staged_archive_path(&data_root);
    if !staged.is_file() {
        return Err(nothing_staged(&staged));
    }
    let api = probe_api(runtime)?;
    if api.available {
        return Err(CliError::new(
            "instance_running",
            format!(
                "The API answers at {}, so the instance is running and its data is in use. The staged backup is restored by the container's next start; restart the container instead.",
                api.url
            ),
            exit::CONFLICT,
        ));
    }
    runtime.confirm(&format!(
        "Will replace the instance's data under {} with the staged backup now.",
        runtime.data_root
    ))?;
    let now = Timestamp::now();
    let folder = staged.parent().unwrap_or(&data_root).to_path_buf();
    let staged_text = staged.display().to_string();
    match runtime.restore.apply(&staged, &data_root, now) {
        Ok(applied) => {
            std::fs::remove_file(&staged)?;
            let backup = summary(&staged_text, &applied.manifest);
            let outcome = Json::object([
                ("outcome", "restored".into()),
                ("at", moment(now, Zone::Utc)),
                ("backup", backup.clone()),
                ("previousData", Json::text(&applied.previous_data_directory)),
            ]);
            runtime.restore.write_result(&data_root, &outcome)?;
            Ok(CliResult::new(
                "backup.apply",
                Json::object([
                    ("backup", backup),
                    ("previousData", Json::text(&applied.previous_data_directory)),
                ]),
                vec![
                    format!(
                        "Restored the backup of {} from {}.",
                        applied.manifest.instance_name,
                        universal(applied.manifest.created_at)
                    ),
                    format!(
                        "The data it replaced is in {}; delete it once the instance looks right.",
                        applied.previous_data_directory
                    ),
                ],
            )
            .warnings(warnings(&applied.manifest))
            .changed())
        }
        Err(RestoreError::Refused(reason)) => {
            let kept = folder.join(format!("refused-{}.zip", stamp(now)));
            std::fs::rename(&staged, &kept)?;
            let outcome = Json::object([
                ("outcome", "refused".into()),
                ("at", moment(now, Zone::Utc)),
                ("reason", Json::text(&reason)),
                ("archive", Json::text(kept.display().to_string())),
            ]);
            let _ = runtime.restore.write_result(&data_root, &outcome);
            Err(CliError::new(
                "restore_refused",
                format!(
                    "{reason} The data was left as it was; the archive is kept as {}.",
                    kept.display()
                ),
                exit::CONFLICT,
            ))
        }
        Err(RestoreError::Broken(reason)) => {
            // The entrypoint refuses to start while this marker exists, so a
            // half-swapped volume is never served.
            let marker = folder.join(runtime.restore.broken_marker_name());
            std::fs::write(&marker, &reason)?;
            std::fs::rename(&staged, folder.join(format!("broken-{}.zip", stamp(now))))?;
            let outcome = Json::object([
                ("outcome", "broken".into()),
                ("at", moment(now, Zone::Utc)),
                ("reason", Json::text(&reason)),
            ]);
            let _ = runtime.restore.write_result(&data_root, &outcome);
            Err(CliError::new(
                "restore_broken",
                format!(
                    "{reason} The container will not start until {} is removed.",
                    marker.display()
                ),
                exit::FAILURE,
            ))
        }
    }
}

/// Whether `size` bytes fit where the archive is staged. Space that cannot be
/// read is not space that is there: the copy could fill the volume the
/// instance runs on, so the archive is not staged.
fn enough_space(size: u64, available: Option<u64>) -> CliOutcome<()> {
    let Some(available) = available else {
        return Err(CliError::new(
            "free_space_unknown",
            format!(
                "The free space on the data volume could not be read (`df` failed), and staging the archive needs {size} bytes, so it was not staged. Nothing was staged."
            ),
            exit::UNAVAILABLE,
        ));
    };
    if available < size {
        return Err(CliError::new(
            "not_enough_space",
            format!(
                "Staging the archive needs {size} bytes on the data volume, and {available} are free. Nothing was staged."
            ),
            exit::CONFLICT,
        ));
    }
    Ok(())
}

/// `yyyyMMdd-HHmmss`.
fn stamp(at: Timestamp) -> String {
    let text = at.to_wire_text();
    format!(
        "{}{}{}-{}{}{}",
        &text[0..4],
        &text[5..7],
        &text[8..10],
        &text[11..13],
        &text[14..16],
        &text[17..19]
    )
}

fn nothing_staged(staged: &Path) -> CliError {
    CliError::new(
        "restore_not_staged",
        format!(
            "No backup is staged for restore ({} does not exist); nothing changed.",
            staged.display()
        ),
        exit::NOT_FOUND,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_needs_space_that_is_known_and_enough() {
        assert!(enough_space(100, Some(100)).is_ok());
        let short = enough_space(101, Some(100)).unwrap_err();
        assert_eq!(
            (short.code, short.exit),
            ("not_enough_space", exit::CONFLICT)
        );
        let unknown = enough_space(1, None).unwrap_err();
        assert_eq!(
            (unknown.code, unknown.exit),
            ("free_space_unknown", exit::UNAVAILABLE)
        );
        assert!(unknown.message.contains("Nothing was staged"));
    }
}
