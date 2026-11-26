//! What a run prunes and what it leaves, at the edge of each retention.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::{Connection, params};
use stellarr_storage::{
    BoxFuture, ByteRange, Error, FolderStore, Listing, ObjectStorage, Result as StoreResult,
    StoredObject,
};
use tokio::io::AsyncRead;

use super::*;
use crate::database::Database;
use crate::events::Unconnected;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::testing::TempDir;
use crate::values::Id;

const NEVER: Stopping<'static> = &|| false;

fn now() -> Timestamp {
    Timestamp::parse("2026-11-24 12:00:00").unwrap()
}

fn ago(seconds: u64) -> Timestamp {
    now().minus(Duration::from_secs(seconds))
}

const DAY: u64 = 86_400;

/// A store that fails on request, over a folder of its own.
struct Flaky {
    inner: FolderStore,
    failing: AtomicBool,
}

impl ObjectStorage for Flaky {
    fn prepare(&self) -> BoxFuture<'_, StoreResult<()>> {
        self.inner.prepare()
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        data: &'a mut (dyn AsyncRead + Send + Unpin),
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.inner.put(key, data)
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<ByteRange>,
    ) -> BoxFuture<'a, StoreResult<StoredObject>> {
        self.inner.get(key, range)
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, StoreResult<()>> {
        if self.failing.load(Ordering::Relaxed) {
            return Box::pin(async { Err(Error::Io(std::io::Error::other("the store is down"))) });
        }
        self.inner.delete(key)
    }

    fn list(&self) -> Listing {
        self.inner.list()
    }

    fn probe(&self) -> BoxFuture<'_, StoreResult<()>> {
        self.inner.probe()
    }
}

struct Place {
    folder: TempDir,
    state: AppState,
    store: Arc<Flaky>,
}

impl Place {
    fn new(extra: &[(&str, &str)]) -> Place {
        let folder = TempDir::new();
        let mut variables: Vec<(String, String)> = vec![
            ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
            (
                "STELLARR_DATA_ROOT".into(),
                folder.path().display().to_string(),
            ),
        ];
        for (name, value) in extra {
            variables.retain(|(have, _)| have != name);
            variables.push((name.to_string(), value.to_string()));
        }
        let settings = Settings::load(&variables).unwrap();
        std::fs::create_dir_all(settings.database.path.parent().unwrap()).unwrap();
        let database = Database::open(&settings.database).unwrap();
        let store = Arc::new(Flaky {
            inner: FolderStore::new(folder.path().join("files")),
            failing: AtomicBool::new(false),
        });
        let state = AppState::new(
            settings,
            database,
            Arc::new(Unconnected),
            Arc::new(Unconnected),
            Sealer::from_key([5; 32]),
            Sealer::from_key([6; 32]),
        )
        .with_storage(store.clone());
        Place {
            folder,
            state,
            store,
        }
    }

    /// A connection of the test's own, with the foreign keys off so that a row
    /// can be written without the ones it would refer to.
    fn raw(&self) -> Connection {
        let connection =
            Connection::open(self.folder.path().join("db").join("stellarr.db")).unwrap();
        connection.busy_timeout(Duration::from_secs(30)).unwrap();
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        connection
    }

    fn count(&self, table: &str) -> i64 {
        self.raw()
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    async fn run(&self) -> Summary {
        run(&self.state, now(), NEVER).await.unwrap()
    }
}

#[tokio::test]
async fn the_journal_keeps_what_is_inside_its_days_and_leaves_one_reset_per_channel() {
    let place = Place::new(&[]);
    let (busy, quiet) = (Id::random(), Id::random());
    let connection = place.raw();
    let cutoff = now().minus(Duration::from_secs(30 * DAY));
    for (channel, at) in [
        (busy, cutoff.minus(Duration::from_secs(1))),
        (busy, cutoff.minus(Duration::from_secs(100))),
        (busy, cutoff.plus(Duration::from_secs(1))),
        (quiet, cutoff.plus(Duration::from_secs(5))),
    ] {
        connection
            .execute(
                "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\") VALUES (?1, ?2, 0, ?3)",
                params![channel, Id::random(), at],
            )
            .unwrap();
    }

    place.run().await;
    let rows: Vec<(Id, i64, Timestamp)> = connection
        .prepare("SELECT \"ChannelId\", \"Kind\", \"CreatedAt\" FROM \"MessageChanges\" ORDER BY \"Sequence\"")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    // The two that were inside stay; one reset takes the place of the two
    // that were not, dated by the run.
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows.iter().filter(|row| row.1 == 2).count(), 1);
    let reset = rows.iter().find(|row| row.1 == 2).unwrap();
    assert_eq!((reset.0, reset.2), (busy, now()));
    assert!(rows.iter().all(|row| row.2 >= cutoff));

    // A second run finds nothing older and writes nothing.
    place.run().await;
    assert_eq!(place.count("MessageChanges"), 3);
}

#[tokio::test]
async fn the_journal_days_are_kept_between_seven_and_a_year() {
    for (setting, kept_days_ago, removed_days_ago) in [("1", 6, 8), ("5000", 364, 366)] {
        let place = Place::new(&[("Maintenance__MessageChangeRetentionDays", setting)]);
        let connection = place.raw();
        for days in [kept_days_ago, removed_days_ago] {
            connection
                .execute(
                    "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\") VALUES (?1, ?2, 0, ?3)",
                    params![Id::random(), Id::random(), now().minus(Duration::from_secs(days * DAY))],
                )
                .unwrap();
        }
        place.run().await;
        let kinds: Vec<i64> = connection
            .prepare("SELECT \"Kind\" FROM \"MessageChanges\" ORDER BY \"Sequence\"")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(kinds, [0, 2], "{setting}");
    }
}

/// The columns each kind of short-lived record needs, with the moment that
/// decides whether it is expired.
fn insert_expired_kinds(
    connection: &Connection,
    id: Id,
    at: Timestamp,
    used_at: Option<Timestamp>,
) {
    let user = Id::random();
    connection
        .execute(
            "INSERT INTO \"AuthChallenges\" (\"Id\", \"UserId\", \"Purpose\", \"SecretHash\", \"PrimarySource\", \"SecurityVersion\", \"ShortLived\", \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, 'login', ?1, 'password', 1, 0, 0, 5, ?3, ?3)",
            params![id, user, at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"EmailOtpCodes\" (\"Id\", \"UserId\", \"Purpose\", \"EmailAddress\", \"ProtectedCode\", \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, 'login', 'a@b.c', 'p', 0, 5, ?3, ?3)",
            params![id, user, at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"CliRecoveryCredentials\" (\"Id\", \"UserId\", \"TokenHash\", \"Reason\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, ?1, 'r', 1, ?3, ?3)",
            params![id, user, at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"PendingExternalIdentities\" (\"Id\", \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"EmailVerified\", \"Amr\", \"ShortLived\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?1, 'p', 'i', 's', 0, '', 0, ?2, ?2)",
            params![id, at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\")
             VALUES (?1, ?2, ?1, 'n', 'd', '1.1.1.1', ?3, ?3)",
            params![id, user, at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"RecoveryCodes\" (\"Id\", \"UserId\", \"BatchId\", \"CodeHash\", \"CreatedAt\", \"UsedAt\") VALUES (?1, ?2, ?2, ?1, ?3, ?4)",
            params![id, user, at, used_at],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"SecurityNotifications\" (\"Id\", \"SecurityEventId\", \"UserId\", \"EmailAddress\", \"Attempts\", \"CreatedAt\", \"NextAttemptAt\", \"SentAt\", \"LastError\")
             VALUES (?1, ?2, ?2, 'a@b.c', 0, ?3, ?3, ?4, '')",
            params![id, user, at, used_at],
        )
        .unwrap();
}

const SHORT_LIVED: [&str; 4] = [
    "AuthChallenges",
    "EmailOtpCodes",
    "CliRecoveryCredentials",
    "PendingExternalIdentities",
];
#[tokio::test]
async fn short_lived_records_go_a_day_after_they_expire_and_the_rest_thirty() {
    let place = Place::new(&[]);
    let connection = place.raw();
    // A day: just past it goes, just inside stays. Thirty days for the rest.
    let (past_day, inside_day) = (Id::random(), Id::random());
    insert_expired_kinds(&connection, past_day, ago(DAY + 1), Some(ago(DAY + 1)));
    insert_expired_kinds(&connection, inside_day, ago(DAY - 1), Some(ago(DAY - 1)));
    let (past_month, inside_month) = (Id::random(), Id::random());
    insert_expired_kinds(
        &connection,
        past_month,
        ago(30 * DAY + 1),
        Some(ago(30 * DAY + 1)),
    );
    insert_expired_kinds(
        &connection,
        inside_month,
        ago(30 * DAY - 1),
        Some(ago(30 * DAY - 1)),
    );
    // A code never used and a notification never sent are not expired by age.
    let never = Id::random();
    insert_expired_kinds(&connection, never, ago(400 * DAY), None);

    place.run().await;
    let left = |table: &str| -> Vec<Id> {
        let mut ids: Vec<Id> = connection
            .prepare(&format!("SELECT \"Id\" FROM \"{table}\""))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        ids.sort();
        ids
    };
    let sorted = |mut ids: Vec<Id>| {
        ids.sort();
        ids
    };
    // A challenge or a code is past use a day after it expires.
    for table in SHORT_LIVED {
        assert_eq!(left(table), [inside_day], "{table}");
    }
    // A trusted device is kept thirty days after it expires; a recovery code
    // and a notification thirty days after they were used or sent, and never
    // when they were not.
    assert_eq!(
        left("TrustedDevices"),
        sorted(vec![past_day, inside_day, inside_month])
    );
    for table in ["RecoveryCodes", "SecurityNotifications"] {
        assert_eq!(
            left(table),
            sorted(vec![past_day, inside_day, inside_month, never]),
            "{table}"
        );
    }
}

fn add_user(connection: &Connection) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
             VALUES (?1, ?1, ?1, 'Person', 'hash', 0, 1, 0, 'c', ?2)",
            params![id, now()],
        )
        .unwrap();
    id
}

fn add_server(connection: &Connection) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Home', ?2, 1, 0)",
            params![id, now()],
        )
        .unwrap();
    id
}

fn add_attachment(connection: &Connection, id: Id, created: Timestamp, key: &str) {
    connection
        .execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"PosterObjectKey\", \"IsEncrypted\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, 'f', 'image/png', 1, ?4, 0, ?5)",
            params![id, Id::random(), key, format!("{key}.poster"), created],
        )
        .unwrap();
}

fn queued(connection: &Connection) -> Vec<String> {
    let mut keys: Vec<String> = connection
        .prepare("SELECT \"ObjectKey\" FROM \"ObjectDeletions\"")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    keys.sort();
    keys
}

fn attachments(connection: &Connection) -> Vec<Id> {
    let mut ids: Vec<Id> = connection
        .prepare("SELECT \"Id\" FROM \"ImageAttachments\"")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn an_upload_nobody_uses_goes_after_its_day_and_takes_its_objects_with_it() {
    let place = Place::new(&[]);
    let connection = place.raw();
    let (old, young) = (Id::random(), Id::random());
    add_attachment(&connection, old, ago(DAY + 1), "images/old");
    add_attachment(&connection, young, ago(DAY - 1), "images/young");

    let summary = place.run().await;
    assert_eq!(summary.orphan_attachments, 1);
    assert_eq!(attachments(&connection), [young]);
    // Its objects were queued and then removed from the store in the same run.
    assert!(queued(&connection).is_empty());
}

#[tokio::test]
async fn an_upload_that_something_refers_to_is_kept_whatever_its_age() {
    let place = Place::new(&[]);
    let connection = place.raw();
    let long_ago = ago(100 * DAY);
    let message = Id::random();
    let kept: Vec<Id> = (0..8).map(|_| Id::random()).collect();
    for (at, id) in kept.iter().enumerate() {
        add_attachment(&connection, *id, long_ago, &format!("images/kept-{at}"));
    }
    connection
        .execute(
            "INSERT INTO \"MessageAttachments\" (\"MessageId\", \"AttachmentId\") VALUES (?1, ?2)",
            params![message, kept[0]],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"DirectMessageAttachments\" (\"DirectMessageId\", \"AttachmentId\") VALUES (?1, ?2)",
            params![message, kept[1]],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"MessageEmbeds\" (\"Id\", \"MessageId\", \"Url\", \"Kind\", \"Provider\", \"Domain\", \"ThumbnailAttachmentId\", \"Position\", \"CreatedAt\")
             VALUES (?1, ?2, 'u', 0, 'p', 'd', ?3, 0, ?4)",
            params![Id::random(), message, kept[2], long_ago],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"SoundboardSounds\" (\"Id\", \"ServerId\", \"AttachmentId\", \"CreatedById\", \"Name\", \"Glyph\", \"TrimStartSeconds\", \"TrimEndSeconds\", \"Position\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, ?2, 'n', 'g', 0, 1, 0, ?4)",
            params![Id::random(), Id::random(), kept[3], long_ago],
        )
        .unwrap();
    // A picture of a person (any of three), and the icon of a server, are
    // named by address.
    for (column, at) in [("AvatarUrl", 4), ("BannerUrl", 5), ("WallpaperUrl", 7)] {
        let user = add_user(&connection);
        connection
            .execute(
                &format!("UPDATE \"Users\" SET \"{column}\" = ?1 WHERE \"Id\" = ?2"),
                params![format!("/assets/{}", kept[at]), user],
            )
            .unwrap();
    }
    let server = add_server(&connection);
    connection
        .execute(
            "UPDATE \"Servers\" SET \"IconUrl\" = ?1 WHERE \"Id\" = ?2",
            params![format!("/assets/{}", kept[6]), server],
        )
        .unwrap();
    let orphan = Id::random();
    add_attachment(&connection, orphan, long_ago, "images/orphan");

    let summary = place.run().await;
    assert_eq!(summary.orphan_attachments, 1);
    let mut expected = kept.clone();
    expected.sort();
    assert_eq!(attachments(&connection), expected);
}

#[tokio::test]
async fn pictures_in_use_do_not_fill_the_batch_and_hide_the_orphans() {
    for kind in ["AvatarUrl", "BannerUrl", "WallpaperUrl", "IconUrl"] {
        pictures_of_one_kind_do_not_fill_the_batch(kind).await;
    }
}

async fn pictures_of_one_kind_do_not_fill_the_batch(kind: &str) {
    let place = Place::new(&[("Maintenance__BatchSize", "3")]);
    let connection = place.raw();
    let mut old = Vec::new();
    // Ten old pictures that are in use, then the orphans that came later.
    for at in 0..10 {
        let id = Id::random();
        add_attachment(
            &connection,
            id,
            ago(200 * DAY - at),
            &format!("images/avatar-{at}"),
        );
        let (table, owner) = if kind == "IconUrl" {
            ("Servers", add_server(&connection))
        } else {
            ("Users", add_user(&connection))
        };
        connection
            .execute(
                &format!("UPDATE \"{table}\" SET \"{kind}\" = ?1 WHERE \"Id\" = ?2"),
                params![format!("/assets/{id}"), owner],
            )
            .unwrap();
        old.push(id);
    }
    for at in 0..5 {
        add_attachment(
            &connection,
            Id::random(),
            ago(10 * DAY - at),
            &format!("images/orphan-{at}"),
        );
    }

    // Three orphans go in a run, the oldest first, and the pictures stay.
    assert_eq!(place.run().await.orphan_attachments, 3, "{kind}");
    assert_eq!(attachments(&connection).len(), 12, "{kind}");
    assert_eq!(place.run().await.orphan_attachments, 2, "{kind}");
    assert_eq!(place.run().await.orphan_attachments, 0, "{kind}");
    assert_eq!(attachments(&connection).len(), 10, "{kind}");
}

#[tokio::test]
async fn an_upload_that_became_referenced_during_the_sweep_is_kept() {
    let place = Place::new(&[]);
    let connection = place.raw();
    let (message_id, direct_id, free) = (Id::random(), Id::random(), Id::random());
    let (in_message, in_direct) = (Id::random(), Id::random());
    for (id, key) in [(in_message, "a"), (in_direct, "b"), (free, "c")] {
        add_attachment(&connection, id, ago(5 * DAY), &format!("images/{key}"));
    }
    // They were candidates a moment ago; now a message and a direct message
    // take them, as a request that landed between the choice and the removal
    // would. The database's own refusal is what holds the direct one.
    connection
        .execute(
            "INSERT INTO \"MessageAttachments\" (\"MessageId\", \"AttachmentId\") VALUES (?1, ?2)",
            params![message_id, in_message],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"DirectMessageAttachments\" (\"DirectMessageId\", \"AttachmentId\") VALUES (?1, ?2)",
            params![direct_id, in_direct],
        )
        .unwrap();

    for id in [in_message, in_direct] {
        assert!(
            !crate::uploads::retire_if_unreferenced(
                &place.state,
                Some(format!("/assets/{id}")),
                None
            )
            .await
        );
    }
    assert!(
        crate::uploads::retire_if_unreferenced(&place.state, Some(format!("/assets/{free}")), None)
            .await
    );
    let mut kept = vec![in_message, in_direct];
    kept.sort();
    assert_eq!(attachments(&connection), kept);
    // What was refused queued nothing for deletion.
    assert_eq!(
        queued(&connection),
        ["c", "c.poster"].map(|k| format!("images/{k}"))
    );
}

#[tokio::test]
async fn a_store_that_fails_keeps_what_is_queued_and_the_next_run_empties_it() {
    let place = Place::new(&[]);
    let connection = place.raw();
    for (key, minutes) in [("a", 5), ("b", 4), ("c", 3)] {
        connection
            .execute(
                "INSERT INTO \"ObjectDeletions\" (\"ObjectKey\", \"QueuedAt\") VALUES (?1, ?2)",
                params![key, ago(minutes * 60)],
            )
            .unwrap();
        place
            .store
            .put(key, &mut std::io::Cursor::new(b"x".to_vec()))
            .await
            .unwrap();
    }

    place.store.failing.store(true, Ordering::Relaxed);
    place.run().await;
    assert_eq!(queued(&connection), ["a", "b", "c"]);

    place.store.failing.store(false, Ordering::Relaxed);
    place.run().await;
    assert!(queued(&connection).is_empty());
    for key in ["a", "b", "c"] {
        assert!(
            place.store.get(key, None).await.is_err(),
            "{key} is still in the store"
        );
    }
}

#[tokio::test]
async fn one_run_removes_no_more_queued_objects_than_the_batch_and_the_oldest_first() {
    let place = Place::new(&[("Maintenance__BatchSize", "2")]);
    let connection = place.raw();
    for (key, minutes) in [("new", 1), ("oldest", 9), ("old", 8)] {
        connection
            .execute(
                "INSERT INTO \"ObjectDeletions\" (\"ObjectKey\", \"QueuedAt\") VALUES (?1, ?2)",
                params![key, ago(minutes * 60)],
            )
            .unwrap();
    }
    place.run().await;
    assert_eq!(queued(&connection), ["new"]);
}

#[tokio::test]
async fn with_cleanup_off_the_queue_is_still_emptied_and_nothing_else_is_pruned() {
    let place = Place::new(&[]);
    let connection = place.raw();
    let server = add_server(&connection);
    {
        let tx = connection.unchecked_transaction().unwrap();
        operations::ensure(&tx, place.state.settings(), server).unwrap();
        tx.commit().unwrap();
    }
    connection
        .execute(
            "UPDATE \"OperationsSettings\" SET \"AutomaticCleanupEnabled\" = 0",
            [],
        )
        .unwrap();
    add_attachment(&connection, Id::random(), ago(50 * DAY), "images/old");
    connection
        .execute(
            "INSERT INTO \"ObjectDeletions\" (\"ObjectKey\", \"QueuedAt\") VALUES ('q', ?1)",
            [ago(60)],
        )
        .unwrap();
    insert_expired_kinds(
        &connection,
        Id::random(),
        ago(400 * DAY),
        Some(ago(400 * DAY)),
    );
    std::fs::create_dir_all(place.folder.path().join("backups")).unwrap();
    let old_backup = place.folder.path().join("backups/backup-old.zip");
    std::fs::write(&old_backup, b"x").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old_backup)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(400 * DAY))
        .unwrap();

    let summary = run(&place.state, Timestamp::now(), NEVER).await.unwrap();
    assert!(!summary.enabled);
    assert_eq!(
        (
            summary.orphan_attachments,
            summary.backup_files,
            summary.log_files
        ),
        (0, 0, 0)
    );
    assert_eq!(place.count("ObjectDeletions"), 0);
    assert_eq!(place.count("ImageAttachments"), 1);
    assert_eq!(place.count("AuthChallenges"), 1);
    assert!(old_backup.exists());
}

#[tokio::test]
async fn the_housekeeping_goes_by_the_settings_written_last() {
    let place = Place::new(&[]);
    let connection = place.raw();
    for (retention, updated) in [(1, ago(2 * DAY)), (400, ago(DAY))] {
        let server = add_server(&connection);
        {
            let tx = connection.unchecked_transaction().unwrap();
            operations::ensure(&tx, place.state.settings(), server).unwrap();
            tx.commit().unwrap();
        }
        connection
            .execute(
                "UPDATE \"OperationsSettings\" SET \"OrphanObjectRetentionDays\" = ?1, \"UpdatedAt\" = ?2 WHERE \"ServerId\" = ?3",
                params![retention, updated, server],
            )
            .unwrap();
    }
    add_attachment(&connection, Id::random(), ago(30 * DAY), "images/a");
    // The server written last says 400 days, so a month is young.
    assert_eq!(place.run().await.orphan_attachments, 0);
}

fn age_file(path: &std::path::Path, seconds: u64) {
    std::fs::write(path, b"x").unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(seconds))
        .unwrap();
}

#[tokio::test]
async fn backups_and_logs_are_pruned_by_the_days_of_their_own_retention() {
    let place = Place::new(&[]);
    let root = place.folder.path();
    std::fs::create_dir_all(root.join("backups/sub")).unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    // Backups: fourteen days, logs thirty, by default.
    age_file(&root.join("backups/backup-old.zip"), 14 * DAY + 60);
    age_file(&root.join("backups/backup-old.zip.verified"), 14 * DAY + 60);
    age_file(&root.join("backups/backup-young.zip"), 14 * DAY - 60);
    age_file(&root.join("logs/old.log"), 30 * DAY + 60);
    age_file(&root.join("logs/young.log"), 30 * DAY - 60);
    age_file(&root.join("backups/sub/inside"), 100 * DAY);

    let summary = run(&place.state, Timestamp::now(), NEVER).await.unwrap();
    assert_eq!((summary.backup_files, summary.log_files), (2, 1));
    assert!(root.join("backups/backup-young.zip").exists());
    assert!(root.join("logs/young.log").exists());
    assert!(
        root.join("backups/sub/inside").exists(),
        "only the files of the folder itself"
    );
    assert!(!root.join("backups/backup-old.zip").exists());
}

/// How long a run holds the one writer, on a database with hundreds of
/// thousands of rows to remove.
///
/// The first measure is the run alone: the longest any one transaction of it
/// took, as seen from outside the call, which is how long it kept anybody else
/// from writing. The second runs the same work while a task writes one small
/// row at a time through the same handle, and reports the longest it had to
/// wait. The second number includes what the machine was doing besides.
///
///   cargo test -p stellarr-api --lib housekeeping::tests::cost -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "fills a database of about a hundred megabytes and takes minutes"]
async fn cost_of_a_run_on_a_database_with_hundreds_of_thousands_of_rows() {
    let alone = cost_run(false).await;
    println!("COST alone: {alone}");
    let beside = cost_run(true).await;
    println!("COST beside a writer: {beside}");
}

async fn cost_run(with_writer: bool) -> String {
    let place = Place::new(&[]);
    let connection = place.raw();
    let old = ago(40 * DAY);
    let young = ago(60);
    let fill = |sql: &str| {
        if sql.contains("?2") {
            connection.execute(sql, params![old, young]).unwrap();
        } else {
            connection.execute(sql, params![old]).unwrap();
        }
    };
    let numbers = |n: u32, body: &str| {
        format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {n}) {body}"
        )
    };
    fill(&numbers(300_000, "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\")
         SELECT printf('00000000-0000-4000-8000-%012d', i % 100), printf('00000000-0000-4000-8000-%012d', i), 0,
                CASE WHEN i % 2 = 0 THEN ?1 ELSE ?2 END FROM n"));
    let uid = "printf('00000000-0000-4000-8000-%012d', i)";
    for (table, columns, values) in [
        (
            "AuthChallenges",
            "\"Id\", \"UserId\", \"Purpose\", \"SecretHash\", \"PrimarySource\", \"SecurityVersion\", \"ShortLived\", \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\"",
            "'u', 'login', 'h' || i, 'password', 1, 0, 0, 5, ?1, ?1",
        ),
        (
            "EmailOtpCodes",
            "\"Id\", \"UserId\", \"Purpose\", \"EmailAddress\", \"ProtectedCode\", \"Attempts\", \"MaxAttempts\", \"CreatedAt\", \"ExpiresAt\"",
            "'u', 'login', 'a@b.c', 'p', 0, 5, ?1, ?1",
        ),
        (
            "CliRecoveryCredentials",
            "\"Id\", \"UserId\", \"TokenHash\", \"Reason\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\"",
            "'u', 'h' || i, 'r', 1, ?1, ?1",
        ),
        (
            "PendingExternalIdentities",
            "\"Id\", \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"EmailVerified\", \"Amr\", \"ShortLived\", \"CreatedAt\", \"ExpiresAt\"",
            "'h', 'p', 'i', 's', 0, '', 0, ?1, ?1",
        ),
        (
            "TrustedDevices",
            "\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\"",
            "'u', 'h' || i, 'n', 'd', '1.1.1.1', ?1, ?1",
        ),
        (
            "RecoveryCodes",
            "\"Id\", \"UserId\", \"BatchId\", \"CodeHash\", \"CreatedAt\", \"UsedAt\"",
            "'u', 'b', 'h' || i, ?1, ?1",
        ),
        (
            "SecurityNotifications",
            "\"Id\", \"SecurityEventId\", \"UserId\", \"EmailAddress\", \"Attempts\", \"CreatedAt\", \"NextAttemptAt\", \"SentAt\", \"LastError\"",
            "'e', 'u', 'a@b.c', 0, ?1, ?1, ?1, ''",
        ),
    ] {
        fill(&numbers(
            50_000,
            &format!("INSERT INTO \"{table}\" ({columns}) SELECT {uid}, {values} FROM n"),
        ));
    }
    fill(&numbers(2_000, "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"PosterObjectKey\", \"IsEncrypted\", \"CreatedAt\")
         SELECT printf('10000000-0000-4000-8000-%012d', i), 'u', 'images/' || i, 'f', 'image/png', 1, 'images/' || i || '.p', 0, ?1 FROM n"));
    fill(&numbers(
        5_000,
        "INSERT INTO \"ObjectDeletions\" (\"ObjectKey\", \"QueuedAt\") SELECT 'queued/' || i, ?1 FROM n",
    ));
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    drop(connection);

    let state = place.state.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let writer = with_writer.then(|| {
        let (state, stop) = (state.clone(), stop.clone());
        tokio::spawn(async move {
            let mut waits = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let began = std::time::Instant::now();
                state
                    .database()
                    .write(|tx| -> Result<(), crate::database::DbError> {
                        tx.execute("INSERT OR REPLACE INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('probe', 'x', '2026-11-24 00:00:00')", [])?;
                        Ok(())
                    })
                    .await
                    .unwrap();
                waits.push(began.elapsed());
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            waits.sort();
            waits
        })
    });
    super::LONGEST_WRITE_MICROS.store(0, Ordering::Relaxed);
    let began = std::time::Instant::now();
    let mut runs = 0;
    loop {
        super::journal::compact(&state, now(), 30, NEVER)
            .await
            .unwrap();
        let removed = super::orphans::sweep(&state, super::cutoff(now(), 1), 1000, NEVER)
            .await
            .unwrap();
        super::queue::drain(&state, 1000, NEVER).await;
        super::expired::delete(&state, Timestamp::now(), NEVER)
            .await
            .unwrap();
        runs += 1;
        if (removed == 0 && super::queue::pending(&state).await.unwrap() == 0) || runs > 20 {
            break;
        }
    }
    let total = began.elapsed();
    let longest = Duration::from_micros(super::LONGEST_WRITE_MICROS.load(Ordering::Relaxed));
    stop.store(true, Ordering::Relaxed);
    let probe = match writer {
        Some(writer) => {
            let waits = writer.await.unwrap();
            format!(
                "; {} small writes beside it waited at most {:?} (median {:?}, 99th percentile {:?})",
                waits.len(),
                waits.last().copied().unwrap_or_default(),
                waits[waits.len() / 2],
                waits[waits.len() * 99 / 100],
            )
        }
        None => String::new(),
    };
    assert_eq!(place.count("MessageChanges") as usize, 150_000 + 50);
    assert_eq!(place.count("ObjectDeletions"), 0);
    format!(
        "{runs} runs in {total:?}; the longest transaction of the run took {longest:?} (chunk of {} rows){probe}",
        super::journal::CHUNK
    )
}

#[tokio::test]
async fn an_object_that_could_not_be_removed_after_a_failed_upload_waits_for_the_store() {
    let place = Place::new(&[]);
    let connection = place.raw();
    place
        .store
        .put("images/half", &mut std::io::Cursor::new(b"x".to_vec()))
        .await
        .unwrap();
    place.store.failing.store(true, Ordering::Relaxed);

    // The upload's own rollback cannot reach the store, so it queues the key.
    crate::uploads::roll_back(&place.state, vec!["images/half".to_string()]).await;
    assert_eq!(queued(&connection), ["images/half"]);
    place.run().await;
    assert_eq!(queued(&connection), ["images/half"]);

    place.store.failing.store(false, Ordering::Relaxed);
    place.run().await;
    assert!(queued(&connection).is_empty());
    assert!(place.store.get("images/half", None).await.is_err());
}

#[test]
fn a_setting_below_its_minimum_is_raised_to_it() {
    let settings = |first, every| crate::settings::MaintenanceSettings {
        enabled: true,
        initial_delay_minutes: first,
        interval_hours: every,
        batch_size: 1000,
        message_change_retention_days: 30,
    };
    assert_eq!(
        intervals(&settings(-5, 0)),
        (Duration::ZERO, Duration::from_secs(3600))
    );
    assert_eq!(
        intervals(&settings(5, 3)),
        (Duration::from_secs(300), Duration::from_secs(3 * 3600))
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_comes_after_the_first_wait_and_then_at_every_interval_and_a_failed_one_keeps_the_schedule()
 {
    let place = Place::new(&[]);
    let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = runs.clone();
    let worker = crate::background::Worker::new("test-schedule", move |_state, stop| {
        let counter = counter.clone();
        async move {
            on_schedule(
                Duration::from_secs(300),
                Duration::from_secs(3600),
                &stop,
                || {
                    let counter = counter.clone();
                    async move {
                        // The second pass fails; the third is still due.
                        match counter.fetch_add(1, Ordering::SeqCst) {
                            1 => Err("the database was busy".to_string()),
                            _ => Ok(()),
                        }
                    }
                },
            )
            .await;
        }
    });
    let running = crate::background::start(vec![worker], &place.state);
    let settle = || async {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    };
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 0, "nothing runs at start");
    tokio::time::advance(Duration::from_secs(299)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(3599)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    tokio::time::advance(Duration::from_secs(3600)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 3);
    // Stopping ends the wait at once.
    running.stop(Duration::from_secs(5)).await;
    tokio::time::advance(Duration::from_secs(7200)).await;
    settle().await;
    assert_eq!(runs.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn the_delivery_rows_are_the_dispatchers_and_a_run_never_touches_them() {
    // The dispatcher prunes finished delivery rows after thirty days, and this
    // server's does the same. A leased row is being sent, and a run that took
    // it would take it from under the sender.
    let place = Place::new(&[]);
    let connection = place.raw();
    let long_ago = ago(400 * DAY);
    for (status, lease) in [
        ("sent", None),
        ("pending", Some(now().plus(Duration::from_secs(500)))),
        ("pending", None),
    ] {
        connection
            .execute(
                "INSERT INTO \"NotificationDeliveries\" (\"Id\", \"SourceMessageId\", \"RecipientId\", \"EventKind\", \"Channel\", \"DestinationKey\", \"CreatedAt\", \"ExpiresAt\", \"Attempts\", \"NextAttemptAt\", \"Status\", \"LeaseId\", \"LeaseUntil\")
                 VALUES (?1, ?1, ?1, 'mention', 'push', ?1, ?2, ?2, 1, ?2, ?3, ?4, ?5)",
                params![Id::random(), long_ago, status, lease.map(|_| Id::random()), lease],
            )
            .unwrap();
    }
    place.run().await;
    assert_eq!(place.count("NotificationDeliveries"), 3);
}
