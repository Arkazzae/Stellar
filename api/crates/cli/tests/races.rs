//! A change that someone else made while a command waited for its
//! confirmation: the command writes nothing, records nothing, and says that
//! what it was asked to change was not there.

mod support;

use rusqlite::{Connection, params};
use stellarr_api::values::Id;
use support::Instance;

fn count(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn user_id(connection: &Connection, name: &str) -> Id {
    connection
        .query_row(
            "SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn a_session_revoked_while_the_question_waits_is_not_revoked_again() {
    let instance = Instance::started();
    let jo = user_id(&instance.db(), "jo");
    let session = instance.session(jo, "Firefox");
    let path = instance.database_path();

    let run = instance.run_asked(
        &["--json", "sessions", "revoke", &session.to_string()],
        "y\n",
        move || {
            Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE \"UserSessions\" SET \"RevokedAt\" = '2026-11-01 10:00:00'",
                    [],
                )
                .unwrap();
        },
    );

    assert_eq!((run.exit, run.code().as_str()), (3, "session_not_found"));
    let connection = instance.db();
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM \"OperatorAuditEntries\""),
        0,
        "an audit row for a change this run did not make"
    );
    // The time the other one revoked it is the time it keeps.
    let revoked: String = connection
        .query_row("SELECT \"RevokedAt\" FROM \"UserSessions\"", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(revoked.starts_with("2026-11-01 10:00:00"), "{revoked}");
}

#[test]
fn an_identity_unlinked_while_the_question_waits_leaves_no_record() {
    let instance = Instance::started();
    let jo = user_id(&instance.db(), "jo");
    let link = Id::random();
    instance
        .db()
        .execute(
            "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\") VALUES (?1, ?2, 'https://lab.example.test', 's', 'lab', '2026-11-12 09:00:00')",
            params![link, jo],
        )
        .unwrap();
    let path = instance.database_path();

    let run = instance.run_asked(
        &["--json", "security", "unlink", "jo", &link.to_string()],
        "y\n",
        move || {
            Connection::open(path)
                .unwrap()
                .execute("DELETE FROM \"ExternalIdentities\"", [])
                .unwrap();
        },
    );

    assert_eq!((run.exit, run.code().as_str()), (3, "identity_not_found"));
    let connection = instance.db();
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM \"OperatorAuditEntries\""),
        0
    );
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM \"SecurityEvents\""),
        0,
        "an event for an unlink this run did not make"
    );
}
