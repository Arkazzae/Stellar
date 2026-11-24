//! What the writes of the password and the address do when what was checked
//! before them no longer holds: the session ended, or the password changed,
//! between the hashing and the transaction. The two halves of each route are
//! called with the change made between them.

use std::time::Duration;

use rusqlite::{Connection, params};

use super::email_change::{self, StartRequest};
use super::password::{self, ChangeRequest, SetRequest};
use crate::auth::{Caller, password as hashing};
use crate::http::Origin;
use crate::mail::tests::support::instance;
use crate::state::AppState;
use crate::testing::TempDir;
use crate::values::{Id, Timestamp};

const OLD: &str = "the old password";
const NEW: &str = "the new password";

struct World {
    _folder: TempDir,
    state: AppState,
    caller: Caller,
}

impl World {
    fn new() -> World {
        let (folder, state) = instance(&[]);
        let db = Connection::open(state.database().path()).unwrap();
        let (user, session, now) = (Id::random(), Id::random(), Timestamp::now());
        db.execute(
            "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\", \"NormalizedEmail\",
                 \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
             VALUES (?1, 'maya', 'MAYA', 'Maya', ?2, 'maya@example.test', 'MAYA@EXAMPLE.TEST', 0, 1, 0, 'c', ?3)",
            params![user, hashing::hash(OLD), now],
        )
        .unwrap();
        db.execute(
            "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\", \"AbsoluteExpiresAt\",
                 \"PrimarySource\", \"AuthenticatedAt\", \"SecurityVersion\", \"ShortLived\")
             VALUES (?1, ?2, 'Device', '127.0.0.1', ?3, ?3, ?4, ?4, 'password', ?3, 1, 0)",
            params![session, user, now, now.plus(Duration::from_secs(86_400))],
        )
        .unwrap();
        World { _folder: folder, state, caller: Caller { user, session } }
    }

    fn db(&self) -> Connection {
        let connection = Connection::open(self.state.database().path()).unwrap();
        connection.busy_timeout(Duration::from_secs(10)).unwrap();
        connection
    }

    fn end_the_session(&self) {
        self.db().execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1", params![self.caller.session, Timestamp::now()]).unwrap();
    }

    fn hash(&self) -> String {
        self.db().query_row("SELECT \"PasswordHash\" FROM \"Users\" WHERE \"Id\" = ?1", [self.caller.user], |row| row.get(0)).unwrap()
    }
}

fn origin() -> Origin {
    Origin { address: "127.0.0.1".into(), https: false, user_agent: "Test".into(), headers: axum::http::HeaderMap::new() }
}

fn change_request() -> ChangeRequest {
    serde_json::from_str(&format!("{{\"currentPassword\":\"{OLD}\",\"newPassword\":\"{NEW}\"}}")).unwrap()
}

async fn body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn what_is_printed_of_a_request_holds_no_password() {
    let secret = "s3cret-value-1234";
    let printed = [
        format!("{:?}", serde_json::from_str::<ChangeRequest>(&format!("{{\"currentPassword\":\"{secret}\",\"newPassword\":\"{secret}x\"}}")).unwrap()),
        format!("{:?}", serde_json::from_str::<SetRequest>(&format!("{{\"newPassword\":\"{secret}\"}}")).unwrap()),
        format!("{:?}", serde_json::from_str::<StartRequest>(&format!("{{\"email\":\"a@example.test\",\"password\":\"{secret}\"}}")).unwrap()),
        format!("{:?}", serde_json::from_str::<super::methods::Proof>(&format!("{{\"password\":\"{secret}\"}}")).unwrap()),
    ];
    for text in printed {
        assert!(!text.contains(secret), "{text}");
    }
}

#[tokio::test]
async fn a_password_changes_when_nothing_moved_between_the_check_and_the_write() {
    let world = World::new();
    let prepared = password::prepare(&world.state, world.caller, change_request()).await.unwrap().expect("allowed");

    let answer = password::commit(&world.state, world.caller, &origin(), prepared).await.unwrap();

    assert_eq!(body(answer).await, "{\"result\":\"changed\"}");
    assert!(hashing::verify(NEW, Some(&world.hash())));
}

#[tokio::test]
async fn a_session_ended_while_its_password_change_was_in_flight_changes_nothing() {
    let world = World::new();
    let before = world.hash();
    let prepared = password::prepare(&world.state, world.caller, change_request()).await.unwrap().expect("allowed");
    world.end_the_session();

    let answer = password::commit(&world.state, world.caller, &origin(), prepared).await.unwrap();

    assert_eq!(body(answer).await, "{\"result\":\"second-step-required\"}");
    assert_eq!(world.hash(), before);
    let events: i64 = world.db().query_row("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'password-changed'", [], |row| row.get(0)).unwrap();
    assert_eq!(events, 0);
}

#[tokio::test]
async fn a_password_that_changed_after_it_was_checked_is_no_longer_the_current_one() {
    let world = World::new();
    let prepared = password::prepare(&world.state, world.caller, change_request()).await.unwrap().expect("allowed");
    let other = hashing::hash("somebody got there first");
    world.db().execute("UPDATE \"Users\" SET \"PasswordHash\" = ?2 WHERE \"Id\" = ?1", params![world.caller.user, other]).unwrap();

    let answer = password::commit(&world.state, world.caller, &origin(), prepared).await.unwrap();

    assert_eq!(body(answer).await, "{\"result\":\"wrong-current\"}");
    assert_eq!(world.hash(), other, "their password stands");
    let events: i64 = world.db().query_row("SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'password-changed'", [], |row| row.get(0)).unwrap();
    assert_eq!(events, 0);
}

#[tokio::test]
async fn a_session_ended_while_its_address_change_was_in_flight_writes_no_request() {
    let world = World::new();
    let request: StartRequest = serde_json::from_str(&format!("{{\"email\":\"new@example.test\",\"password\":\"{OLD}\"}}")).unwrap();
    let prepared = email_change::prepare(&world.state, world.caller, request).await.unwrap().expect("allowed");
    world.end_the_session();

    let written = email_change::write(&world.state, world.caller, prepared).await.unwrap();

    assert_eq!(written.err(), Some("second-step-required"));
    let rows: i64 = world.db().query_row("SELECT COUNT(*) FROM \"EmailChangeRequests\"", [], |row| row.get(0)).unwrap();
    assert_eq!(rows, 0);
}
