//! What every request goes through, tested with routes that exist only here:
//! one that fails, one that is rate limited, one that reads a body.
//!
//! The server runs in the test's own process with these routes next to the
//! health group, and the client talks to it over a socket like any other.

use axum::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use http::Method;
use serde::Serialize;
use serde_json::{Value, json};
use stellarr_api::events::{Audience, Event};
use stellarr_api::http::{
    ApiError, ApiResult, ClientInfo, Endpoints, Json, Problem, RateLimitPolicy,
};
use stellarr_api::state::AppState;
use stellarr_api::values::{Id, Timestamp};
use stellarr_messages::MessageKey;

use crate::support::{Request, Server};

/// A production-like environment, where the limits are the real ones.
const PRODUCTION: [(&str, &str); 2] = [
    ("ASPNETCORE_ENVIRONMENT", "Production"),
    ("Rtc__TurnSharedSecret", "a-secret"),
];

fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/test/fails", fails)
        .get("/api/test/panics", panics)
        .get("/api/test/problem", problem)
        .post("/api/auth/login", login)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/test/body", echo)
        .json_body()
        .get("/api/test/client", client)
        .get("/api/test/wire", wire)
        .get("/api/test/counts", counts)
        .post("/api/test/announce", announce)
}

async fn fails() -> ApiResult<Json<Value>> {
    Err(ApiError::internal("on purpose"))
}

async fn panics() -> Json<Value> {
    panic!("on purpose");
}

async fn problem() -> ApiResult<Json<Value>> {
    Err(Problem::validation(vec![("username".into(), vec!["Too short.".into()])]).into())
}

async fn login(State(state): State<AppState>, Json(_): Json<Value>) -> ApiResult<Json<Value>> {
    let title = state.messages().get(MessageKey::SignInFailed, &[])?;
    Err(Problem::new(StatusCode::UNAUTHORIZED).title(title).into())
}

async fn echo(Json(body): Json<Value>) -> Json<Value> {
    Json(body)
}

async fn client(Extension(client): Extension<ClientInfo>) -> Json<Value> {
    Json(
        json!({ "address": client.address.to_string(), "scheme": client.scheme, "host": client.host }),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    id: Id,
    display_name: &'static str,
    avatar_url: Option<String>,
    created_at: Timestamp,
    kind: Kind,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum Kind {
    Text,
    Voice,
}

async fn wire() -> Json<Wire> {
    Json(Wire {
        id: Id::parse("8F62A4D1-BDCB-4D86-BD1C-6E351900E629").unwrap(),
        display_name: "Maya Lin",
        avatar_url: None,
        created_at: Timestamp::parse("2026-11-24 00:33:18.6419643").unwrap(),
        kind: Kind::Voice,
    })
}

async fn counts(State(state): State<AppState>) -> Json<Value> {
    let metrics = state.metrics();
    Json(json!({
        "requests": metrics.requests(),
        "clientErrors": metrics.client_errors(),
        "serverErrors": metrics.server_errors(),
    }))
}

/// Commits a change and only then announces it.
async fn announce(State(state): State<AppState>) -> ApiResult<StatusCode> {
    let server = Id::random();
    let written = state
        .database()
        .write(move |tx| -> Result<(), stellarr_api::database::DbError> {
            tx.execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Announced', ?2, 1, 1)",
                rusqlite::params![server, Timestamp::now()],
            )?;
            Ok(())
        })
        .await;
    written?;
    state.events().publish(Event::new(
        Audience::Server(server),
        "ServerCreated",
        &json!({ "id": server }),
    ));
    Ok(StatusCode::CREATED)
}

async fn server_with(extra: &[(&str, &str)]) -> Server {
    Server::in_process(routes(), extra).await
}

#[tokio::test]
async fn an_unknown_route_is_a_404_with_no_body() {
    let server = server_with(&[]).await;

    let response = server.client().get("/nothing/here").await;

    assert_eq!(response.status, 404);
    assert!(response.body.is_empty());
    assert_eq!(response.header("content-type"), None);
}

#[tokio::test]
async fn a_handler_that_failed_is_a_500_problem_that_says_nothing_of_the_cause() {
    let server = server_with(&[]).await;

    for path in ["/api/test/fails", "/api/test/panics"] {
        let response = server.client().get(path).await;

        assert_eq!(response.status, 500, "{path}");
        assert_eq!(
            response.header("content-type"),
            Some("application/problem+json"),
            "{path}"
        );
        assert_eq!(
            response.header("cache-control"),
            Some("no-cache,no-store"),
            "{path}"
        );
        assert_eq!(response.header("pragma"), Some("no-cache"), "{path}");
        assert_eq!(response.header("expires"), Some("-1"), "{path}");
        let text = response.text();
        assert!(
            text.starts_with(
                "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.6.1\",\
                 \"title\":\"An error occurred while processing your request.\",\"status\":500,\"traceId\":\"00-"
            ),
            "{path}: {text}"
        );
        assert!(!text.contains("on purpose"), "{path}");
    }
    // The server is still there for the next request.
    assert_eq!(server.client().get("/health/live").await.status, 200);
}

#[tokio::test]
async fn a_problem_keeps_the_order_of_its_members() {
    let server = server_with(&[]).await;

    let response = server.client().get("/api/test/problem").await;

    assert_eq!(response.status, 400);
    assert_eq!(
        response.header("content-type"),
        Some("application/problem+json")
    );
    assert!(
        response.text().starts_with(
            "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.1\",\
             \"title\":\"One or more validation errors occurred.\",\"status\":400,\
             \"errors\":{\"username\":[\"Too short.\"]},\"traceId\":\"00-"
        ),
        "{}",
        response.text()
    );
}

#[tokio::test]
async fn every_value_is_written_the_way_the_clients_read_it() {
    let server = server_with(&[]).await;

    let response = server.client().get("/api/test/wire").await;

    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        response.text(),
        "{\"id\":\"8f62a4d1-bdcb-4d86-bd1c-6e351900e629\",\"displayName\":\"Maya Lin\",\"avatarUrl\":null,\
         \"createdAt\":\"2026-11-24T00:33:18.6419643Z\",\"kind\":\"voice\"}"
    );
}

#[tokio::test]
async fn a_request_over_its_limit_is_a_429_with_no_body() {
    let server = server_with(&[
        PRODUCTION[0],
        PRODUCTION[1],
        ("RateLimiting__Login__PermitLimit", "3"),
    ])
    .await;
    let client = server.client();
    let login = || {
        Request::new(Method::POST, "/api/auth/login")
            .json(&json!({ "username": "x", "password": "y" }))
    };

    // The gate refuses this one, but the limit counts it first.
    let refused = server.anonymous().send(login()).await;
    // A body of the wrong type and a method the route does not have never
    // reach the limit.
    let wrong_type = client
        .send(Request::new(Method::POST, "/api/auth/login").body("text/plain", "x"))
        .await;
    let wrong_method = client.get("/api/auth/login").await;
    let second = client.send(login()).await;
    let third = client.send(login()).await;
    let over = client.send(login()).await;

    assert_eq!(
        [
            refused.status,
            wrong_type.status,
            wrong_method.status,
            second.status,
            third.status,
            over.status
        ],
        [403, 415, 405, 401, 401, 429]
    );
    assert_eq!(
        second.json::<Value>()["title"],
        "Invalid username or password."
    );
    assert!(over.body.is_empty());
    assert_eq!(over.header("retry-after"), None);
    assert_eq!(over.header("content-type"), None);
    // Other paths are not limited by it.
    assert_eq!(client.get("/health/live").await.status, 200);
}

#[tokio::test]
async fn every_address_has_a_count_of_its_own() {
    let server = server_with(&[
        PRODUCTION[0],
        PRODUCTION[1],
        ("RateLimiting__Login__PermitLimit", "1"),
        ("STELLARR_TRUSTED_PROXIES", "172.18.0.1"),
    ])
    .await;
    let from = |client: &str| {
        Request::new(Method::POST, "/api/auth/login")
            .header("X-Forwarded-For", &format!("{client}, 172.18.0.1"))
            .json(&json!({}))
    };

    let first = server.client().send(from("198.51.100.7")).await;
    let same_again = server.client().send(from("198.51.100.7")).await;
    let another = server.client().send(from("198.51.100.8")).await;

    assert_eq!(
        [first.status, same_again.status, another.status],
        [401, 429, 401]
    );
}

#[tokio::test]
async fn a_body_of_another_type_is_a_415_and_a_missing_or_broken_one_a_400() {
    let server = server_with(&[]).await;
    let client = server.client();

    let text = client
        .send(Request::new(Method::POST, "/api/test/body").body("text/plain", "x"))
        .await;
    let json_with_charset = client
        .send(
            Request::new(Method::POST, "/api/test/body")
                .body("application/json; charset=utf-8", "{\"a\":1}"),
        )
        .await;
    let suffixed = client
        .send(
            Request::new(Method::POST, "/api/test/body")
                .body("application/problem+json", "{\"a\":1}"),
        )
        .await;
    let broken = client
        .send(Request::new(Method::POST, "/api/test/body").body("application/json", "{bad"))
        .await;
    let none = client
        .send(Request::new(Method::POST, "/api/test/body"))
        .await;

    assert_eq!(text.status, 415);
    assert!(text.body.is_empty());
    assert_eq!(json_with_charset.json::<Value>(), json!({ "a": 1 }));
    assert_eq!(suffixed.status, 200);
    assert_eq!(broken.status, 400);
    assert!(broken.body.is_empty());
    assert_eq!(none.status, 400);
}

#[tokio::test]
async fn the_ceiling_on_a_body_is_128_mib() {
    const CEILING: u64 = 128 * 1024 * 1024;
    let server = server_with(&[]).await;
    let client = server.client();

    // Declared, and over: refused before a byte is read, and the connection
    // is closed after the answer.
    let declared = client
        .post_zeros("/api/test/body", Some(CEILING + 1), 0)
        .await;
    assert_eq!(declared.status, 413);
    assert!(declared.body.is_empty());
    assert_eq!(declared.header("connection"), Some("close"));

    // Not declared, and over: read until it is too much.
    let streamed = client.post_zeros("/api/test/body", None, CEILING + 1).await;
    assert_eq!(streamed.status, 400);

    // A route that reads no body is not refused for it.
    let unknown = client.post_zeros("/api/nope", Some(CEILING + 1), 0).await;
    assert_eq!(unknown.status, 404);

    // Exactly the ceiling is not over it; the body is not JSON, so the route
    // says so, which it can only do once it has read all of it.
    let exact = client
        .post_zeros("/api/test/body", Some(CEILING), CEILING)
        .await;
    assert_eq!(exact.status, 400);
}

#[tokio::test]
async fn the_client_is_the_address_the_proxies_vouch_for() {
    let server = server_with(&[
        PRODUCTION[0],
        PRODUCTION[1],
        ("STELLARR_TRUSTED_PROXIES", "172.18.0.1"),
    ])
    .await;
    let ask = |forwarded_for: &str| {
        Request::new(Method::GET, "/api/test/client")
            .header("X-Forwarded-For", forwarded_for)
            .header("X-Forwarded-Proto", "https")
            .header("X-Forwarded-Host", "chat.example.test")
    };

    let behind_a_proxy = server
        .client()
        .send(ask("203.0.113.66, 198.51.100.7, 172.18.0.1"))
        .await;
    let direct = server
        .client()
        .send(ask("203.0.113.66, 198.51.100.7"))
        .await;
    let nobody = server.client().get("/api/test/client").await;

    let seen = behind_a_proxy.json::<Value>();
    assert_eq!(
        seen,
        json!({ "address": "198.51.100.7", "scheme": "https", "host": "chat.example.test" })
    );
    assert_eq!(direct.json::<Value>()["address"], "198.51.100.7");
    assert_eq!(
        nobody.json::<Value>(),
        json!({ "address": "127.0.0.1", "scheme": "http", "host": null })
    );
}

#[tokio::test]
async fn a_trusted_proxy_that_is_not_an_address_stops_the_start_and_is_named() {
    let said = Server::refused_to_start(&[("STELLARR_TRUSTED_PROXIES", "10.0.0.1/24")]).await;

    assert!(said.contains("STELLARR_TRUSTED_PROXIES"), "{said}");
    assert!(said.contains("10.0.0.1/24"), "{said}");
}

#[tokio::test]
async fn the_turn_secret_is_required_outside_development() {
    let said = Server::refused_to_start(&[("ASPNETCORE_ENVIRONMENT", "Production")]).await;

    assert!(
        said.contains("Rtc:TurnSharedSecret is required outside development"),
        "{said}"
    );
}

#[tokio::test]
async fn an_unknown_language_stops_the_start() {
    let said = Server::refused_to_start(&[("STELLARR_LANGUAGE", "de")]).await;

    assert!(said.contains("Stellarr:Language"), "{said}");
}

#[tokio::test]
async fn every_response_is_counted_as_the_client_got_it() {
    let server = server_with(&[]).await;
    let client = server.client();
    let before = client.get("/api/test/counts").await.json::<Value>();

    client.get("/nothing").await;
    client.get("/api/test/fails").await;
    client.get("/api/test/panics").await;
    server.anonymous().get("/api/session").await;
    let after = client.get("/api/test/counts").await.json::<Value>();

    let delta = |name: &str| after[name].as_u64().unwrap() - before[name].as_u64().unwrap();
    // The counts request itself is counted after its answer was written.
    assert_eq!(
        (
            delta("requests"),
            delta("clientErrors"),
            delta("serverErrors")
        ),
        (5, 2, 2)
    );
}

#[tokio::test]
async fn an_event_is_published_after_the_change_is_committed() {
    let server = server_with(&[]).await;
    assert!(server.events().is_empty());

    let response = server
        .client()
        .send(Request::new(Method::POST, "/api/test/announce"))
        .await;

    assert_eq!(response.status, 201);
    let events = server.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "ServerCreated");
    let id = events[0]["audience"]["server"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(events[0]["payload"]["id"], id.as_str());
    // The row the event speaks of is there to be read by whoever it reaches.
    let connection = rusqlite::Connection::open(server.database_path()).unwrap();
    let name: String = connection
        .query_row(
            "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
            [id.to_uppercase()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "Announced");
}

#[tokio::test]
async fn a_development_server_answers_the_web_dev_servers_origin_and_no_other() {
    let server = server_with(&[("ASPNETCORE_ENVIRONMENT", "Development")]).await;
    let client = server.client();
    let from = |origin: &str| Request::new(Method::GET, "/health/live").header("Origin", origin);

    let allowed = client.send(from("http://localhost:3000")).await;
    let other = client.send(from("http://evil.example")).await;
    let preflight = client
        .send(
            Request::new(Method::OPTIONS, "/api/auth/login")
                .header("Origin", "http://127.0.0.1:3000")
                .header("Access-Control-Request-Method", "POST")
                .header(
                    "Access-Control-Request-Headers",
                    "content-type,x-stellarr-client",
                ),
        )
        .await;
    let preflight_from_other = client
        .send(
            Request::new(Method::OPTIONS, "/api/auth/login")
                .header("Origin", "http://evil.example")
                .header("Access-Control-Request-Method", "POST"),
        )
        .await;

    assert_eq!(
        allowed.header("access-control-allow-origin"),
        Some("http://localhost:3000")
    );
    assert_eq!(
        allowed.header("access-control-allow-credentials"),
        Some("true")
    );
    assert_eq!(allowed.header("vary"), Some("Origin"));
    assert_eq!(other.header("access-control-allow-origin"), None);
    assert_eq!(preflight.status, 204);
    assert_eq!(
        preflight.header("access-control-allow-origin"),
        Some("http://127.0.0.1:3000")
    );
    assert_eq!(
        preflight.header("access-control-allow-methods"),
        Some("POST")
    );
    assert_eq!(
        preflight.header("access-control-allow-headers"),
        Some("content-type,x-stellarr-client")
    );
    assert_eq!(preflight_from_other.status, 204);
    assert_eq!(
        preflight_from_other.header("access-control-allow-origin"),
        None
    );
}

#[tokio::test]
async fn other_environments_answer_no_origin() {
    let server = server_with(&[PRODUCTION[0], PRODUCTION[1]]).await;

    let response = server
        .client()
        .send(Request::new(Method::GET, "/health/live").header("Origin", "http://localhost:3000"))
        .await;

    assert_eq!(response.header("access-control-allow-origin"), None);
}
