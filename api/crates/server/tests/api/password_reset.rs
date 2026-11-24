//! A forgotten password: a link by mail, once, and a reset that ends everything
//! the old password signed in.

use std::time::{Duration, Instant};

use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::auth::tokens;
use stellarr_api::values::{Id, Timestamp};

use crate::steps::{Instance, token_in, title};
use crate::support::{Client, HubClient, PASSWORD, Request};

const NEW: &str = "Corona!2026-long";
const INVALID: &str = "The password reset link is invalid or has expired.";

async fn forgot(client: &Client, email: &str) -> crate::support::Response {
    client.post_json("/api/auth/password/forgot", &json!({ "email": email })).await
}

async fn reset(client: &Client, token: &str, password: &str) -> crate::support::Response {
    client.post_json("/api/auth/password/reset", &json!({"token": token, "newPassword": password})).await
}

fn rows(instance: &Instance, sql: &str) -> i64 {
    instance.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn an_unknown_address_is_answered_like_a_known_one_and_gets_no_mail() {
    let instance = Instance::with(&["maya"], &[]).await;
    let client = instance.server.client();

    let started = Instant::now();
    let unknown = forgot(&client, "nobody@example.test").await;
    let known = forgot(&client, "  MAYA@example.test ").await;

    // The same answer, and neither comes before the wait every answer has.
    for answer in [&unknown, &known] {
        assert_eq!(answer.status, 202);
        assert!(answer.body.is_empty());
        assert_eq!(answer.header("content-type"), None);
    }
    assert!(started.elapsed() >= Duration::from_millis(160));
    let mails = instance.smtp.until(1).await;
    assert_eq!(mails.len(), 1);
    assert!(mails[0].contains("To: maya@example.test") || mails[0].contains("maya@example.test"));
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"PasswordResetTokens\""), 1);
    // A body that is not one, and one of another type, are not answered as one.
    let broken = client.send(Request::new(http::Method::POST, "/api/auth/password/forgot").body("application/json", "{nope")).await;
    assert_eq!(broken.status, 400);
    let text = client.send(Request::new(http::Method::POST, "/api/auth/password/forgot").body("text/plain", "x")).await;
    assert_eq!(text.status, 415);
}

#[tokio::test]
async fn the_link_sets_a_new_password_once_and_the_token_is_stored_only_as_a_hash() {
    let instance = Instance::with(&["maya"], &[]).await;
    let client = instance.server.client();
    forgot(&client, "maya@example.test").await;
    let token = token_in(&instance.smtp.until(1).await[0]);

    // What is kept is the hash of the token, never the token.
    let stored: String = instance.database().query_row("SELECT \"TokenHash\" FROM \"PasswordResetTokens\"", [], |row| row.get(0)).unwrap();
    assert_eq!(stored, tokens::hash(&token));
    assert_ne!(stored, token);

    // What is wrong with the request is said field by field, and no link is spent on it.
    let short = client.post_json("/api/auth/password/reset", &json!({"newPassword": "short"})).await;
    assert_eq!(short.status, 400);
    let errors = short.json::<Value>()["errors"].clone();
    assert_eq!(errors["newPassword"][0], "Password must be at least 8 characters long.");
    assert_eq!(errors["token"][0], INVALID);
    assert_eq!(reset(&client, &token, &"p".repeat(257)).await.json::<Value>()["errors"]["newPassword"].as_array().unwrap().len(), 1);
    assert_eq!(client.post_json("/api/auth/password/reset", &json!({})).await.status, 400);
    let unknown = reset(&client, "nope", NEW).await;
    assert_eq!((unknown.status.as_u16(), title(&unknown).as_str()), (400, INVALID));

    let done = reset(&client, &token, NEW).await;
    assert_eq!(done.status, 204, "{}", done.text());
    let twice = reset(&client, &token, "another password").await;
    assert_eq!((twice.status.as_u16(), title(&twice).as_str()), (400, INVALID));

    assert_eq!(instance.server.login(&client, "maya", PASSWORD).await.status, 401);
    assert_eq!(instance.server.login(&client, "maya", NEW).await.status, 200);
    assert!(instance.events("maya").contains(&"password-changed password reset".to_string()));
    // The event of a reset by mail says nothing of where it came from: there is no session.
    let (ip, device): (String, String) = instance
        .database()
        .query_row("SELECT \"IpAddress\", \"Device\" FROM \"SecurityEvents\" WHERE \"Kind\" = 'password-changed'", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!((ip.as_str(), device.as_str()), ("", ""));
}

#[tokio::test]
async fn a_reset_ends_every_session_remembered_browser_and_approved_handoff() {
    let instance = Instance::with(&["maya"], &[("Testing__Workers", "realtime-sessions")]).await;
    instance.require();
    let app = instance.enrolled("maya", true).await;
    let session = app.client.cookie("stellarr-dev-session").unwrap();
    let mut hub = HubClient::connect(instance.server.address(), &format!("stellarr-dev-session={session}")).await;
    hub.event("PresenceChanged").await;
    let user: Id = instance.database().query_row("SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'maya'", [], |row| row.get(0)).unwrap();
    instance
        .database()
        .execute(
            "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"UserId\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\")
             VALUES (?1, 'h', ?2, 1, ?3, ?3, ?3)",
            params![Id::random(), user, Timestamp::now()],
        )
        .unwrap();
    assert_eq!(app.client.get("/api/session").await.json::<Value>()["user"]["username"], "maya");

    let anonymous = instance.server.client();
    forgot(&anonymous, "maya@example.test").await;
    let token = token_in(&instance.smtp.until(1).await[0]);
    assert_eq!(reset(&anonymous, &token, NEW).await.status, 204);

    // Its cookie no longer opens anything, and its connection to the hub is closed.
    assert!(app.client.get("/api/session").await.json::<Value>()["user"].is_null());
    let started = Instant::now();
    let close = hub.finish_close().await;
    assert!(close == Some(4401) && started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL"), 1, "only the owner's remains");
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"TrustedDevices\" WHERE \"RevokedAt\" IS NULL"), 0);
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"RedeemedAt\" IS NULL"), 0);
    // The remembered browser is not one any more.
    let challenge = instance.server.login(&app.client, "maya", NEW).await.json::<Value>();
    assert_eq!(challenge["trustedDeviceAvailable"], false);
}

#[tokio::test]
async fn a_newer_link_retires_the_older_one_once_it_is_out() {
    let instance = Instance::with(&["maya"], &[]).await;
    let client = instance.server.client();
    forgot(&client, "maya@example.test").await;
    forgot(&client, "maya@example.test").await;
    let mails = instance.smtp.until(2).await;
    let (older, newer) = (token_in(&mails[0]), token_in(&mails[1]));
    // The older link is retired once the newer is out, a moment after the answer.
    for _ in 0..200 {
        if rows(&instance, "SELECT COUNT(*) FROM \"PasswordResetTokens\" WHERE \"UsedAt\" IS NOT NULL") == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let retired = reset(&client, &older, NEW).await;
    assert_eq!((retired.status.as_u16(), title(&retired).as_str()), (400, INVALID));
    assert_eq!(reset(&client, &newer, NEW).await.status, 204);
}

#[tokio::test]
async fn a_failed_delivery_is_answered_alike_and_keeps_the_previous_link() {
    let instance = Instance::with(&["maya"], &[]).await;
    let client = instance.server.client();
    forgot(&client, "maya@example.test").await;
    let older = token_in(&instance.smtp.until(1).await[0]);

    // The mail server is somewhere nobody listens.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    instance.settings(&format!("\"MailSmtpPort\" = {closed}"));
    let failed = forgot(&client, "maya@example.test").await;
    assert_eq!(failed.status, 202);
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"PasswordResetTokens\""), 2);
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"PasswordResetTokens\" WHERE \"UsedAt\" IS NOT NULL"), 0);
    assert_eq!(reset(&client, &older, NEW).await.status, 204);
}

#[tokio::test]
async fn links_that_ran_out_or_belong_to_disabled_accounts_open_nothing_and_off_switches_send_nothing() {
    let instance = Instance::with(&["maya", "noor", "omar"], &[]).await;
    let client = instance.server.client();
    forgot(&client, "maya@example.test").await;
    forgot(&client, "noor@example.test").await;
    let mails = instance.smtp.until(2).await;
    let (maya, noor) = (token_in(&mails[0]), token_in(&mails[1]));
    let database = instance.database();
    database
        .execute(
            "UPDATE \"PasswordResetTokens\" SET \"ExpiresAt\" = ?1 WHERE \"TokenHash\" = ?2",
            params![Timestamp::now().minus(Duration::from_secs(5)), tokens::hash(&maya)],
        )
        .unwrap();
    database.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'noor'", []).unwrap();
    for token in [&maya, &noor] {
        assert_eq!(reset(&client, token, NEW).await.status, 400);
    }
    // A disabled account is no account.
    assert_eq!(forgot(&client, "noor@example.test").await.status, 202);

    instance.settings("\"RecoveryMailEnabled\" = 0");
    forgot(&client, "omar@example.test").await;
    instance.settings("\"RecoveryMailEnabled\" = 1, \"MailingEnabled\" = 0");
    forgot(&client, "omar@example.test").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(instance.smtp.mails().len(), 2);
    assert_eq!(rows(&instance, "SELECT COUNT(*) FROM \"PasswordResetTokens\""), 2);
}

#[tokio::test]
async fn an_account_without_a_password_gets_its_first_one_through_the_link() {
    let instance = Instance::with(&["maya"], &[]).await;
    instance.database().execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Username\" = 'maya'", []).unwrap();
    let client = instance.server.client();
    forgot(&client, "maya@example.test").await;
    let token = token_in(&instance.smtp.until(1).await[0]);

    assert_eq!(reset(&client, &token, NEW).await.status, 204);
    assert_eq!(instance.server.login(&client, "maya", NEW).await.status, 200);
}

#[tokio::test]
async fn a_slow_mail_server_does_not_make_a_known_address_answer_later() {
    let smtp = crate::steps::Smtp::start_slow(Duration::from_secs(1)).await;
    let instance = Instance::with_mail(&["maya"], &[], smtp).await;
    let client = instance.server.client();

    let timed = |email: &'static str| {
        let client = &client;
        async move {
            let started = Instant::now();
            assert_eq!(forgot(client, email).await.status, 202);
            started.elapsed()
        }
    };
    let known = timed("maya@example.test").await;
    let unknown = timed("nobody@example.test").await;

    assert!(known < Duration::from_millis(600), "{known:?}");
    assert!(unknown < Duration::from_millis(600), "{unknown:?}");
    // The mail still arrives, a second later, and the link works.
    let mails = instance.smtp.until(1).await;
    assert_eq!(mails.len(), 1);
    assert_eq!(reset(&client, &token_in(&mails[0]), NEW).await.status, 204);
}
