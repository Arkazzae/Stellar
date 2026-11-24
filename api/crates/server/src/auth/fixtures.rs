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

/// An account with the address given (stored the way the first server stores
/// it: as written, and in capitals beside it).
pub fn add_user(connection: &Connection, email: Option<&str>) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\",
                 \"NormalizedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
             VALUES (?1, ?2, ?2, 'Person', 'hash', ?3, ?4, 0, 1, 0, 'c', ?5)",
            params![
                id,
                id.to_string(),
                email,
                email.map(|email| email.trim().to_uppercase()),
                Timestamp::now()
            ],
        )
        .expect("an account is added");
    id
}

/// A confirmed method of the type given.
pub fn add_method(connection: &Connection, user: Id, kind: &str, address: Option<&str>) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"EmailAddress\")
             VALUES (?1, ?2, ?3, 'Method', ?4, ?4, ?5)",
            params![id, user, kind, Timestamp::now(), address],
        )
        .expect("a method is added");
    id
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
