//! Signing in and out, and what a session is allowed to do afterwards.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Timestamp;

use crate::support::{PASSWORD, Person, Request, Server};

const POLISH: (&str, &str) = ("STELLARR_LANGUAGE", "pl");

async fn user_of(client: &crate::support::Client) -> Value {
    client.get("/api/session").await.json::<Value>()["user"].clone()
}

fn session_row(server: &Server, user: &str) -> (Timestamp, Timestamp) {
    server
        .database()
        .query_row(
            "SELECT \"LastSeenAt\", \"ExpiresAt\" FROM \"UserSessions\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\" DESC LIMIT 1",
            [user.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn set_session_times(server: &Server, user: &str, last_seen: Timestamp, expires: Timestamp) {
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"LastSeenAt\" = ?2, \"ExpiresAt\" = ?3 WHERE \"UserId\" = ?1",
            params![user.to_uppercase(), last_seen, expires],
        )
        .unwrap();
}

fn ago(minutes: u64) -> Timestamp {
    Timestamp::now().minus(Duration::from_secs(minutes * 60))
}

fn ahead(days: u64) -> Timestamp {
    Timestamp::now().plus(Duration::from_secs(days * 24 * 3600))
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_account_are_turned_away_alike() {
    let server = Server::start_with(&[POLISH]).await;
    server.found("owner").await;

    let wrong = server.login(&server.client(), "owner", "wrong password").await;
    let unknown = server.login(&server.client(), "nobody", "wrong password").await;

    for response in [&wrong, &unknown] {
        assert_eq!(response.status, 401);
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        assert_eq!(response.header("cache-control"), Some("no-store"));
        assert_eq!(response.header("pragma"), Some("no-cache"));
        assert!(response.headers.get("set-cookie").is_none());
        assert_eq!(response.json::<Value>()["title"], "Nieprawidłowy login lub hasło.");
    }
    // Only the account that exists has the failure written down.
    let failures: i64 = server.database().query_row("SELECT count(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'login-failed'", [], |row| row.get(0)).unwrap();
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn the_answer_to_a_wrong_password_is_slowed_a_little() {
    let server = Server::start().await;
    server.found("owner").await;

    let started = std::time::Instant::now();
    server.login(&server.client(), "nobody", "x").await;

    assert!(started.elapsed() >= Duration::from_millis(80));
}

#[tokio::test]
async fn login_accepts_the_username_or_the_email_in_any_letter_case() {
    let server = Server::start().await;
    server.found("owner").await;

    for identifier in ["owner", "OWNER", " Owner ", "owner@example.test", "Owner@EXAMPLE.test"] {
        let response = server.login(&server.client(), identifier, PASSWORD).await;

        assert_eq!(response.status, 200, "{identifier}");
        assert_eq!(response.json::<Value>()["username"], "owner");
    }
}

#[tokio::test]
async fn a_disabled_account_cannot_sign_in() {
    let server = Server::start().await;
    server.found("owner").await;
    server.database().execute("UPDATE \"Users\" SET \"IsDisabled\" = 1", []).unwrap();

    let response = server.login(&server.client(), "owner", PASSWORD).await;

    assert_eq!(response.status, 401);
}

#[tokio::test]
async fn a_sign_in_records_the_device_it_came_from_and_a_security_event() {
    let server = Server::start().await;
    server.found("owner").await;

    let client = server.client().with_header("User-Agent", "Stellarr-Test/1.0");
    assert_eq!(server.login(&client, "owner", PASSWORD).await.status, 200);

    let (device, ip, source): (String, String, String) = server
        .database()
        .query_row("SELECT \"Device\", \"IpAddress\", \"PrimarySource\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert_eq!((device.as_str(), ip.as_str(), source.as_str()), ("Stellarr-Test/1.0", "127.0.0.1", "password"));
    let (kind, event_device, detail): (String, String, String) = server
        .database()
        .query_row("SELECT \"Kind\", \"Device\", \"Detail\" FROM \"SecurityEvents\" ORDER BY \"CreatedAt\" DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert_eq!((kind.as_str(), event_device.as_str(), detail.as_str()), ("login-succeeded", "Stellarr-Test/1.0", "password"));
}

#[tokio::test]
async fn an_address_that_was_confirmed_is_queued_an_alert_and_one_that_was_not_is_not() {
    let server = Server::start().await;
    server.found("owner").await;
    assert_eq!(server.login(&server.client(), "owner", PASSWORD).await.status, 200);
    let none: i64 = server.database().query_row("SELECT count(*) FROM \"SecurityNotifications\"", [], |row| row.get(0)).unwrap();

    server
        .database()
        .execute("UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"", [])
        .unwrap();
    assert_eq!(server.login(&server.client(), "owner", PASSWORD).await.status, 200);

    assert_eq!(none, 0);
    let (address, attempts, sent): (String, i64, Option<String>) = server
        .database()
        .query_row("SELECT \"EmailAddress\", \"Attempts\", \"SentAt\" FROM \"SecurityNotifications\"", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert_eq!((address.as_str(), attempts, sent), ("owner@example.test", 0, None));
}

#[tokio::test]
async fn the_session_check_answers_anonymous_callers_without_a_user_and_the_signed_in_with_theirs() {
    let server = Server::start().await;
    let owner = server.found("owner").await;

    let anonymous = server.client().get("/api/session").await;
    let signed_in = owner.client.get("/api/session").await;

    assert_eq!(anonymous.json::<Value>(), json!({ "user": null }));
    assert!(anonymous.headers.get("set-cookie").is_none());
    let user = signed_in.json::<Value>()["user"].clone();
    assert_eq!((user["username"].as_str(), user["id"].as_str()), (Some("owner"), Some(owner.id.as_str())));
    assert_eq!(user["presence"], "offline");
}

#[tokio::test]
async fn a_logout_revokes_the_session_so_its_cookie_stops_working() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let stale_cookie = owner.client.cookie("__Host-stellarr-session").or(owner.client.cookie("stellarr-dev-session")).unwrap();
    let name = if owner.client.cookie("stellarr-dev-session").is_some() { "stellarr-dev-session" } else { "__Host-stellarr-session" };

    let response = owner.client.post_json("/api/auth/logout", &json!({})).await;

    assert_eq!(response.status, 204);
    assert!(response.header("set-cookie").unwrap().starts_with(&format!("{name}=; expires=Thu, 01 Jan 1970 00:00:00 GMT")));
    assert_eq!(response.header("cache-control"), Some("no-cache,no-store"));
    let revoked: Option<String> = server.database().query_row("SELECT \"RevokedAt\" FROM \"UserSessions\"", [], |row| row.get(0)).unwrap();
    assert!(revoked.is_some());
    let event: i64 = server.database().query_row("SELECT count(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'session-revoked' AND \"Detail\" = 'logout'", [], |row| row.get(0)).unwrap();
    assert_eq!(event, 1);
    // The hub is told, so that it closes what the session had open.
    let ended: Vec<Value> = server.events().into_iter().filter(|event| event["name"] == "SessionEnded").collect();
    let session: String = server.database().query_row("SELECT \"Id\" FROM \"UserSessions\"", [], |row| row.get(0)).unwrap();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0]["audience"]["users"][0], owner.id.as_str());
    assert_eq!(ended[0]["payload"]["session"].as_str().unwrap().to_uppercase(), session);
    // The cookie alone is not trusted: sent again by hand, it is refused.
    let replay = server.anonymous().send(Request::new(Method::GET, "/api/me").header("Cookie", &format!("{name}={stale_cookie}"))).await;
    assert_eq!(replay.status, 401);
}

#[tokio::test]
async fn a_logout_needs_a_signed_in_caller() {
    let server = Server::start().await;
    server.found("owner").await;

    let response = server.client().post_json("/api/auth/logout", &json!({})).await;

    assert_eq!(response.status, 401);
    assert!(response.body.is_empty());
    assert!(response.headers.get("set-cookie").is_none());
}

#[tokio::test]
async fn a_cookie_is_refused_once_its_session_or_account_no_longer_allows_it() {
    for change in ["revoked", "expired", "absolute-missing", "disabled", "version-bumped", "session-deleted"] {
        let server = Server::start().await;
        let owner = server.found("owner").await;
        assert!(!user_of(&owner.client).await.is_null());
        let connection = server.database();
        match change {
            "revoked" => connection.execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1", [Timestamp::now()]).unwrap(),
            "expired" => connection.execute("UPDATE \"UserSessions\" SET \"ExpiresAt\" = ?1", [ago(1)]).unwrap(),
            "absolute-missing" => connection.execute("UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = NULL", []).unwrap(),
            "disabled" => connection.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1", []).unwrap(),
            "version-bumped" => connection.execute("UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1", []).unwrap(),
            _ => connection.execute("DELETE FROM \"UserSessions\"", []).unwrap(),
        };

        let session = owner.client.get("/api/session").await;
        let logout = owner.client.post_json("/api/auth/logout", &json!({})).await;

        assert!(session.json::<Value>()["user"].is_null(), "{change}");
        // The refused cookie is taken away.
        assert!(session.header("set-cookie").is_some_and(|line| line.contains("expires=Thu, 01 Jan 1970")), "{change}");
        assert_eq!(logout.status, 401, "{change}");
    }
}

#[tokio::test]
async fn a_cookie_that_is_not_ours_is_just_not_signed_in() {
    let server = Server::start().await;
    server.found("owner").await;

    for cookie in ["garbage", "", "AAAA"] {
        let response = server
            .anonymous()
            .send(Request::new(Method::GET, "/api/session").header("Cookie", &format!("stellarr-dev-session={cookie}; __Host-stellarr-session={cookie}")).header("X-Stellarr-Client", "web").header("X-Stellarr-Client-Version", "2.0.0"))
            .await;

        assert_eq!(response.json::<Value>(), json!({ "user": null }), "{cookie:?}");
        assert!(response.headers.get("set-cookie").is_none(), "{cookie:?}");
    }
}

#[tokio::test]
async fn the_other_sessions_of_an_account_survive_a_logout() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let second = server.sign_in(&owner).await;

    assert_eq!(owner.client.post_json("/api/auth/logout", &json!({})).await.status, 204);

    assert!(user_of(&owner.client).await.is_null());
    assert!(!user_of(&second).await.is_null());
}

#[tokio::test]
async fn a_session_seen_long_ago_is_marked_seen_and_renewed_when_its_end_is_near() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    set_session_times(&server, &owner.id, ago(30), ahead(3));

    assert!(!user_of(&owner.client).await.is_null());

    let (seen, expires) = session_row(&server, &owner.id);
    assert!(seen > ago(1));
    assert!(expires > ahead(29));
}

#[tokio::test]
async fn a_session_seen_recently_is_left_alone() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let (seen, expires) = (ago(2), ahead(3));
    set_session_times(&server, &owner.id, seen, expires);

    assert!(!user_of(&owner.client).await.is_null());

    assert_eq!(session_row(&server, &owner.id), (seen, expires));
}

#[tokio::test]
async fn a_session_with_more_than_half_its_life_left_keeps_its_expiry() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let expires = ahead(20);
    set_session_times(&server, &owner.id, ago(30), expires);

    assert!(!user_of(&owner.client).await.is_null());

    let (seen, kept) = session_row(&server, &owner.id);
    assert!(seen > ago(1));
    assert_eq!(kept, expires);
}

#[tokio::test]
async fn a_session_older_than_thirty_days_is_accepted_within_its_absolute_expiry() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let now = Timestamp::now();
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"CreatedAt\" = ?1, \"LastSeenAt\" = ?2, \"ExpiresAt\" = ?3, \"AbsoluteExpiresAt\" = ?3",
            params![now.minus(Duration::from_secs(40 * 24 * 3600)), now, ahead(5)],
        )
        .unwrap();

    assert!(!user_of(&owner.client).await.is_null());
}

#[tokio::test]
async fn a_renewal_never_goes_past_the_absolute_end() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let end = ahead(5);
    set_session_times(&server, &owner.id, ago(30), ahead(1));
    server.database().execute("UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = ?1", [end]).unwrap();

    assert!(!user_of(&owner.client).await.is_null());

    assert_eq!(session_row(&server, &owner.id).1, end);
}

#[tokio::test]
async fn a_sign_in_that_does_not_keep_the_person_signed_in_ends_after_twelve_hours_and_leaves_with_the_browser() {
    let server = Server::start().await;
    server.found("owner").await;

    let response = server
        .client()
        .post_json("/api/auth/login", &json!({ "username": "owner", "password": PASSWORD, "keepSignedIn": false }))
        .await;

    assert_eq!(response.status, 200);
    let cookie = response.header("set-cookie").unwrap();
    assert!(!cookie.contains("expires="), "{cookie}");
    assert!(cookie.ends_with("path=/; samesite=lax; httponly"), "{cookie}");
    let (created, expires, short): (Timestamp, Timestamp, bool) = server
        .database()
        .query_row("SELECT \"CreatedAt\", \"ExpiresAt\", \"ShortLived\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert!(short);
    assert_eq!(created.until(expires), Duration::from_secs(12 * 3600));
}

#[tokio::test]
async fn a_kept_sign_in_has_a_cookie_that_outlives_the_browser_by_thirty_days() {
    let server = Server::start().await;
    server.found("owner").await;

    let response = server.login(&server.client(), "owner", PASSWORD).await;

    let cookie = response.header("set-cookie").unwrap();
    assert!(cookie.starts_with("stellarr-dev-session=") || cookie.starts_with("__Host-stellarr-session="), "{cookie}");
    assert!(cookie.contains("; expires="), "{cookie}");
    assert!(cookie.ends_with("path=/; samesite=lax; httponly"), "{cookie}");
    let (created, expires): (Timestamp, Timestamp) = server
        .database()
        .query_row("SELECT \"CreatedAt\", \"ExpiresAt\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!(created.until(expires), Duration::from_secs(30 * 24 * 3600));
}

#[tokio::test]
async fn the_session_cookie_is_secure_with_a_host_prefix_in_production_and_plain_in_testing() {
    let production = Server::start_with(&[("ASPNETCORE_ENVIRONMENT", "Production"), ("Rtc__TurnSharedSecret", "a-secret")]).await;
    let testing = Server::start().await;
    production.found("owner").await;
    testing.found("owner").await;

    let secure = production.login(&production.client(), "owner", PASSWORD).await;
    let plain = testing.login(&testing.client(), "owner", PASSWORD).await;
    let behind_https = testing
        .login(&testing.client().with_header("X-Forwarded-Proto", "https"), "owner", PASSWORD)
        .await;

    assert!(secure.header("set-cookie").unwrap().starts_with("__Host-stellarr-session="));
    assert!(secure.header("set-cookie").unwrap().contains("; path=/; secure; samesite=lax; httponly"));
    assert!(plain.header("set-cookie").unwrap().starts_with("stellarr-dev-session="));
    assert!(!plain.header("set-cookie").unwrap().contains("secure"));
    assert!(behind_https.header("set-cookie").unwrap().contains("; secure;"));
}

/// A cookie of the signed-in person that was issued twenty days ago and ends in
/// ten, which the server makes by sealing a ticket with the key on its volume.
fn cookie_due_for_renewal(server: &Server, owner: &Person) -> String {
    use stellarr_api::seal::Sealer;
    let sealer = Sealer::load_or_create(&server.folder().join("keys")).unwrap();
    let now = Timestamp::now();
    let current = owner.client.cookie("stellarr-dev-session").unwrap();
    let payload = sealer.open("session-cookie", b"", &current, now).unwrap();
    let mut ticket: Value = serde_json::from_slice(&payload).unwrap();
    ticket["issued"] = json!(now.minus(Duration::from_secs(20 * 24 * 3600)).unix_seconds());
    ticket["expires"] = json!(now.plus(Duration::from_secs(10 * 24 * 3600)).unix_seconds());
    let old = sealer.seal(
        "session-cookie",
        b"",
        Timestamp::from_unix_seconds(ticket["expires"].as_i64().unwrap()),
        ticket.to_string().as_bytes(),
    );
    format!("stellarr-dev-session={old}")
}

fn browser_request(method: Method, path: &str, cookie: &str) -> Request {
    Request::new(method, path)
        .header("Cookie", cookie)
        .header("X-Stellarr-Client", "web")
        .header("X-Stellarr-Client-Version", "2.0.0")
}

#[tokio::test]
async fn the_cookie_is_renewed_once_less_of_its_life_is_left_than_has_gone() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let old = cookie_due_for_renewal(&server, &owner);

    let response = server.anonymous().send(browser_request(Method::GET, "/api/session", &old)).await;

    assert!(response.json::<Value>()["user"].is_object());
    let renewed = response.header("set-cookie").expect("a renewed cookie");
    assert!(renewed.starts_with("stellarr-dev-session="), "{renewed}");
    assert_ne!(renewed.split(';').next(), old.split(';').next());
    assert_eq!(response.header("cache-control"), Some("no-cache,no-store"));
}

#[tokio::test]
async fn a_cookie_due_for_renewal_is_left_alone_when_the_handler_signs_in_or_out_itself() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let old = cookie_due_for_renewal(&server, &owner);

    let mut login = browser_request(Method::POST, "/api/auth/login", &old);
    login = login.json(&json!({ "username": "owner", "password": PASSWORD }));
    let signed_in = server.anonymous().send(login).await;
    let signed_out = server.anonymous().send(browser_request(Method::POST, "/api/auth/logout", &old)).await;

    for response in [&signed_in, &signed_out] {
        let lines: Vec<_> = response.headers.get_all("set-cookie").iter().collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
    }
    assert!(signed_out.header("set-cookie").unwrap().contains("expires=Thu, 01 Jan 1970"));
}

#[tokio::test]
async fn password_attempts_are_limited_per_address() {
    let server = Server::start_with(&[("ASPNETCORE_ENVIRONMENT", "Production"), ("Rtc__TurnSharedSecret", "a-secret")]).await;
    server.found("owner").await;
    let client = server.client();

    let mut statuses = Vec::new();
    for _ in 0..12 {
        statuses.push(server.login(&client, "owner", "wrong").await.status.as_u16());
    }

    assert_eq!(&statuses[..10], &[401; 10]);
    assert_eq!(&statuses[10..], &[429, 429]);
}

#[tokio::test]
async fn me_needs_a_session_and_answers_the_account_as_only_its_owner_sees_it() {
    let server = Server::start().await;
    let owner = server.found("owner").await;

    let anonymous = server.client().get("/api/me").await;
    let mine = owner.client.get("/api/me").await;

    assert_eq!(anonymous.status, 401);
    assert!(anonymous.body.is_empty());
    let account = mine.json::<Value>();
    assert_eq!(account["email"], "owner@example.test");
    assert_eq!(account["customStatus"], "");
    assert_eq!(account["preferences"]["vadSensitivity"], 60);
    assert_eq!(account["preferences"]["mutedChannelIds"], json!([]));
    assert!(account["wallpaperUrl"].is_null());
    let created = account["createdAt"].as_str().unwrap();
    assert!(!created.ends_with('Z') && created.contains('T'), "{created}");
    assert!(account.get("passwordHash").is_none());
}

#[tokio::test]
async fn a_person_who_needs_a_second_step_gets_a_challenge_and_never_a_session_without_it() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let server_id: String = server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    server
        .database()
        .execute(
            "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"InviteMailEnabled\",
                 \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\",
                 \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\")
             VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, '2026-11-24 00:00:00', 'everyone')",
            [&server_id],
        )
        .unwrap();

    let client = server.client();
    let response = server.login(&client, "maya", PASSWORD).await;

    assert_eq!(response.status, 200);
    let challenge = response.json::<Value>();
    assert_eq!(challenge["status"], "mfa_enrollment_required");
    assert_eq!(challenge["purpose"], "enrollment");
    assert_eq!(challenge["methods"], json!([]));
    assert_eq!(challenge["account"]["username"], "maya");
    assert_eq!(challenge["trustedDeviceDays"], 14);
    assert_eq!(challenge["passkeyAvailable"], false);
    assert!(challenge["challengeId"].is_string());
    let preauth = response.header("set-cookie").unwrap();
    assert!(preauth.starts_with("stellarr-dev-preauth="), "{preauth}");
    assert!(client.cookie("stellarr-dev-session").is_none());
    assert!(user_of(&client).await.is_null());
    let sessions: i64 = server.database().query_row("SELECT count(*) FROM \"UserSessions\" WHERE \"UserId\" = ?1", [maya.id.to_uppercase()], |row| row.get(0)).unwrap();
    // The one session is the registration's, made before the policy asked.
    assert_eq!(sessions, 1);
    let challenges: (String, i64) = server
        .database()
        .query_row("SELECT \"Purpose\", \"MaxAttempts\" FROM \"AuthChallenges\"", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!(challenges, ("enrollment".to_string(), 5));
    // The session the registration made does not survive the policy either.
    assert!(user_of(&maya.client).await.is_null());
}

#[tokio::test]
async fn an_account_with_a_second_step_gets_the_challenge_with_its_methods() {
    let server = Server::start().await;
    server.found("owner").await;
    server.register("maya").await;
    let user: String = server.database().query_row("SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'maya'", [], |row| row.get(0)).unwrap();
    server
        .database()
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"ProtectedSecret\")
             VALUES ('11111111-1111-1111-1111-111111111111', ?1, 'totp', 'Phone', '2026-11-20 10:00:00.5', '2026-11-20 10:01:00', 'x')",
            [&user],
        )
        .unwrap();

    let response = server.login(&server.client(), "maya", PASSWORD).await;

    let challenge = response.json::<Value>();
    assert_eq!(challenge["status"], "mfa_required");
    assert_eq!(challenge["purpose"], "mfa");
    assert_eq!(challenge["emailCodeAvailable"], false);
    assert_eq!(
        challenge["methods"],
        json!([{ "id": "11111111-1111-1111-1111-111111111111", "type": "totp", "name": "Phone", "createdAt": "2026-11-20T10:00:00.5Z", "lastUsedAt": null, "emailAddress": null, "pending": false }])
    );
}

#[tokio::test]
async fn an_account_that_must_be_recovered_at_the_console_gets_no_session() {
    let server = Server::start().await;
    server.found("owner").await;
    server.database().execute("UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1", []).unwrap();
    let client = server.client();

    let response = server.login(&client, "owner", PASSWORD).await;

    assert_eq!(response.json::<Value>(), json!({ "status": "security_recovery_required" }));
    assert!(response.headers.get("set-cookie").is_none());
}

#[tokio::test]
async fn a_login_that_asks_to_link_a_waiting_identity_finds_none() {
    let server = Server::start().await;
    server.found("owner").await;

    let response = server
        .client()
        .post_json("/api/auth/login", &json!({ "username": "owner", "password": PASSWORD, "linkPending": true }))
        .await;

    let body = response.json::<Value>();
    assert_eq!((body["identityLink"].as_str(), body["user"]["username"].as_str()), (Some("failed"), Some("owner")));
    let cookies: Vec<_> = response.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cookies[0].starts_with("stellarr-dev-session="), "{cookies:?}");
    assert!(cookies[1].starts_with("stellarr-dev-pending-identity=; expires=Thu, 01 Jan 1970"), "{cookies:?}");
}

#[tokio::test]
async fn a_sign_in_without_a_body_is_turned_away_as_not_to_be_stored() {
    let server = Server::start().await;

    let none = server.client().send(Request::new(Method::POST, "/api/auth/login")).await;
    let broken = server.client().send(Request::new(Method::POST, "/api/auth/login").body("application/json", "{bad")).await;
    let empty = server.client().post_json("/api/auth/login", &json!({})).await;

    assert_eq!(none.status, 400);
    assert_eq!(none.header("cache-control"), Some("no-store"));
    assert_eq!(broken.status, 400);
    assert_eq!(broken.header("cache-control"), None);
    assert_eq!(empty.status, 401);
}

#[tokio::test]
async fn a_session_of_an_unsupported_build_is_refused() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let without_headers = server.anonymous();
    let cookie = owner.client.cookie("stellarr-dev-session").unwrap();

    let session = without_headers.send(Request::new(Method::GET, "/api/session").header("Cookie", &format!("stellarr-dev-session={cookie}"))).await;
    let bootstrap = without_headers.send(Request::new(Method::GET, "/api/bootstrap").header("Cookie", &format!("stellarr-dev-session={cookie}"))).await;

    assert_eq!([session.status, bootstrap.status], [403, 403]);
}

#[tokio::test]
async fn a_current_desktop_build_signs_in_and_reads_its_session() {
    let server = Server::start_with(&[("Desktop__MinimumVersion", "0.1.3")]).await;
    server.found("owner").await;
    let client = server.client_of("desktop-linux", "0.1.3");

    assert_eq!(server.login(&client, "owner", PASSWORD).await.status, 200);

    assert_eq!(user_of(&client).await["username"], "owner");
}

#[tokio::test]
async fn an_instance_that_asks_for_no_identification_lets_a_script_sign_in() {
    let server = Server::start_with(&[("Clients__RequireIdentification", "false")]).await;
    server.found("owner").await;
    let script = server.anonymous();

    assert_eq!(server.login(&script, "owner", PASSWORD).await.status, 200);
    assert_eq!(user_of(&script).await["username"], "owner");
}

#[tokio::test]
async fn an_account_written_by_one_server_signs_in_on_the_other_because_the_stored_form_is_the_same() {
    // The hash a first-server database holds for this password, made by the
    // same parameters: written elsewhere, it verifies here.
    let server = Server::start().await;
    server.found("owner").await;
    server
        .database()
        .execute(
            "UPDATE \"Users\" SET \"PasswordHash\" = 'pbkdf2-sha256$210000$AAECAwQFBgcICQoLDA0ODw==$F7XbbcEiPZ9xiSyI75OfVvMnEQBrxxhoPRwTxTL13LM='",
            [],
        )
        .unwrap();

    let response = server.login(&server.client(), "owner", "Passw0rd!2026").await;

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn a_sign_in_records_the_address_the_listed_proxy_reports() {
    for (proxies, forwarded, recorded) in [
        ("172.18.0.1", "203.0.113.66, 198.51.100.7, 172.18.0.1", "198.51.100.7"),
        ("", "198.51.100.7, 172.18.0.1", "172.18.0.1"),
        ("2001:db8::/32", "2001:db8:ff::7, 2001:db8::1", "2001:db8:ff::7"),
    ] {
        let server = Server::start_with(&[("STELLARR_TRUSTED_PROXIES", proxies)]).await;
        server.found("owner").await;
        let client = server.client().with_header("X-Forwarded-For", forwarded);

        assert_eq!(server.login(&client, "owner", PASSWORD).await.status, 200);

        let (session_address, event_address): (String, String) = server
            .database()
            .query_row(
                "SELECT (SELECT \"IpAddress\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1),
                        (SELECT \"IpAddress\" FROM \"SecurityEvents\" WHERE \"Kind\" = 'login-succeeded')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((session_address.as_str(), event_address.as_str()), (recorded, recorded), "{proxies}");
    }
}

#[tokio::test]
async fn a_short_session_is_not_stretched_by_use_and_ends_on_time() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let client = server.client();
    let login = client
        .post_json("/api/auth/login", &json!({ "username": "owner", "password": PASSWORD, "keepSignedIn": false }))
        .await;
    assert_eq!(login.status, 200);
    let (absolute, expires, short): (Timestamp, Timestamp, bool) = server
        .database()
        .query_row(
            "SELECT \"AbsoluteExpiresAt\", \"ExpiresAt\", \"ShortLived\" FROM \"UserSessions\" WHERE \"ShortLived\" = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(short);
    assert_eq!(absolute, expires);
    // Seen long enough ago that the next request renews it.
    server.database().execute("UPDATE \"UserSessions\" SET \"LastSeenAt\" = ?1 WHERE \"ShortLived\" = 1", [ago(60)]).unwrap();

    assert!(!user_of(&client).await.is_null());

    let (renewed_seen, renewed_end): (Timestamp, Timestamp) = server
        .database()
        .query_row("SELECT \"LastSeenAt\", \"ExpiresAt\" FROM \"UserSessions\" WHERE \"ShortLived\" = 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert!(renewed_seen > ago(1));
    assert!(renewed_end <= absolute);
    server
        .database()
        .execute("UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = ?1, \"LastSeenAt\" = ?2 WHERE \"ShortLived\" = 1", params![ago(1), ago(60)])
        .unwrap();
    assert!(user_of(&client).await.is_null());
    // The kept session of the first sign-in is another one.
    assert!(!user_of(&owner.client).await.is_null());
}

/// Counts, through the seam of the password module, that every way of being
/// turned away derives the password once. It is one test because the count is
/// the process's, and nothing else in this binary derives passwords in it.
#[tokio::test]
async fn every_refusal_of_a_sign_in_does_the_work_of_a_real_check() {
    use stellarr_api::auth::password;
    let server = Server::in_process(stellarr_api::accounts::routes(), &[]).await;
    let connection = server.database();
    let good = password::hash(PASSWORD);
    for (name, hash, disabled) in [
        ("real", Some(good.as_str()), false),
        ("disabled", Some(good.as_str()), true),
        ("nohash", None, false),
        ("garbage", Some("not a hash"), false),
        ("otherscheme", Some("pbkdf2-sha512$210000$AAECAwQFBgcICQoLDA0ODw==$F7XbbcEiPZ9xiSyI75OfVvMnEQBrxxhoPRwTxTL13LM="), false),
        ("fewrounds", Some("pbkdf2-sha256$10$AAECAwQFBgcICQoLDA0ODw==$F7XbbcEiPZ9xiSyI75OfVvMnEQBrxxhoPRwTxTL13LM="), false),
    ] {
        connection
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\", \"SessionVersion\",
                     \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\") VALUES (?1, ?2, ?3, 'X', ?4, ?5, 0, 0, 'c', '2026-11-24 00:00:00')",
                params![stellarr_api::values::Id::random().to_db_text(), name, name.to_uppercase(), hash, disabled],
            )
            .unwrap();
    }
    let client = server.client();

    let mut counts = Vec::new();
    for name in ["real", "disabled", "nohash", "garbage", "otherscheme", "fewrounds", "nobody-here"] {
        let before = password::derivations();
        let response = server.login(&client, name, "not the password").await;
        assert_eq!(response.status, 401, "{name}");
        counts.push((name, password::derivations() - before));
    }

    assert!(counts.iter().all(|(_, derived)| *derived == 1), "{counts:?}");
    let failures: i64 = connection.query_row("SELECT count(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'login-failed'", [], |row| row.get(0)).unwrap();
    assert_eq!(failures, 6);
}

/// An account that has proved its address to the instance, as an alert needs.
fn verify_address(server: &Server, user: &str) {
    let connection = server.database();
    connection
        .execute(
            "UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\" WHERE \"Id\" = ?1",
            [user.to_uppercase()],
        )
        .unwrap();
}

fn alerts(server: &Server) -> i64 {
    server
        .database()
        .query_row("SELECT COUNT(*) FROM \"SecurityNotifications\"", [], |row| row.get(0))
        .unwrap()
}

#[tokio::test]
async fn signing_in_alerts_an_account_that_has_an_address_but_signing_out_does_not() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    verify_address(&server, &owner.id);

    let client = server.sign_in(&owner).await;
    // A sign-in is something to be told about; it is queued for the address.
    assert_eq!(alerts(&server), 1);

    let response = client.post_json("/api/auth/logout", &json!({})).await;

    assert_eq!(response.status, 204);
    // The sign-out is on the account's record, and nobody is mailed about it.
    let recorded: i64 = server
        .database()
        .query_row(
            "SELECT COUNT(*) FROM \"SecurityEvents\" WHERE \"Kind\" = 'session-revoked' AND \"Detail\" = 'logout'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recorded, 1);
    assert_eq!(alerts(&server), 1);
}
