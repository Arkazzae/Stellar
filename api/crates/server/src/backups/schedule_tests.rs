//! The schedule of automatic backups at moments the test chooses.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, params};
use stellarr_storage::BoxFuture;

use super::destination::Destination;
use super::policy::{self, Schedule};
use super::schedule::{self, Checkpoint, Clock, Outcome, run_once};
use super::{BackupFailed, listing};
use crate::database::Database;
use crate::events::Unconnected;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::state::AppState;
use crate::testing::TempDir;
use crate::values::{Id, Timestamp};

struct Instance {
    folder: TempDir,
    variables: Vec<(String, String)>,
    state: AppState,
}

impl Instance {
    /// An instance with an owner, whose schedule is on unless the settings
    /// say otherwise.
    fn new(extra: &[(&str, &str)]) -> Instance {
        let folder = TempDir::new();
        let mut variables: Vec<(String, String)> = vec![
            ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
            ("STELLARR_DATA_ROOT".into(), folder.path().display().to_string()),
            ("STELLARR_BACKUP_ENABLED".into(), "true".into()),
        ];
        for (name, value) in extra {
            variables.retain(|(have, _)| have != name);
            variables.push((name.to_string(), value.to_string()));
        }
        let state = Instance::state_of(&variables);
        let instance = Instance { folder, variables, state };
        instance.add_owner();
        instance
    }

    fn state_of(variables: &[(String, String)]) -> AppState {
        let settings = Settings::load(variables).unwrap();
        std::fs::create_dir_all(settings.database.path.parent().unwrap()).unwrap();
        let database = Database::open(&settings.database).unwrap();
        AppState::new(
            settings,
            database,
            Arc::new(Unconnected),
            Arc::new(Unconnected),
            Sealer::from_key([5; 32]),
        )
    }

    /// The process started again on the same files: nothing in memory is
    /// left but what the files hold.
    fn restarted(&mut self) {
        self.state = Instance::state_of(&self.variables);
    }

    fn raw(&self) -> Connection {
        let connection = Connection::open(self.folder.path().join("db").join("stellarr.db")).unwrap();
        connection.busy_timeout(Duration::from_secs(30)).unwrap();
        connection.pragma_update(None, "foreign_keys", false).unwrap();
        connection
    }

    fn add_owner(&self) {
        let (server, user) = (Id::random(), Id::random());
        let connection = self.raw();
        connection
            .execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Home', ?2, 1, 0)",
                params![server, Timestamp::now()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, ?1, ?1, 'Owner', 'x', 0, 1, 0, 'c', ?2)",
                params![user, Timestamp::now()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"OwnerSlot\", \"JoinedAt\") VALUES (?1, ?2, ?3, 1, ?4)",
                params![server, user, Id::random(), Timestamp::now()],
            )
            .unwrap();
    }

    fn set_schedule(&self, schedule: Schedule) {
        policy::save_schedule(&self.raw(), self.state.sealer(), &schedule).unwrap();
    }

    fn archives(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.folder.path().join("backups"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "zip"))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.folder.path().join("backups"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

/// Time that moves when the test says.
#[derive(Clone)]
struct Moments(Arc<AtomicI64>);

impl Moments {
    fn new() -> Moments {
        Moments(Arc::new(AtomicI64::new(
            Timestamp::parse("2026-11-24 10:00:00").unwrap().unix_seconds(),
        )))
    }

    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_secs() as i64, Ordering::Relaxed);
    }

    fn clock(&self) -> Clock {
        let seconds = self.0.clone();
        Arc::new(move || Timestamp::from_unix_seconds(seconds.load(Ordering::Relaxed)).unwrap())
    }
}

/// A destination that remembers what it was given, and fails on request.
#[derive(Default)]
struct Recording {
    paths: Mutex<Vec<PathBuf>>,
    fail: AtomicBool,
    off: AtomicBool,
}

impl Recording {
    fn paths(&self) -> Vec<PathBuf> {
        self.paths.lock().unwrap().clone()
    }
}

impl Destination for Recording {
    fn enabled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { !self.off.load(Ordering::Relaxed) })
    }

    fn upload<'a>(&'a self, archive: &'a Path, _server: Id) -> BoxFuture<'a, Result<(), BackupFailed>> {
        Box::pin(async move {
            self.paths.lock().unwrap().push(archive.to_path_buf());
            if self.fail.load(Ordering::Relaxed) {
                Err(BackupFailed::new("The offsite copy failed."))
            } else {
                Ok(())
            }
        })
    }
}

async fn run(instance: &Instance, destination: &Recording, moments: &Moments) -> Outcome {
    run_once(&instance.state, &[destination], &moments.clock()).await
}

fn hours(count: u64) -> Duration {
    Duration::from_secs(count * 3600)
}

fn status(instance: &Instance) -> schedule::Status {
    let schedule = policy::current(&instance.raw(), instance.state.sealer(), &instance.state.settings().backups)
        .unwrap()
        .0;
    schedule::status(&instance.state.backups().checkpoint(), &schedule, false)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_of_schedule_counts_at_the_next_pass_without_a_restart() {
    let instance = Instance::new(&[("STELLARR_BACKUP_ENABLED", "false")]);
    let (moments, destination) = (Moments::new(), Recording::default());
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    assert!(destination.paths().is_empty());

    let on = |interval_hours| Schedule {
        enabled: true,
        interval_hours,
        retry_minutes: 15,
        initial_delay_minutes: 0,
    };
    instance.set_schedule(on(24));
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);
    assert_eq!(destination.paths().len(), 1);
    moments.advance(hours(1));
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    assert_eq!(destination.paths().len(), 1);

    // An interval of one hour: the hour has passed since the last success.
    instance.set_schedule(on(1));
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);
    assert_eq!(destination.paths().len(), 2);
    instance.set_schedule(Schedule { enabled: false, ..on(1) });
    moments.advance(hours(24));
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    assert_eq!(destination.paths().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_backup_is_verified_and_does_not_run_again_before_its_interval() {
    let instance = Instance::new(&[]);
    let (moments, destination) = (Moments::new(), Recording::default());

    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);
    let checkpoint = instance.state.backups().checkpoint();
    assert_eq!(checkpoint.last_success_at, Some(moments.clock()()));
    assert_eq!(checkpoint.last_error, None);
    let path = destination.paths().pop().unwrap();
    assert!(listing::marker_path(&path).exists(), "an automatic backup is marked as verified");
    assert_eq!(super::archive::verify(&path, path.parent().unwrap()), "ok");
    assert_eq!(
        super::archive::read_manifest(&path).unwrap().parts,
        ["database", "objects", "keys", "instance"]
    );
    // Not before the interval, and then a new archive.
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    moments.advance(hours(23));
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    moments.advance(hours(1));
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);
    assert_eq!(destination.paths().len(), 2);
    assert_ne!(destination.paths()[0], destination.paths()[1]);
    let audit: i64 = instance
        .raw()
        .query_row(
            "SELECT COUNT(*) FROM \"ServerAuditEntries\" WHERE \"Summary\" LIKE '% [automatic] created backup %'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_archive_missing_an_object_is_neither_published_nor_uploaded() {
    let instance = Instance::new(&[]);
    // A row names an object the store does not hold.
    instance
        .raw()
        .execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"IsEncrypted\", \"CreatedAt\")
             VALUES (?1, ?1, 'images/gone', 'f', 'image/png', 3, 0, ?2)",
            params![Id::random(), Timestamp::now()],
        )
        .unwrap();
    // One that is there, so that the store itself is not what is broken.
    let store = instance.state.storage_handle();
    store
        .put("images/here", &mut std::io::Cursor::new(vec![1u8, 2, 3]))
        .await
        .unwrap();
    let (moments, destination) = (Moments::new(), Recording::default());

    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Failed);
    let checkpoint = instance.state.backups().checkpoint();
    assert_eq!(checkpoint.last_success_at, None);
    assert!(checkpoint.last_error.is_some());
    assert!(destination.paths().is_empty());
    // No archive, no marker, no partial file.
    assert_eq!(instance.files(), Vec::<String>::new());
    assert_eq!(
        instance.state.backups().last_failure().unwrap().message,
        "Automatic backup contains missing objects."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_copy_to_a_destination_is_tried_again_with_the_same_archive_after_a_restart() {
    let mut instance = Instance::new(&[]);
    let (moments, destination) = (Moments::new(), Recording::default());
    destination.fail.store(true, Ordering::Relaxed);

    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Failed);
    let first = instance.state.backups().checkpoint();
    assert_eq!(first.last_success_at, None);
    assert!(first.pending_backup_id.is_some());
    assert_eq!(
        first.last_error.as_deref(),
        Some("Automatic backup failed; inspect the server logs.")
    );
    // The retry waits for its minutes.
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    assert_eq!(destination.paths().len(), 1);

    destination.fail.store(false, Ordering::Relaxed);
    moments.advance(Duration::from_secs(15 * 60));
    instance.restarted();
    schedule::load(&instance.state).await;
    assert_eq!(
        instance.state.backups().checkpoint().pending_backup_id,
        first.pending_backup_id,
        "the checkpoint survived the restart"
    );
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);

    let done = instance.state.backups().checkpoint();
    assert_eq!(done.pending_backup_id, None);
    assert_eq!(done.last_error, None);
    assert!(done.last_success_at.is_some());
    assert_eq!(destination.paths().len(), 2);
    assert_eq!(destination.paths()[0], destination.paths()[1]);
    assert_eq!(instance.archives().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_overlaps_the_next_makes_one_archive() {
    let instance = Instance::new(&[]);
    let (moments, destination) = (Moments::new(), Recording::default());
    let (first, second) = tokio::join!(
        run(&instance, &destination, &moments),
        run(&instance, &destination, &moments)
    );
    let mut outcomes = [first, second];
    outcomes.sort_by_key(|outcome| matches!(outcome, Outcome::Completed));
    assert_eq!(outcomes, [Outcome::Skipped, Outcome::Completed]);
    assert_eq!(destination.paths().len(), 1);
    assert_eq!(instance.archives().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manual_backup_that_is_running_defers_the_automatic_one() {
    let instance = Instance::new(&[]);
    let (moments, destination) = (Moments::new(), Recording::default());
    let (server, user): (Id, Id) = instance
        .raw()
        .query_row(
            "SELECT \"ServerId\", \"UserId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let manual = instance
        .state
        .backups()
        .try_start(&instance.state, super::Parts::ALL, server, user, false)
        .unwrap();
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Skipped);
    assert!(instance
        .state
        .backups()
        .try_start(&instance.state, super::Parts::ALL, server, user, false)
        .is_none());
    assert!(manual.finished.await.unwrap());
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_destination_that_is_off_is_not_asked_and_an_instance_without_an_owner_waits() {
    let instance = Instance::new(&[]);
    let (moments, destination) = (Moments::new(), Recording::default());
    destination.off.store(true, Ordering::Relaxed);
    assert_eq!(run(&instance, &destination, &moments).await, Outcome::Completed);
    assert!(destination.paths().is_empty());
    assert_eq!(instance.archives().len(), 1);

    // No account yet: setup has not made an instance to back up.
    let empty = Instance::new(&[]);
    empty.raw().execute("DELETE FROM \"ServerMembers\"", []).unwrap();
    assert_eq!(run(&empty, &Recording::default(), &moments).await, Outcome::Skipped);
    assert!(empty.archives().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_attempt_is_where_the_first_server_put_it() {
    let schedule = Schedule {
        enabled: true,
        interval_hours: 24,
        retry_minutes: 15,
        initial_delay_minutes: 5,
    };
    let at = |text| Timestamp::parse(text).unwrap();
    let nothing = Checkpoint::default();
    assert_eq!(nothing.next_attempt(&schedule), None);
    let succeeded = Checkpoint {
        last_success_at: Some(at("2026-11-24 10:00:00")),
        ..Checkpoint::default()
    };
    assert_eq!(succeeded.next_attempt(&schedule), Some(at("2026-11-25 10:00:00")));
    let failed = Checkpoint {
        last_success_at: Some(at("2026-11-24 10:00:00")),
        last_error: Some("x".into()),
        last_failure_at: Some(at("2026-11-24 12:00:00")),
        ..Checkpoint::default()
    };
    assert_eq!(failed.next_attempt(&schedule), Some(at("2026-11-24 12:15:00")));
    let recorded = Checkpoint {
        next_attempt_at: Some(at("2026-11-30 00:00:00")),
        ..Checkpoint::default()
    };
    assert_eq!(recorded.next_attempt(&schedule), Some(at("2026-11-30 00:00:00")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_the_first_server_wrote_is_read_and_one_that_is_broken_starts_over() {
    let mut instance = Instance::new(&[]);
    let path = instance.folder.path().join("automatic-backup.json");
    std::fs::write(
        &path,
        r#"{"LastSuccessAt":"2026-11-24T10:00:00.1234567Z","NextAttemptAt":"2026-11-25T10:00:00Z","PendingBackupId":null,"ServerId":"5f0a2a3c-0000-4000-8000-000000000001","LastError":null,"LastFailureAt":null}"#,
    )
    .unwrap();
    instance.restarted();
    schedule::load(&instance.state).await;
    let read = instance.state.backups().checkpoint();
    assert_eq!(read.last_success_at, Timestamp::parse("2026-11-24T10:00:00.1234567Z"));
    assert!(read.pending_backup_id.is_none());

    std::fs::write(&path, b"{ not json").unwrap();
    instance.restarted();
    schedule::load(&instance.state).await;
    let broken = instance.state.backups().checkpoint();
    assert!(broken.last_error.as_deref().unwrap().contains("unreadable"));
    assert_eq!(broken.last_success_at, None);
    assert_eq!(status(&instance).last_success_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_offsite_destination_that_is_on_without_its_credentials_fails_the_cycle() {
    let instance = Instance::new(&[]);
    let destination = policy::Destination {
        enabled: true,
        mode: "s3".into(),
        endpoint: "https://bucket.invalid".into(),
        region: "auto".into(),
        bucket: "backups".into(),
        prefix: "stellarr".into(),
    };
    policy::save_destination(&instance.raw(), instance.state.sealer(), &destination).unwrap();
    let offsite = super::offsite::Cloudflare::new(instance.state.clone());
    assert!(offsite.enabled().await);
    let archive = instance.folder.path().join("backup-1.zip");
    std::fs::write(&archive, b"x").unwrap();
    let failure = offsite.upload(&archive, Id::random()).await.unwrap_err();
    assert_eq!(
        failure.message(),
        "Backup destination credentials or settings are missing."
    );

    // The schedule counts that as the cycle failing, with the archive kept
    // for the next try, and says nothing of the credentials.
    let moments = Moments::new();
    let outcome = run_once(&instance.state, &[&offsite as &dyn Destination], &moments.clock()).await;
    assert_eq!(outcome, Outcome::Failed);
    let checkpoint = instance.state.backups().checkpoint();
    assert!(checkpoint.pending_backup_id.is_some());
    assert_eq!(
        checkpoint.last_error.as_deref(),
        Some("Automatic backup failed; inspect the server logs.")
    );

    policy::save_destination(
        &instance.raw(),
        instance.state.sealer(),
        &policy::Destination { enabled: false, ..destination },
    )
    .unwrap();
    assert!(!offsite.enabled().await);
    assert!(offsite.upload(&archive, Id::random()).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backup_without_room_or_without_a_figure_for_it_fails_and_leaves_nothing() {
    use super::space::fake;
    let instance = Instance::new(&[]);
    let (server, user): (Id, Id) = instance
        .raw()
        .query_row(
            "SELECT \"ServerId\", \"UserId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    for (told, words) in [(Some(1000), "MB"), (None, "could not be read")] {
        fake::tell(instance.folder.path(), told);
        let started = instance
            .state
            .backups()
            .try_start(&instance.state, super::Parts::ALL, server, user, false)
            .unwrap();
        assert!(!started.finished.await.unwrap());
        let failure = instance.state.backups().last_failure().unwrap();
        assert!(failure.message.contains(words), "{told:?}: {}", failure.message);
        assert_eq!(instance.files(), Vec::<String>::new());
    }
    fake::tell(instance.folder.path(), Some(u64::MAX / 2));
    let started = instance
        .state
        .backups()
        .try_start(&instance.state, super::Parts::ALL, server, user, false)
        .unwrap();
    assert!(started.finished.await.unwrap());
}
