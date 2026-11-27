use std::sync::{Arc, Barrier};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use super::*;
use crate::testing::{TempDir, database_settings, new_database};
use crate::values::{Id, Timestamp};

/// SQLITE_CONSTRAINT_FOREIGNKEY, as the extended code of the refusal.
const FOREIGN_KEY_VIOLATION: i32 = 787;

fn insert_user(connection: &Connection, name: &str) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\",
                                \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?2, 'hash', 0, 0, 0, '#777777', ?4)",
        params![Id::random(), name, name.to_uppercase(), Timestamp::now()],
    )
}

#[tokio::test]
async fn every_connection_the_pool_hands_out_enforces_foreign_keys() {
    let (_folder, database) = new_database();
    let barrier = Arc::new(Barrier::new(3));
    let mut readers = Vec::new();
    for _ in 0..3 {
        let database = database.clone();
        let barrier = Arc::clone(&barrier);
        readers.push(tokio::spawn(async move {
            database
                .read(move |db| -> rusqlite::Result<bool> {
                    barrier.wait();
                    assert!(db.is_readonly("main")?);
                    db.pragma_query_value(None, "foreign_keys", |row| row.get(0))
                })
                .await
        }));
    }
    for reader in readers {
        assert!(reader.await.unwrap().unwrap());
    }
    let refusal = database.write(|tx| -> Result<(), DbError> {
        assert!(!tx.is_readonly("main")?);
        tx.execute(
            "INSERT INTO ServerMembers (ServerId, UserId, RoleId, JoinedAt) VALUES (?1, ?2, ?3, ?4)",
            params![Id::random(), Id::random(), Id::random(), Timestamp::now()],
        )?;
        Ok(())
    }).await.unwrap_err();
    assert_eq!(refusal.extended_code(), Some(FOREIGN_KEY_VIOLATION));
}

#[tokio::test]
async fn the_next_writer_restores_connection_settings_changed_by_the_last_lease() {
    let (_folder, database) = new_database();
    database
        .writer
        .run(|connection| -> rusqlite::Result<()> {
            connection.pragma_update(None, "foreign_keys", false)?;
            connection.pragma_update(None, "query_only", true)?;
            Ok(())
        })
        .await
        .unwrap();
    database
        .write(|tx| -> rusqlite::Result<()> {
            assert!(tx.pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))?);
            insert_user(tx, "writable")?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_read_that_tries_to_change_something_fails_and_a_write_after_it_works() {
    let (_folder, database) = new_database();

    let attempt = database
        .read(|connection| -> Result<(), DbError> {
            insert_user(connection, "sneaky")?;
            Ok(())
        })
        .await;

    assert!(attempt.is_err());
    database
        .write(|tx| -> Result<(), DbError> {
            insert_user(tx, "honest")?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn the_database_is_in_write_ahead_logging_on_every_connection_and_in_the_file() {
    let (folder, database) = new_database();
    let barrier = Arc::new(Barrier::new(2));
    let modes = tokio::join!(
        journal_mode(&database, &barrier),
        journal_mode(&database, &barrier)
    );

    assert_eq!(modes, ("wal".to_string(), "wal".to_string()));
    // Stored in the file: a connection from outside finds it too.
    let outside = Connection::open(&database_settings(&folder).path).unwrap();
    let mode: String = outside
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

async fn journal_mode(database: &Database, barrier: &Arc<Barrier>) -> String {
    let barrier = Arc::clone(barrier);
    database
        .read(move |connection| -> Result<String, DbError> {
            barrier.wait();
            Ok(connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn writers_wait_for_each_other_instead_of_failing() {
    const WRITERS: usize = MAX_READERS + 1;
    let (folder, database) = new_database();
    let settings = database_settings(&folder);

    // Another connection holds the write lock the way a long transaction does.
    let mut holder = Connection::open(&settings.path).unwrap();
    let transaction = holder
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    insert_user(&transaction, "holder").unwrap();

    let writers: Vec<_> = (0..WRITERS)
        .map(|index| {
            let database = database.clone();
            tokio::spawn(async move {
                database
                    .write(move |tx| -> Result<(), DbError> {
                        insert_user(tx, &format!("writer{index}"))?;
                        Ok(())
                    })
                    .await
            })
        })
        .collect();

    // Each writer is waiting for the lock, not done and not failed.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(writers.iter().all(|writer| !writer.is_finished()));
    let seen = tokio::time::timeout(
        Duration::from_secs(2),
        database
            .read(|db| db.query_row("SELECT count(*) FROM Users", [], |row| row.get::<_, i64>(0))),
    )
    .await
    .expect("queued writers leave room to read")
    .unwrap();
    assert_eq!(seen, 0);
    transaction.commit().unwrap();

    for writer in writers {
        writer.await.unwrap().unwrap();
    }
    let count: i64 = database
        .read(|connection| -> Result<i64, DbError> {
            Ok(connection.query_row("SELECT count(*) FROM \"Users\"", [], |row| row.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(count, WRITERS as i64 + 1);
}

#[tokio::test]
async fn a_failed_write_leaves_nothing_behind() {
    let (_folder, database) = new_database();

    let outcome = database
        .write(|tx| -> Result<(), DbError> {
            insert_user(tx, "ghost")?;
            // The second one has the same username, which is unique.
            insert_user(tx, "ghost")?;
            Ok(())
        })
        .await;

    assert!(outcome.unwrap_err().is_unique_violation());
    let count: i64 = database
        .read(|connection| -> Result<i64, DbError> {
            Ok(connection.query_row("SELECT count(*) FROM \"Users\"", [], |row| row.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a_unique_value_taken_by_another_request_comes_back_as_none() {
    let (_folder, database) = new_database();
    let add = |name: &'static str| {
        let database = database.clone();
        async move {
            database
                .try_write(move |tx| {
                    insert_user(tx, name)?;
                    Ok(name)
                })
                .await
        }
    };

    assert_eq!(add("taken").await.unwrap(), Some("taken"));
    assert_eq!(add("taken").await.unwrap(), None);
    assert_eq!(add("free").await.unwrap(), Some("free"));
}

#[tokio::test]
async fn the_kind_of_a_refusal_is_told_apart() {
    let (_folder, database) = new_database();
    let foreign = database
        .write(|tx| -> Result<(), DbError> {
            tx.execute(
                "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"JoinedAt\") VALUES (?1, ?1, ?1, ?2)",
                params![Id::random(), Timestamp::now()],
            )?;
            Ok(())
        })
        .await
        .unwrap_err();

    assert!(foreign.is_foreign_key_violation());
    assert!(!foreign.is_unique_violation());
}

#[tokio::test]
async fn a_new_database_has_the_schema_and_both_one_time_steps() {
    let (_folder, database) = new_database();

    let (objects, steps, search): (i64, Vec<String>, bool) = database
        .read(|connection| -> Result<_, DbError> {
            let objects =
                connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
            let mut statement =
                connection.prepare("SELECT \"Name\" FROM \"OneTimeSteps\" ORDER BY \"Name\"")?;
            let steps = statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            let search = connection
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE name = 'MessageSearch'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .is_some();
            Ok((objects, steps, search))
        })
        .await
        .unwrap();

    // Includes the temporary scrub marker, cleared during key backup
    // initialization, the access change journal with its triggers, and the
    // tables of scheduled messages and of polls.
    assert_eq!(objects, 349);
    assert_eq!(steps, ["ordinary-permissions", "view-channel"]);
    assert!(search);
}

#[tokio::test]
async fn the_triggers_and_the_full_text_index_of_the_schema_work() {
    let (_folder, database) = new_database();

    database
        .write(|tx| -> Result<(), DbError> {
            let author = Id::random();
            tx.execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\",
                                        \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, 'maya', 'MAYA', 'Maya Lin', 'hash', 0, 0, 0, '#777777', ?2)",
                params![author, Timestamp::now()],
            )?;
            let (server, channel) = (Id::random(), Id::random());
            tx.execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Stellarr', ?2, 1, 1)",
                params![server, Timestamp::now()],
            )?;
            tx.execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"SlowModeSeconds\",
                                           \"HideAfterInactivityHours\", \"UserLimit\")
                 VALUES (?1, ?2, 'general', 0, 0, 0, 0, 0)",
                params![channel, server],
            )?;
            tx.execute(
                "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, 0, 'a diacritic: zażółć', ?4)",
                params![Id::random(), channel, author, Timestamp::now()],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let (changes, indexed, found): (i64, i64, i64) = database
        .read(|connection| -> Result<_, DbError> {
            Ok((
                connection.query_row("SELECT count(*) FROM \"MessageChanges\"", [], |row| row.get(0))?,
                connection.query_row("SELECT count(*) FROM \"MessageIndexEntries\"", [], |row| row.get(0))?,
                connection.query_row(
                    "SELECT count(*) FROM \"MessageSearch\" WHERE \"MessageSearch\" MATCH 'diacritic'",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
        .await
        .unwrap();
    assert_eq!((changes, indexed, found), (1, 1, 1));
}

#[tokio::test]
async fn a_database_that_has_the_schema_is_opened_as_it_is() {
    let folder = TempDir::new();
    let settings = database_settings(&folder);
    {
        let first = Database::open(&settings).unwrap();
        first
            .write(|tx| -> Result<(), DbError> {
                insert_user(tx, "kept")?;
                // An administrator undid what the one-time step did.
                tx.execute(
                    "DELETE FROM \"OneTimeSteps\" WHERE \"Name\" = 'view-channel'",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    let second = Database::open(&settings).unwrap();
    let (users, steps): (i64, i64) = second
        .read(|connection| -> Result<_, DbError> {
            Ok((
                connection.query_row("SELECT count(*) FROM \"Users\"", [], |row| row.get(0))?,
                connection.query_row("SELECT count(*) FROM \"OneTimeSteps\"", [], |row| {
                    row.get(0)
                })?,
            ))
        })
        .await
        .unwrap();

    assert_eq!(users, 1);
    assert_eq!(steps, 1);
}

#[test]
fn an_unversioned_database_upgrades_atomically_and_keeps_its_rows() {
    let folder = TempDir::new();
    let settings = database_settings(&folder);
    drop(Database::open(&settings).unwrap());
    let db = Connection::open(&settings.path).unwrap();
    insert_user(&db, "kept").unwrap();
    let version = || {
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let current_version = version();
    assert!(current_version > 0);
    db.execute_batch(
        "PRAGMA user_version = 0;
         DROP TABLE DiscordImportJobs;
         DROP TABLE UploadLimits;
         CREATE INDEX UploadLimits ON Users(Username);",
    )
    .unwrap();
    assert!(Database::open(&settings).is_err());
    assert_eq!(version(), 0);
    assert!(db.prepare("SELECT * FROM DiscordImportJobs").is_err());

    db.execute_batch("DROP INDEX UploadLimits").unwrap();
    drop(Database::open(&settings).unwrap());
    assert_eq!(version(), current_version);
    assert!(
        db.prepare("SELECT * FROM DiscordImportJobs, UploadLimits")
            .is_ok()
    );
    assert_eq!(
        db.query_row("SELECT Username FROM Users", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "kept"
    );
}

#[test]
fn a_database_from_a_newer_server_is_refused_with_its_version_intact() {
    let folder = TempDir::new();
    let settings = database_settings(&folder);
    let db = Connection::open(&settings.path).unwrap();
    db.pragma_update(None, "user_version", 9999).unwrap();
    assert!(
        matches!(Database::open(&settings), Err(DbError::Task(message)) if message.contains("9999"))
    );
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        9999
    );
}

#[tokio::test]
async fn the_one_time_steps_of_a_new_database_carry_a_moment_in_the_stored_form() {
    let (_folder, database) = new_database();

    let moments: Vec<Timestamp> = database
        .read(|connection| -> Result<_, DbError> {
            let mut statement = connection.prepare("SELECT \"AppliedAt\" FROM \"OneTimeSteps\"")?;
            Ok(statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<Timestamp>, _>>()?)
        })
        .await
        .unwrap();

    assert_eq!(moments.len(), 2);
    assert!(Timestamp::now().until(moments[0]).is_zero());
}

#[tokio::test]
async fn a_panic_in_the_work_is_an_error_and_the_database_goes_on() {
    let (_folder, database) = new_database();

    let outcome = database
        .read(|_| -> Result<(), DbError> { panic!("on purpose") })
        .await;

    assert!(matches!(outcome, Err(DbError::Task(_))));
    let still: Result<i64, DbError> = database
        .read(|connection| Ok(connection.query_row("SELECT 1", [], |row| row.get(0))?))
        .await;
    assert_eq!(still.unwrap(), 1);
}

#[tokio::test]
async fn the_gate_lets_one_through_at_a_time() {
    let gate = Arc::new(MutationGate::default());
    let first = gate.enter().await;
    let other = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move {
            let _turn = gate.enter().await;
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!other.is_finished());
    drop(first);
    other.await.unwrap();
}

#[test]
fn lock_retry_waits_on_extended_contention_and_preserves_other_errors() {
    use rusqlite::ffi;

    let mut codes = [
        ffi::SQLITE_BUSY_RECOVERY,
        ffi::SQLITE_LOCKED_SHAREDCACHE,
        ffi::SQLITE_IOERR_READ,
    ]
    .into_iter();
    let result: rusqlite::Result<()> = retry_lock(Duration::from_secs(1), || {
        Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(codes.next().unwrap()),
            None,
        ))
    });
    assert_eq!(
        DbError::from(result.unwrap_err()).extended_code(),
        Some(ffi::SQLITE_IOERR_READ)
    );

    let mut attempts = 0;
    let result: rusqlite::Result<()> = retry_lock(Duration::ZERO, || {
        attempts += 1;
        Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_BUSY),
            None,
        ))
    });
    assert_eq!(attempts, 1);
    assert_eq!(
        DbError::from(result.unwrap_err()).extended_code(),
        Some(ffi::SQLITE_BUSY)
    );
}

#[tokio::test]
async fn a_timed_out_write_restores_the_pooled_connections_busy_timeout() {
    let folder = TempDir::new();
    let mut settings = database_settings(&folder);
    settings.busy_timeout = Duration::from_millis(25);
    let database = Database::open(&settings).unwrap();
    let mut holder = Connection::open(&settings.path).unwrap();
    let lock = holder
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();

    let failure = database
        .write_with_lock_retry(|_| panic!("the write must not run without the lock"))
        .await
        .unwrap_err();
    assert_eq!(failure.extended_code(), Some(rusqlite::ffi::SQLITE_BUSY));
    lock.commit().unwrap();

    let timeout: i64 = database
        .write(|db| db.query_row("PRAGMA busy_timeout", [], |row| row.get(0)))
        .await
        .unwrap();
    assert_eq!(timeout, 25);

    database
        .write_with_lock_retry(|tx| insert_user(tx, "after-timeout").map(|_| ()))
        .await
        .unwrap();
}

#[tokio::test]
async fn another_process_signals_only_through_a_token_file_the_api_made() {
    let (_root, db) = new_database();
    let wake = db.wake_file();
    let mut external = Connection::open(db.path()).unwrap();
    let signal = AccessSignal::external(db.path());
    signal.register(&external).unwrap();
    let mut add_user = |name: &str| {
        let tx = external
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let tx = AccessTransaction::new(tx, Some(&signal));
        insert_user(&tx, name).unwrap();
        tx.commit()
    };
    let users = |name: &str| {
        Connection::open(db.path())
            .unwrap()
            .query_row(
                "SELECT count(*) FROM Users WHERE Username = ?1",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };

    // Written in place, so the API keeps a file it can write.
    let before = std::fs::read(&wake).unwrap();
    add_user("signalled").unwrap();
    assert_ne!(std::fs::read(&wake).unwrap(), before);

    // A change that cannot be signalled is not made.
    std::fs::remove_file(&wake).unwrap();
    std::fs::create_dir(&wake).unwrap();
    let refused = add_user("refused").unwrap_err();
    assert!(refused.to_string().contains("cannot signal access changes"));
    assert_eq!(users("refused"), 0);

    // No API has made the file: nothing to wake, and nothing left behind
    // that the API could not write.
    std::fs::remove_dir(&wake).unwrap();
    add_user("offline").unwrap();
    assert!(!wake.exists());
    assert_eq!(users("offline"), 1);
}

#[tokio::test]
async fn chat_idle_time_and_rolled_back_changes_leave_the_watcher_asleep() {
    use crate::auth::fixtures;
    use crate::background;
    use crate::realtime::{Realtime, registry::LiveConnection, watcher};
    use crate::seal::Sealer;
    use crate::settings::Settings;
    use crate::state::AppState;
    use rusqlite::trace::{TraceEvent, TraceEventCodes};

    // What the watcher runs: a read of the journal, a check of sessions.
    const JOURNAL: &str = "SELECT Seq, Kind";
    const SESSIONS: &str = "FROM UserSessions s JOIN Users u";
    let (root, db) = new_database();
    let writer = Connection::open(db.path()).unwrap();
    let server = fixtures::add_instance_server(&writer);
    let user = fixtures::add_user(&writer, None);
    let now = Timestamp::now();
    let first = fixtures::add_session(&writer, user, now, None);
    let second = fixtures::add_session(&writer, user, now, None);
    let channel = Id::random();
    writer.execute("INSERT INTO Channels(Id,ServerId,Name,Kind,Position,SlowModeSeconds,HideAfterInactivityHours,UserLimit)
        VALUES (?1,?2,'chat',0,0,0,0,0)", params![channel, server]).unwrap();

    let sql = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    for pool in [&db.readers, &db.writer] {
        let sql = sql.clone();
        pool.run(move |connection| {
            connection.trace_v2(
                TraceEventCodes::SQLITE_TRACE_STMT,
                Some(move |event: TraceEvent<'_>| {
                    if let TraceEvent::Stmt(stmt, _) = event {
                        sql.lock().unwrap().push(stmt.expanded_sql().unwrap());
                    }
                }),
            )
        })
        .await
        .unwrap();
    }
    let watched = |needle: &'static str| {
        let sql = sql.clone();
        move || -> Vec<String> {
            sql.lock()
                .unwrap()
                .iter()
                .filter(|s| s.contains(needle))
                .cloned()
                .collect()
        }
    };
    let (journal_reads, session_checks) = (watched(JOURNAL), watched(SESSIONS));
    let settings = Settings::load(&[
        ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
        (
            "STELLARR_DATA_ROOT".into(),
            root.path().display().to_string(),
        ),
    ])
    .unwrap();
    let hub = Realtime::new();
    let state = AppState::new(
        settings,
        db.clone(),
        hub.clone(),
        hub.clone(),
        Sealer::from_key([1; 32]),
        Sealer::from_key([2; 32]),
    );
    state.plug_in_realtime(hub.clone());
    for (name, session) in [("first", first), ("second", second)] {
        hub.registry().register(LiveConnection {
            connection_id: name.into(),
            session,
            user,
            version: 1,
        });
    }
    let running = background::start(watcher::workers(), &state);
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while session_checks().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the first check never ran"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    sql.lock().unwrap().clear();
    tokio::time::sleep(Duration::from_millis(1250)).await;
    assert!(
        sql.lock().unwrap().is_empty(),
        "the idle watcher queried SQLite"
    );

    for _ in 0..10 {
        db.write(move |tx| {
            tx.execute(
                "INSERT INTO Messages(Id,ChannelId,AuthorId,IsSystem,Content,CreatedAt)
                 VALUES (?1,?2,?3,0,'hello',?4)",
                params![Id::random(), channel, user, now],
            )
        })
        .await
        .unwrap();
    }
    db.write(move |tx| {
        tx.execute(
            "UPDATE UserSessions SET LastSeenAt = ?1, ExpiresAt = ExpiresAt",
            [now],
        )
    })
    .await
    .unwrap();
    let rolled: Result<(), DbError> = db
        .write(|tx| {
            tx.execute("UPDATE Users SET IsDisabled = 1", [])?;
            Err(DbError::Task("roll back".into()))
        })
        .await;
    assert!(rolled.is_err());
    db.write(|tx| {
        tx.execute_batch(
            "SAVEPOINT candidate; UPDATE Users SET IsDisabled = 1;
             ROLLBACK TO candidate; RELEASE candidate",
        )
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        journal_reads().is_empty(),
        "chat or a rolled-back change woke the watcher"
    );
    assert!(session_checks().is_empty());
    assert_eq!(hub.registry().count(), 2);

    db.write(move |tx| {
        tx.execute(
            "UPDATE UserSessions SET RevokedAt = ?1 WHERE Id = ?2",
            params![now, first],
        )
    })
    .await
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while hub.registry().count() != 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "{:?}",
            sql.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    running.stop(Duration::from_secs(2)).await;
    assert_eq!(journal_reads().len(), 1);
    let checks = session_checks();
    assert_eq!(checks.len(), 1);
    assert!(checks[0].contains(&first.to_db_text()));
    assert!(!checks[0].contains(&second.to_db_text()));
    assert_eq!(hub.registry().snapshot()[0].session, second);
}
