//! What a command says before or after it takes something away that the
//! person may not have meant to lose.

mod support;

use rusqlite::{Connection, params};
use stellarr_api::seal::Sealer;
use stellarr_api::values::{Id, Timestamp};
use support::Instance;

fn user_id(connection: &Connection, name: &str) -> Id {
    connection
        .query_row(
            "SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn link(connection: &Connection, user: Id) -> Id {
    let id = Id::random();
    connection
        .execute(
            "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\") VALUES (?1, ?2, 'https://lab.example.test', ?3, 'lab', '2026-11-12 09:00:00')",
            params![id, user, id.to_string()],
        )
        .unwrap();
    id
}

#[test]
fn unlinking_the_last_way_in_says_so_with_yes_and_in_the_result() {
    let instance = Instance::started();
    let connection = instance.db();
    let jo = user_id(&connection, "jo");
    connection
        .execute(
            "UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1",
            [jo],
        )
        .unwrap();
    let only = link(&connection, jo);

    let run = instance.run(&[
        "--json",
        "security",
        "unlink",
        "jo",
        &only.to_string(),
        "--yes",
    ]);

    assert_eq!(run.exit, 0, "{}", run.output);
    let json = run.json();
    assert_eq!(json["data"]["lastWayIn"], true);
    let warning = json["warnings"][0].as_str().expect("a warning");
    assert!(warning.contains("no other way in"), "{warning}");
    // The member comes last, after the ones the first tool wrote (the parsed
    // document sorts its keys, so the order is read from the text).
    let after = |name: &str| run.output.find(&format!("\"{name}\"")).unwrap();
    assert!(after("lastWayIn") > after("auditActor"));
    assert!(after("lastWayIn") > after("issuer"));

    let text = {
        let again = link(&connection, jo);
        instance.run(&["security", "unlink", "jo", &again.to_string(), "--yes"])
    };
    assert!(text.error.contains("warning: ") && text.error.contains("no other way in"));
}

#[test]
fn unlinking_one_of_several_ways_in_warns_of_nothing() {
    let instance = Instance::started();
    let connection = instance.db();
    let jo = user_id(&connection, "jo");
    let extra = link(&connection, jo);

    // jo has a password, so the link is not the way in that is left.
    let run = instance.run(&[
        "--json",
        "security",
        "unlink",
        "jo",
        &extra.to_string(),
        "--yes",
    ]);

    assert_eq!(run.exit, 0, "{}", run.output);
    let json = run.json();
    assert_eq!(json["data"]["lastWayIn"], false);
    assert!(json["warnings"].as_array().unwrap().is_empty());
}

fn store_sealed_under(instance: &Instance, key: [u8; 32]) {
    let other = Sealer::from_key(key);
    instance
        .db()
        .execute(
            "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('mail.smtp-password', ?1, ?2)",
            params![
                other.seal("instance-secret", b"mail.smtp-password", None, b"old"),
                Timestamp::now()
            ],
        )
        .unwrap();
}

#[test]
fn replacing_or_removing_a_value_this_server_cannot_open_says_so_in_the_question() {
    let instance = Instance::started();
    store_sealed_under(&instance, [9u8; 32]);

    let set = instance.run_interactive("n\n", &["secrets", "set", "mail.smtp-password"]);
    assert!(
        set.output.contains("cannot be opened by this server")
            && set.output.contains("sealed under another master key")
            && set.output.contains("replaced"),
        "{}",
        set.output
    );
    let clear = instance.run_interactive("n\n", &["secrets", "clear", "mail.smtp-password"]);
    assert!(
        clear.output.contains("cannot be opened by this server")
            && clear.output.contains("removed"),
        "{}",
        clear.output
    );
}

#[test]
fn a_value_this_server_can_open_is_asked_about_without_the_warning() {
    let instance = Instance::started();
    let sealer = Sealer::open_existing(&instance.keys()).unwrap().unwrap();
    instance
        .db()
        .execute(
            "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('mail.smtp-password', ?1, ?2)",
            params![
                sealer.seal("instance-secret", b"mail.smtp-password", None, b"old"),
                Timestamp::now()
            ],
        )
        .unwrap();

    let set = instance.run_interactive("n\n", &["secrets", "set", "mail.smtp-password"]);
    let clear = instance.run_interactive("n\n", &["secrets", "clear", "mail.smtp-password"]);

    assert!(set.output.contains("Will store"), "{}", set.output);
    assert!(clear.output.contains("Will remove"), "{}", clear.output);
    assert!(!set.output.contains("cannot be opened") && !clear.output.contains("cannot be opened"));
}

#[test]
fn a_missing_master_key_is_said_in_the_question_about_a_stored_value() {
    let instance = Instance::started();
    store_sealed_under(&instance, [9u8; 32]);
    std::fs::remove_file(instance.keys().join("master.key")).unwrap();

    let clear = instance.run_interactive("n\n", &["secrets", "clear", "mail.smtp-password"]);
    assert!(
        clear.output.contains("there is no master key"),
        "{}",
        clear.output
    );
}
