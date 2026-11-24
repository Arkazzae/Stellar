//! The account's side of encryption: its key, the PIN backup and the attempts
//! on it, the proof a new PIN or a reset asks for, and the browsers that hold
//! keys.

use std::sync::Arc;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use super::kit::*;
use crate::support::{PASSWORD, Server};

const BACKUP: &str = "/api/me/encryption/backup";
const UNLOCK: &str = "/api/me/encryption/backup/unlock";
const RESET: &str = "/api/me/encryption/account-key/reset";

pub fn add_second_step(server: &Server, user: &str) {
    server
        .database()
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\")
             VALUES (?1, ?2, 'totp', 'Seeded', ?3, ?3)",
            params![Id::random(), Id::parse(user).unwrap(), Timestamp::now()],
        )
        .unwrap();
}

/// Says when the sessions of the account last passed a second step.
pub fn passed_second_step(server: &Server, user: &str, minutes_ago: u64) {
    let when = Timestamp::now().minus(std::time::Duration::from_secs(minutes_ago * 60));
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
            params![Id::parse(user).unwrap(), when],
        )
        .unwrap();
}

#[tokio::test]
async fn an_account_sets_up_one_key_with_a_valid_backup() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    let key = text(&bytes(32, 4));
    let weak = json!({
        "salt": text(&bytes(16, 1)), "iterations": 1000, "verifier": text(&bytes(32, 1)),
        "iv": text(&bytes(12, 1)), "ciphertext": text(&bytes(64, 1)),
    });

    for body in [
        json!({ "algorithm": "x25519", "publicKey": text(&bytes(31, 4)), "backup": backup(&bytes(32, 5)) }),
        json!({ "algorithm": "rsa", "publicKey": key, "backup": backup(&bytes(32, 5)) }),
        json!({ "algorithm": "x25519", "publicKey": key, "backup": weak }),
    ] {
        assert_eq!(post(maja, &format!("{ACCOUNT}/account-key"), &body).await.status, 400);
    }
    assert_eq!(read(maja, ACCOUNT).await["accountKey"], Value::Null);

    let account = set_up(maja).await;
    let again = post(maja, &format!("{ACCOUNT}/account-key"), &set_up_body(&bytes(32, 5))).await;
    assert_eq!(refusal(&again), (409, Some("account-key-exists".into())));

    let state = read(maja, ACCOUNT).await;
    assert_eq!(state["accountKey"]["keyId"], account.key_id.as_str());
    assert_eq!(state["accountKey"]["algorithm"], "x25519");
    assert_eq!(state["backup"]["remainingAttempts"], 10);
    assert_eq!(state["device"], Value::Null);
}

#[tokio::test]
async fn the_answer_to_a_set_up_is_the_first_servers_in_every_member() {
    let crowd = Crowd::new().await;

    let response = post(&crowd.maja.client, &format!("{ACCOUNT}/account-key"), &set_up_body(&bytes(32, 5))).await;

    assert_eq!(response.status, 201);
    assert_eq!(response.header("location"), Some("/api/me/encryption"));
    assert_eq!(response.header("content-type"), Some("application/json; charset=utf-8"));
    let text = response.text();
    assert!(
        text.starts_with("{\"accountKey\":{\"keyId\":\"") && text.contains("\",\"algorithm\":\"x25519\",\"publicKey\":\""),
        "{text}"
    );
    assert!(text.contains("\"backup\":{\"iterations\":600000,\"failedAttempts\":0,\"remainingAttempts\":10,\"updatedAt\":\""), "{text}");
    assert!(text.ends_with("\"device\":null}"), "{text}");
}

#[tokio::test]
async fn wrong_pins_are_counted_and_the_tenth_in_a_row_deletes_the_backup_but_not_the_key() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    let account = set_up(maja).await;

    let parameters = read(maja, BACKUP).await;
    assert_eq!(parameters["iterations"], 600_000);
    assert_eq!(parameters["remainingAttempts"], 10);
    assert_eq!(parameters["salt"], text(&bytes(16, 1)));
    let short = post(maja, UNLOCK, &json!({ "authKey": text(&bytes(16, 0)) })).await;
    assert_eq!(short.status, 400);

    let wrong = post(maja, UNLOCK, &json!({ "authKey": text(&bytes(32, 9)) })).await;
    assert_eq!(refusal(&wrong), (403, Some("pin-incorrect".into())));
    let text_of_wrong = wrong.text();
    assert!(
        text_of_wrong.contains("\"status\":403,\"remainingAttempts\":9,\"code\":\"pin-incorrect\",\"traceId\""),
        "{text_of_wrong}"
    );

    // A right PIN hands out the backup and forgets the wrong ones.
    let right = post(maja, UNLOCK, &json!({ "authKey": text(&account.auth_key) })).await;
    assert_eq!(right.status, 200);
    let unlocked: Value = right.json();
    assert_eq!(unlocked["accountKeyId"], account.key_id.as_str());
    assert_eq!(unlocked["iv"], text(&bytes(12, 2)));
    assert_eq!(unlocked["ciphertext"], text(&bytes(64, 3)));
    assert_eq!(read(maja, BACKUP).await["remainingAttempts"], 10);

    let mut remaining = Vec::new();
    for _ in 0..9 {
        let response = post(maja, UNLOCK, &json!({ "authKey": text(&bytes(32, 9)) })).await;
        assert_eq!(response.status, 403);
        remaining.push(response.json::<Value>()["remainingAttempts"].as_i64().unwrap());
    }
    assert_eq!(remaining, [9, 8, 7, 6, 5, 4, 3, 2, 1]);
    let tenth = post(maja, UNLOCK, &json!({ "authKey": text(&bytes(32, 9)) })).await;
    assert_eq!(refusal(&tenth), (410, Some("backup-deleted".into())));
    assert_eq!(tenth.json::<Value>()["remainingAttempts"], 0);
    assert!(tenth.text().contains("\"status\":410,\"remainingAttempts\":0,\"code\":\"backup-deleted\""));

    let gone = post(maja, UNLOCK, &json!({ "authKey": text(&account.auth_key) })).await;
    assert_eq!(refusal(&gone), (404, Some("backup-missing".into())));
    assert_eq!(refusal(&maja.get(BACKUP).await), (404, Some("backup-missing".into())));
    let state = read(maja, ACCOUNT).await;
    assert_eq!(state["accountKey"]["keyId"], account.key_id.as_str());
    assert_eq!(state["backup"], Value::Null);
}

#[tokio::test]
async fn attempts_sent_at_once_are_counted_one_after_another() {
    let crowd = Crowd::new().await;
    set_up(&crowd.maja.client).await;
    let client = Arc::new(crowd.server.sign_in(&crowd.maja).await);

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let client = Arc::clone(&client);
        tasks.spawn(async move { post(&client, UNLOCK, &json!({ "authKey": text(&bytes(32, 9)) })).await });
    }
    let mut answers = Vec::new();
    while let Some(answer) = tasks.join_next().await {
        answers.push(answer.unwrap());
    }

    let count = |status: u16| answers.iter().filter(|a| a.status.as_u16() == status).count();
    assert_eq!((count(403), count(410), count(404)), (9, 1, 2));
    let mut remaining: Vec<i64> = answers
        .iter()
        .filter(|a| a.status == 403)
        .map(|a| a.json::<Value>()["remainingAttempts"].as_i64().unwrap())
        .collect();
    remaining.sort_unstable();
    assert_eq!(remaining, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[tokio::test]
async fn a_new_pin_needs_the_password_a_fresh_second_step_and_a_browser_that_holds_the_key() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    let account = set_up(maja).await;
    let new_auth = bytes(32, 11);
    let new_pin = |password: &str, key_id: &str| {
        json!({ "currentPassword": password, "accountKeyId": key_id, "backup": backup(&new_auth) })
    };

    assert_eq!(refusal(&put(maja, BACKUP, &new_pin("not-the-password", &account.key_id)).await), (403, Some("password-incorrect".into())));
    assert_eq!(refusal(&put(maja, BACKUP, &new_pin(PASSWORD, &account.key_id)).await), (409, Some("device-missing".into())));
    register_device(maja).await;
    let changed = put(maja, BACKUP, &new_pin(PASSWORD, &Id::random().to_string())).await;
    assert_eq!(refusal(&changed), (409, Some("account-key-changed".into())));
    assert_eq!(changed.json::<Value>()["accountKeyId"], account.key_id.as_str());
    // The key's id in capitals is not the id it has.
    assert_eq!(refusal(&put(maja, BACKUP, &new_pin(PASSWORD, &account.key_id.to_uppercase())).await).1, Some("account-key-changed".into()));

    // A new PIN also brings back a backup that wrong PINs deleted.
    for _ in 0..10 {
        post(maja, UNLOCK, &json!({ "authKey": text(&bytes(32, 9)) })).await;
    }
    assert_eq!(maja.get(BACKUP).await.status, 404);
    let replaced = put(maja, BACKUP, &new_pin(PASSWORD, &account.key_id)).await;
    assert_eq!(replaced.status, 200);
    let status: Value = replaced.json();
    assert_eq!((status["remainingAttempts"].clone(), status["failedAttempts"].clone()), (json!(10), json!(0)));
    assert!(replaced.text().starts_with("{\"iterations\":600000,\"failedAttempts\":0,\"remainingAttempts\":10,\"updatedAt\":\""));
    assert_eq!(refusal(&post(maja, UNLOCK, &json!({ "authKey": text(&account.auth_key) })).await).0, 403);
    assert_eq!(post(maja, UNLOCK, &json!({ "authKey": text(&new_auth) })).await.status, 200);

    // Once the account has a second step, it has to be fresh, whatever the password.
    add_second_step(&crowd.server, &crowd.maja.id);
    passed_second_step(&crowd.server, &crowd.maja.id, 15);
    for password in [PASSWORD, "not-the-password"] {
        let stale = put(maja, BACKUP, &new_pin(password, &account.key_id)).await;
        assert_eq!(refusal(&stale), (403, Some("fresh-mfa-required".into())), "{password}");
    }
    passed_second_step(&crowd.server, &crowd.maja.id, 0);
    assert_eq!(put(maja, BACKUP, &new_pin(PASSWORD, &account.key_id)).await.status, 200);
    // Fresh, and still the password decides.
    assert_eq!(refusal(&put(maja, BACKUP, &new_pin("not-the-password", &account.key_id)).await).1, Some("password-incorrect".into()));
}

#[tokio::test]
async fn a_session_that_is_not_fresh_is_told_so_after_its_body_is_read_and_before_it_is_checked() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    let account = set_up(maja).await;
    register_device(maja).await;
    add_second_step(&crowd.server, &crowd.maja.id);
    passed_second_step(&crowd.server, &crowd.maja.id, 15);
    let raw = async |method: Method, path: &str, body: &str| {
        let response = maja
            .send(crate::support::Request::new(method, path).body("application/json", body))
            .await;
        refusal(&response)
    };
    let key = text(&bytes(32, 12));
    let good = json!({ "currentPassword": PASSWORD, "algorithm": "x25519", "publicKey": key, "backup": backup(&bytes(32, 13)) }).to_string();
    let fresh = Some("fresh-mfa-required".to_string());

    // As the first server answered each of these, for a stale session.
    for (what, body, wanted) in [
        ("not JSON", "{".to_string(), (400, None)),
        ("a key in a wrong encoding", good.replace(&key, "***"), (400, None)),
        ("an integer that is not one", good.replace("600000", "\"x\""), (400, None)),
        ("null", "null".to_string(), (403, fresh.clone())),
        ("empty", String::new(), (403, fresh.clone())),
        ("an unknown algorithm", good.replace("x25519", "rsa"), (403, fresh.clone())),
        ("a wrong password", good.replace(PASSWORD, "no"), (403, fresh.clone())),
        ("a good body", good.clone(), (403, fresh.clone())),
    ] {
        assert_eq!(raw(Method::POST, RESET, &body).await, wanted, "reset: {what}");
    }
    let pin = json!({ "currentPassword": PASSWORD, "accountKeyId": account.key_id, "backup": backup(&bytes(32, 13)) }).to_string();
    assert_eq!(raw(Method::PUT, BACKUP, "{").await, (400, None));
    assert_eq!(raw(Method::PUT, BACKUP, &pin).await, (403, fresh));
    // Fresh, a body that says nothing is a 400 of its own.
    passed_second_step(&crowd.server, &crowd.maja.id, 0);
    assert_eq!(raw(Method::POST, RESET, "null").await, (400, None));
    assert_eq!(raw(Method::POST, RESET, "").await, (400, None));
}

#[tokio::test]
async fn a_new_pin_or_reset_with_no_password_given_is_the_wrong_password_not_a_fresh_proof() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    let account = set_up(maja).await;
    register_device(maja).await;

    for body in [
        json!({ "accountKeyId": account.key_id, "backup": backup(&bytes(32, 1)) }),
        json!({ "currentPassword": null, "accountKeyId": account.key_id, "backup": backup(&bytes(32, 1)) }),
        json!({ "currentPassword": "", "accountKeyId": account.key_id, "backup": backup(&bytes(32, 1)) }),
    ] {
        assert_eq!(refusal(&put(maja, BACKUP, &body).await), (403, Some("password-incorrect".into())), "{body}");
    }
    assert_eq!(read(maja, BACKUP).await["remainingAttempts"], 10);
}

#[tokio::test]
async fn a_reset_replaces_the_key_revokes_the_other_devices_and_tells_the_encrypted_conversations() {
    let crowd = Crowd::ready().await;
    let laptop = crowd.server.sign_in(&crowd.maja).await;
    register_device(&laptop).await;
    let with_ada = crowd.conversation(&crowd.maja, &crowd.ada);
    let with_ilya = crowd.conversation(&crowd.maja, &crowd.ilya);
    crowd.enable(&crowd.maja, &crowd.ada, &with_ada).await;
    assert_eq!(action(&crowd.maja.client, &with_ilya, "request").await.status, 200);
    let before = read(&crowd.maja.client, ACCOUNT).await;
    let old_key = before["accountKey"]["keyId"].as_str().unwrap().to_string();
    let own_device = before["device"]["id"].as_str().unwrap().to_string();
    let reset = |password: &str| {
        json!({ "currentPassword": password, "algorithm": "x25519", "publicKey": text(&bytes(32, 12)), "backup": backup(&bytes(32, 13)) })
    };

    assert_eq!(refusal(&post(&crowd.maja.client, RESET, &reset("not-the-password")).await), (403, Some("password-incorrect".into())));
    add_second_step(&crowd.server, &crowd.maja.id);
    passed_second_step(&crowd.server, &crowd.maja.id, 15);
    assert_eq!(refusal(&post(&crowd.maja.client, RESET, &reset(PASSWORD)).await), (403, Some("fresh-mfa-required".into())));
    passed_second_step(&crowd.server, &crowd.maja.id, 0);

    let done = post(&crowd.maja.client, RESET, &reset(PASSWORD)).await;
    assert_eq!(done.status, 200, "{}", done.text());
    let result: Value = done.json();
    assert_ne!(result["accountKey"]["keyId"], old_key.as_str());
    assert_eq!(result["revokedDevices"], 1);
    assert_eq!(result["backup"]["remainingAttempts"], 10);

    let seen = crowd.keys(&crowd.ada.client, &with_ada).await;
    assert_eq!(seen["peer"]["accountKey"]["keyId"], result["accountKey"]["keyId"]);
    let devices: Vec<&str> = seen["peer"]["devices"].as_array().unwrap().iter().map(|d| d["keyId"].as_str().unwrap()).collect();
    assert_eq!(devices, [own_device.as_str()]);
    assert!(crowd.timeline(&with_ada).contains(&"keys_reset:maja".to_string()));
    assert!(!crowd.timeline(&with_ilya).contains(&"keys_reset:maja".to_string()));

    // The laptop stays signed in, without keys until it registers again.
    let state = read(&laptop, ACCOUNT).await;
    assert_eq!(state["device"], Value::Null);
}

#[tokio::test]
async fn a_reset_with_no_key_to_replace_is_refused_after_the_password() {
    let crowd = Crowd::new().await;
    let body = |password: &str| json!({ "currentPassword": password, "algorithm": "x25519", "publicKey": text(&bytes(32, 12)), "backup": backup(&bytes(32, 13)) });

    assert_eq!(refusal(&post(&crowd.maja.client, RESET, &body("wrong")).await).1, Some("password-incorrect".into()));
    assert_eq!(refusal(&post(&crowd.maja.client, RESET, &body(PASSWORD)).await).1, Some("account-key-missing".into()));
    let kept = put(
        &crowd.maja.client,
        BACKUP,
        &json!({ "currentPassword": PASSWORD, "accountKeyId": Id::random().to_string(), "backup": backup(&bytes(32, 1)) }),
    )
    .await;
    assert_eq!(refusal(&kept).1, Some("account-key-missing".into()));
}

#[tokio::test]
async fn an_account_without_a_password_gets_new_keys_with_a_fresh_sign_in_instead() {
    let crowd = Crowd::ready().await;
    let maja = &crowd.maja.client;
    let connection = crowd.server.database();
    let user = Id::parse(&crowd.maja.id).unwrap();
    connection
        .execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1", [user])
        .unwrap();
    let signed_in_at = |minutes_ago: u64| {
        let when = Timestamp::now().minus(std::time::Duration::from_secs(minutes_ago * 60));
        connection
            .execute(
                "UPDATE \"UserSessions\" SET \"AuthenticatedAt\" = ?2, \"CreatedAt\" = ?2 WHERE \"UserId\" = ?1",
                params![user, when],
            )
            .unwrap();
    };
    let reset = json!({ "currentPassword": "", "algorithm": "x25519", "publicKey": text(&bytes(32, 12)), "backup": backup(&bytes(32, 13)) });

    // No password to give: an old sign-in gets the refusal the client answers with a fresh one.
    signed_in_at(60);
    assert_eq!(refusal(&post(maja, RESET, &reset).await), (403, Some("fresh-mfa-required".into())));
    signed_in_at(0);
    assert_eq!(post(maja, RESET, &reset).await.status, 200);
}

#[tokio::test]
async fn a_device_lives_as_long_as_its_session_and_registering_it_again_changes_nothing() {
    let crowd = Crowd::new().await;
    let (maja, ada) = (&crowd.maja.client, &crowd.ada.client);
    assert_eq!(refusal(&register_device_with(maja, &bytes(32, 6), "Firefox, Linux").await), (409, Some("account-key-missing".into())));
    set_up(maja).await;
    set_up(ada).await;
    register_device(ada).await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;

    let key = bytes(32, 20);
    let first = register_device_with(maja, &key, "Firefox, Linux").await;
    assert_eq!(first.status, 201);
    assert_eq!(first.header("location"), Some("/api/me/encryption/devices"));
    let device = first.json::<Value>()["id"].as_str().unwrap().to_string();
    let again = register_device_with(maja, &key, "Firefox, Linux (renamed)").await;
    assert_eq!(again.status, 200);
    let again: Value = again.json();
    assert_eq!((again["id"].as_str(), again["label"].as_str()), (Some(device.as_str()), Some("Firefox, Linux (renamed)")));
    assert_eq!(register_device_with(maja, &bytes(32, 21), " ").await.status, 400);

    // A new key for the same browser replaces its device.
    let replaced = register_device_with(maja, &bytes(32, 22), "Firefox, Linux").await;
    assert_eq!(replaced.status, 201);
    let device = replaced.json::<Value>()["id"].as_str().unwrap().to_string();
    let added = crowd.timeline(&conversation).into_iter().filter(|line| line.starts_with("device_added")).count();
    assert_eq!(added, 2);

    let laptop = crowd.server.sign_in(&crowd.maja).await;
    let laptop_device = register_device(&laptop).await;
    let phone = crowd.server.sign_in(&crowd.maja).await;
    let phone_device = register_device(&phone).await;
    let own = read(maja, &format!("{ACCOUNT}/devices")).await;
    let currents: Vec<bool> = own["devices"].as_array().unwrap().iter().map(|d| d["current"].as_bool().unwrap()).collect();
    assert_eq!(currents, [true, false, false]);
    let peer_devices = |keys: &Value| -> Vec<String> {
        keys["peer"]["devices"].as_array().unwrap().iter().map(|d| d["keyId"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(peer_devices(&crowd.keys(ada, &conversation).await), [device.clone(), laptop_device.clone(), phone_device.clone()]);

    // Removing the keys of another browser signs it out; one's own are removed by signing out.
    assert_eq!(delete(maja, &format!("{ACCOUNT}/devices/{phone_device}")).await.status, 204);
    assert_eq!(phone.get("/api/me").await.status, 401);
    assert_eq!(delete(maja, &format!("{ACCOUNT}/devices/{phone_device}")).await.status, 404);
    assert_eq!(refusal(&delete(maja, &format!("{ACCOUNT}/devices/{device}")).await), (409, Some("current-device".into())));
    assert_eq!(delete(ada, &format!("{ACCOUNT}/devices/{device}")).await.status, 404);
    assert_eq!(delete(maja, &format!("{ACCOUNT}/devices/not-an-id")).await.status, 404);
    assert_eq!(peer_devices(&crowd.keys(ada, &conversation).await), [device, laptop_device]);
    let recorded: i64 = crowd
        .server
        .database()
        .query_row(
            "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'session-revoked' AND \"Detail\" = 'encryption keys removed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recorded, 1);
    let notified: i64 = crowd.server.database().query_row("SELECT COUNT(*) FROM \"SecurityNotifications\"", [], |row| row.get(0)).unwrap();
    assert_eq!(notified, 0, "the removal of keys sends no mail");
}

#[tokio::test]
async fn a_new_device_is_announced_only_in_the_accounts_encrypted_conversations() {
    let crowd = Crowd::ready().await;
    let with_ada = crowd.conversation(&crowd.maja, &crowd.ada);
    let with_ilya = crowd.conversation(&crowd.maja, &crowd.ilya);
    let ada_ilya = crowd.conversation(&crowd.ada, &crowd.ilya);
    crowd.enable(&crowd.maja, &crowd.ada, &with_ada).await;
    crowd.enable(&crowd.ada, &crowd.ilya, &ada_ilya).await;
    assert_eq!(action(&crowd.maja.client, &with_ilya, "request").await.status, 200);

    let laptop = crowd.server.sign_in(&crowd.maja).await;
    let key = bytes(32, 30);
    assert_eq!(register_device_with(&laptop, &key, "Laptop").await.status, 201);
    assert_eq!(register_device_with(&laptop, &key, "Laptop").await.status, 200);

    assert_eq!(crowd.timeline(&with_ada), ["encryption_requested:maja", "encryption_enabled:ada", "device_added:maja"]);
    assert_eq!(crowd.timeline(&with_ilya), ["encryption_requested:maja"]);
    assert_eq!(crowd.timeline(&ada_ilya), ["encryption_requested:ada", "encryption_enabled:ilya"]);
}

#[tokio::test]
async fn a_route_that_needs_a_session_answers_nobody_with_401() {
    let crowd = Crowd::new().await;
    let nobody = crowd.server.client();
    for (method, path) in [
        (Method::GET, ACCOUNT.to_string()),
        (Method::POST, format!("{ACCOUNT}/account-key")),
        (Method::POST, RESET.to_string()),
        (Method::GET, BACKUP.to_string()),
        (Method::PUT, BACKUP.to_string()),
        (Method::POST, UNLOCK.to_string()),
        (Method::GET, format!("{ACCOUNT}/devices")),
        (Method::POST, format!("{ACCOUNT}/devices")),
        (Method::DELETE, format!("{ACCOUNT}/devices/{}", Id::random())),
        (Method::POST, format!("/api/social/conversations/{}/encryption/request", Id::random())),
        (Method::GET, format!("/api/social/conversations/{}/encryption/keys", Id::random())),
    ] {
        let response = send(&nobody, method.clone(), &path, Some(&json!({}))).await;
        assert_eq!((response.status.as_u16(), response.body.len()), (401, 0), "{method} {path}");
    }
}
