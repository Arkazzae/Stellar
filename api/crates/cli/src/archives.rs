//! The backups library of the server, as the restore commands use it.
//!
//! A second restore is not written here: every call is the one of
//! `stellarr_api::backups::restore`, with the types of this tool's own around
//! it.

use std::path::{Path, PathBuf};

use stellarr_api::backups::restore::{self, RestoreError as Library};
use stellarr_api::values::Timestamp;

use crate::backup::{Applied, Manifest, RestoreError, RestoreLibrary};
use crate::json::Json;

pub struct Archives;

fn error(error: Library) -> RestoreError {
    match error {
        Library::Refused(message) => RestoreError::Refused(message),
        Library::Broken(message) => RestoreError::Broken(message),
    }
}

fn manifest(manifest: stellarr_api::backups::manifest::Manifest) -> Manifest {
    Manifest {
        created_at: manifest.created_at,
        instance_name: manifest.instance_name,
        database_bytes: manifest.database_bytes,
        object_count: manifest.object_count,
        object_bytes: manifest.object_bytes,
        missing_objects: manifest.missing_objects,
        key_files: manifest.key_files,
        instance_files: manifest.instance_files,
        environment_secrets: manifest.environment_secrets,
    }
}

impl RestoreLibrary for Archives {
    fn check(&self, archive: &Path, work: &Path) -> Result<Manifest, RestoreError> {
        restore::check(archive, work).map(manifest).map_err(error)
    }

    fn staged_archive_path(&self, data_root: &Path) -> PathBuf {
        restore::staged_archive_path(data_root)
    }

    fn folder_name(&self) -> &'static str {
        restore::FOLDER_NAME
    }

    fn broken_marker_name(&self) -> &'static str {
        restore::BROKEN_MARKER_NAME
    }

    fn clear_leftovers(&self, data_root: &Path) -> std::io::Result<()> {
        restore::clear_leftovers(data_root);
        Ok(())
    }

    fn apply(
        &self,
        staged: &Path,
        data_root: &Path,
        now: Timestamp,
    ) -> Result<Applied, RestoreError> {
        let done = restore::apply(staged, data_root, now).map_err(error)?;
        Ok(Applied {
            manifest: manifest(done.manifest),
            previous_data_directory: done.previous_data_directory.display().to_string(),
        })
    }

    fn write_result(&self, data_root: &Path, outcome: &Json) -> std::io::Result<()> {
        restore::write_result_text(data_root, &outcome.pretty())
    }
}
