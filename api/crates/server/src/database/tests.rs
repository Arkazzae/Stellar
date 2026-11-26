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
    // Three leases that are all open at once, so that three connections are.
    let barrier = Arc::new(Barrier::new(3));
    let mut reads = Vec::new();
    for _ in 0..3 {
        let database = database.clone();
        let barrier = Arc::clone(&barrier);
        reads.push(tokio::spawn(async move {
            // Lent a connection directly: a read could not insert a row.
            database
                .run(move |connection| -> Result<(i64, i32), DbError> {
                    barrier.wait();
                    let enforced = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
                    let refusal = connection
                        .execute(
                            "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"JoinedAt\")
                             VALUES (?1, ?2, ?3, ?4)",
                            params![Id::random(), Id::random(), Id::random(), Timestamp::now()],
                        )
                        .map_err(DbError::from)
                        .unwrap_err();
                    Ok((enforced, refusal.extended_code().unwrap_or_default()))
                })
                .await
        }));
    }

    for read in reads {
        assert_eq!(read.await.unwrap().unwrap(), (1, FOREIGN_KEY_VIOLATION));
    }
}

#[tokio::test]
async fn a_connection_that_had_foreign_keys_switched_off_has_them_back_when_lent_again() {
    let (_folder, database) = new_database();
    // Outside a transaction, where the setting can be changed at all.
    database
        .run(|connection| -> Result<(), DbError> {
            connection.pragma_update(None, "foreign_keys", false)?;
            let now: i64 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
            assert_eq!(now, 0);
            Ok(())
        })
        .await
        .unwrap();

    let enforced: i64 = database
        .read(|connection| -> Result<i64, DbError> {
            Ok(connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?)
        })
        .await
        .unwrap();

    assert_eq!(enforced, 1);
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
    const WRITERS: usize = 4;
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

    // The original 280 objects plus the import, upload limit and activity
    // tables and their indexes.
    assert_eq!(objects, 294);
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

    let timeout: i64 = database
        .read(|db| db.query_row("PRAGMA busy_timeout", [], |row| row.get(0)))
        .await
        .unwrap();
    assert_eq!(timeout, 25);
    lock.commit().unwrap();

    database
        .write_with_lock_retry(|tx| insert_user(tx, "after-timeout").map(|_| ()))
        .await
        .unwrap();
}
