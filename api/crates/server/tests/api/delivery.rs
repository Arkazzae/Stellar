//! The routes a browser subscribes to push with, and the worker that sends
//! what is owed, with the real binary.
//!
//! What a push service does with a request is held in the unit tests of the
//! module, which can wire a sender to a fake of their own: the binary only
//! ever connects to public addresses, which a test cannot be.

use std::time::Duration;

use http::Method;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use stellarr_webpush::{base64url, generate_keys};

use crate::support::{Client, Person, Request, Server};

struct Keys {
    public: String,
    private: String,
}

fn vapid() -> Keys {
    let made = generate_keys().unwrap();
    Keys {
        public: made.public_key,
        private: made.private_key,
    }
}

fn push_settings(keys: &Keys) -> Vec<(&'static str, String)> {
    vec![
        ("STELLARR_PUSH_ENABLED", "true".to_string()),
        ("STELLARR_PUSH_SUBJECT", "mailto:push@example.test".to_string()),
        ("STELLARR_PUSH_PUBLIC_KEY", keys.public.clone()),
        ("STELLARR_PUSH_PRIVATE_KEY", keys.private.clone()),
    ]
}

async fn start_with_push(keys: &Keys) -> Server {
    let settings = push_settings(keys);
    let borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
    Server::start_with(&borrowed).await
}

/// What a browser hands in when it subscribes.
fn browser() -> (String, String) {
    let point = generate_keys().unwrap().public_key;
    let mut auth = [0u8; 16];
    aws_lc_rs::rand::fill(&mut auth).unwrap();
    (point, base64url::encode(&auth))
}

fn subscription(endpoint: &str, keys: &(String, String)) -> Value {
    json!({ "endpoint": endpoint, "keys": { "p256dh": keys.0, "auth": keys.1 } })
}

async fn put(client: &Client, body: &Value) -> crate::support::Response {
    client
        .send(Request::new(Method::PUT, "/api/me/push-subscriptions").json(body))
        .await
}

async fn delete(client: &Client, endpoint: &str) -> crate::support::Response {
    client
        .send(Request::new(Method::DELETE, "/api/me/push-subscriptions").json(&json!({ "endpoint": endpoint })))
        .await
}

fn rows(server: &Server, endpoint: &str) -> Vec<(String, String, String)> {
    let db = server.database();
    let mut statement = db
        .prepare("SELECT \"Id\", \"UserId\", \"SessionId\" FROM \"PushSubscriptions\" WHERE \"Endpoint\" = ?1")
        .unwrap();
    statement
        .query_map([endpoint], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The id of the session a client's cookie names, found as the newest one of
/// the account.
fn latest_session(server: &Server, person: &Person) -> String {
    server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"UserSessions\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\" DESC, rowid DESC LIMIT 1",
            [person.id.to_uppercase()],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn the_capabilities_are_the_accounts_own_and_say_what_is_configured() {
    let keys = vapid();
    let server = start_with_push(&keys).await;
    assert_eq!(server.anonymous().get("/api/me/notification-delivery").await.status, 401);
    let owner = server.found("owner").await;

    let response = owner.client.get("/api/me/notification-delivery").await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(
        response.text(),
        format!(
            "{{\"emailAvailable\":false,\"emailVerified\":false,\"pushAvailable\":true,\"applicationServerKey\":\"{}\"}}",
            keys.public
        )
    );
    assert!(!response.text().contains(&keys.private));
}

#[tokio::test]
async fn a_pair_that_is_wrong_or_missing_makes_push_unavailable_and_stops_nothing() {
    let good = vapid();
    let other = vapid();
    for settings in [
        vec![],
        vec![("STELLARR_PUSH_ENABLED", "true".to_string()), ("STELLARR_PUSH_SUBJECT", "mailto:a@b.test".into()),
             ("STELLARR_PUSH_PUBLIC_KEY", "not-a-key".into()), ("STELLARR_PUSH_PRIVATE_KEY", "not-a-key".into())],
        vec![("STELLARR_PUSH_ENABLED", "true".to_string()), ("STELLARR_PUSH_SUBJECT", "mailto:a@b.test".into()),
             ("STELLARR_PUSH_PUBLIC_KEY", good.public.clone()), ("STELLARR_PUSH_PRIVATE_KEY", other.private.clone())],
        vec![("STELLARR_PUSH_ENABLED", "false".to_string()), ("STELLARR_PUSH_SUBJECT", "mailto:a@b.test".into()),
             ("STELLARR_PUSH_PUBLIC_KEY", good.public.clone()), ("STELLARR_PUSH_PRIVATE_KEY", good.private.clone())],
        vec![("STELLARR_PUSH_ENABLED", "true".to_string()), ("STELLARR_PUSH_SUBJECT", "gopher://x".into()),
             ("STELLARR_PUSH_PUBLIC_KEY", good.public.clone()), ("STELLARR_PUSH_PRIVATE_KEY", good.private.clone())],
    ] {
        let borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let server = Server::start_with(&borrowed).await;
        let owner = server.found("owner").await;

        let body = owner.client.get("/api/me/notification-delivery").await.json::<Value>();

        assert_eq!(body["pushAvailable"], false, "{settings:?}");
        assert_eq!(body["applicationServerKey"], Value::Null);
    }
}

#[tokio::test]
async fn a_browser_is_subscribed_once_and_the_same_row_answers_again() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let keys = browser();
    let endpoint = "https://push.example.test/send/first";

    let first = put(&owner.client, &subscription(endpoint, &keys)).await;
    let again = put(&owner.client, &subscription(endpoint, &keys)).await;

    assert_eq!(first.status, 200);
    assert_eq!(first.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(again.json::<Value>(), first.json::<Value>());
    let stored = rows(&server, endpoint);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0.to_lowercase(), first.json::<Value>()["id"].as_str().unwrap());
}

#[tokio::test]
async fn an_endpoint_is_never_given_to_another_account_or_a_live_sign_in() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let ada = server.register("ada").await;
    let keys = browser();
    let endpoint = "https://push.example.test/send/owned";
    let original = put(&owner.client, &subscription(endpoint, &keys)).await.json::<Value>();

    assert_eq!(put(&ada.client, &subscription(endpoint, &keys)).await.status, 409);
    let second_sign_in = server.sign_in(&owner).await;
    assert_eq!(put(&second_sign_in, &subscription(endpoint, &keys)).await.status, 409);

    assert_eq!(rows(&server, endpoint).len(), 1);
    assert_eq!(put(&owner.client, &subscription(endpoint, &keys)).await.json::<Value>(), original);
}

#[tokio::test]
async fn an_endpoint_moves_to_a_new_sign_in_once_the_old_one_has_ended() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let keys = browser();
    let endpoint = "https://push.example.test/send/rebind";
    let original = put(&owner.client, &subscription(endpoint, &keys)).await.json::<Value>();
    let old = rows(&server, endpoint)[0].2.clone();
    server
        .database()
        .execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1", params![old, "2026-01-01 00:00:00"])
        .unwrap();
    let second = server.sign_in(&owner).await;

    let moved = put(&second, &subscription(endpoint, &keys)).await;

    assert_eq!(moved.status, 200);
    assert_eq!(moved.json::<Value>(), original);
    let now = rows(&server, endpoint);
    assert_eq!(now.len(), 1);
    assert_ne!(now[0].2, old);
    assert_eq!(now[0].2, latest_session(&server, &owner));
}

#[tokio::test]
async fn a_person_has_at_most_ten_browsers() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let keys = browser();
    for index in 0..10 {
        let response = put(&owner.client, &subscription(&format!("https://push.example.test/send/{index}"), &keys)).await;
        assert_eq!(response.status, 200);
    }

    let refused = put(&owner.client, &subscription("https://push.example.test/send/one-too-many", &keys)).await;
    let again = put(&owner.client, &subscription("https://push.example.test/send/3", &keys)).await;

    assert_eq!(refused.status, 409);
    assert_eq!(refused.text(), "");
    assert_eq!(again.status, 200, "an endpoint already held is not a new one");
}

#[tokio::test]
async fn a_subscription_that_is_not_valid_is_a_validation_problem() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let keys = browser();
    for endpoint in [
        "http://push.example.test/send/plain",
        "https://user:secret@push.example.test/send/userinfo",
        "https://push.example.test/send/fragment#part",
        "https://push.example.test:8443/send/port",
        "https://localhost/send/local",
        "https://127.0.0.1/send/loopback",
        "https://[::1]/send/loopback-v6",
        "https://10.0.0.1/send/private",
        "https://192.168.1.1/send/private-v4",
        "https://169.254.1.1/send/link-local",
        "https://2130706433/send/decimal",
        "https://0x7f.1/send/hex",
        "https://[::ffff:127.0.0.1]/send/mapped",
        "https://[64:ff9b::7f00:1]/send/nat64",
        "https://printer.local/send",
        "ftp://push.example.test/send",
        "",
        "   ",
    ] {
        let response = put(&owner.client, &subscription(endpoint, &keys)).await;

        assert_eq!(response.status, 400, "{endpoint}");
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        let body = response.json::<Value>();
        assert_eq!(body["title"], "One or more validation errors occurred.");
        assert_eq!(
            body["errors"]["subscription"],
            json!(["The push subscription endpoint or its keys are not valid."])
        );
    }
    let valid = "https://push.example.test/send/keys";
    let too_long = format!("https://push.example.test/{}", "a".repeat(2100));
    let off_curve = base64url::encode(&{
        let mut bytes = vec![0u8; 65];
        bytes[0] = 4;
        bytes
    });
    for (endpoint, p256dh, auth) in [
        (valid, "not-base64!".to_string(), keys.1.clone()),
        (valid, base64url::encode(&[0u8; 64]), keys.1.clone()),
        (valid, off_curve, keys.1.clone()),
        (valid, keys.0.clone(), base64url::encode(&[0u8; 15])),
        (valid, keys.0.clone(), format!("{}+", keys.1)),
        (valid, "A".repeat(300), keys.1.clone()),
        (valid, keys.0.clone(), "A".repeat(70)),
        (too_long.as_str(), keys.0.clone(), keys.1.clone()),
    ] {
        let body = json!({ "endpoint": endpoint, "keys": { "p256dh": p256dh, "auth": auth } });
        assert_eq!(put(&owner.client, &body).await.status, 400, "{endpoint:.40} {p256dh:.20} {auth:.20}");
    }
    assert!(rows(&server, valid).is_empty());
}

#[tokio::test]
async fn a_body_that_is_missing_or_of_the_wrong_shape_is_refused_as_the_first_server_did() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let validation = |response: &crate::support::Response| {
        response.status == 400 && response.header("content-type") == Some("application/problem+json")
    };
    for body in ["", "null", "{}", "{\"endpoint\":null}", "{\"keys\":null}", "{\"keys\":{}}"] {
        let response = owner
            .client
            .send(Request::new(Method::PUT, "/api/me/push-subscriptions").body("application/json", body))
            .await;
        assert!(validation(&response), "{body:?}: {}", response.status);
    }
    for body in ["{bad", "{\"endpoint\":5}", "[]", "\"text\"", "{\"endpoint\":\"x\",\"keys\":5}"] {
        let response = owner
            .client
            .send(Request::new(Method::PUT, "/api/me/push-subscriptions").body("application/json", body))
            .await;
        assert_eq!((response.status.as_u16(), response.text()), (400, String::new()), "{body:?}");
    }
    let wrong_type = owner
        .client
        .send(Request::new(Method::PUT, "/api/me/push-subscriptions").body("text/plain", "{}"))
        .await;
    assert_eq!(wrong_type.status, 415);
    let none = owner
        .client
        .send(Request::new(Method::PUT, "/api/me/push-subscriptions"))
        .await;
    assert!(validation(&none), "{}", none.status);
    let removal = owner
        .client
        .send(Request::new(Method::DELETE, "/api/me/push-subscriptions").json(&json!({})))
        .await;
    assert!(validation(&removal));
}

#[tokio::test]
async fn only_the_sign_in_that_made_a_subscription_removes_it() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let other_device = server.sign_in(&owner).await;
    let ada = server.register("ada").await;
    let keys = browser();
    let endpoint = "https://push.example.test/send/delete";
    assert_eq!(put(&owner.client, &subscription(endpoint, &keys)).await.status, 200);

    for client in [&ada.client, &other_device] {
        let response = delete(client, endpoint).await;
        assert_eq!((response.status.as_u16(), response.text()), (204, String::new()));
    }
    assert_eq!(rows(&server, endpoint).len(), 1);

    let removed = delete(&owner.client, endpoint).await;
    assert_eq!(removed.status, 204);
    assert!(rows(&server, endpoint).is_empty());
    // Removing what is not there is the same answer.
    assert_eq!(delete(&owner.client, endpoint).await.status, 204);
    // The endpoint is normalized as it was when it was saved.
    assert_eq!(put(&owner.client, &subscription(endpoint, &keys)).await.status, 200);
    assert_eq!(delete(&owner.client, "HTTPS://Push.Example.TEST:443/send/delete").await.status, 204);
    assert!(rows(&server, endpoint).is_empty());
}

#[tokio::test]
async fn a_sign_in_that_has_ended_can_neither_read_nor_change_delivery() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let keys = browser();
    server
        .database()
        .execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1", ["2026-01-01 00:00:00"])
        .unwrap();

    assert_eq!(owner.client.get("/api/me/notification-delivery").await.status, 401);
    assert_eq!(put(&owner.client, &subscription("https://push.example.test/x", &keys)).await.status, 401);
    assert_eq!(delete(&owner.client, "https://push.example.test/x").await.status, 401);
}

#[tokio::test]
async fn mail_and_the_verified_address_are_reported_for_the_account() {
    let server = start_with_push(&vapid()).await;
    let owner = server.found("owner").await;
    let before = owner.client.get("/api/me/notification-delivery").await.json::<Value>();
    assert_eq!((before["emailAvailable"].as_bool(), before["emailVerified"].as_bool()), (Some(false), Some(false)));
    let id = owner.id.to_uppercase();
    server
        .database()
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"EmailAddress\")
             SELECT ?1, \"Id\", 'email', 'E-mail', ?2, ?2, \"NormalizedEmail\" FROM \"Users\" WHERE \"Id\" = ?3",
            params![stellarr_api::values::Id::random(), "2026-01-01 00:00:00", id],
        )
        .unwrap();

    let after = owner.client.get("/api/me/notification-delivery").await.json::<Value>();

    assert_eq!(after["emailVerified"], true);
}

/// A row owed to the person for a message that is not there, which the
/// worker cancels without calling anybody.
fn owe_a_ghost(server: &Server, person: &Person) -> String {
    let db = server.database();
    let id = stellarr_api::values::Id::random();
    db.execute(
        "INSERT INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\",
             \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\",
             \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"MutedChannelIdsJson\", \"VadSensitivity\")
         SELECT ?1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, '[]', 60 WHERE NOT EXISTS (SELECT 1 FROM \"UserPreferences\" WHERE \"UserId\" = ?1)",
        [person.id.to_uppercase()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"NotificationDeliveries\" (\"Id\", \"SourceMessageId\", \"RecipientId\", \"EventKind\", \"Channel\", \"DestinationKey\",
             \"PushSubscriptionId\", \"CreatedAt\", \"ExpiresAt\", \"Attempts\", \"NextAttemptAt\", \"Status\", \"LastError\")
         VALUES (?1, ?2, ?3, 'mention', 'push', 'ghost', ?2, ?4, ?5, 0, ?4, 'pending', '')",
        params![id, stellarr_api::values::Id::random(), person.id.to_uppercase(), "2026-01-01 00:00:00", "2099-01-01 00:00:00"],
    )
    .unwrap();
    id.to_string()
}

#[tokio::test]
async fn the_worker_looks_at_the_outbox_when_it_starts_and_a_stopped_one_does_not() {
    let keys = vapid();
    let first = start_with_push(&keys).await;
    let owner = first.found("owner").await;
    let row = owe_a_ghost(&first, &owner);
    // Without the worker named, nothing looks: the row waits.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let status = |server: &Server| -> Option<String> {
        server
            .database()
            .query_row("SELECT \"Status\" FROM \"NotificationDeliveries\" WHERE \"Id\" = ?1", [row.to_uppercase()], |r| r.get(0))
            .optional()
            .unwrap()
    };
    assert_eq!(status(&first).as_deref(), Some("pending"));
    let folder = first.folder().to_path_buf();

    let settings = push_settings(&keys);
    let mut borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
    borrowed.push(("Testing__Workers", "notification-delivery"));
    let second = Server::start_in(&folder, &borrowed).await;
    for _ in 0..100 {
        if status(&second).as_deref() == Some("cancelled") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(status(&second).as_deref(), Some("cancelled"));
}

/// The worker of this server on a copy of a database that the first server
/// also worked on, sending to a push service of the comparison's own.
///
/// Not part of the ordinary run: `tests/delivery-wire-oracle.py` starts it
/// with the variables it reads below. The sender is wired to the fake on the
/// loopback (the only way a test can reach one); everything else is the
/// server's own.
#[tokio::test]
#[ignore = "run by tests/delivery-wire-oracle.py"]
async fn oracle_worker() {
    use std::sync::Arc;

    use stellarr_api::Services;
    use stellarr_api::delivery::sender::{Sender, Wiring};
    use stellarr_outbound::{AddressPolicy, TableResolver, tls};

    let database = std::env::var("DELIVERY_DB").expect("DELIVERY_DB");
    let port: u16 = std::env::var("FAKE_PORT").expect("FAKE_PORT").parse().unwrap();
    let ca = std::env::var("FAKE_CA").expect("FAKE_CA");
    let seconds: u64 = std::env::var("RUN_SECONDS").map_or(8, |text| text.parse().unwrap());
    let settings = [
        ("STELLARR_PUSH_ENABLED", "true".to_string()),
        ("STELLARR_PUSH_SUBJECT", "mailto:push@example.test".to_string()),
        ("STELLARR_PUSH_PUBLIC_KEY", std::env::var("PUSH_PUBLIC_KEY").unwrap()),
        ("STELLARR_PUSH_PRIVATE_KEY", std::env::var("PUSH_PRIVATE_KEY").unwrap()),
        ("Operations__PublicUrl", std::env::var("PUBLIC_URL").unwrap_or_default()),
        ("STELLARR_LANGUAGE", std::env::var("LANGUAGE").unwrap_or_else(|_| "en".into())),
        ("ConnectionStrings__Database", format!("Data Source={database};Pooling=True")),
    ];
    let borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut variables: Vec<(String, String)> = borrowed.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    variables.push(("ASPNETCORE_ENVIRONMENT".into(), "Testing".into()));
    let push = stellarr_api::settings::Settings::load(&variables).unwrap().push;
    let sender = Sender::wired(
        &push,
        Wiring {
            roots: Some(tls::roots_from_file(std::path::Path::new(&ca)).unwrap()),
            resolver: Some(Arc::new(TableResolver(vec![(
                "push.example.test".to_string(),
                vec!["127.0.0.1".parse().unwrap()],
            )]))),
            policy: AddressPolicy::LoopbackOnly,
            port: Some(port),
        },
    );
    let services = Services {
        workers: stellarr_api::delivery::workers_with(Some(Arc::new(sender))),
        ..Services::default()
    };

    let server = Server::in_process_with(stellarr_api::http::Endpoints::new(), services, &borrowed).await;
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    server.stop().await;
}
