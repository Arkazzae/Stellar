//! Rows the unit tests of the account-security modules start from.

use rusqlite::{Connection, params};

use crate::testing::{TempDir, new_database};
use crate::values::{Id, Timestamp};

/// A database of its own, opened for the test.
pub fn open() -> (TempDir, Connection) {
    let (folder, database) = new_database();
    let connection = Connection::open(database.path()).expect("the test database opens");
    (folder, connection)
}

/// The first server of the instance, so that the settings have a row to hang on.
pub fn add_instance_server(connection: &Connection) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
             VALUES (?1, 'Home', ?2, 1, 0)",
            params![id, Timestamp::now()],
        )
        .expect("a server is added");
    id
}
