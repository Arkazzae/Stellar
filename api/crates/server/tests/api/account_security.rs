//! What a signed-in person does to the security of their own account: the
//! security page, the second step's methods, recovery codes, remembered
//! browsers, sessions, the password and the address.
//!
//! The accounts are made through the routes that exist (founding, registering,
//! signing in) and given what the test needs by writing rows, as the
//! routes that would give it are another group's: a second step, a passkey, a
//! session that passed it a while ago. The one test that goes the whole way,
//! from the page's request for a challenge to the method that is then on the
//! page, runs the server in this process with a route of its own that calls
//! the second step's ceremony.
//!
//! The first server's answers to the same requests are compared by
//! `acctsec-shared-database.sh`, which runs both on copies of one file.

use std::time::Duration;

use http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, HubClient, PASSWORD, Person, Request, Response, Server};

const SECURE: &str = "/api/me/security";
const COOKIE: &str = "stellarr-dev-session";
const NIL: &str = "00000000-0000-0000-0000-000000000000";
const WATCHER: [(&str, &str); 1] = [("Testing__Workers", "realtime-sessions")];
const NEW_PASSWORD: &str = "A-long-new-password";

// ---------------------------------------------------------------------------
// What the tests share.

fn user_of(person: &Person) -> Id {
    Id::parse(&person.id).expect("an id")
}

fn ago(seconds: u64) -> Timestamp {
    Timestamp::now().minus(Duration::from_secs(seconds))
}

fn add_method(db: &Connection, user: Id, kind: &str, address: Option<&str>, age_days: u64) -> Id {
    let id = Id::random();
    let created = ago(age_days * 86_400);
    db.execute(
        "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"EmailAddress\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![id, user, kind, format!("My {kind}"), created, address],
    )
    .expect("a method is added");
    id
}

fn add_codes(db: &Connection, user: Id, count: usize) {
    let batch = Id::random();
    for _ in 0..count {
        db.execute(
            "INSERT INTO \"RecoveryCodes\" (\"Id\", \"UserId\", \"BatchId\", \"CodeHash\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, ?5)",
            params![Id::random(), user, batch, Id::random().to_string(), Timestamp::now()],
        )
        .unwrap();
    }
}

fn add_device(db: &Connection, user: Id) -> Id {
    let id = Id::random();
    db.execute(
        "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\")
         VALUES (?1, ?2, ?3, 'Kitchen laptop', 'Firefox on Linux', '203.0.113.5', ?4, ?5)",
        params![id, user, Id::random().to_string(), ago(3600), Timestamp::now().plus(Duration::from_secs(86_400))],
    )
    .unwrap();
    id
}

fn add_identity(db: &Connection, user: Id, issuer: &str, age: u64) {
    db.execute(
        "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        params![Id::random(), user, issuer, Id::random().to_string(), ago(age)],
    )
    .unwrap();
}

/// What a session has passed: a sign-in `signed_in` seconds ago, and a second
/// step `second_step` seconds ago if it passed one.
fn set_session(db: &Connection, session: Id, signed_in: u64, second_step: Option<u64>) {
    db.execute(
        "UPDATE \"UserSessions\" SET \"AuthenticatedAt\" = ?2, \"MfaCompletedAt\" = ?3, \"MfaEvidence\" = ?4 WHERE \"Id\" = ?1",
        params![session, ago(signed_in), second_step.map(ago), second_step.map(|_| "totp")],
    )
    .unwrap();
}

fn remove_password(db: &Connection, user: Id) {
    db.execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1", [user]).unwrap();
}

/// The kind and detail of every security event of the account, oldest first.
fn events_of(db: &Connection, user: Id) -> Vec<(String, String)> {
    let mut statement = db
        .prepare("SELECT \"Kind\", \"Detail\" FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\", rowid")
        .unwrap();
    statement
        .query_map([user], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// What the events of the account added since `before` were.
fn new_events(db: &Connection, user: Id, before: usize) -> Vec<(String, String)> {
    events_of(db, user).split_off(before)
}

fn count(db: &Connection, sql: &str, user: Id) -> i64 {
    db.query_row(sql, [user], |row| row.get(0)).unwrap()
}

async fn send(client: &Client, method: Method, path: &str, body: Option<&Value>) -> Response {
    let mut request = Request::new(method, path);
    if let Some(body) = body {
        request = request.json(body);
    }
    client.send(request).await
}

async fn page(client: &Client) -> Value {
    let response = client.get(SECURE).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

/// The id of the session the client's cookie names.
async fn session_of(client: &Client) -> Id {
    let sessions: Vec<Value> = client.get(&format!("{SECURE}/sessions")).await.json();
    let current = sessions.iter().find(|session| session["current"] == true).expect("a current session");
    Id::parse(current["id"].as_str().unwrap()).unwrap()
}

/// A hub connection for the client's session, settled.
async fn connect(server: &Server, client: &Client) -> HubClient {
    let cookie = format!("{COOKIE}={}", client.cookie(COOKIE).expect("signed in"));
    let mut connection = HubClient::connect(server.address(), &cookie).await;
    connection.event("PresenceChanged").await;
    connection
}

/// The hub says goodbye to a connection within a few seconds.
async fn closes(connection: &mut HubClient) {
    let started = std::time::Instant::now();
    let _ = connection.until_close().await;
    assert!(started.elapsed() < Duration::from_secs(5), "closed after {:?}", started.elapsed());
}

/// A connection that nothing has closed answers a call.
async fn alive(connection: &mut HubClient) {
    assert_eq!(connection.invoke("GetRtcClock", json!([])).await["type"], 3);
}

fn title(response: &Response) -> String {
    response.json::<Value>()["title"].as_str().unwrap_or_default().to_string()
}

const NEEDS_A_STEP: &str = "This needs a second step from the last 5 minutes. Confirm it and try again.";
const FRESH_PROOF: &str = "Confirm a current second factor before changing security settings.";
const UNAVAILABLE: &str = "That security method is not available for this account.";

/// The person founding the instance, signed in just now and with nothing set
/// up, and the database to arrange what else they have.
async fn owner() -> (Server, Person, Connection) {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let db = server.database();
    (server, owner, db)
}

// ---------------------------------------------------------------------------
// Reading: the page, the events, the sessions.

#[tokio::test]
async fn nobody_signed_in_is_refused_on_every_route() {
    let server = Server::start().await;
    let client = server.client();
    for (method, path) in [
        (Method::GET, SECURE.to_string()),
        (Method::GET, format!("{SECURE}/events")),
        (Method::POST, format!("{SECURE}/mfa/enroll/start")),
        (Method::POST, format!("{SECURE}/mfa/reverify/start")),
        (Method::POST, format!("{SECURE}/mfa/{NIL}/remove")),
        (Method::POST, format!("{SECURE}/recovery/regenerate")),
        (Method::DELETE, format!("{SECURE}/trusted/{NIL}")),
        (Method::GET, format!("{SECURE}/sessions")),
        (Method::DELETE, format!("{SECURE}/sessions/{NIL}")),
        (Method::POST, format!("{SECURE}/sessions/revoke-others")),
        (Method::POST, format!("{SECURE}/password")),
        (Method::POST, "/api/me/password".to_string()),
        (Method::POST, "/api/me/email".to_string()),
    ] {
        let response = send(&client, method.clone(), &path, Some(&json!({}))).await;
        assert_eq!(response.status, 401, "{method} {path}");
        assert!(response.body.is_empty(), "{method} {path}");
    }
}

#[tokio::test]
async fn the_page_of_an_account_with_nothing_set_up_is_written_member_by_member() {
    let (_server, owner, _db) = owner().await;

    let response = owner.client.get(SECURE).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.header("pragma"), Some("no-cache"));
    let text = response.text();
    let (head, rest) = text.split_once("\"recentEvents\":").expect("the events");
    assert_eq!(
        head,
        "{\"requirement\":\"optional\",\"mfaRequired\":false,\"freshMfa\":false,\"mfaEvidence\":null,\"mfaCompletedAt\":null,\
         \"trustedDeviceAvailable\":false,\"methods\":[],\"remainingRecoveryCodes\":0,\"trustedDevices\":[],"
    );
    let (events, tail) = rest.split_once("],\"freshMfaWindowSeconds\"").expect("what follows the events");
    assert!(events.starts_with('['), "{events}");
    assert_eq!(
        tail,
        ":300,\"trustedDeviceDays\":14,\"emailCodeAvailable\":false,\"emailResendCooldownSeconds\":45,\
         \"securityRecoveryRequired\":false,\"passkeyAvailable\":false,\"identities\":[],\"identityProviders\":[],\"hasPassword\":true}"
    );
}

#[tokio::test]
async fn the_page_lists_what_the_account_has() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    let totp = add_method(&db, user, "totp", None, 5);
    let mail = add_method(&db, user, "email", Some("OWNER@EXAMPLE.TEST"), 3);
    let passkey = add_method(&db, user, "passkey", Some("not shown"), 1);
    // A method that was never confirmed, and one that was taken back, are not there.
    db.execute("UPDATE \"MfaMethods\" SET \"ConfirmedAt\" = NULL WHERE \"Id\" = ?1", [add_method(&db, user, "totp", None, 0)]).unwrap();
    db.execute("UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1", params![add_method(&db, user, "totp", None, 0), Timestamp::now()]).unwrap();
    add_codes(&db, user, 7);
    db.execute("UPDATE \"RecoveryCodes\" SET \"UsedAt\" = ?1 WHERE rowid IN (SELECT rowid FROM \"RecoveryCodes\" LIMIT 2)", [Timestamp::now()]).unwrap();
    let device = add_device(&db, user);
    // A device that ran out, and one that was forgotten.
    db.execute(
        "INSERT INTO \"TrustedDevices\" (\"Id\", \"UserId\", \"TokenHash\", \"Name\", \"Device\", \"IpAddress\", \"CreatedAt\", \"ExpiresAt\") VALUES (?1, ?2, 'x', 'Old', 'd', 'i', ?3, ?3)",
        params![Id::random(), user, ago(86_400)],
    )
    .unwrap();
    add_identity(&db, user, "https://Id.Example.test:8443/realms/main", 60);
    add_identity(&db, user, "not an address", 30);
    for n in 0..25 {
        db.execute(
            "INSERT INTO \"SecurityEvents\" (\"Id\", \"UserId\", \"Kind\", \"Detail\", \"IpAddress\", \"Device\", \"CreatedAt\") VALUES (?1, ?2, 'login-succeeded', ?3, '203.0.113.7', 'Firefox', ?4)",
            params![Id::random(), user, format!("event {n}"), ago(1000 - n)],
        )
        .unwrap();
    }

    let page = page(&owner.client).await;

    let methods = page["methods"].as_array().unwrap();
    assert_eq!(methods.len(), 3, "oldest first, confirmed and in use only");
    assert_eq!(methods[0]["id"], totp.to_string());
    assert_eq!(methods[0]["type"], "totp");
    assert_eq!(methods[0]["name"], "My totp");
    assert_eq!(methods[0]["emailAddress"], Value::Null);
    assert_eq!(methods[0]["pending"], false);
    assert_eq!(methods[1]["id"], mail.to_string());
    assert_eq!(methods[1]["emailAddress"], "OWNER@EXAMPLE.TEST");
    assert_eq!(methods[2]["id"], passkey.to_string());
    assert_eq!(methods[2]["emailAddress"], Value::Null, "only a mail method shows its address");
    assert_eq!(page["mfaRequired"], true, "an account with a method is asked for it");
    assert_eq!(page["emailCodeAvailable"], true);
    assert_eq!(page["remainingRecoveryCodes"], 5);
    let devices = page["trustedDevices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["id"], device.to_string());
    assert_eq!(devices[0]["name"], "Kitchen laptop");
    assert_eq!(devices[0]["device"], "Firefox on Linux");
    assert!(devices[0].get("ipAddress").is_none());
    let events = page["recentEvents"].as_array().unwrap();
    assert_eq!(events.len(), 20);
    assert_eq!(events[0]["detail"], "event 24", "newest first");
    // No provider is set up on this server, so an identity is named by its issuer.
    let identities = page["identities"].as_array().unwrap();
    assert_eq!(identities.len(), 2);
    assert_eq!(identities[0]["name"], "id.example.test");
    assert_eq!(identities[0]["providerId"], "default");
    assert_eq!(identities[1]["name"], "not an address");
    assert_eq!(page["identityProviders"], json!([]));
}

#[tokio::test]
async fn the_page_says_whether_this_session_passed_the_second_step_lately() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    let second_client = server.sign_in(&owner).await;
    add_method(&db, user, "totp", None, 1);
    let first = session_of(&owner.client).await;
    let second = session_of(&second_client).await;
    set_session(&db, first, 10, Some(60));
    set_session(&db, second, 10, Some(301));

    let fresh = page(&owner.client).await;
    let stale = page(&second_client).await;

    assert_eq!(fresh["freshMfa"], true);
    assert_eq!(fresh["mfaEvidence"], "totp");
    assert!(fresh["mfaCompletedAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(stale["freshMfa"], false);
    assert_eq!(stale["mfaEvidence"], "totp", "what it passed is still said");
}

#[tokio::test]
async fn the_events_are_the_newest_hundred() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    for n in 0..130 {
        db.execute(
            "INSERT INTO \"SecurityEvents\" (\"Id\", \"UserId\", \"Kind\", \"Detail\", \"IpAddress\", \"Device\", \"CreatedAt\") VALUES (?1, ?2, 'mfa-failed', ?3, '203.0.113.7', 'Firefox', ?4)",
            params![Id::random(), user, format!("event {n}"), ago(10_000 - n)],
        )
        .unwrap();
    }

    let response = owner.client.get(&format!("{SECURE}/events")).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    let events: Vec<Value> = response.json();
    assert_eq!(events.len(), 100);
    assert_eq!(events[0]["detail"], "event 129");
    let text = response.text();
    let at: Vec<usize> = ["\"id\":", "\"kind\":", "\"detail\":", "\"ipAddress\":", "\"device\":", "\"createdAt\":"].iter().map(|member| text.find(member).unwrap()).collect();
    assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
}

#[tokio::test]
async fn sessions_are_listed_most_recently_seen_first_with_this_one_marked() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    let second_client = server.sign_in(&owner).await;
    let first = session_of(&owner.client).await;
    let second = session_of(&second_client).await;
    db.execute("UPDATE \"UserSessions\" SET \"LastSeenAt\" = ?2 WHERE \"Id\" = ?1", params![first, ago(10)]).unwrap();
    db.execute("UPDATE \"UserSessions\" SET \"LastSeenAt\" = ?2 WHERE \"Id\" = ?1", params![second, ago(100)]).unwrap();
    // Ended and run-out sessions are not listed.
    let ended = server.sign_in(&owner).await;
    let ended_id = session_of(&ended).await;
    db.execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1", params![ended_id, Timestamp::now()]).unwrap();
    let _ = user;

    let response = owner.client.get(&format!("{SECURE}/sessions")).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    let sessions: Vec<Value> = response.json();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0]["id"], first.to_string());
    assert_eq!(sessions[0]["current"], true);
    assert_eq!(sessions[1]["id"], second.to_string());
    assert_eq!(sessions[1]["current"], false);
    assert_eq!(sessions[0]["primarySource"], "registration", "how this one began");
    assert_eq!(sessions[1]["primarySource"], "password");
    assert_eq!(sessions[0]["mfaEvidence"], Value::Null);
    let text = response.text();
    assert!(
        text.starts_with("[{\"id\":\"") && text.contains("\"device\":") && text.contains("\"ipAddress\":\"")
            && text.contains("\"createdAt\":") && text.contains("\"lastSeenAt\":") && text.contains("\"expiresAt\":")
            && text.contains("\"primarySource\":\"registration\",\"mfaEvidence\":null,\"current\":true}"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// Ending sessions.

#[tokio::test]
async fn ending_another_session_shuts_what_it_had_open_and_touches_nothing_else() {
    // No watcher runs: the route tells the hub itself.
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let db = server.database();
    let user = user_of(&owner);
    let other_client = server.sign_in(&owner).await;
    let third_client = server.sign_in(&owner).await;
    let other = session_of(&other_client).await;
    let mut mine = connect(&server, &owner.client).await;
    let mut theirs = connect(&server, &other_client).await;
    let mut third = connect(&server, &third_client).await;
    let before = events_of(&db, user).len();
    // The address is verified, so an alert would be queued if this wrote one.
    db.execute("UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"", []).unwrap();
    let device = add_device(&db, user);

    let response = send(&owner.client, Method::DELETE, &format!("{SECURE}/sessions/{other}"), None).await;

    assert_eq!(response.status, 204);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    closes(&mut theirs).await;
    alive(&mut mine).await;
    alive(&mut third).await;
    assert_eq!(other_client.get("/api/session").await.json::<Value>()["user"], Value::Null);
    assert_eq!(third_client.get("/api/session").await.json::<Value>()["user"]["username"], "owner");
    let events = new_events(&db, user, before);
    assert_eq!(events, [("session-revoked".to_string(), "device".to_string())]);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"SecurityNotifications\" WHERE \"UserId\" = ?1", user), 0, "ending a session sends no alert");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user), 1);
    let _ = device;
    // Ending it again finds nothing to end.
    let again = send(&owner.client, Method::DELETE, &format!("{SECURE}/sessions/{other}"), None).await;
    assert_eq!(again.status, 404);
    assert!(again.body.is_empty());
    assert_eq!(new_events(&db, user, before).len(), 1);
}

#[tokio::test]
async fn ending_ones_own_session_somebody_elses_or_one_that_is_not_there_is_refused() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let mine = session_of(&owner.client).await;
    let mayas = session_of(&maya.client).await;

    let own = send(&owner.client, Method::DELETE, &format!("{SECURE}/sessions/{mine}"), None).await;
    let theirs = send(&owner.client, Method::DELETE, &format!("{SECURE}/sessions/{mayas}"), None).await;
    let nobody = send(&owner.client, Method::DELETE, &format!("{SECURE}/sessions/{NIL}"), None).await;
    let not_an_id = send(&owner.client, Method::DELETE, &format!("{SECURE}/sessions/nonsense"), None).await;

    assert_eq!([own.status, theirs.status, nobody.status, not_an_id.status], [409, 404, 404, 404]);
    for response in [&own, &theirs, &nobody] {
        assert!(response.body.is_empty());
        assert_eq!(response.header("cache-control"), Some("no-store"));
    }
    // An id that is not one names no route: nothing of the page's applies.
    assert_eq!(not_an_id.header("cache-control"), None);
    assert_eq!(maya.client.get("/api/session").await.json::<Value>()["user"]["username"], "maya", "her session stands");
    assert_eq!(owner.client.get("/api/session").await.json::<Value>()["user"]["username"], "owner");
}

#[tokio::test]
async fn ending_all_the_others_with_two_tabs_open_leaves_this_one() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let db = server.database();
    let user = user_of(&owner);
    let tab_b = server.sign_in(&owner).await;
    let tab_c = server.sign_in(&owner).await;
    let mut a = connect(&server, &owner.client).await;
    let mut b = connect(&server, &tab_b).await;
    let mut c = connect(&server, &tab_c).await;
    let device = add_device(&db, user);
    let before = events_of(&db, user).len();
    let versions: i64 = db.query_row("SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1", [user], |row| row.get(0)).unwrap();

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/sessions/revoke-others"), None).await;

    assert_eq!(response.status, 204);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    closes(&mut b).await;
    closes(&mut c).await;
    alive(&mut a).await;
    assert_eq!(owner.client.get("/api/session").await.json::<Value>()["user"]["username"], "owner");
    assert_eq!(tab_b.get("/api/session").await.json::<Value>()["user"], Value::Null);
    assert_eq!(new_events(&db, user, before), [("session-revoked".to_string(), "others".to_string())]);
    // Nothing else about the account changes: no new version, devices stay.
    let after: i64 = db.query_row("SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1", [user], |row| row.get(0)).unwrap();
    assert_eq!(after, versions);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user), 1);
    let _ = device;
    // With nobody else signed in there is nothing to end and nothing to record.
    let again = send(&owner.client, Method::POST, &format!("{SECURE}/sessions/revoke-others"), None).await;
    assert_eq!(again.status, 204);
    assert_eq!(new_events(&db, user, before).len(), 1);
}

// ---------------------------------------------------------------------------
// Remembered browsers.

#[tokio::test]
async fn forgetting_a_remembered_browser_needs_no_proof_and_is_the_account_s_own() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    let maya = server.register("maya").await;
    let mine = add_device(&db, user);
    let hers = add_device(&db, user_of(&maya));
    add_method(&db, user, "totp", None, 1);
    // The session did not pass the second step lately: forgetting is allowed all the same.
    let session = session_of(&owner.client).await;
    set_session(&db, session, 10_000, None);
    let before = events_of(&db, user).len();

    let hers_by_me = send(&owner.client, Method::DELETE, &format!("{SECURE}/trusted/{hers}"), None).await;
    let unknown = send(&owner.client, Method::DELETE, &format!("{SECURE}/trusted/{NIL}"), None).await;
    let not_an_id = send(&owner.client, Method::DELETE, &format!("{SECURE}/trusted/x"), None).await;
    let done = send(&owner.client, Method::DELETE, &format!("{SECURE}/trusted/{mine}"), None).await;
    let twice = send(&owner.client, Method::DELETE, &format!("{SECURE}/trusted/{mine}"), None).await;

    assert_eq!([hers_by_me.status, unknown.status, not_an_id.status, done.status, twice.status], [404, 404, 404, 204, 404]);
    assert_eq!(done.header("cache-control"), Some("no-store"));
    assert_eq!(not_an_id.header("cache-control"), None);
    assert_eq!(new_events(&db, user, before), [("trusted-device-removed".to_string(), String::new())]);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user_of(&maya)), 1);
}

// ---------------------------------------------------------------------------
// The guard of the routes that change the second step.

/// The states a caller can be in, for the table below.
struct Caller {
    label: &'static str,
    method: bool,
    signed_in: u64,
    second_step: Option<u64>,
}

const STATES: [Caller; 6] = [
    Caller { label: "no second step, just signed in", method: false, signed_in: 10, second_step: None },
    Caller { label: "no second step, signed in long ago", method: false, signed_in: 3600, second_step: None },
    Caller { label: "a second step, passed a minute ago", method: true, signed_in: 3600, second_step: Some(60) },
    Caller { label: "a second step, passed an hour ago", method: true, signed_in: 10, second_step: Some(3600) },
    Caller { label: "a second step, never passed in this session", method: true, signed_in: 10, second_step: None },
    Caller { label: "a second step, passed just over five minutes ago", method: true, signed_in: 10, second_step: Some(301) },
];

async fn in_state(state: &Caller) -> (Server, Person, Connection, Id) {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    if state.method {
        add_method(&db, user, "totp", None, 1);
        add_codes(&db, user, 3);
    }
    let session = session_of(&owner.client).await;
    set_session(&db, session, state.signed_in, state.second_step);
    (server, owner, db, user)
}

#[tokio::test]
async fn starting_a_second_step_asks_for_what_the_account_can_prove() {
    // With a password in the body: only an account without a second step may use it.
    for (state, with_password, without) in [
        (&STATES[0], 200, 200),
        (&STATES[1], 200, 403),
        (&STATES[2], 200, 200),
        (&STATES[3], 403, 403),
        (&STATES[4], 403, 403),
        (&STATES[5], 403, 403),
    ] {
        let (_server, owner, _db, _user) = in_state(state).await;

        let bare = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/enroll/start"), None).await;
        let wrong = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/enroll/start"), Some(&json!({ "password": "nope-nope-nope" }))).await;
        let right = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/enroll/start"), Some(&json!({ "password": PASSWORD }))).await;

        assert_eq!(bare.status, without, "{}: {}", state.label, bare.text());
        let wrong_expected = if state.method { without } else if state.signed_in > 300 { 403 } else { 200 };
        assert_eq!(wrong.status, wrong_expected, "{}: wrong password", state.label);
        assert_eq!(right.status, with_password, "{}: {}", state.label, right.text());
        for response in [&bare, &wrong, &right] {
            assert_eq!(response.header("cache-control"), Some("no-store"));
            if response.status == 403 {
                assert_eq!(title(response), FRESH_PROOF, "{}", state.label);
                assert!(response.json::<Value>().get("code").is_none(), "a sentence and no code");
            }
        }
    }
}

#[tokio::test]
async fn starting_a_second_step_gives_a_challenge_bound_to_this_browser() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    let session = session_of(&owner.client).await;

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/enroll/start"), Some(&json!({}))).await;

    assert_eq!(response.status, 200);
    let body: Value = response.json();
    assert_eq!(body["status"], "mfa_enrollment_required");
    assert_eq!(body["purpose"], "enrollment");
    assert_eq!(body["methods"], json!([]));
    assert_eq!(body["account"]["username"], "owner");
    assert_eq!(body["trustedDeviceDays"], 14);
    assert!(owner.client.cookie("stellarr-dev-preauth").is_some(), "the browser holds the challenge");
    let (purpose, parent): (String, Option<Id>) = db
        .query_row("SELECT \"Purpose\", \"ParentSessionId\" FROM \"AuthChallenges\" WHERE \"UserId\" = ?1", [user], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!(purpose, "enrollment");
    assert_eq!(parent, Some(session), "it is completed from this session or not at all");
}

#[tokio::test]
async fn proving_the_second_step_again_needs_one_to_prove() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);

    let none = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/reverify/start"), None).await;

    assert_eq!(none.status, 409);
    assert_eq!(title(&none), UNAVAILABLE);
    assert_eq!(none.header("cache-control"), Some("no-store"));
    add_method(&db, user, "totp", None, 1);
    add_method(&db, user, "email", Some("OWNER@EXAMPLE.TEST"), 1);
    // The session need not have passed anything: this is how it does.
    let stale = session_of(&owner.client).await;
    set_session(&db, stale, 10_000, None);
    let some = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/reverify/start"), None).await;
    assert_eq!(some.status, 200, "{}", some.text());
    let body: Value = some.json();
    assert_eq!(body["status"], "mfa_required");
    assert_eq!(body["purpose"], "reverify");
    assert_eq!(body["methods"].as_array().unwrap().len(), 2);
    assert_eq!(body["trustedDeviceAvailable"], false, "a remembered device proves nothing now");
    assert_eq!(body["emailCodeAvailable"], true);
}

#[tokio::test]
async fn new_recovery_codes_ask_for_a_fresh_second_step_and_need_one() {
    for state in &STATES {
        let (_server, owner, db, user) = in_state(state).await;
        let before = events_of(&db, user).len();

        let response = send(&owner.client, Method::POST, &format!("{SECURE}/recovery/regenerate"), Some(&json!({}))).await;

        let expected = match (state.method, state.second_step) {
            (false, _) if state.signed_in > 300 => 403,
            (false, _) => 409,
            (true, Some(passed)) if passed <= 300 => 200,
            _ => 403,
        };
        assert_eq!(response.status, expected, "{}: {}", state.label, response.text());
        assert_eq!(response.header("cache-control"), Some("no-store"));
        if expected == 200 {
            let codes = response.json::<Value>()["recoveryCodes"].as_array().unwrap().clone();
            assert_eq!(codes.len(), 10);
            assert!(codes.iter().all(|code| code.as_str().unwrap().len() == 7 * 4 + 3));
            assert_eq!(count(&db, "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL", user), 10, "the old set is spent");
            assert_eq!(new_events(&db, user, before), [("recovery-codes-regenerated".to_string(), String::new())]);
            assert!(response.text().starts_with("{\"recoveryCodes\":["));
        } else {
            assert_eq!(new_events(&db, user, before), []);
            if expected == 409 {
                assert_eq!(title(&response), UNAVAILABLE);
            } else {
                assert_eq!(title(&response), FRESH_PROOF);
            }
        }
    }
}

#[tokio::test]
async fn a_request_for_new_codes_with_no_body_is_a_bad_request_that_is_not_cached() {
    let (_server, owner, _db) = owner().await;

    let empty = send(&owner.client, Method::POST, &format!("{SECURE}/recovery/regenerate"), None).await;
    let text = owner.client.send(Request::new(Method::POST, &format!("{SECURE}/recovery/regenerate")).body("text/plain", "x")).await;
    let broken = owner.client.send(Request::new(Method::POST, &format!("{SECURE}/recovery/regenerate")).body("application/json", "{")).await;

    assert_eq!(empty.status, 400);
    assert_eq!(empty.header("cache-control"), Some("no-store"));
    assert_eq!(empty.header("pragma"), Some("no-cache"));
    // Broken JSON is refused as the framework refuses it, before the page's headers are set.
    assert_eq!(broken.status, 400);
    assert_eq!(broken.header("cache-control"), None);
    // Refused before anything is read, as for every route that takes JSON.
    assert_eq!(text.status, 415);
}

#[tokio::test]
async fn taking_a_method_away_ends_the_other_sessions_and_keeps_this_one() {
    let server = Server::start_with(&WATCHER).await;
    let owner = server.found("owner").await;
    let db = server.database();
    let user = user_of(&owner);
    // Signed in twice before the second step is set up: afterwards a sign-in asks for it.
    let tab = server.sign_in(&owner).await;
    let totp = add_method(&db, user, "totp", None, 2);
    let mail = add_method(&db, user, "email", Some("OWNER@EXAMPLE.TEST"), 1);
    add_codes(&db, user, 4);
    add_device(&db, user);
    let session = session_of(&owner.client).await;
    set_session(&db, session, 10, Some(60));
    let mut other = connect(&server, &tab).await;
    let mut mine = connect(&server, &owner.client).await;
    let before = events_of(&db, user).len();
    let old_cookie = owner.client.cookie(COOKIE).unwrap();

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{totp}/remove"), Some(&json!({}))).await;

    assert_eq!(response.status, 204, "{}", response.text());
    assert!(response.body.is_empty());
    assert_eq!(response.header("cache-control"), Some("no-cache,no-store"), "the cookie's own headers");
    assert_ne!(owner.client.cookie(COOKIE).unwrap(), old_cookie, "this session moves to the new version");
    assert_eq!(owner.client.get("/api/session").await.json::<Value>()["user"]["username"], "owner");
    assert_eq!(tab.get("/api/session").await.json::<Value>()["user"], Value::Null);
    // The first server closes every connection the account held, this one's
    // as well: its client opens another with the new cookie.
    closes(&mut other).await;
    closes(&mut mine).await;
    let _reconnected = connect(&server, &owner.client).await;
    let events = new_events(&db, user, before);
    assert_eq!(events, [("method-removed".to_string(), "totp".to_string())]);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL", user), 4, "another method is left, so the codes are too");
    let page = page(&owner.client).await;
    assert_eq!(page["methods"].as_array().unwrap().len(), 1);
    assert_eq!(page["methods"][0]["id"], mail.to_string());
    // The last one goes: the codes go with it.
    let last = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{mail}/remove"), Some(&json!({}))).await;
    assert_eq!(last.status, 204);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"RecoveryCodes\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL", user), 0);
    let gone = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{mail}/remove"), Some(&json!({}))).await;
    assert_eq!(gone.status, 404);
    assert_eq!(title(&gone), "No such security method.");
}

#[tokio::test]
async fn removing_a_method_asks_for_a_fresh_second_step_in_every_state() {
    for state in &STATES {
        let (_server, owner, db, user) = in_state(state).await;
        let method: Option<Id> = db.query_row("SELECT \"Id\" FROM \"MfaMethods\" WHERE \"UserId\" = ?1", [user], |row| row.get(0)).ok();
        let target = method.map_or(NIL.to_string(), |id| id.to_string());

        let response = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{target}/remove"), Some(&json!({}))).await;

        // What a refusal for want of a proof wins over: the method not being there.
        let expected = match (state.method, state.second_step) {
            (false, _) if state.signed_in > 300 => 403,
            (false, _) => 404,
            (true, Some(passed)) if passed <= 300 => 204,
            _ => 403,
        };
        assert_eq!(response.status, expected, "{}: {}", state.label, response.text());
        if expected == 403 {
            assert_eq!(title(&response), FRESH_PROOF);
            assert!(response.json::<Value>().get("code").is_none());
        }
    }
}

#[tokio::test]
async fn a_method_that_is_not_the_account_s_is_not_found() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    add_method(&db, user, "totp", None, 1);
    let session = session_of(&owner.client).await;
    set_session(&db, session, 10, Some(10));
    let maya = server.register("maya").await;
    let hers = add_method(&db, user_of(&maya), "totp", None, 1);

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{hers}/remove"), Some(&json!({}))).await;
    let not_an_id = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/x/remove"), Some(&json!({}))).await;

    assert_eq!(response.status, 404);
    assert_eq!(title(&response), "No such security method.");
    assert_eq!(not_an_id.status, 404);
    assert_eq!(not_an_id.header("cache-control"), None);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user_of(&maya)), 1);
}

#[tokio::test]
async fn the_last_way_in_is_not_taken_away_and_the_neighbouring_cases_are_allowed() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    // A passkey and nothing else to sign in with: no password, no provider set up here.
    remove_password(&db, user);
    let passkey = add_method(&db, user, "passkey", None, 2);
    let totp = add_method(&db, user, "totp", None, 1);
    add_identity(&db, user, "https://idp.example.test", 10);
    let session = session_of(&owner.client).await;
    set_session(&db, session, 10, Some(10));
    let before = events_of(&db, user).len();

    let refused = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{passkey}/remove"), Some(&json!({}))).await;

    assert_eq!(refused.status, 409);
    let body: Value = refused.json();
    assert_eq!(body["code"], "last-way-in");
    assert_eq!(body["title"], "This passkey is the only way to log in to your account. Set a password or link a provider first, then remove it.");
    assert_eq!(refused.header("cache-control"), Some("no-store"));
    assert_eq!(events_of(&db, user).len(), before, "a refusal writes nothing");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user), 2);
    // The authenticator app is no way in: it may go.
    let allowed = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{totp}/remove"), Some(&json!({}))).await;
    assert_eq!(allowed.status, 204);
    // With a password the passkey may go, though it is the only method.
    let session = session_of(&owner.client).await;
    set_session(&db, session, 10, Some(10));
    let first = send(&owner.client, Method::POST, &format!("{SECURE}/password"), Some(&json!({ "newPassword": NEW_PASSWORD }))).await;
    assert_eq!(first.status, 204, "{}", first.text());
    let gone = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{passkey}/remove"), Some(&json!({}))).await;
    assert_eq!(gone.status, 204);
    // Two passkeys: either may go, not both.
    let maya = server.register("maya").await;
    let maya_id = user_of(&maya);
    remove_password(&db, maya_id);
    let one = add_method(&db, maya_id, "passkey", None, 2);
    let two = add_method(&db, maya_id, "passkey", None, 1);
    set_session(&db, session_of(&maya.client).await, 10, Some(10));
    assert_eq!(send(&maya.client, Method::POST, &format!("{SECURE}/mfa/{one}/remove"), Some(&json!({}))).await.status, 204);
    set_session(&db, session_of(&maya.client).await, 10, Some(10));
    let last = send(&maya.client, Method::POST, &format!("{SECURE}/mfa/{two}/remove"), Some(&json!({}))).await;
    assert_eq!(last.status, 409);
    assert_eq!(last.json::<Value>()["code"], "last-way-in");
}

#[tokio::test]
async fn the_last_method_stays_when_the_instance_asks_for_one() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    let totp = add_method(&db, user, "totp", None, 1);
    // The session has passed the second step before the instance asks for it.
    let session = session_of(&owner.client).await;
    set_session(&db, session, 10, Some(10));
    let server_id: Id = db.query_row("SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1", [], |row| row.get(0)).unwrap();
    db.execute(
        "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"InviteMailEnabled\", \"RecoveryMailEnabled\",
             \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\")
         VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, ?2, 'admins')",
        params![server_id, Timestamp::now()],
    )
    .unwrap();
    let before = events_of(&db, user).len();

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{totp}/remove"), Some(&json!({}))).await;

    assert_eq!(response.status, 409, "{}", response.text());
    assert_eq!(title(&response), UNAVAILABLE);
    assert!(response.json::<Value>().get("code").is_none());
    assert_eq!(events_of(&db, user).len(), before);
    assert_eq!(page(&owner.client).await["requirement"], "admins");
}

// ---------------------------------------------------------------------------
// The password.

#[tokio::test]
async fn the_first_password_of_an_account_without_one_is_set_and_its_sessions_stay() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    // Signed in before the password goes: an account without one cannot.
    let other = server.sign_in(&owner).await;
    remove_password(&db, user);
    db.execute("UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"", []).unwrap();
    let before = events_of(&db, user).len();
    let notified = count(&db, "SELECT COUNT(*) FROM \"SecurityNotifications\" WHERE \"UserId\" = ?1", user);
    assert_eq!(page(&owner.client).await["hasPassword"], false);

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/password"), Some(&json!({ "newPassword": NEW_PASSWORD }))).await;

    assert_eq!(response.status, 204, "{}", response.text());
    assert!(response.body.is_empty());
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(new_events(&db, user, before), [("password-changed".to_string(), "password set".to_string())]);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"SecurityNotifications\" WHERE \"UserId\" = ?1", user), notified + 1, "the verified address is told");
    assert_eq!(page(&owner.client).await["hasPassword"], true);
    assert_eq!(other.get("/api/session").await.json::<Value>()["user"]["username"], "owner", "the sessions already open stay");
    let login = server.login(&server.client(), "owner", NEW_PASSWORD).await;
    assert_eq!(login.status, 200);
    // A second time it is a change, which asks for the current one.
    let again = send(&owner.client, Method::POST, &format!("{SECURE}/password"), Some(&json!({ "newPassword": NEW_PASSWORD }))).await;
    assert_eq!(again.status, 409);
    assert_eq!(title(&again), "This account already has a password. Change it instead.");
}

#[tokio::test]
async fn the_first_password_is_refused_for_every_reason_the_first_server_has() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    remove_password(&db, user);
    let path = format!("{SECURE}/password");
    let too_short = send(&owner.client, Method::POST, &path, Some(&json!({ "newPassword": "short" }))).await;
    let missing = send(&owner.client, Method::POST, &path, Some(&json!({}))).await;
    let too_long = send(&owner.client, Method::POST, &path, Some(&json!({ "newPassword": "x".repeat(257) }))).await;
    let exactly = send(&owner.client, Method::POST, &path, Some(&json!({ "newPassword": "12345678" }))).await;

    for response in [&too_short, &missing] {
        assert_eq!(response.status, 400);
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        assert_eq!(response.json::<Value>()["errors"]["newPassword"], json!(["Password must be at least 8 characters long."]));
        assert_eq!(response.header("cache-control"), Some("no-store"));
    }
    assert_eq!(too_long.status, 400);
    assert_eq!(too_long.json::<Value>()["errors"]["newPassword"], json!(["Password can be at most 256 characters long."]));
    assert!(too_short.text().starts_with("{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.1\",\"title\":\"One or more validation errors occurred.\",\"status\":400,\"errors\":{\"newPassword\":["));
    assert_eq!(exactly.status, 204, "eight characters are enough");
    // The policy is looked at before the account is.
    let policy_first = send(&owner.client, Method::POST, &path, Some(&json!({ "newPassword": "short" }))).await;
    assert_eq!(policy_first.status, 400);
}

#[tokio::test]
async fn the_first_password_needs_a_fresh_proof_of_an_account_that_has_none_to_give() {
    // (has a second step, signed in seconds ago, passed a second step seconds ago) and the answer.
    for (method, signed_in, second_step, expected) in [
        (false, 10, None, 204),
        (false, 3600, None, 403),
        (true, 10, None, 403),
        (true, 10, Some(60), 204),
        (true, 10, Some(3600), 403),
    ] {
        let (_server, owner, db) = owner().await;
        let user = user_of(&owner);
        remove_password(&db, user);
        if method {
            add_method(&db, user, "totp", None, 1);
        }
        set_session(&db, session_of(&owner.client).await, signed_in, second_step);

        let response = send(&owner.client, Method::POST, &format!("{SECURE}/password"), Some(&json!({ "newPassword": NEW_PASSWORD }))).await;

        assert_eq!(response.status, expected, "{method} {signed_in} {second_step:?}: {}", response.text());
        if expected == 403 {
            // The word the password routes use for it, and the code.
            let body: Value = response.json();
            assert_eq!(body["title"], NEEDS_A_STEP);
            assert_eq!(body["code"], "fresh-mfa-required");
            assert_eq!(count(&db, "SELECT COUNT(*) FROM \"Users\" WHERE \"Id\" = ?1 AND \"PasswordHash\" IS NULL", user), 1);
        }
    }
}

#[tokio::test]
async fn changing_the_password_answers_in_outcomes_in_the_first_server_s_order() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    let ask = |current: Value, new: Value| {
        let client = &owner.client;
        async move { send(client, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": current, "newPassword": new }))).await }
    };

    let short = ask(json!(PASSWORD), json!("short")).await;
    let missing = send(&owner.client, Method::POST, "/api/me/password", Some(&json!({}))).await;
    let long = ask(json!(PASSWORD), json!("x".repeat(257))).await;
    let no_current = send(&owner.client, Method::POST, "/api/me/password", Some(&json!({ "newPassword": NEW_PASSWORD }))).await;
    let null_current = ask(Value::Null, json!(NEW_PASSWORD)).await;
    let wrong = ask(json!("nope-nope-nope"), json!(NEW_PASSWORD)).await;
    let same = ask(json!(PASSWORD), json!(PASSWORD)).await;
    // The new password is looked at before the current one: eight characters are enough.
    let wrong_and_short = ask(json!("nope-nope-nope"), json!("12345678")).await;

    for (response, result) in [(&short, "too-short"), (&missing, "too-short"), (&no_current, "wrong-current"), (&null_current, "wrong-current"), (&wrong, "wrong-current"), (&same, "same-as-current"), (&wrong_and_short, "wrong-current")] {
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.text(), format!("{{\"result\":\"{result}\"}}"));
        assert_eq!(response.header("cache-control"), None, "this route is not under the page's headers");
    }
    assert_eq!(long.status, 400);
    assert_eq!(long.json::<Value>()["errors"]["newPassword"], json!(["New password can be at most 256 characters long."]));
    // Nothing was written, and a wrong password costs nothing but the route's limit.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 AND \"Kind\" = 'password-changed'", user), 0);
    for _ in 0..12 {
        assert_eq!(ask(json!("nope-nope-nope"), json!(NEW_PASSWORD)).await.status, 200);
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 AND \"Kind\" LIKE 'mfa-%'", user), 0);
}

#[tokio::test]
async fn changing_the_password_asks_for_the_second_step_before_it_looks_at_the_password() {
    for state in &STATES {
        let (_server, owner, db, user) = in_state(state).await;
        let response = send(&owner.client, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": "nope-nope-nope", "newPassword": NEW_PASSWORD }))).await;

        let fresh = !state.method || state.second_step.is_some_and(|passed| passed <= 300);
        let expected = if fresh { "wrong-current" } else { "second-step-required" };
        assert_eq!(response.text(), format!("{{\"result\":\"{expected}\"}}"), "{}", state.label);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 AND \"Kind\" = 'password-changed'", user), 0);
    }
}

#[tokio::test]
async fn a_changed_password_ends_everything_but_this_session() {
    let server = Server::start_with(&WATCHER).await;
    let owner = server.found("owner").await;
    let db = server.database();
    let user = user_of(&owner);
    db.execute("UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"", []).unwrap();
    add_device(&db, user);
    db.execute(
        "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\", \"RedeemedAt\")
         VALUES (?1, 'h', ?2, 1, ?3, ?3, ?3, NULL), (?4, 'h', ?2, 1, ?3, ?3, NULL, NULL)",
        params![Id::random(), user, Timestamp::now(), Id::random()],
    )
    .unwrap();
    let tab = server.sign_in(&owner).await;
    let mut other = connect(&server, &tab).await;
    let mut mine = connect(&server, &owner.client).await;
    let before = events_of(&db, user).len();
    let notified = count(&db, "SELECT COUNT(*) FROM \"SecurityNotifications\" WHERE \"UserId\" = ?1", user);
    let old_cookie = owner.client.cookie(COOKIE).unwrap();

    let response = send(&owner.client, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": PASSWORD, "newPassword": NEW_PASSWORD }))).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.text(), "{\"result\":\"changed\"}");
    assert_eq!(response.header("cache-control"), Some("no-cache,no-store"));
    assert_ne!(owner.client.cookie(COOKIE).unwrap(), old_cookie);
    assert_eq!(owner.client.get("/api/session").await.json::<Value>()["user"]["username"], "owner");
    assert_eq!(tab.get("/api/session").await.json::<Value>()["user"], Value::Null);
    // The first server's watcher closes this session's own connection as well,
    // since it was let in with the version before the change.
    closes(&mut other).await;
    closes(&mut mine).await;
    assert_eq!(new_events(&db, user, before), [("password-changed".to_string(), "password changed".to_string())]);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"SecurityNotifications\" WHERE \"UserId\" = ?1", user), notified + 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL", user), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"UserId\" = ?1 AND \"RedeemedAt\" IS NULL AND \"ApprovedAt\" IS NOT NULL", user), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"UserId\" = ?1 AND \"RedeemedAt\" IS NULL", user), 1, "one nobody approved is not touched");
    assert_eq!(server.login(&server.client(), "owner", PASSWORD).await.status, 401);
    assert_eq!(server.login(&server.client(), "owner", NEW_PASSWORD).await.status, 200);
    // Still the same password is no change.
    let again = send(&owner.client, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": NEW_PASSWORD, "newPassword": NEW_PASSWORD }))).await;
    assert_eq!(again.text(), "{\"result\":\"same-as-current\"}");
}

#[tokio::test]
async fn an_account_without_a_password_has_no_current_one_to_give() {
    let (_server, owner, db) = owner().await;
    remove_password(&db, user_of(&owner));

    let response = send(&owner.client, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": PASSWORD, "newPassword": NEW_PASSWORD }))).await;

    assert_eq!(response.text(), "{\"result\":\"wrong-current\"}");
}

// ---------------------------------------------------------------------------
// The address.

async fn change_address(client: &Client, email: &str, password: Option<&str>) -> Response {
    send(client, Method::POST, "/api/me/email", Some(&json!({ "email": email, "password": password }))).await
}

fn result(response: &Response) -> String {
    assert_eq!(response.status, 200, "{}", response.text());
    response.json::<Value>()["result"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn an_address_that_is_not_one_is_refused_before_anything_else_is_looked_at() {
    let (_server, owner, db) = owner().await;
    // Not signed in recently, no second step to ask for: still the address is judged first.
    set_session(&db, session_of(&owner.client).await, 10_000, None);
    for email in ["", "not-an-address", "Ala <ala@example.test>", "a@@example.test", &format!("{}@example.test", "a".repeat(310))] {
        let response = change_address(&owner.client, email, Some(PASSWORD)).await;

        assert_eq!(response.status, 400, "{email}");
        assert_eq!(response.json::<Value>()["errors"]["email"], json!(["Enter a valid e-mail address."]), "{email}");
    }
    let missing = send(&owner.client, Method::POST, "/api/me/email", Some(&json!({ "password": PASSWORD }))).await;
    assert_eq!(missing.status, 400);
}

#[tokio::test]
async fn an_address_change_is_refused_in_the_order_the_first_server_refuses_it() {
    for state in &STATES {
        let (server, owner, _db, _user) = in_state(state).await;
        server.register("maya").await;
        let fresh = !state.method || state.second_step.is_some_and(|passed| passed <= 300);
        let stale_primary = !state.method && state.signed_in > 300;

        let wrong = change_address(&owner.client, "owner-new@example.test", Some("nope-nope-nope")).await;
        let none = change_address(&owner.client, "owner-new@example.test", None).await;
        let own = change_address(&owner.client, "OWNER@example.test", Some(PASSWORD)).await;
        let taken = change_address(&owner.client, "Maya@Example.Test", Some(PASSWORD)).await;
        let unavailable = change_address(&owner.client, "owner-new@example.test", Some(PASSWORD)).await;

        let blocked = if fresh { None } else { Some("second-step-required") };
        let _ = stale_primary;
        assert_eq!(result(&wrong), blocked.unwrap_or("wrong-password"), "{}", state.label);
        assert_eq!(result(&none), blocked.unwrap_or("wrong-password"), "{}", state.label);
        assert_eq!(result(&own), blocked.unwrap_or("same"), "{}", state.label);
        assert_eq!(result(&taken), blocked.unwrap_or("taken"), "{}", state.label);
        // Mail is off on a fresh instance.
        assert_eq!(result(&unavailable), blocked.unwrap_or("mail-unavailable"), "{}", state.label);
    }
}

#[tokio::test]
async fn an_account_without_a_password_answers_for_the_address_with_its_fresh_proof() {
    for (signed_in, expected) in [(10, "mail-unavailable"), (3600, "second-step-required")] {
        let (_server, owner, db) = owner().await;
        remove_password(&db, user_of(&owner));
        set_session(&db, session_of(&owner.client).await, signed_in, None);

        let response = change_address(&owner.client, "owner-new@example.test", None).await;

        assert_eq!(result(&response), expected, "signed in {signed_in} seconds ago");
        let given = change_address(&owner.client, "owner-new@example.test", Some(PASSWORD)).await;
        assert_eq!(result(&given), expected, "a password is of no use to it");
    }
}

#[tokio::test]
async fn addresses_are_compared_as_the_first_server_compares_them() {
    let (server, owner, db) = owner().await;
    let maya = server.register("maya").await;
    db.execute("UPDATE \"Users\" SET \"Email\" = 'Maya.Ünï@Example.Test', \"NormalizedEmail\" = 'MAYA.ÜNÏ@EXAMPLE.TEST' WHERE \"Id\" = ?1", [user_of(&maya)]).unwrap();

    let same_letters = change_address(&owner.client, "maya.ünï@example.test", Some(PASSWORD)).await;
    // The dotless i is not capitalised to I, so this is another address.
    let other_dotless = change_address(&owner.client, "maya.ünı@example.test", Some(PASSWORD)).await;
    let with_spaces = change_address(&owner.client, "  OWNER@example.test  ", Some(PASSWORD)).await;

    assert_eq!(result(&same_letters), "taken");
    assert_eq!(result(&other_dotless), "mail-unavailable");
    assert_eq!(result(&with_spaces), "same", "the address is trimmed first");
}

// ---------------------------------------------------------------------------
// The address, with mail.

/// A mail server that keeps what it is given.
struct Sink {
    port: u16,
    mails: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Sink {
    async fn start() -> Sink {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mails = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let shared = std::sync::Arc::clone(&mails);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mails = std::sync::Arc::clone(&shared);
                tokio::spawn(async move {
                    let (read, mut write) = stream.into_split();
                    let mut lines = BufReader::new(read).lines();
                    let _ = write.write_all(b"220 sink.test ESMTP\r\n").await;
                    while let Ok(Some(line)) = lines.next_line().await {
                        let answer = match line.split(' ').next().unwrap_or("").to_ascii_uppercase().as_str() {
                            "EHLO" => "250 sink.test".to_string(),
                            "DATA" => {
                                let _ = write.write_all(b"354 go on\r\n").await;
                                let mut data = String::new();
                                while let Ok(Some(body)) = lines.next_line().await {
                                    if body == "." {
                                        break;
                                    }
                                    data.push_str(&body);
                                    data.push('\n');
                                }
                                mails.lock().unwrap().push(data);
                                "250 queued".to_string()
                            }
                            "QUIT" => {
                                let _ = write.write_all(b"221 bye\r\n").await;
                                return;
                            }
                            _ => "250 OK".to_string(),
                        };
                        let _ = write.write_all(format!("{answer}\r\n").as_bytes()).await;
                    }
                });
            }
        });
        Sink { port, mails, task }
    }

    fn mails(&self) -> Vec<String> {
        self.mails.lock().unwrap().clone()
    }
}

/// The instance sends its mail to `port`.
fn send_mail_to(db: &Connection, port: u16, alerts: bool) {
    let server: Id = db.query_row("SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1", [], |row| row.get(0)).unwrap();
    db.execute(
        "INSERT OR REPLACE INTO \"OperationsSettings\"
            (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\", \"MailSmtpPort\", \"MailSmtpUsername\", \"MailSmtpSecurity\",
             \"MailResendEnabled\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\",
             \"AutomaticCleanupEnabled\", \"UpdatedAt\")
         VALUES (?1, '', 'UTC', 1, 'noreply@chat.test', '127.0.0.1', ?2, '', 'none', 0, 1, 1, ?3, 14, 30, 1, 1, ?4)",
        params![server, port, alerts, Timestamp::now()],
    )
    .unwrap();
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..400 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{what} did not happen in ten seconds");
}

#[tokio::test]
async fn a_new_address_is_asked_to_confirm_and_only_the_latest_request_counts() {
    let server = Server::start_with(&[("Operations__PublicUrl", "https://chat.test")]).await;
    let owner = server.found("owner").await;
    let db = server.database();
    let user = user_of(&owner);
    let sink = Sink::start().await;
    send_mail_to(&db, sink.port, false);
    let before = events_of(&db, user).len();

    let first = change_address(&owner.client, "  Owner.New@Example.Test ", Some(PASSWORD)).await;
    let second = change_address(&owner.client, "newer@example.test", Some(PASSWORD)).await;

    assert_eq!(first.text(), "{\"result\":\"sent\"}");
    assert_eq!(second.text(), "{\"result\":\"sent\"}");
    assert_eq!(first.header("content-type"), Some("application/json; charset=utf-8"));
    until("both mails", || sink.mails().len() == 2).await;
    // Quoted-printable breaks long lines and writes an equals sign as =3D.
    let mails: Vec<String> = sink.mails().iter().map(|mail| mail.replace("=\n", "").replace("=3D", "=")).collect();
    // The mail library writes the domain of an address in lower case.
    assert!(mails[0].contains("To: Owner.New@example.test"), "the address as it was written, trimmed: {}", mails[0]);
    assert!(mails[0].contains("Confirm your new address on Stellarr"));
    assert!(mails[0].contains("https://chat.test/confirm-email?token="), "{}", mails[0]);
    let rows: Vec<(String, String, bool, i64, bool)> = db
        .prepare(
            "SELECT \"NewEmail\", \"NormalizedNewEmail\", \"UsedAt\" IS NOT NULL, LENGTH(\"TokenHash\"), \"SessionId\" IS NOT NULL
             FROM \"EmailChangeRequests\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\"",
        )
        .unwrap()
        .query_map([user], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, [
        ("Owner.New@Example.Test".to_string(), "OWNER.NEW@EXAMPLE.TEST".to_string(), true, 64, true),
        ("newer@example.test".to_string(), "NEWER@EXAMPLE.TEST".to_string(), false, 64, true),
    ]);
    let version: i64 = db.query_row("SELECT \"SecurityVersion\" FROM \"EmailChangeRequests\" ORDER BY \"CreatedAt\" DESC LIMIT 1", [], |row| row.get(0)).unwrap();
    let account: i64 = db.query_row("SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1", [user], |row| row.get(0)).unwrap();
    assert_eq!(version, account, "the request belongs to the account as it stands");
    assert_eq!(new_events(&db, user, before), [], "asking changes nothing yet, so nothing is recorded");
}

#[tokio::test]
async fn a_mail_that_cannot_be_sent_leaves_the_request_written() {
    let server = Server::start_with(&[("Operations__PublicUrl", "https://chat.test")]).await;
    let owner = server.found("owner").await;
    let db = server.database();
    // Nothing listens at this port.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    send_mail_to(&db, port, false);

    let response = change_address(&owner.client, "owner-new@example.test", Some(PASSWORD)).await;

    assert_eq!(result(&response), "mail-unavailable");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"EmailChangeRequests\" WHERE \"UserId\" = ?1 AND \"UsedAt\" IS NULL", user_of(&owner)), 1);
}

// ---------------------------------------------------------------------------
// The limits.

#[tokio::test]
async fn each_route_counts_against_the_limit_the_first_server_gives_it() {
    const PRODUCTION: [(&str, &str); 2] = [("ASPNETCORE_ENVIRONMENT", "Production"), ("Rtc__TurnSharedSecret", "a-secret")];
    // One request is what a policy allows; the next request of any route that
    // carries the policy is refused before anybody is looked at.
    for (policy, spent_on, carriers, free) in [
        (
            "SecurityWrite",
            format!("{SECURE}/mfa/enroll/start"),
            vec![
                format!("{SECURE}/mfa/reverify/start"),
                format!("{SECURE}/mfa/{NIL}/remove"),
                format!("{SECURE}/recovery/regenerate"),
                format!("{SECURE}/password"),
            ],
            vec![format!("{SECURE}/sessions/revoke-others"), "/api/me/password".to_string(), "/api/me/email".to_string()],
        ),
        ("Login", "/api/me/password".to_string(), vec![], vec![format!("{SECURE}/password"), "/api/me/email".to_string()]),
        ("AuthWrite", "/api/me/email".to_string(), vec![], vec![format!("{SECURE}/password"), "/api/me/password".to_string()]),
    ] {
        let setting = format!("RateLimiting__{policy}__PermitLimit");
        let mut environment = PRODUCTION.to_vec();
        environment.push((setting.as_str(), "1"));
        let server = Server::start_with(&environment).await;
        let client = server.client();

        let first = send(&client, Method::POST, &spent_on, Some(&json!({}))).await;
        assert_eq!(first.status, 401, "{policy}: {spent_on}");
        let refused = send(&client, Method::POST, &spent_on, Some(&json!({}))).await;
        assert_eq!(refused.status, 429, "{policy}: {spent_on}");
        for path in &carriers {
            assert_eq!(send(&client, Method::POST, path, Some(&json!({}))).await.status, 429, "{policy} also limits {path}");
        }
        for path in &free {
            assert_eq!(send(&client, Method::POST, path, Some(&json!({}))).await.status, 401, "{policy} does not limit {path}");
        }
        // The routes that read or end sessions are not limited at all.
        for (method, path) in [(Method::GET, SECURE.to_string()), (Method::DELETE, format!("{SECURE}/sessions/{NIL}")), (Method::DELETE, format!("{SECURE}/trusted/{NIL}"))] {
            assert_eq!(send(&client, method, &path, None).await.status, 401);
        }
    }
}

// ---------------------------------------------------------------------------
// What a change keeps of the session that makes it.

/// Whether a response sets a cookie that lasts beyond the browser.
fn outlives_the_browser(response: &Response) -> bool {
    response
        .headers
        .get_all("set-cookie")
        .iter()
        .any(|line| line.to_str().unwrap().to_ascii_lowercase().contains("expires=") && !line.to_str().unwrap().contains("1970"))
}

#[tokio::test]
async fn changes_made_from_a_short_session_keep_it_short() {
    let (server, owner, db) = owner().await;
    let user = user_of(&owner);
    let short = server.client();
    let login = send(&short, Method::POST, "/api/auth/login", Some(&json!({ "username": "owner", "password": PASSWORD, "keepSignedIn": false }))).await;
    assert_eq!(login.status, 200);
    assert!(!outlives_the_browser(&login));
    let session = session_of(&short).await;
    let totp = add_method(&db, user, "totp", None, 1);
    set_session(&db, session, 10, Some(10));

    let removed = send(&short, Method::POST, &format!("{SECURE}/mfa/{totp}/remove"), Some(&json!({}))).await;
    let changed = send(&short, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": PASSWORD, "newPassword": NEW_PASSWORD }))).await;

    assert_eq!(removed.status, 204);
    assert!(!outlives_the_browser(&removed), "the cookie of a short session is not made to last");
    assert_eq!(changed.status, 200);
    assert!(!outlives_the_browser(&changed));
    let (short_lived, expires, hard_end): (bool, Timestamp, Timestamp) = db
        .query_row("SELECT \"ShortLived\", \"ExpiresAt\", \"AbsoluteExpiresAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1", [session], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert!(short_lived);
    assert!(expires <= hard_end, "a renewal never goes past the hard end of a short session");
    // The one that stays signed in long keeps a cookie that lasts.
    let long = server.client();
    assert_eq!(server.login(&long, "owner", NEW_PASSWORD).await.status, 200);
    let kept = send(&long, Method::POST, "/api/me/password", Some(&json!({ "currentPassword": NEW_PASSWORD, "newPassword": PASSWORD }))).await;
    assert_eq!(kept.status, 200);
    assert!(outlives_the_browser(&kept));
}

#[tokio::test]
async fn the_address_that_is_told_of_a_removed_mail_method_is_read_before_it_goes() {
    let (_server, owner, db) = owner().await;
    let user = user_of(&owner);
    // The founder's address, confirmed by a mail method, is the one alerts go to.
    let mail = add_method(&db, user, "email", Some("OWNER@EXAMPLE.TEST"), 1);
    set_session(&db, session_of(&owner.client).await, 10, Some(10));

    let response = send(&owner.client, Method::POST, &format!("{SECURE}/mfa/{mail}/remove"), Some(&json!({}))).await;

    assert_eq!(response.status, 204, "{}", response.text());
    let alerted: Vec<(String, String)> = db
        .prepare("SELECT e.\"Kind\", n.\"EmailAddress\" FROM \"SecurityNotifications\" n JOIN \"SecurityEvents\" e ON e.\"Id\" = n.\"SecurityEventId\" WHERE n.\"UserId\" = ?1 AND e.\"Kind\" = 'method-removed'")
        .unwrap()
        .query_map([user], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(alerted, [("method-removed".to_string(), "owner@example.test".to_string())]);
}

// ---------------------------------------------------------------------------
// The hand-over: a challenge this group starts is answered by the second
// step's own calls. Routes that answer one are another group's, so the
// server runs in this process with routes of the test's own that call the
// ceremony the way those will.

mod hand_over {
    use axum::extract::State;
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use stellarr_api::auth::ceremony::{self, Answer, Answered, Ask, Enrolled, TotpConfirmation, TotpStarted};
    use stellarr_api::auth::flow::{self, Context};
    use stellarr_api::auth::session::{NewSession, sign_in_cookie};
    use stellarr_api::auth::totp;
    use stellarr_api::database::DbError;
    use stellarr_api::http::{ApiError, ApiResult, Endpoints, Json, json_response, with_session_cookie};
    use stellarr_api::state::AppState;
    use stellarr_api::values::{Id, Timestamp};

    use super::{SECURE, page, send, session_of};
    use crate::support::Server;
    use http::Method;

    fn routes() -> Endpoints {
        stellarr_api::account_security::routes().merge(
            Endpoints::new()
                .get("/api/test/session", session)
                .post("/api/test/enrol", enrol)
                .json_body()
                .post("/api/test/answer", answer)
                .json_body(),
        )
    }

    /// An account with a password and nothing else, signed in just now.
    async fn session(State(state): State<AppState>) -> ApiResult<Response> {
        let user = Id::random();
        let new = NewSession::new(user, "Test".into(), "127.0.0.1".into(), "password", 1, false);
        let id = new.id;
        state
            .database()
            .write(move |tx| -> Result<(), DbError> {
                tx.execute(
                    "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\",
                         \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                     VALUES (?1, ?1, ?1, 'Test', 'x', 0, 1, 0, 'c', ?2)",
                    rusqlite::params![user, Timestamp::now()],
                )?;
                new.insert(tx)?;
                Ok(())
            })
            .await?;
        let cookie = sign_in_cookie(state.sealer(), &state.settings().environment, false, user, 1, id, false);
        Ok(with_session_cookie(StatusCode::NO_CONTENT.into_response(), &cookie))
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Request {
        challenge_id: String,
        method: Option<Id>,
        secret: Option<String>,
    }

    const CONTEXT: Context<'static> =
        Context { ip_address: "127.0.0.1", user_agent: "Test", https: false, trusted_device_cookie: None, passkeys_available: false };

    fn with_cookies(mut response: Response, lines: &[String]) -> Response {
        for line in lines {
            response.headers_mut().append(header::SET_COOKIE, HeaderValue::from_str(line).unwrap());
        }
        response
    }

    /// Adds an authenticator app to the account of an enrolment challenge: the
    /// first of the calls that follow the page's request.
    async fn enrol(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<Request>) -> ApiResult<Response> {
        let preauth = flow::read_preauth(&headers, false);
        let ask = Ask { context: &CONTEXT, preauth: preauth.as_ref(), challenge_id: &request.challenge_id };
        let TotpStarted::Started { method, secret, .. } = ceremony::start_totp(&state, &ask, Some("Phone")).await? else {
            return Err(ApiError::internal("the challenge was not accepted"));
        };
        let code = totp::code_at(&secret, Timestamp::now()).expect("a code");
        let confirmation = TotpConfirmation { method: Some(method), name: None, code: &code };
        match ceremony::confirm_totp(&state, &ask, confirmation, false, |_, _| Ok::<(), DbError>(())).await? {
            Enrolled::Done { completed, .. } => Ok(with_cookies(
                json_response(
                    StatusCode::OK,
                    &json!({ "method": method, "secret": secret, "session": completed.session, "recoveryCodes": completed.recovery_codes }),
                ),
                &completed.cookies,
            )),
            _ => Err(ApiError::internal("the enrolment was not completed")),
        }
    }

    /// Answers a request to prove the second step again with the next code.
    async fn answer(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<Request>) -> ApiResult<Response> {
        let preauth = flow::read_preauth(&headers, false);
        let ask = Ask { context: &CONTEXT, preauth: preauth.as_ref(), challenge_id: &request.challenge_id };
        let later = Timestamp::now().plus(std::time::Duration::from_secs(30));
        let code = totp::code_at(request.secret.as_deref().unwrap_or(""), later).expect("a code");
        match ceremony::answer(&state, &ask, Answer::Code { method: request.method, code: &code }, false, |_, _| Ok::<(), DbError>(())).await? {
            Answered::SignedIn { completed, .. } => Ok(with_cookies(
                json_response(StatusCode::OK, &json!({ "session": completed.session })),
                &completed.cookies,
            )),
            _ => Err(ApiError::internal("the answer was not accepted")),
        }
    }

    #[tokio::test]
    async fn a_challenge_started_from_the_page_is_answered_by_the_ceremony_and_the_method_is_then_on_the_page() {
        let server = Server::in_process(routes(), &[]).await;
        let client = server.client();
        assert_eq!(client.get("/api/test/session").await.status, 204);
        let first_session = session_of(&client).await;

        // Starting: a challenge, bound to this browser, for this session.
        let started = send(&client, Method::POST, &format!("{SECURE}/mfa/enroll/start"), Some(&json!({}))).await;
        assert_eq!(started.status, 200, "{}", started.text());
        let challenge = started.json::<Value>()["challengeId"].as_str().unwrap().to_string();
        let enrolled = send(&client, Method::POST, "/api/test/enrol", Some(&json!({ "challengeId": challenge }))).await;
        assert_eq!(enrolled.status, 200, "{}", enrolled.text());
        let body: Value = enrolled.json();
        assert_eq!(body["recoveryCodes"].as_array().unwrap().len(), 10);
        let method = body["method"].as_str().unwrap().to_string();
        let secret = body["secret"].as_str().unwrap().to_string();
        assert_ne!(Id::parse(body["session"].as_str().unwrap()).unwrap(), first_session, "the proof gives a new session");

        // The method is on the page, and this session has just passed it.
        let after = page(&client).await;
        assert_eq!(after["methods"].as_array().unwrap().len(), 1);
        assert_eq!(after["methods"][0]["id"], method);
        assert_eq!(after["methods"][0]["type"], "totp");
        assert_eq!(after["freshMfa"], true);
        assert_eq!(after["remainingRecoveryCodes"], 10);
        assert_eq!(after["mfaRequired"], true);

        // Proving it again: a challenge for the page's request, answered with the next code.
        let again = send(&client, Method::POST, &format!("{SECURE}/mfa/reverify/start"), None).await;
        assert_eq!(again.status, 200, "{}", again.text());
        assert_eq!(again.json::<Value>()["purpose"], "reverify");
        let challenge = again.json::<Value>()["challengeId"].as_str().unwrap().to_string();
        let answered = send(&client, Method::POST, "/api/test/answer", Some(&json!({ "challengeId": challenge, "method": method, "secret": secret }))).await;
        assert_eq!(answered.status, 200, "{}", answered.text());
        let proved = session_of(&client).await;
        assert_eq!(proved.to_string(), answered.json::<Value>()["session"].as_str().unwrap());

        // A challenge of another browser is nobody else's: the page's own routes
        // refuse a second one while it holds this one's cookie only for itself.
        let other = server.client();
        let refused = send(&other, Method::POST, "/api/test/answer", Some(&json!({ "challengeId": challenge, "method": method, "secret": secret }))).await;
        assert_eq!(refused.status, 500, "no cookie, no challenge");

        // And the account can take the method away with the session that proved it.
        let removed = send(&client, Method::POST, &format!("{SECURE}/mfa/{method}/remove"), Some(&json!({}))).await;
        assert_eq!(removed.status, 204, "{}", removed.text());
        assert_eq!(page(&client).await["methods"], json!([]));
    }
}
