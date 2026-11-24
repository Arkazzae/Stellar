//! The commands the operator's tool will give backups, written against the
//! library alone, so that the archives this server makes and restores can be
//! tried against the first server's (see `tests/backups-archives.sh`).
//!
//!   backups make   DATA_ROOT PARTS OUT.zip   parts: any of d o k i, or "all"
//!   backups verify ARCHIVE                   prints ok, or what is wrong
//!   backups check  ARCHIVE                   can it be restored (prints the manifest)
//!   backups apply  ARCHIVE DATA_ROOT         restores it over the data root
//!
//! The data root is laid out as the container's entrypoint lays it out:
//! `db/stellarr.db`, `files/`, `keys/`, `secrets/`, `tls/` and `releases/`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use stellarr_api::backups::archive::{self, Sources};
use stellarr_api::backups::progress::Progress;
use stellarr_api::backups::{Parts, restore};
use stellarr_api::values::Timestamp;
use stellarr_storage::{FOLDER_NAME, FolderStore, ObjectStorage};

fn parts_of(text: &str) -> Parts {
    if text == "all" {
        return Parts::ALL;
    }
    let mut parts = Parts::NONE;
    for letter in text.chars() {
        parts = parts.with(match letter {
            'd' => Parts::DATABASE,
            'o' => Parts::OBJECTS,
            'k' => Parts::KEYS,
            'i' => Parts::INSTANCE,
            other => panic!("no part called {other}"),
        });
    }
    parts
}

fn make(root: &Path, parts: Parts, out: &Path) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let database = root.join("db/stellarr.db");
    let store: Arc<dyn ObjectStorage> = Arc::new(FolderStore::new(root.join(FOLDER_NAME)));
    let objects = runtime
        .block_on(store.list().collect())
        .map_err(|e| e.to_string())?;
    let name: String = rusqlite::Connection::open(&database)
        .and_then(|c| {
            c.query_row(
                "SELECT \"Name\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1",
                [],
                |row| row.get(0),
            )
        })
        .map_err(|e| e.to_string())?;
    let work = out
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let sources = Sources {
        database,
        store,
        objects,
        keys_folder: root.join("keys"),
        instance_name: name,
        environment_secrets: Vec::new(),
        work_folder: work,
        data_root: root.to_path_buf(),
        database_snapshot: None,
    };
    archive::write(
        out,
        parts,
        &sources,
        &Progress::new(0),
        &AtomicBool::new(false),
        runtime.handle(),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let at = |n: usize| PathBuf::from(arguments.get(n).expect("an argument"));
    let outcome = match arguments.first().map(String::as_str) {
        Some("make") => make(&at(1), parts_of(&arguments[2]), &at(3)),
        Some("verify") => {
            let work = std::env::temp_dir();
            let verdict = archive::verify(&at(1), &work);
            println!("{verdict}");
            if verdict == "ok" {
                Ok(())
            } else {
                Err(verdict)
            }
        }
        Some("check") => {
            let work = std::env::temp_dir().join(format!("backups-check-{}", std::process::id()));
            let outcome = restore::check(&at(1), &work);
            let _ = std::fs::remove_dir_all(&work);
            outcome
                .map(|manifest| println!("ok {}", String::from_utf8_lossy(&manifest.to_json())))
                .map_err(|e| e.to_string())
        }
        Some("apply") => restore::apply(&at(1), &at(2), Timestamp::now())
            .map(|r| {
                println!(
                    "restored; previous data in {}",
                    r.previous_data_directory.display()
                )
            })
            .map_err(|e| e.to_string()),
        _ => Err("make | verify | check | apply".to_string()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
