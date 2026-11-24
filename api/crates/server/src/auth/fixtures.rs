//! Rows the unit tests of the account-security modules start from.

use rusqlite::{Connection, params};

use super::flow::StoredChallenge;
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

/// A session of the account; `mfa_completed_at` is when it passed a real
/// second step, if it did.
pub fn add_session(
    connection: &Connection,
    user: Id,
    authenticated_at: Timestamp,
    mfa_completed_at: Option<Timestamp>,
) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
                 \"AbsoluteExpiresAt\", \"PrimarySource\", \"AuthenticatedAt\", \"MfaCompletedAt\", \"MfaEvidence\", \"SecurityVersion\", \"ShortLived\")
             VALUES (?1, ?2, 'Device', '127.0.0.1', ?3, ?3, ?4, ?4, 'password', ?3, ?5, NULL, 1, 0)",
            params![
                id,
                user,
                authenticated_at,
                authenticated_at.plus(std::time::Duration::from_secs(86_400)),
                mfa_completed_at
            ],
        )
        .expect("a session is added");
    id
}

/// A second step that is waiting for its first code.
pub fn add_pending_method(connection: &Connection, user: Id, kind: &str) -> Id {
    let id = add_method(connection, user, kind, None);
    connection
        .execute(
            "UPDATE \"MfaMethods\" SET \"ConfirmedAt\" = NULL WHERE \"Id\" = ?1",
            [id],
        )
        .expect("a method is made pending");
    id
}

/// The secret every test challenge's cookie holds.
pub const SECRET: &str = "secret";

/// What the browser's cookie holds for the challenge.
pub fn preauth_of(challenge: &StoredChallenge) -> (Id, String) {
    (challenge.id, SECRET.to_string())
}

/// A challenge of the purpose given, as the flow looks it up, and the row
/// that stands behind it.
pub fn add_challenge(connection: &Connection, user: Id, purpose: &str) -> StoredChallenge {
    let id = Id::random();
    let now = Timestamp::now();
    connection
        .execute(
            "INSERT INTO \"AuthChallenges\" SELECT ?1, ?2, ?3, ?6, 'password', NULL, \"SessionVersion\", 0, NULL, NULL, 0, 5, ?4, ?5, NULL FROM \"Users\" WHERE \"Id\" = ?2",
            params![id, user, purpose, now, now.plus(std::time::Duration::from_secs(300)), super::tokens::hash(SECRET)],
        )
        .expect("a challenge is added");
    StoredChallenge {
        id,
        user,
        purpose: purpose.to_string(),
        primary_source: "password".to_string(),
        parent_session: None,
        short_lived: false,
        pending_identity: None,
        attempts: 0,
        max_attempts: 5,
        expires_at: now.plus(std::time::Duration::from_secs(300)),
    }
}

/// How many attempts the challenge has spent, and whether it is closed.
pub fn attempts_of(connection: &Connection, challenge: Id) -> (i64, bool) {
    connection
        .query_row(
            "SELECT \"Attempts\", \"ConsumedAt\" IS NOT NULL FROM \"AuthChallenges\" WHERE \"Id\" = ?1",
            [challenge],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the challenge is there")
}

/// The details of the account's events of one kind, oldest first.
pub fn events_of(connection: &Connection, user: Id, kind: &str) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT \"Detail\" FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 AND \"Kind\" = ?2 ORDER BY \"CreatedAt\"")
        .expect("a statement");
    statement
        .query_map(params![user, kind], |row| row.get(0))
        .expect("events are read")
        .map(|detail| detail.expect("a detail"))
        .collect()
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
