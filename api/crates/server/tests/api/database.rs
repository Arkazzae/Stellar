//! What the binary does to the database on disk, read from outside with a
//! connection of the test's own.

use rusqlite::{Connection, OpenFlags};
use serde_json::json;

use crate::support::{Server, TempDir};

fn open(server: &Server) -> Connection {
    Connection::open_with_flags(server.database_path(), OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap()
}

#[tokio::test]
async fn a_new_data_folder_gets_a_database_with_the_schema_and_the_keys_folder() {
    let server = Server::start().await;

    let connection = open(&server);
    let objects: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    let steps: Vec<String> = connection
        .prepare("SELECT \"Name\" FROM \"OneTimeSteps\" ORDER BY \"Name\"")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(objects, 364);
    // Stored in the file: a connection from outside finds it too.
    assert_eq!(mode, "wal");
    assert_eq!(steps, ["ordinary-permissions", "view-channel"]);
    assert!(server.folder().join("keys").is_dir());
}

#[tokio::test]
async fn a_database_that_has_the_schema_is_opened_as_it_is() {
    let folder = TempDir::new();
    let first = Server::start_in(folder.path(), &[]).await;
    let applied: String = open(&first)
        .query_row(
            "SELECT \"AppliedAt\" FROM \"OneTimeSteps\" WHERE \"Name\" = 'view-channel'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    open(&first)
        .execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
             VALUES ('3D82DA44-8AA5-4BDF-9EE1-2902DF99880C', 'Kept', '2026-11-24 00:33:18.6419643', 1, 1)",
            [],
        )
        .unwrap();
    assert!(first.stop().await);

    let second = Server::start_in(folder.path(), &[]).await;

    let connection = open(&second);
    let kept: String = connection
        .query_row("SELECT \"Name\" FROM \"Servers\"", [], |row| row.get(0))
        .unwrap();
    let again: String = connection
        .query_row(
            "SELECT \"AppliedAt\" FROM \"OneTimeSteps\" WHERE \"Name\" = 'view-channel'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((kept.as_str(), again), ("Kept", applied));
}

#[tokio::test]
async fn the_server_waits_for_a_writer_instead_of_failing() {
    // A writer outside holds the write lock while the server is asked to
    // become ready, which only reads: it is not held up by the writer.
    let server = Server::start().await;
    let mut connection = open(&server);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();

    let response = server.client().get("/health/ready").await;

    assert_eq!(response.status, 200);
    transaction.rollback().unwrap();
}

#[tokio::test]
async fn the_database_is_created_in_the_db_folder_of_the_data_root() {
    let server = Server::start().await;

    assert!(server.folder().join("db").join("stellarr.db").is_file());
    assert_eq!(
        server
            .client()
            .get("/health/ready")
            .await
            .json::<serde_json::Value>(),
        json!({ "status": "ready" })
    );
}
