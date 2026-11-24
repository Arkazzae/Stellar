//! The two timers of this part of the server, run by the binary: the hourly
//! housekeeping and the automatic backup, at the start and across a restart.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::support::{Server, TempDir};

const WORKERS: (&str, &str) = ("Testing__Workers", "housekeeping,automatic-backups");
const NOW: (&str, &str) = ("Maintenance__InitialDelayMinutes", "0");

async fn log_line(server: &Server, needle: &str) -> Option<String> {
    for _ in 0..200 {
        if let Some(line) = server.log().into_iter().find(|line| line.contains(needle)) {
            return Some(line);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

fn old_file(path: &Path, days: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(days * 86_400))
        .unwrap();
}

#[tokio::test]
async fn the_housekeeping_runs_once_at_start_and_says_what_it_did() {
    let folder = TempDir::new();
    old_file(&folder.path().join("backups/backup-old.zip"), 400);
    old_file(&folder.path().join("logs/old.log"), 400);
    old_file(&folder.path().join("backups/backup-new.zip"), 1);
    let server = Server::start_in(folder.path(), &[WORKERS, NOW]).await;

    let line = log_line(&server, "Maintenance removed")
        .await
        .expect("the run was logged");
    assert!(
        line.contains("0 orphan attachments, 1 backup files and 1 log files; 0 object deletions remain queued."),
        "{line}"
    );
    assert!(!folder.path().join("backups/backup-old.zip").exists());
    assert!(folder.path().join("backups/backup-new.zip").exists());
    server.stop().await;
}

#[tokio::test]
async fn a_housekeeping_that_is_switched_off_ends_at_once_and_prunes_nothing() {
    let folder = TempDir::new();
    old_file(&folder.path().join("backups/backup-old.zip"), 400);
    let server = Server::start_in(
        folder.path(),
        &[WORKERS, NOW, ("Maintenance__Enabled", "false")],
    )
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(log_line(&server, "Maintenance removed").await.is_none());
    assert!(folder.path().join("backups/backup-old.zip").exists());
    server.stop().await;
}

fn archives(folder: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(folder.join("backups"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".zip"))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

#[tokio::test]
async fn the_automatic_backup_makes_one_verified_archive_and_a_restart_makes_no_more() {
    let folder = TempDir::new();
    // The owner exists before the timer looks for one.
    let first = Server::start_in(folder.path(), &[]).await;
    first.found("owner").await;
    assert!(first.stop().await);

    let automatic = [
        WORKERS,
        ("Backups__Enabled", "true"),
        ("Backups__InitialDelayMinutes", "0"),
    ];
    let second = Server::start_in(folder.path(), &automatic).await;
    let checkpoint = folder.path().join("automatic-backup.json");
    for _ in 0..400 {
        if checkpoint.exists() && archives(folder.path()).len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let made = archives(folder.path());
    assert_eq!(made.len(), 1, "{made:?}");
    assert!(
        folder
            .path()
            .join("backups")
            .join(format!("{}.verified", made[0]))
            .exists()
    );
    let state: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    let object = state.as_object().unwrap();
    // A Value sorts its members; the names are the first server's, PascalCase.
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "LastError",
            "LastFailureAt",
            "LastSuccessAt",
            "NextAttemptAt",
            "PendingBackupId",
            "ServerId"
        ]
    );
    assert!(object["LastSuccessAt"].is_string());
    assert!(object["NextAttemptAt"].is_string());
    assert!(second.stop().await);

    // The checkpoint says the next one is a day off.
    let third = Server::start_in(folder.path(), &automatic).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(archives(folder.path()), made);
    third.stop().await;
}
