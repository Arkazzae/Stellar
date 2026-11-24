//! The desktop browser sign-in, as far as the browser's half: starting a
//! request, the page that asks the person, and the post that approves.
//!
//! The pages are compared byte for byte with what the first server sent (the
//! copies below were taken from it), and `tests/desktop-login-shared-database.sh`
//! compares the whole conversation with the first server's, request by request,
//! on one database file.

use http::Method;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use stellarr_api::auth::tokens;
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, Person, Request, Response, Server};

const PROVIDER: [(&str, &str); 4] = [
    ("Oidc__Enabled", "true"),
    ("Oidc__Authority", "https://identity.test"),
    ("Oidc__ClientId", "stellarr-tests"),
    ("Oidc__ClientSecret", "test-client-secret"),
];

/// The page of a request that is no longer good, in English.
const EXPIRED: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>The link has expired</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, system-ui, sans-serif; }
    body { min-height: 100vh; margin: 0; display: grid; place-items: center; background: #101112; color: #f3f1ec; }
    main { width: min(34rem, calc(100% - 2rem)); padding: 2rem; border: 1px solid #343638; border-radius: 1rem; background: #191b1d; }
    h1 { margin: 0 0 .75rem; font-size: 1.5rem; }
    p { margin: 0; color: #b8b9b7; line-height: 1.55; }
  </style>
</head>
<body><main><h1>The link has expired</h1><p>Go back to Stellarr Desktop and start signing in again.</p></main></body>
</html>"#;
/// The page of an approval that was made, in English.
const DONE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Signed in to Stellarr</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, system-ui, sans-serif; }
    body { min-height: 100vh; margin: 0; display: grid; place-items: center; background: #101112; color: #f3f1ec; }
    main { width: min(34rem, calc(100% - 2rem)); padding: 2rem; border: 1px solid #343638; border-radius: 1rem; background: #191b1d; }
    h1 { margin: 0 0 .75rem; font-size: 1.5rem; }
    p { margin: 0; color: #b8b9b7; line-height: 1.55; }
  </style>
</head>
<body><main><h1>Signed in to Stellarr</h1><p>You can close this tab and return to the desktop app.</p></main></body>
</html>"#;
/// The page that asks, with `<id>` and `<token>` where the request and the
/// token go.
const ASK: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Approve this sign-in?</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, system-ui, sans-serif; }
    body { min-height: 100vh; margin: 0; display: grid; place-items: center; background: #101112; color: #f3f1ec; }
    main { width: min(34rem, calc(100% - 2rem)); padding: 2rem; border: 1px solid #343638; border-radius: 1rem; background: #191b1d; }
    h1 { margin: 0 0 .75rem; font-size: 1.5rem; }
    p { margin: 0 0 1.5rem; color: #b8b9b7; line-height: 1.55; }
    button { padding: .8rem 1.1rem; border: 0; border-radius: .5rem; background: #d6a554; color: #17130d; font: inherit; font-weight: 700; cursor: pointer; }
  </style>
</head>
<body><main>
  <h1>Approve this sign-in?</h1>
  <p>Only continue if you started this sign-in in Stellarr Desktop. Approving signs that app in as you.</p>
  <form method="post" action="/api/auth/desktop/browser/verify/<id>">
    <input type="hidden" name="token" value="<token>">
    <button type="submit">Approve sign-in</button>
  </form>
</main></body>
</html>"#;
const FORM: &str = "application/x-www-form-urlencoded";

/// A way of writing the form: what it is called, its type, and how it writes a token.
type Spelling = (&'static str, &'static str, Box<dyn Fn(&str) -> String>);

/// The application: it says what the desktop build says, and holds no session.
fn application(server: &Server) -> Client {
    server.client_of("desktop-linux", "0.1.3")
}

struct Started {
    id: String,
    secret: String,
}

async fn start(server: &Server) -> Started {
    let response = application(server)
        .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
        .await;
    assert_eq!(response.status, 200, "{}", response.text());
    let body: Value = response.json();
    Started {
        id: body["requestId"].as_str().unwrap().to_string(),
        secret: body["secret"].as_str().unwrap().to_string(),
    }
}

fn page_of(id: &str) -> String {
    format!("/api/auth/desktop/browser/verify/{id}")
}

fn token_of(html: &str) -> String {
    let marker = "name=\"token\" value=\"";
    let start = html.find(marker).expect("the page carries a token") + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_string()
}

/// The person looks at the page: the token it carries.
async fn look(client: &Client, id: &str) -> String {
    let page = client.get(&page_of(id)).await;
    assert_eq!(page.status, 200, "{}", page.text());
    token_of(&page.text())
}

fn post(id: &str, body: &str) -> Request {
    Request::new(Method::POST, &page_of(id)).body(FORM, body)
}

async fn approve(client: &Client, id: &str, token: &str) -> Response {
    client.send(post(id, &format!("token={token}"))).await
}

/// What the database holds about a request.
#[derive(Debug)]
struct Row {
    secret_hash: String,
    token_hash: Option<String>,
    approval_user: Option<String>,
    approval_session: Option<String>,
    user: Option<String>,
    approved_session: Option<String>,
    primary_source: Option<String>,
    mfa_completed_at: Option<String>,
    mfa_evidence: Option<String>,
    version: i64,
    created_at: Timestamp,
    expires_at: Timestamp,
    approved_at: Option<String>,
    redeemed_at: Option<String>,
}

fn row(connection: &Connection, id: &str) -> Row {
    connection
        .query_row(
            "SELECT \"SecretHash\", \"ApprovalTokenHash\", \"ApprovalUserId\", \"ApprovalSessionId\", \"UserId\", \"ApprovedSessionId\",
                    \"PrimarySource\", \"MfaCompletedAt\", \"MfaEvidence\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\", \"ApprovedAt\", \"RedeemedAt\"
             FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
            [Id::parse(id).unwrap()],
            |row| {
                Ok(Row {
                    secret_hash: row.get(0)?,
                    token_hash: row.get(1)?,
                    approval_user: row.get(2)?,
                    approval_session: row.get(3)?,
                    user: row.get(4)?,
                    approved_session: row.get(5)?,
                    primary_source: row.get(6)?,
                    mfa_completed_at: row.get(7)?,
                    mfa_evidence: row.get(8)?,
                    version: row.get(9)?,
                    created_at: row.get(10)?,
                    expires_at: row.get(11)?,
                    approved_at: row.get(12)?,
                    redeemed_at: row.get(13)?,
                })
            },
        )
        .expect("the request has a row")
}

fn approval_events(connection: &Connection, user: &str) -> Vec<(String, String, String)> {
    let mut statement = connection
        .prepare("SELECT \"Detail\", \"IpAddress\", \"Device\" FROM \"SecurityEvents\" WHERE \"UserId\" = ?1 AND \"Kind\" = 'desktop-approved'")
        .unwrap();
    statement
        .query_map([Id::parse(user).unwrap()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

async fn with_provider() -> (Server, Person) {
    let server = Server::start_with(&PROVIDER).await;
    let maja = server.found("maja").await;
    (server, maja)
}

#[tokio::test]
async fn a_request_cannot_be_started_without_a_provider_to_send_the_person_to() {
    let server = Server::start().await;

    let response = application(&server)
        .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
        .await;

    assert_eq!(response.status, 404);
    assert!(response.body.is_empty());
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.header("pragma"), Some("no-cache"));
    let rows: i64 = server
        .database()
        .query_row("SELECT COUNT(*) FROM \"DesktopBrowserLogins\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_provider_that_is_not_complete_is_no_provider() {
    for missing in ["Oidc__Enabled", "Oidc__ClientId", "Oidc__ClientSecret"] {
        let settings: Vec<_> = PROVIDER.into_iter().filter(|(name, _)| *name != missing).collect();
        let server = Server::start_with(&settings).await;
        let response = application(&server)
            .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
            .await;
        assert_eq!(response.status, 404, "{missing}");
    }
    let http = Server::start_with(&[
        ("Oidc__Enabled", "true"),
        ("Oidc__Authority", "http://identity.test"),
        ("Oidc__ClientId", "x"),
        ("Oidc__ClientSecret", "y"),
    ])
    .await;
    let response = application(&http)
        .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
        .await;
    assert_eq!(response.status, 404);
}

#[tokio::test]
async fn a_request_is_started_with_a_secret_that_is_kept_only_as_a_hash() {
    let server = Server::start_with(&PROVIDER).await;

    let response = application(&server)
        .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
        .await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.header("pragma"), Some("no-cache"));
    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert!(response.headers.get_all("set-cookie").iter().next().is_none());
    // The members are in the order the application reads them in.
    let text = response.text();
    let order: Vec<usize> = ["requestId", "secret", "verificationUrl", "expiresAt"]
        .iter()
        .map(|name| text.find(&format!("\"{name}\"")).unwrap())
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    let body: Value = response.json();
    let id = body["requestId"].as_str().unwrap();
    let secret = body["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 43);
    assert!(secret.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    assert_eq!(
        body["verificationUrl"],
        format!("/api/auth/oidc/login?returnUrl=%2Fapi%2Fauth%2Fdesktop%2Fbrowser%2Fverify%2F{id}")
    );

    let row = row(&server.database(), id);
    assert_eq!(row.secret_hash, tokens::hash(secret));
    assert_eq!(row.secret_hash.len(), 64);
    assert!(!row.secret_hash.contains(secret));
    assert_eq!(row.expires_at.to_wire_text(), body["expiresAt"]);
    assert_eq!(row.expires_at.unix_seconds() - row.created_at.unix_seconds(), 120);
    assert!(row.token_hash.is_none() && row.user.is_none() && row.approved_at.is_none() && row.redeemed_at.is_none());
    assert_eq!(row.version, 0);
    // Two requests share nothing.
    let other = start(&server).await;
    assert_ne!(other.id, id);
    assert_ne!(other.secret, secret);
}

#[tokio::test]
async fn with_several_providers_the_person_is_sent_to_the_sign_in_page() {
    let mut settings = PROVIDER.to_vec();
    settings.extend([
        ("Oidc__Providers__work__Authority", "https://work.test"),
        ("Oidc__Providers__work__ClientId", "w"),
        ("Oidc__Providers__work__ClientSecret", "s"),
    ]);
    let server = Server::start_with(&settings).await;

    let response = application(&server)
        .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
        .await;

    let body: Value = response.json();
    let id = body["requestId"].as_str().unwrap();
    assert_eq!(
        body["verificationUrl"],
        format!("/login?returnUrl=%2Fapi%2Fauth%2Fdesktop%2Fbrowser%2Fverify%2F{id}")
    );
    // Only a named provider: its own sign-in is where the person is sent.
    let named = Server::start_with(&[
        ("Oidc__Providers__work__Authority", "https://work.test"),
        ("Oidc__Providers__work__ClientId", "w"),
        ("Oidc__Providers__work__ClientSecret", "s"),
    ])
    .await;
    let response = application(&named)
        .send(Request::new(Method::POST, "/api/auth/desktop/browser/start"))
        .await;
    let body: Value = response.json();
    assert!(body["verificationUrl"].as_str().unwrap().starts_with("/api/auth/oidc/work/login?returnUrl="));
}

#[tokio::test]
async fn starting_clears_out_the_requests_that_ran_out() {
    let server = Server::start_with(&PROVIDER).await;
    let stale = Id::random();
    let live = Id::random();
    let now = Timestamp::now();
    {
        let connection = server.database();
        for (id, expires) in [(stale, now.minus(std::time::Duration::from_secs(1))), (live, now.plus(std::time::Duration::from_secs(60)))] {
            connection
                .execute(
                    "INSERT INTO \"DesktopBrowserLogins\" (\"Id\", \"SecretHash\", \"SecurityVersion\", \"CreatedAt\", \"ExpiresAt\") VALUES (?1, 'h', 0, ?2, ?3)",
                    params![id, now.minus(std::time::Duration::from_secs(600)), expires],
                )
                .unwrap();
        }
    }

    let started = start(&server).await;

    let connection = server.database();
    let exists = |id: &str| -> bool {
        connection
            .query_row(
                "SELECT 1 FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
                [Id::parse(id).unwrap()],
                |_| Ok(()),
            )
            .optional()
            .unwrap()
            .is_some()
    };
    assert!(!exists(&stale.to_string()));
    assert!(exists(&live.to_string()));
    assert!(exists(&started.id));
}

#[tokio::test]
async fn the_start_of_a_request_counts_against_the_limit_on_signing_in() {
    let server = Server::start_with(&[
        ("ASPNETCORE_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret"),
        ("RateLimiting__Login__PermitLimit", "2"),
        ("Oidc__Enabled", "true"),
        ("Oidc__Authority", "https://identity.test"),
        ("Oidc__ClientId", "x"),
        ("Oidc__ClientSecret", "y"),
    ])
    .await;
    let client = application(&server);
    let ask = || client.send(Request::new(Method::POST, "/api/auth/desktop/browser/start"));

    assert_eq!(ask().await.status, 200);
    assert_eq!(ask().await.status, 200);
    let over = ask().await;
    assert_eq!(over.status, 429);
    assert!(over.body.is_empty());
}

#[tokio::test]
async fn the_page_is_for_somebody_who_is_signed_in() {
    let server = Server::start_with(&PROVIDER).await;
    let started = start(&server).await;
    let nobody = server.client();

    let page = nobody.get(&page_of(&started.id)).await;
    assert_eq!(page.status, 401);
    assert!(page.body.is_empty());
    let posted = nobody.send(post(&started.id, "token=x")).await;
    assert_eq!(posted.status, 401);
    // Nothing was bound by it.
    assert!(row(&server.database(), &started.id).approval_user.is_none());
}

#[tokio::test]
async fn an_id_that_is_no_id_is_no_route_and_one_nobody_started_is_an_expired_link() {
    let (_server, maja) = with_provider().await;

    let bad = maja.client.get(&page_of("nope")).await;
    assert_eq!(bad.status, 404);
    assert!(bad.body.is_empty());

    let unknown = maja.client.get(&page_of("11111111-2222-4333-8444-555555555555")).await;
    assert_eq!(unknown.status, 410);
    assert_eq!(unknown.text(), EXPIRED);
    assert_eq!(unknown.header("content-length"), Some("762"));
}

#[tokio::test]
async fn the_page_is_the_first_servers_and_binds_the_request_to_the_person_and_their_session() {
    let (server, maja) = with_provider().await;
    let started = start(&server).await;

    let page = maja.client.get(&page_of(&started.id)).await;

    assert_eq!(page.status, 200);
    let token = token_of(&page.text());
    assert_eq!(token.len(), 43);
    assert_eq!(
        page.text(),
        ASK.replace("<id>", &started.id).replace("<token>", &token)
    );
    // Every header a browser looks at, and no way to put the page in a frame.
    assert_eq!(page.header("content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(page.header("cache-control"), Some("no-store"));
    assert_eq!(page.header("pragma"), Some("no-cache"));
    assert_eq!(page.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(page.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        page.header("content-security-policy"),
        Some("default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'")
    );
    assert_eq!(page.header("content-length"), Some("1235"));
    assert!(page.headers.get("set-cookie").is_none());

    let row = row(&server.database(), &started.id);
    assert_eq!(row.approval_user.as_deref(), Some(maja.id.to_uppercase().as_str()));
    assert!(row.approval_session.is_some());
    assert_eq!(row.token_hash, Some(tokens::hash(&token)));
    // Looking is not approving.
    assert!(row.user.is_none() && row.approved_at.is_none() && row.approved_session.is_none());
}

#[tokio::test]
async fn each_look_at_the_page_makes_the_earlier_one_stop_working() {
    let (server, maja) = with_provider().await;
    let started = start(&server).await;
    let first = look(&maja.client, &started.id).await;
    let second = look(&maja.client, &started.id).await;
    assert_ne!(first, second);

    let old = approve(&maja.client, &started.id, &first).await;
    assert_eq!(old.status, 410);
    assert_eq!(old.text(), EXPIRED);
    let new = approve(&maja.client, &started.id, &second).await;
    assert_eq!(new.status, 200);
    assert_eq!(new.text(), DONE);
}

#[tokio::test]
async fn a_request_looked_at_by_one_person_is_taken_for_everyone_else() {
    let (server, maja) = with_provider().await;
    let ada = server.register("ada").await;
    let started = start(&server).await;
    let token = look(&maja.client, &started.id).await;

    let taken = ada.client.get(&page_of(&started.id)).await;
    assert_eq!(taken.status, 409);
    assert_eq!(
        taken.text(),
        "{\"title\":\"This sign-in was already approved by another account.\"}"
    );
    assert_eq!(taken.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(taken.header("cache-control"), Some("no-store"));
    // Their token is no use to the other person, who is told the link is bad.
    let stolen = approve(&ada.client, &started.id, &token).await;
    assert_eq!(stolen.status, 410);
    assert_eq!(stolen.text(), EXPIRED);
    // And the first person's page still works.
    assert_eq!(approve(&maja.client, &started.id, &token).await.status, 200);
    assert_eq!(ada.client.get(&page_of(&started.id)).await.status, 409);
}

#[tokio::test]
async fn approving_records_what_the_session_proved_and_says_so_in_the_security_events() {
    let (server, maja) = with_provider().await;
    // A session that passed a second step, as the approval must carry it on.
    let browser = server.sign_in(&maja).await;
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = '2026-11-20 10:00:00', \"MfaEvidence\" = 'totp'
             WHERE \"Id\" = (SELECT \"Id\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1)",
            [],
        )
        .unwrap();
    let started = start(&server).await;
    let token = look(&browser, &started.id).await;

    let approved = browser
        .send(
            post(&started.id, &format!("token={token}"))
                .header("Origin", &format!("http://{}", server.address()))
                .header("User-Agent", "Firefox 140 <b>"),
        )
        .await;

    assert_eq!(approved.status, 200);
    assert_eq!(approved.text(), DONE);
    assert_eq!(approved.header("cache-control"), Some("no-store"));
    let connection = server.database();
    let row = row(&connection, &started.id);
    let upper = maja.id.to_uppercase();
    assert_eq!(row.user.as_deref(), Some(upper.as_str()));
    assert_eq!(row.approval_user.as_deref(), Some(upper.as_str()));
    assert_eq!(row.approved_session, row.approval_session);
    assert_eq!(row.primary_source.as_deref(), Some("password"));
    assert_eq!(row.mfa_completed_at.as_deref(), Some("2026-11-20 10:00:00"));
    assert_eq!(row.mfa_evidence.as_deref(), Some("totp"));
    assert_eq!(row.version, 0);
    assert!(row.approved_at.is_some() && row.redeemed_at.is_none());
    assert!(row.token_hash.is_none(), "the token is spent");
    let events = approval_events(&connection, &maja.id);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, "browser approval");
    assert_eq!(events[0].1, "127.0.0.1");
    assert_eq!(events[0].2, "Firefox 140 <b>");
    // The page is the same for the one who comes back to it.
    assert_eq!(browser.get(&page_of(&started.id)).await.text(), DONE);
}

#[tokio::test]
async fn a_second_step_passed_long_ago_does_not_stop_the_approval() {
    // The approval does not ask for a fresh second step: what the session had
    // proved is carried on, and the exchange decides what it allows.
    let (server, maja) = with_provider().await;
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = '2020-01-01 00:00:00', \"MfaEvidence\" = 'totp'",
            [],
        )
        .unwrap();
    let started = start(&server).await;
    let token = look(&maja.client, &started.id).await;

    assert_eq!(approve(&maja.client, &started.id, &token).await.status, 200);
    assert_eq!(
        row(&server.database(), &started.id).mfa_completed_at.as_deref(),
        Some("2020-01-01 00:00:00")
    );
}

#[tokio::test]
async fn a_post_that_does_not_carry_the_right_token_from_the_right_session_is_an_expired_link() {
    let (server, maja) = with_provider().await;
    let other_browser = server.sign_in(&maja).await;
    let started = start(&server).await;
    let token = look(&maja.client, &started.id).await;
    let long = "a".repeat(513);
    let wrong = "a".repeat(43);
    let bodies = [
        "".to_string(),
        "token=".to_string(),
        "token=abc".to_string(),
        format!("token={wrong}"),
        format!("token={long}"),
        format!("token={token}&token={token}"),
        format!("other={token}"),
        format!("token={token}x"),
        format!("token=%20{token}"),
    ];
    for body in bodies {
        let refused = maja.client.send(post(&started.id, &body)).await;
        assert_eq!(refused.status, 410, "{body}");
        assert_eq!(refused.text(), EXPIRED, "{body}");
    }
    // The right token, from another browser of the same person.
    let moved = approve(&other_browser, &started.id, &token).await;
    assert_eq!(moved.status, 410);
    assert!(row(&server.database(), &started.id).approved_at.is_none());
    // None of that spent the token.
    assert_eq!(approve(&maja.client, &started.id, &token).await.status, 200);
    // And it is spent now.
    assert_eq!(approve(&maja.client, &started.id, &token).await.status, 410);
}

#[tokio::test]
async fn a_post_from_another_site_is_refused_before_anything_is_read_and_costs_nothing() {
    let (server, maja) = with_provider().await;
    let started = start(&server).await;
    let token = look(&maja.client, &started.id).await;
    let own = format!("http://{}", server.address());

    for (name, value) in [
        ("Origin", "https://evil.test"),
        ("Origin", "null"),
        ("Origin", "http://127.0.0.1:1"),
        ("Referer", "https://evil.test/x"),
        ("Referer", "not an address"),
    ] {
        let refused = maja
            .client
            .send(post(&started.id, &format!("token={token}")).header(name, value))
            .await;
        assert_eq!(refused.status, 400, "{name}: {value}");
        assert!(refused.body.is_empty());
        assert_eq!(refused.header("cache-control"), Some("no-store"));
        assert_eq!(refused.header("pragma"), Some("no-cache"));
    }
    // Not even a body that is no form is read: the origin is looked at first.
    let unread = maja
        .client
        .send(Request::new(Method::POST, &page_of(&started.id)).json(&serde_json::json!({})).header("Origin", "https://evil.test"))
        .await;
    assert_eq!(unread.status, 400);
    assert!(row(&server.database(), &started.id).approved_at.is_none());
    // The browser's own word, in any spelling of it, is accepted.
    for (name, value) in [
        ("Origin", own.clone()),
        ("Origin", format!("{own}/")),
        ("Referer", format!("{own}/api/anything?x=1#y")),
    ] {
        let started = start(&server).await;
        let token = look(&maja.client, &started.id).await;
        let accepted = maja
            .client
            .send(post(&started.id, &format!("token={token}")).header(name, &value))
            .await;
        assert_eq!(accepted.status, 200, "{name}: {value}");
    }
}

#[tokio::test]
async fn a_form_may_come_in_the_spellings_the_first_server_read() {
    let (server, maja) = with_provider().await;
    let own = format!("http://{}", server.address());
    let many = (0..1023).map(|i| format!("f{i}=1")).collect::<Vec<_>>().join("&");
    let file_first = |t: &str| {
        format!("--B\r\nContent-Disposition: form-data; name=\"x\"; filename=\"a\"\r\n\r\nzz\r\n--B\r\nContent-Disposition: form-data; name=\"token\"\r\n\r\n{t}\r\n--B--\r\n")
    };
    let spellings: Vec<Spelling> = vec![
        ("a name in capitals", FORM, Box::new(|t| format!("TOKEN={t}"))),
        ("percent-encoded", FORM, Box::new(|t| format!("t%6Fken={}", t.replace('-', "%2D").replace('_', "%5F")))),
        ("an empty one before it", FORM, Box::new(|t| format!("token=&token={t}"))),
        ("a charset", "application/x-www-form-urlencoded; charset=utf-8", Box::new(|t| format!("token={t}"))),
        ("1024 fields", FORM, Box::new(move |t| format!("{many}&token={t}"))),
        ("multipart, a file before it", "multipart/form-data; boundary=B", Box::new(file_first)),
    ];
    for (name, content_type, make) in spellings {
        let started = start(&server).await;
        let token = look(&maja.client, &started.id).await;
        let request = Request::new(Method::POST, &page_of(&started.id))
            .header("Origin", &own)
            .body(content_type, &make(&token));
        assert_eq!(maja.client.send(request).await.status, 200, "{name}");
    }
}

#[tokio::test]
async fn what_is_no_form_or_breaks_the_limits_of_one_is_the_servers_failure_and_nothing_is_spent() {
    let (server, maja) = with_provider().await;
    let started = start(&server).await;
    let token = look(&maja.client, &started.id).await;
    let many = (0..1024).map(|i| format!("f{i}=1")).collect::<Vec<_>>().join("&");
    let cases: Vec<(&str, Request)> = vec![
        ("JSON", Request::new(Method::POST, &page_of(&started.id)).json(&serde_json::json!({ "token": token }))),
        ("no type", Request::new(Method::POST, &page_of(&started.id)).untyped_body(&format!("token={token}"))),
        ("plain text", Request::new(Method::POST, &page_of(&started.id)).body("text/plain", &format!("token={token}"))),
        ("no body", Request::new(Method::POST, &page_of(&started.id))),
        ("a multipart that is none", Request::new(Method::POST, &page_of(&started.id)).body("multipart/form-data; boundary=B", "garbage")),
        ("no boundary", Request::new(Method::POST, &page_of(&started.id)).body("multipart/form-data", "garbage")),
        ("1025 fields", post(&started.id, &format!("{many}&token={token}"))),
        ("a name of 2049", post(&started.id, &format!("{}=1&token={token}", "k".repeat(2049)))),
        ("a value of 4 MiB and one", post(&started.id, &format!("x={}&token={token}", "a".repeat(4 * 1024 * 1024 + 1)))),
    ];
    for (name, request) in cases {
        let failed = maja.client.send(request).await;
        assert_eq!(failed.status, 500, "{name}");
        let body: Value = failed.json();
        assert_eq!(body["title"], "An error occurred while processing your request.", "{name}");
        // The reason is the log's, never the answer's.
        assert!(!failed.text().contains("form"), "{name}: {}", failed.text());
    }
    assert_eq!(approve(&maja.client, &started.id, &token).await.status, 200);
}

#[tokio::test]
async fn a_request_that_ran_out_or_was_used_is_an_expired_link_for_the_page_and_the_post() {
    let (server, maja) = with_provider().await;
    for how in ["ran out", "used"] {
        let started = start(&server).await;
        let token = look(&maja.client, &started.id).await;
        let id = Id::parse(&started.id).unwrap();
        let sql = if how == "ran out" {
            "UPDATE \"DesktopBrowserLogins\" SET \"ExpiresAt\" = '2000-01-01 00:00:00' WHERE \"Id\" = ?1"
        } else {
            "UPDATE \"DesktopBrowserLogins\" SET \"RedeemedAt\" = '2026-11-24 00:00:00' WHERE \"Id\" = ?1"
        };
        server.database().execute(sql, [id]).unwrap();

        let page = maja.client.get(&page_of(&started.id)).await;
        assert_eq!((page.status, page.text()), (http::StatusCode::GONE, EXPIRED.to_string()), "{how}");
        let posted = approve(&maja.client, &started.id, &token).await;
        assert_eq!((posted.status, posted.text()), (http::StatusCode::GONE, EXPIRED.to_string()), "{how}");
        assert!(row(&server.database(), &started.id).approved_at.is_none());
    }
}

#[tokio::test]
async fn a_session_that_ended_or_an_account_that_changed_cannot_approve() {
    let cases = [
        ("the session was revoked", "UPDATE \"UserSessions\" SET \"RevokedAt\" = '2026-11-24 00:00:00' WHERE \"Id\" = ?1"),
        ("the session ran out", "UPDATE \"UserSessions\" SET \"ExpiresAt\" = '2000-01-01 00:00:00' WHERE \"Id\" = ?1"),
        ("the session reached its end", "UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = '2000-01-01 00:00:00' WHERE \"Id\" = ?1"),
        ("the password was changed", "UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1 WHERE \"Id\" = (SELECT \"UserId\" FROM \"UserSessions\" WHERE \"Id\" = ?1)"),
        ("the account was disabled", "UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = (SELECT \"UserId\" FROM \"UserSessions\" WHERE \"Id\" = ?1)"),
    ];
    for (what, sql) in cases {
        let (server, maja) = with_provider().await;
        let started = start(&server).await;
        let token = look(&maja.client, &started.id).await;
        let session: Id = server
            .database()
            .query_row(
                "SELECT \"ApprovalSessionId\" FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
                [Id::parse(&started.id).unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        server.database().execute(sql, [session]).unwrap();

        let posted = approve(&maja.client, &started.id, &token).await;
        assert_eq!(posted.status, 401, "{what}");
        let page = maja.client.get(&page_of(&started.id)).await;
        assert_eq!(page.status, 401, "{what}");
        assert!(row(&server.database(), &started.id).approved_at.is_none(), "{what}");
    }
}

#[tokio::test]
async fn nothing_the_application_or_the_browser_sends_can_reach_the_page() {
    let (server, maja) = with_provider().await;
    let marker = "<i>marker</i>\u{202e}\u{7}";

    // Start takes nothing from the request: not a body, not a header.
    let steered = application(&server)
        .send(
            Request::new(Method::POST, "/api/auth/desktop/browser/start")
                .header("User-Agent", marker.replace('\u{7}', "").as_str())
                .header("X-Forwarded-Host", "evil.test")
                .json(&serde_json::json!({ "device": marker, "name": "x".repeat(10_000) })),
        )
        .await;
    assert_eq!(steered.status, 200);
    let id = steered.json::<Value>()["requestId"].as_str().unwrap().to_string();

    // The page shows what the first server's did: the same bytes whatever the
    // browser says about itself.
    let page = maja
        .client
        .send(
            Request::new(Method::GET, &format!("{}?x=%3Cscript%3E", page_of(&id)))
                .header("User-Agent", "<script>alert(1)</script>")
                .header("Accept-Language", "pl")
                .header("Referer", "https://evil.test/<b>"),
        )
        .await;
    let token = token_of(&page.text());
    assert_eq!(page.text(), ASK.replace("<id>", &id).replace("<token>", &token));
}

#[tokio::test]
async fn the_page_speaks_the_language_of_the_instance() {
    let server = Server::start_with(&[
        ("Oidc__Enabled", "true"),
        ("Oidc__Authority", "https://identity.test"),
        ("Oidc__ClientId", "x"),
        ("Oidc__ClientSecret", "y"),
        ("Stellarr__Language", "pl"),
    ])
    .await;
    let maja = server.found("maja").await;
    let started = start(&server).await;

    let page = maja.client.get(&page_of(&started.id)).await.text();

    assert!(page.starts_with("<!doctype html>\n<html lang=\"pl\">"));
    assert!(page.contains("<h1>Zatwierdzić to logowanie?</h1>"));
    assert!(page.contains("<p>Kontynuuj tylko wtedy, gdy rozpocząłeś to logowanie w Stellarr Desktop. Zatwierdzenie zaloguje tę aplikację jako Ciebie.</p>"));
    assert!(page.contains("<button type=\"submit\">Zatwierdź logowanie</button>"));
    let gone = maja.client.get(&page_of("11111111-2222-4333-8444-555555555555")).await;
    assert!(gone.text().contains("<h1>Link wygasł</h1>"));
}

#[tokio::test]
async fn only_the_methods_the_routes_have_are_served() {
    let (server, maja) = with_provider().await;
    let started = start(&server).await;

    for method in [Method::PUT, Method::DELETE, Method::HEAD] {
        let answer = maja.client.send(Request::new(method.clone(), &page_of(&started.id))).await;
        assert_eq!(answer.status, 405, "{method}");
        assert_eq!(answer.header("allow"), Some("GET, POST"), "{method}");
    }
    let start_get = application(&server).get("/api/auth/desktop/browser/start").await;
    assert_eq!(start_get.status, 405);
    assert_eq!(start_get.header("allow"), Some("POST"));
}

// ---------------------------------------------------------------------------
// The application's half: exchanging an approved request for a session.

const EXCHANGE: &str = "/api/auth/desktop/browser/exchange";

async fn exchange(application: &Client, id: &str, secret: &str) -> Response {
    application
        .send(Request::new(Method::POST, EXCHANGE).json(&serde_json::json!({ "requestId": id, "secret": secret })))
        .await
}

/// A request that the person approved, from `browser`.
async fn approved(server: &Server, browser: &Client) -> Started {
    let started = start(server).await;
    let token = look(browser, &started.id).await;
    assert_eq!(approve(browser, &started.id, &token).await.status, 200);
    started
}

fn count(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

const INVALID: &str = "The sign-in link is invalid or has expired.";

#[tokio::test]
async fn the_whole_hand_over_ends_in_a_session_that_carries_what_the_approval_proved() {
    let (server, maja) = with_provider().await;
    let browser = server.sign_in(&maja).await;
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = '2026-11-20 10:00:00', \"MfaEvidence\" = 'totp'
             WHERE \"Id\" = (SELECT \"Id\" FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1)",
            [],
        )
        .unwrap();
    let application = application(&server);
    let started = start(&server).await;

    let pending = exchange(&application, &started.id, &started.secret).await;
    assert_eq!((pending.status.as_u16(), pending.text()), (202, "{\"status\":\"pending\"}".to_string()));
    assert_eq!(pending.header("cache-control"), Some("no-store"));
    let token = look(&browser, &started.id).await;
    assert_eq!(exchange(&application, &started.id, &started.secret).await.status, 202);
    approve(&browser, &started.id, &token).await;

    let signed_in = exchange(&application, &started.id, &started.secret).await;

    assert_eq!(signed_in.status, 200, "{}", signed_in.text());
    let user: Value = signed_in.json();
    assert_eq!(user["username"], "maja");
    assert_eq!(user["id"], maja.id);
    assert_eq!(user["presence"], "online");
    // The cookie is kept across the application's restarts, and the answer is kept from caches.
    let cookie = signed_in.headers.get_all("set-cookie").iter().next().unwrap().to_str().unwrap().to_string();
    assert!(cookie.starts_with("stellarr-dev-session="), "{cookie}");
    assert!(cookie.contains("; expires="), "{cookie}");
    assert!(cookie.ends_with("; path=/; samesite=lax; httponly"), "{cookie}");
    assert_eq!(signed_in.header("cache-control"), Some("no-cache,no-store"));
    assert_eq!(signed_in.header("pragma"), Some("no-cache"));

    let connection = server.database();
    assert!(row(&connection, &started.id).redeemed_at.is_some());
    let made: (Option<String>, Option<String>, String, bool, String, String, i64, String, String) = connection
        .query_row(
            "SELECT \"MfaCompletedAt\", \"MfaEvidence\", \"PrimarySource\", \"ShortLived\", \"Device\", \"IpAddress\", \"SecurityVersion\",
                    \"CreatedAt\", \"ExpiresAt\"
             FROM \"UserSessions\" ORDER BY \"CreatedAt\" DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?)),
        )
        .unwrap();
    assert_eq!(made.0.as_deref(), Some("2026-11-20 10:00:00"));
    assert_eq!(made.1.as_deref(), Some("totp"));
    assert_eq!(made.2, "password");
    assert!(!made.3, "the application's session is never short-lived");
    assert!(!made.4.is_empty() && made.5 == "127.0.0.1" && made.6 == 0);
    let lifetime = Timestamp::parse(&made.8).unwrap().unix_seconds() - Timestamp::parse(&made.7).unwrap().unix_seconds();
    assert_eq!(lifetime, 30 * 24 * 3600);
    let sign_ins: Vec<String> = connection
        .prepare("SELECT \"Detail\" FROM \"SecurityEvents\" WHERE \"Kind\" = 'login-succeeded' AND \"Detail\" = 'desktop'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(sign_ins.len(), 1);

    // The session works where a session does: on the API, and on the hub.
    let me = application.get("/api/me").await;
    assert_eq!(me.status, 200);
    assert_eq!(me.json::<Value>()["username"], "maja");
    let session = application.cookie("stellarr-dev-session").unwrap();
    let mut hub = crate::support::HubClient::connect(server.address(), &format!("stellarr-dev-session={session}")).await;
    let _ = hub.quiet_for(std::time::Duration::from_millis(50)).await;

    // And once is all it was good for.
    let again = exchange(&application, &started.id, &started.secret).await;
    assert_eq!(again.status, 400);
}

#[tokio::test]
async fn an_exchange_that_is_not_for_a_request_with_its_secret_is_one_answer() {
    let (server, maja) = with_provider().await;
    let application = application(&server);
    let started = approved(&server, &maja.client).await;
    let long = "x".repeat(513);

    for (what, id, secret) in [
        ("a wrong secret", started.id.as_str(), "x".repeat(43)),
        ("a secret too short", started.id.as_str(), "x".repeat(15)),
        ("a secret too long", started.id.as_str(), long.clone()),
        ("the secret with spaces around it", started.id.as_str(), format!(" {} ", started.secret)),
        ("an id that is no id", "not-a-guid", started.secret.clone()),
        ("an id nobody started", "11111111-2222-4333-8444-555555555555", started.secret.clone()),
    ] {
        let refused = exchange(&application, id, &secret).await;
        assert_eq!(refused.status, 400, "{what}");
        let body: Value = refused.json();
        assert_eq!(body["title"], INVALID, "{what}");
        assert_eq!(refused.header("cache-control"), Some("no-store"), "{what}");
        assert_eq!(refused.header("content-type"), Some("application/problem+json"), "{what}");
    }
    // None of them cost the right holder of the secret anything.
    assert!(row(&server.database(), &started.id).redeemed_at.is_none());
    assert_eq!(exchange(&application, &started.id, &started.secret).await.status, 200);
}

#[tokio::test]
async fn a_body_that_is_not_the_request_is_refused_as_the_first_server_refused_it() {
    let server = Server::start_with(&PROVIDER).await;
    let application = application(&server);

    for body in ["{}", "{\"requestId\":null,\"secret\":null}"] {
        let refused = application
            .send(Request::new(Method::POST, EXCHANGE).body("application/json", body))
            .await;
        assert_eq!(refused.status, 400, "{body}");
        assert_eq!(refused.json::<Value>()["title"], INVALID);
    }
    for body in ["{\"requestId\":1,\"secret\":2}", "[1]", "{"] {
        let refused = application
            .send(Request::new(Method::POST, EXCHANGE).body("application/json", body))
            .await;
        assert_eq!(refused.status, 400, "{body}");
        assert!(refused.body.is_empty() && refused.header("cache-control").is_none(), "{body}");
    }
    let untyped = application.send(Request::new(Method::POST, EXCHANGE).untyped_body("{}")).await;
    assert_eq!(untyped.status, 415);
    assert_eq!(application.get(EXCHANGE).await.status, 405);
}

#[tokio::test]
async fn two_exchanges_at_once_make_one_session() {
    let (server, maja) = with_provider().await;
    let started = approved(&server, &maja.client).await;
    let (first, second) = (application(&server), application(&server));
    let before = count(&server.database(), "SELECT COUNT(*) FROM \"UserSessions\"");

    let (a, b) = tokio::join!(
        exchange(&first, &started.id, &started.secret),
        exchange(&second, &started.id, &started.secret)
    );

    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 400]);
    assert_eq!(count(&server.database(), "SELECT COUNT(*) FROM \"UserSessions\""), before + 1);
}

#[tokio::test]
async fn a_poll_that_finds_nothing_yet_reads_and_writes_nothing() {
    let (server, _maja) = with_provider().await;
    let started = start(&server).await;
    let application = application(&server);
    // Another connection holds the write lock for the whole of the polls: a
    // poll that wrote would wait for it, and would not be answered.
    let holder = server.database();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = count(&holder, "SELECT total_changes()");

    for _ in 0..3 {
        let poll = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            exchange(&application, &started.id, &started.secret),
        )
        .await
        .expect("a poll writes nothing, so it does not wait for the writer");
        assert_eq!(poll.status, 202);
    }
    let wrong = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        exchange(&application, &started.id, &"x".repeat(43)),
    )
    .await
    .expect("a wrong secret writes nothing either");
    assert_eq!(wrong.status, 400);
    holder.execute_batch("ROLLBACK").unwrap();
    assert_eq!(before, 0);
    assert!(row(&server.database(), &started.id).redeemed_at.is_none());
}

#[tokio::test]
async fn polls_count_against_a_limit_of_their_own() {
    let server = Server::start_with(&[
        ("ASPNETCORE_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret"),
        ("RateLimiting__LoginPoll__PermitLimit", "3"),
        ("Oidc__Enabled", "true"),
        ("Oidc__Authority", "https://identity.test"),
        ("Oidc__ClientId", "x"),
        ("Oidc__ClientSecret", "y"),
    ])
    .await;
    let application = application(&server);
    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(exchange(&application, "11111111-2222-4333-8444-555555555555", &"x".repeat(43)).await.status.as_u16());
    }
    assert_eq!(statuses, [400, 400, 400, 429, 429]);
    // The limit on passwords is not spent by it.
    let login = server.client().post_json("/api/auth/login", &serde_json::json!({ "username": "x", "password": "y" })).await;
    assert_ne!(login.status, 429);
}

#[tokio::test]
async fn a_request_that_ran_out_after_approval_is_no_use() {
    let (server, maja) = with_provider().await;
    let started = approved(&server, &maja.client).await;
    let before = count(&server.database(), "SELECT COUNT(*) FROM \"UserSessions\"");
    server
        .database()
        .execute(
            "UPDATE \"DesktopBrowserLogins\" SET \"ExpiresAt\" = '2000-01-01 00:00:00' WHERE \"Id\" = ?1",
            [Id::parse(&started.id).unwrap()],
        )
        .unwrap();

    let late = exchange(&application(&server), &started.id, &started.secret).await;

    assert_eq!(late.status, 400);
    assert_eq!(count(&server.database(), "SELECT COUNT(*) FROM \"UserSessions\""), before);
}

#[tokio::test]
async fn an_approval_that_no_longer_stands_is_a_403_and_makes_no_session() {
    let cases = [
        ("the approving session was revoked", "UPDATE \"UserSessions\" SET \"RevokedAt\" = '2026-11-24 00:00:00' WHERE \"Id\" = (SELECT \"ApprovedSessionId\" FROM \"DesktopBrowserLogins\")"),
        ("the approving session ran out", "UPDATE \"UserSessions\" SET \"ExpiresAt\" = '2000-01-01 00:00:00' WHERE \"Id\" = (SELECT \"ApprovedSessionId\" FROM \"DesktopBrowserLogins\")"),
        ("the approving session reached its end", "UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = '2000-01-01 00:00:00' WHERE \"Id\" = (SELECT \"ApprovedSessionId\" FROM \"DesktopBrowserLogins\")"),
        ("the approving session has no end", "UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = NULL WHERE \"Id\" = (SELECT \"ApprovedSessionId\" FROM \"DesktopBrowserLogins\")"),
        ("the account's version moved on", "UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1"),
        ("the account was disabled", "UPDATE \"Users\" SET \"IsDisabled\" = 1"),
        ("the account waits for recovery", "UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1"),
        ("the account got a second step the approval did not pass", "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\") SELECT 'AAAAAAAA-0000-4000-8000-000000000001', \"Id\", 'totp', 'Phone', '2026-11-24 00:00:00', '2026-11-24 00:00:00' FROM \"Users\""),
    ];
    for (what, sql) in cases {
        let (server, maja) = with_provider().await;
        let started = approved(&server, &maja.client).await;
        let before = count(&server.database(), "SELECT COUNT(*) FROM \"UserSessions\"");
        server.database().execute(sql, []).unwrap();

        let refused = exchange(&application(&server), &started.id, &started.secret).await;

        assert_eq!(refused.status, 403, "{what}");
        assert!(refused.body.is_empty(), "{what}");
        assert_eq!(count(&server.database(), "SELECT COUNT(*) FROM \"UserSessions\""), before, "{what}");
        assert!(row(&server.database(), &started.id).redeemed_at.is_none(), "{what}");
    }
}

#[tokio::test]
async fn a_password_changed_after_approval_ends_the_approval() {
    let (server, maja) = with_provider().await;
    let started = approved(&server, &maja.client).await;
    {
        let mut connection = server.database();
        let tx = connection.transaction().unwrap();
        // What a change of password does, which every route that makes one calls.
        stellarr_api::auth::recovery::credentials_changed(&tx, Id::parse(&maja.id).unwrap(), None, Timestamp::now()).unwrap();
        tx.commit().unwrap();
    }

    let refused = exchange(&application(&server), &started.id, &started.secret).await;

    assert_eq!(refused.status, 400);
    assert_eq!(refused.json::<Value>()["title"], INVALID);
    assert_eq!(maja.client.get("/api/me").await.status, 401);
}

#[tokio::test]
async fn an_account_with_a_second_step_gets_a_session_only_from_an_approval_that_passed_one() {
    let (server, maja) = with_provider().await;
    let browser = server.sign_in(&maja).await;
    server
        .database()
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\") SELECT 'AAAAAAAA-0000-4000-8000-000000000001', \"Id\", 'totp', 'Phone', '2026-11-24 00:00:00', '2026-11-24 00:00:00' FROM \"Users\"",
            [],
        )
        .unwrap();
    let without = approved(&server, &browser).await;
    assert_eq!(exchange(&application(&server), &without.id, &without.secret).await.status, 403);

    // The same browser after it passed the step (written as the step's own calls would).
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = '2026-11-24 00:00:00', \"MfaEvidence\" = 'totp'",
            [],
        )
        .unwrap();
    let with = approved(&server, &maja.client).await;
    let app = application(&server);
    assert_eq!(exchange(&app, &with.id, &with.secret).await.status, 200);
    assert_eq!(app.get("/api/me").await.status, 200);
}
