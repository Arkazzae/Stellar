//! Helpers the unit tests share.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::database::Database;
use crate::settings::DatabaseSettings;
use crate::values::Id;

/// A folder of its own that is removed when it goes out of scope.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        let path = std::env::temp_dir().join(format!("stellarr-unit-{}", Id::random()));
        std::fs::create_dir_all(&path).expect("a temporary folder");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn database_settings(folder: &TempDir) -> DatabaseSettings {
    DatabaseSettings {
        path: folder.path().join("stellarr.db"),
        busy_timeout: Duration::from_secs(30),
    }
}

/// A new database in a folder of its own.
pub fn new_database() -> (TempDir, Database) {
    let folder = TempDir::new();
    let database = Database::open(&database_settings(&folder)).expect("a new database");
    (folder, database)
}
