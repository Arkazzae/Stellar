//! How the tool holds the database: what a read may not do, what a write
//! waits for, and what the server's rules do to a change made here.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rusqlite::{Connection, ErrorCode};
use stellarr_cli::database::CliDatabase;
use stellarr_cli::invocation::Invocation;
use stellarr_cli::runtime::Runtime;
use support::{Instance, silent_io};

fn runtime_on(instance: &Instance) -> Runtime {
    let words = vec![
        "--data-root".to_string(),
        instance.root().display().to_string(),
        "status".to_string(),
    ];
    Runtime::create(
        Invocation::parse(&words).expect("arguments"),
        silent_io(),
        vec![("ASPNETCORE_ENVIRONMENT".into(), "Testing".into())],
        Arc::new(AtomicBool::new(false)),
    )
}

/// Holds the database's write lock for `hold`, from before this returns.
fn hold_the_write_lock(instance: &Instance, hold: Duration) -> std::thread::JoinHandle<()> {
    let path = instance.database_path();
    let (ready, locked) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        ready.send(()).unwrap();
        std::thread::sleep(hold);
        connection.execute_batch("COMMIT").unwrap();
    });
    locked.recv().unwrap();
    holder
}

#[test]
fn a_read_opens_the_database_read_only_and_a_change_does_not() {
    let instance = Instance::started();
    let runtime = runtime_on(&instance);

    let reading = CliDatabase::open(&runtime, false).unwrap();
    let refused = reading
        .connection
        .execute("UPDATE \"Servers\" SET \"Name\" = 'changed'", [])
        .unwrap_err();
    assert!(
        matches!(
            refused,
            rusqlite::Error::SqliteFailure(ref failure, _) if failure.code == ErrorCode::ReadOnly
        ),
        "{refused}"
    );
    assert!(
        reading
            .connection
            .execute("CREATE TABLE \"Stray\" (\"A\")", [])
            .is_err()
    );

    let writing = CliDatabase::open(&runtime, true).unwrap();
    writing
        .connection
        .execute("UPDATE \"Servers\" SET \"Name\" = 'changed'", [])
        .unwrap();
}

#[test]
fn foreign_keys_are_on_as_the_server_has_them() {
    let instance = Instance::started();
    let runtime = runtime_on(&instance);
    for mutation in [false, true] {
        let database = CliDatabase::open(&runtime, mutation).unwrap();
        let on: i64 = database
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(on, 1, "mutation: {mutation}");
    }
}

#[test]
fn a_change_waits_for_a_writer_instead_of_failing_at_once() {
    let instance = Instance::started();
    let holder = hold_the_write_lock(&instance, Duration::from_millis(1200));
    let started = Instant::now();

    let run = instance.run(&[
        "--json",
        "config",
        "set",
        "instance.name",
        "Harbour",
        "--yes",
    ]);

    holder.join().unwrap();
    assert_eq!(run.exit, 0, "{}", run.output);
    assert!(
        started.elapsed() >= Duration::from_millis(1000),
        "it did not wait for the lock"
    );
}

#[test]
fn a_recovery_takes_the_write_lock_first_and_waits_for_it() {
    let instance = Instance::started();
    let holder = hold_the_write_lock(&instance, Duration::from_millis(1200));

    // A transaction that starts by reading cannot be made to wait later: it
    // would fail at its first write. This one must have asked for the lock at
    // its start.
    let run = instance.run(&[
        "--json",
        "security",
        "recover",
        "jo",
        "--reason",
        "lost phone",
        "--yes",
    ]);

    holder.join().unwrap();
    assert_eq!(run.exit, 0, "{}", run.output);
}

#[test]
fn a_writer_that_stays_gives_up_after_the_busy_timeout() {
    let instance = Instance::started();
    let holder = hold_the_write_lock(&instance, Duration::from_secs(7));
    let started = Instant::now();

    let run = instance.run(&[
        "--json",
        "config",
        "set",
        "instance.name",
        "Harbour",
        "--yes",
    ]);
    let waited = started.elapsed();

    holder.join().unwrap();
    assert_eq!((run.exit, run.code().as_str()), (4, "database_locked"));
    assert!(
        waited >= Duration::from_millis(4500) && waited < Duration::from_millis(6500),
        "waited {waited:?}"
    );
}
