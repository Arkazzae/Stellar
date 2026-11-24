//! An instance's data on disk, and archives of it, for the tests of what a
//! backup holds and what a restore does with one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stellarr_archive::{EntryOptions, ZipWriter};
use stellarr_storage::{FolderStore, ObjectStorage, StoredObjectInfo};

use super::archive::{self, Sources};
use super::manifest::Manifest;
use super::parts::Parts;
use super::progress::Progress;
use crate::database::Database;
use crate::settings::DatabaseSettings;
use crate::testing::TempDir;

/// A data root with every folder a backup covers.
pub struct Instance {
    pub folder: TempDir,
    pub database: Database,
    pub runtime: tokio::runtime::Runtime,
    pub store: Arc<dyn ObjectStorage>,
}

pub const OBJECTS: [(&str, &[u8]); 3] = [
    ("images/2026/a.png", b"\x89PNG not really"),
    ("audio/b.ogg", b"OggS bytes"),
    ("c.bin", b""),
];

impl Instance {
    pub fn new() -> Instance {
        let folder = TempDir::new();
        let root = folder.path();
        fs::create_dir_all(root.join("db")).unwrap();
        let database = Database::open(&DatabaseSettings {
            path: root.join("db/stellarr.db"),
            busy_timeout: std::time::Duration::from_secs(30),
        })
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let store: Arc<dyn ObjectStorage> = Arc::new(FolderStore::new(root.join("files")));
        for (key, bytes) in OBJECTS {
            let mut reader = std::io::Cursor::new(bytes.to_vec());
            runtime.block_on(store.put(key, &mut reader)).unwrap();
        }
        for (path, bytes) in [
            ("keys/master.key", &b"0123456789abcdef0123456789abcdef"[..]),
            ("secrets/turn-shared-secret", b"turn"),
            ("tls/privkey.pem", b"key"),
            ("tls/fullchain.pem", b"chain"),
            ("releases/desktop/1.0/app.AppImage", b"app"),
            ("releases/a-b/x", b"1"),
            ("releases/a/x", b"2"),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let instance = Instance {
            folder,
            database,
            runtime,
            store,
        };
        instance.write_rows("first");
        instance
    }

    pub fn root(&self) -> &Path {
        self.folder.path()
    }

    pub fn database_path(&self) -> PathBuf {
        self.root().join("db/stellarr.db")
    }

    /// A row with a mark in it, to tell one state of the data from another.
    pub fn write_rows(&self, mark: &str) {
        let mark = mark.to_string();
        self.runtime
            .block_on(self.database.write(move |tx| -> Result<(), crate::database::DbError> {
                tx.execute(
                    "INSERT OR REPLACE INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('mark', ?1, '2026-11-24 00:00:00')",
                    [mark],
                )?;
                Ok(())
            }))
            .unwrap();
    }

    pub fn mark(&self) -> String {
        let connection = rusqlite::Connection::open(self.database_path()).unwrap();
        connection
            .query_row(
                "SELECT \"ProtectedValue\" FROM \"InstanceSecrets\" WHERE \"Name\" = 'mark'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn objects(&self) -> Vec<StoredObjectInfo> {
        let mut all: Vec<_> = OBJECTS
            .iter()
            .map(|(key, bytes)| StoredObjectInfo {
                key: key.to_string(),
                size: bytes.len() as u64,
            })
            .collect();
        all.sort_by(|a, b| archive::utf16_order(&a.key, &b.key));
        all
    }

    /// An archive of the instance, written where asked.
    pub fn archive(&self, wanted: Parts, at: &Path) -> Manifest {
        let sources = Sources {
            database: self.database_path(),
            store: self.store.clone(),
            objects: self.objects(),
            keys_folder: self.root().join("keys"),
            instance_name: "Test instance".to_string(),
            environment_secrets: vec!["STELLARR_MAIL_RESEND_API_KEY".to_string()],
            work_folder: self.root().to_path_buf(),
            data_root: self.root().to_path_buf(),
            database_snapshot: None,
        };
        archive::write(
            at,
            wanted,
            &sources,
            &Progress::new(0),
            &AtomicBool::new(false),
            self.runtime.handle(),
        )
        .unwrap()
    }

    /// Every file under the root as `(relative path, bytes)`, in path order,
    /// without the folders that a restore leaves alone and the database's side
    /// files.
    pub fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut found = Vec::new();
        walk(self.root(), self.root(), &mut found);
        found.retain(|(path, _)| {
            !path.starts_with("restore")
                && !path.starts_with("backups")
                && !path.ends_with("-wal")
                && !path.ends_with("-shm")
        });
        found.sort();
        found
    }
}

fn walk(root: &Path, folder: &Path, found: &mut Vec<(String, Vec<u8>)>) {
    for entry in fs::read_dir(folder).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found);
        } else {
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            found.push((relative, fs::read(&path).unwrap()));
        }
    }
}

/// An archive of the entries given, as `(name, bytes, mode)`, with whatever
/// names they carry, followed by a manifest of a full backup unless one of
/// the entries is named `manifest.json`.
pub fn archive_of(entries: &[(&str, &[u8], Option<u32>)], at: &Path) {
    let mut zip = ZipWriter::new(fs::File::create(at).unwrap());
    let mut has_manifest = false;
    for (name, bytes, mode) in entries {
        has_manifest |= *name == "manifest.json";
        let mut options = EntryOptions::stored();
        if let Some(mode) = mode {
            options = options.unix_mode(*mode);
        }
        zip.add_bytes(name, options, bytes).unwrap();
    }
    if !has_manifest {
        let mut manifest = Manifest::read(MANIFEST).unwrap();
        let count = |prefix: &str| entries.iter().filter(|(name, _, _)| name.starts_with(prefix)).count() as i64;
        manifest.object_count = count("objects/");
        manifest.key_files = count("keys/");
        manifest.instance_files = count("instance/");
        zip.add_bytes("manifest.json", EntryOptions::stored(), &manifest.to_json())
            .unwrap();
    }
    zip.finish().unwrap();
}

pub const MANIFEST: &[u8] = br#"{"format":"stellarr-backup","version":1,"createdAt":"2026-11-24T10:00:00Z","instanceName":"x","parts":["database","objects","keys","instance"],"databaseBytes":0,"objectCount":0,"objectBytes":0,"missingObjects":[],"keyFiles":0,"environmentSecrets":[],"instanceFiles":0}"#;

/// The bytes of a small, valid database.
pub fn database_bytes() -> Vec<u8> {
    let folder = TempDir::new();
    let path = folder.path().join("small.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
        .unwrap();
    drop(connection);
    fs::read(path).unwrap()
}

/// The bytes of an instance's database: the schema, with nothing in it.
pub fn instance_database_bytes() -> Vec<u8> {
    let instance = Instance::new();
    let folder = TempDir::new();
    let copy = folder.path().join("instance.db");
    super::snapshot::copy_database(&instance.database_path(), &copy).unwrap();
    fs::read(copy).unwrap()
}
