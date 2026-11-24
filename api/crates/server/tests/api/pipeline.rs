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
    ApiError, ApiResult, ClientInfo, Endpoints, Json, Path, Problem, Query, RateLimitPolicy,
};
use stellarr_api::state::AppState;
use stellarr_api::values::{Id, Timestamp};
use stellarr_messages::MessageKey;

use crate::support::{Request, Server, raw_get, raw_send};

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
        .post("/api/test/big", echo)
        .json_body_up_to(usize::MAX)
        .get("/api/test/client", client)
        .get("/api/test/wire", wire)
        .get("/api/test/counts", counts)
        .post("/api/test/announce", announce)
        .get("/api/test/things/{id}", thing)
        .delete("/api/test/things/{id}", forget)
        .get("/api/test/search", search)
        .get("/api/test/slow-announce", slow_announce)
        .get("/api/test/private", private)
        .authorized()
        .get("/api/test/echo/{what}", spoken)
        .get("/api/test/pairs", pairs)
}

/// Every name and value of a query, in the order they came.
async fn pairs(Query(pairs): Query<Vec<(String, String)>>) -> Json<Value> {
    Json(json!({ "pairs": pairs }))
}

async fn private() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// What the router made of a segment.
async fn spoken(Path(what): Path<String>) -> Json<Value> {
    Json(json!({ "what": what }))
}

/// What `slow_announce` and the test that uses it say to each other, so that
/// nothing depends on how long anything takes: the handler tells the test it
/// has started, and does not go on until the test says the client has gone.
static HANDLER_STARTED: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(0);
static CLIENT_GONE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(0);

/// Waits for the client to be gone before it commits and announces.
async fn slow_announce(State(state): State<AppState>) -> ApiResult<StatusCode> {
    HANDLER_STARTED.add_permits(1);
    CLIENT_GONE.acquire().await.expect("open").forget();
    announce(State(state)).await
}

async fn thing(Path(id): Path<Id>) -> Json<Value> {
    Json(json!({ "id": id }))
}

async fn forget(Path(_): Path<Id>) -> StatusCode {
    StatusCode::NO_CONTENT
}

#[derive(serde::Deserialize)]
struct Search {
    limit: u32,
}

async fn search(Query(search): Query<Search>) -> Json<Value> {
    Json(json!({ "limit": search.limit }))
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
async fn the_ceiling_of_the_server_is_128_mib_for_a_route_that_asks_for_it() {
    const CEILING: u64 = 128 * 1024 * 1024;
    let server = server_with(&[]).await;
    let client = server.client();

    // Declared, and over: refused before a byte is read, and the connection
    // is closed after the answer.
    let declared = client
        .post_zeros("/api/test/big", Some(CEILING + 1), 0)
        .await;
    assert_eq!(declared.status, 413);
    assert!(declared.body.is_empty());
    assert_eq!(declared.header("connection"), Some("close"));

    // Not declared, and over: read until it is too much.
    let streamed = client.post_zeros("/api/test/big", None, CEILING + 1).await;
    assert_eq!(streamed.status, 400);

    // A route that reads no body is not refused for it.
    let unknown = client.post_zeros("/api/nope", Some(CEILING + 1), 0).await;
    assert_eq!(unknown.status, 404);

    // Exactly the ceiling is not over it; the body is not JSON, so the route
    // says so, which it can only do once it has read all of it.
    let exact = client
        .post_zeros("/api/test/big", Some(CEILING), CEILING)
        .await;
    assert_eq!(exact.status, 400);
}

#[tokio::test]
async fn a_route_that_reads_json_reads_one_mib_unless_it_asks_for_more() {
    const MIB: u64 = 1024 * 1024;
    let server = server_with(&[]).await;
    let client = server.client();

    let declared = client.post_zeros("/api/test/body", Some(MIB + 1), 0).await;
    let exact = client.post_zeros("/api/test/body", Some(MIB), MIB).await;
    let big_json = format!("\"{}\"", "x".repeat(2 * MIB as usize));
    let refused = client.send(Request::new(Method::POST, "/api/test/body").body("application/json", &big_json)).await;
    let allowed = client.send(Request::new(Method::POST, "/api/test/big").body("application/json", &big_json)).await;

    // A body that is not declared and outgrows the limit is a 400; the test of
    // the server's own ceiling covers it, since a client that is still sending
    // when the answer comes can lose it to a reset.
    assert_eq!([declared.status, exact.status], [413, 400]);
    assert_eq!(declared.header("connection"), Some("close"));
    assert_eq!(refused.status, 413);
    assert_eq!(allowed.status, 200);
    assert_eq!(allowed.body.len(), 2 * MIB as usize + 2);
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
async fn a_forwarded_scheme_is_passed_on_as_the_proxy_wrote_it() {
    let server = server_with(&[]).await;

    for (written, seen) in [("gopher", "gopher"), ("HTTPS", "HTTPS"), ("http, ws", "ws"), ("", "http"), ("a b", "http")] {
        let answer = server
            .client()
            .send(Request::new(Method::GET, "/api/test/client").header("X-Forwarded-Proto", written))
            .await;
        assert_eq!(answer.json::<Value>()["scheme"], seen, "{written:?}");
    }
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

#[tokio::test]
async fn a_segment_that_is_not_an_id_is_a_path_that_does_not_exist() {
    let server = server_with(&[]).await;
    let client = server.client();

    let found = client
        .get("/api/test/things/8F62A4D1-BDCB-4D86-BD1C-6E351900E629")
        .await;
    let not_an_id = client.get("/api/test/things/xyz").await;

    assert_eq!(
        found.json::<Value>(),
        json!({ "id": "8f62a4d1-bdcb-4d86-bd1c-6e351900e629" })
    );
    assert_eq!(not_an_id.status, 404);
    assert!(not_an_id.body.is_empty());
}

#[tokio::test]
async fn a_query_that_does_not_fit_is_a_bad_request_with_no_body() {
    let server = server_with(&[]).await;
    let client = server.client();

    let fits = client.get("/api/test/search?limit=5").await;
    let missing = client.get("/api/test/search").await;
    let not_a_number = client.get("/api/test/search?limit=many").await;

    assert_eq!(fits.json::<Value>(), json!({ "limit": 5 }));
    assert_eq!([missing.status, not_a_number.status], [400, 400]);
    assert!(missing.body.is_empty() && not_a_number.body.is_empty());
}

#[tokio::test]
async fn a_method_the_route_does_not_have_is_a_405_that_lists_the_ones_it_has() {
    let server = server_with(&[]).await;
    let client = server.client();
    let id = "8f62a4d1-bdcb-4d86-bd1c-6e351900e629";

    let post = client
        .send(Request::new(
            Method::POST,
            &format!("/api/test/things/{id}"),
        ))
        .await;
    let head = client
        .send(Request::new(
            Method::HEAD,
            &format!("/api/test/things/{id}"),
        ))
        .await;
    let put = client
        .send(Request::new(Method::PUT, "/api/test/search"))
        .await;

    assert_eq!([post.status, head.status, put.status], [405, 405, 405]);
    assert_eq!(post.header("allow"), Some("DELETE, GET"));
    assert_eq!(head.header("allow"), Some("DELETE, GET"));
    assert_eq!(put.header("allow"), Some("GET"));
    assert!(post.body.is_empty());
}

#[tokio::test]
async fn a_body_with_no_content_type_is_a_415_and_no_body_at_all_is_left_to_the_handler() {
    let server = server_with(&[]).await;
    let client = server.client();

    let unnamed = client
        .send(Request::new(Method::POST, "/api/test/body").untyped_body("{\"a\":1}"))
        .await;
    let nothing = client
        .send(Request::new(Method::POST, "/api/test/body"))
        .await;

    assert_eq!(unnamed.status, 415);
    assert_eq!(nothing.status, 400);
}

#[tokio::test]
async fn with_nothing_listed_everyone_behind_the_proxy_shares_its_budget() {
    let server = server_with(&[
        PRODUCTION[0],
        PRODUCTION[1],
        ("RateLimiting__Login__PermitLimit", "1"),
    ])
    .await;
    let from = |client: &str| {
        Request::new(Method::POST, "/api/auth/login")
            .header("X-Forwarded-For", &format!("{client}, 172.18.0.1"))
            .json(&json!({}))
    };

    let first = server.client().send(from("198.51.100.7")).await;
    let somebody_else = server.client().send(from("198.51.100.8")).await;

    // The proxy is the only address the server can see.
    assert_eq!([first.status, somebody_else.status], [401, 429]);
}

#[tokio::test]
async fn every_request_is_logged_once_with_the_status_the_client_got() {
    let log = crate::support::LogCapture::start();
    let server = server_with(&[]).await;
    let client = server.client();

    client.get("/nothing/here").await;
    client
        .send(Request::new(Method::POST, "/api/test/fails"))
        .await;
    client.get("/api/test/fails").await;
    client.get("/health/live").await;
    // A request is logged once its response is out.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let lines = log.lines();
    for expected in [
        "GET /***/*** 404 in ",
        "POST /api/test/fails 405 in ",
        "GET /api/test/fails 500 in ",
    ] {
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with(expected))
                .count(),
            1,
            "{expected}: {lines:#?}"
        );
    }
    assert!(
        !lines.iter().any(|line| line.contains("/health/")),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn a_handler_whose_client_went_away_still_commits_and_announces() {
    let server = server_with(&[]).await;
    assert!(server.events().is_empty());

    // The handler is running, the client leaves, and only then is the handler
    // let go on. However slow the machine is, the order is the same.
    server.client().get_and_hang_up("/api/test/slow-announce").await;
    HANDLER_STARTED.acquire().await.expect("open").forget();
    CLIENT_GONE.add_permits(1);

    let mut events = Vec::new();
    for _ in 0..600 {
        events = server.events();
        if !events.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(events.len(), 1);
    let connection = rusqlite::Connection::open(server.database_path()).unwrap();
    let rows: i64 = connection.query_row("SELECT count(*) FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    assert_eq!(rows, 1);
}

// ---------------------------------------------------------------------------
// The path every step sees
// ---------------------------------------------------------------------------

/// Spellings of `/health/live` and what the first server answered to each,
/// asked of it one by one (`paths_are_read_as_the_first_server_reads_them`
/// asks again). A target is sent byte for byte.
const SPELLINGS: &[(&[u8], u16)] = &[
    (b"/health/live", 200),
    // One slash at the end is dropped; two are not.
    (b"/health/live/", 200),
    (b"/health/live//", 404),
    (b"//health/live", 404),
    (b"/health//live", 404),
    (b"/health/live/?a=1", 200),
    (b"/health/live?", 200),
    // Dot segments go before routing, and cannot leave the root.
    (b"/health/./live", 200),
    (b"/health/x/../live", 200),
    (b"/./health/live", 200),
    (b"/health/live/.", 200),
    (b"/health/live/./.", 200),
    (b"/health/live/./", 200),
    (b"/health/./live/", 200),
    (b"/health/live/..", 404),
    (b"/health/live/../live/", 200),
    (b"/../health/live", 200),
    (b"/health/../../health/live", 200),
    (b"/..", 404),
    (b"/.", 404),
    (b"/", 404),
    (b"//", 404),
    (b"/health/.../live", 404),
    (b"/health/..;/live", 404),
    (b"/health/live;x=1", 404),
    (b"/health/live.", 404),
    // Escapes are decoded first, so they cannot hide a dot segment...
    (b"/%68ealth/live", 200),
    (b"/health/li%76e", 200),
    (b"/health/%2e/live", 200),
    (b"/health/%2e%2e/health/live", 200),
    (b"/health/%2E%2E/health/live", 200),
    (b"/health/%2e%2E/health/live", 200),
    (b"/health/%2e%2e/%2e%2e/health/live", 200),
    (b"/health/live/%2e/", 200),
    (b"/health/live/%2e%2e/live", 200),
    (b"/health/live/.%2e/live", 200),
    (b"/health/live/%2e%2E", 404),
    (b"/health/live%2e", 404),
    // ...except an encoded slash, which is never a separator.
    (b"/health%2flive", 404),
    (b"/health%2Flive", 404),
    (b"/health/live%2f", 404),
    (b"/health/live%2F/", 404),
    (b"/health/live/%2f", 404),
    (b"/health/..%2flive", 404),
    (b"/health/%2e%2e%2flive", 404),
    // Other characters decoded are characters of the path, not syntax.
    (b"/health%5clive", 404),
    (b"/health/live%20", 404),
    (b"/health/live%3f", 404),
    (b"/health/live%23", 404),
    (b"/health/live%25", 404),
    (b"/health/live%2500", 404),
    (b"/health/li%zzve", 404),
    (b"/health/li%7", 404),
    (b"/health/live%C3%A9", 404),
    (b"/health/live%ff", 404),
    (b"/health/live%c0%af", 404),
    // The query is not read.
    (b"/health/live?x=%00", 200),
    (b"/health/live?x=%zz&y=%", 200),
    // A null character in the path is refused before anything else.
    (b"/health/live%00", 400),
    (b"/health/%00live", 400),
    (b"/health/li%00ve", 400),
    (b"/%00", 400),
];

#[tokio::test]
async fn a_path_is_read_before_routing_as_the_first_server_reads_it() {
    let server = server_with(&[]).await;

    for (target, expected) in SPELLINGS {
        let answer = raw_get(server.address(), target, &[]).await;
        assert_eq!(
            answer.status,
            *expected,
            "{}",
            String::from_utf8_lossy(target)
        );
    }
}

#[tokio::test]
async fn a_request_line_over_8192_bytes_is_a_414_and_one_that_fits_is_not() {
    let server = server_with(&[]).await;
    // `GET ` and ` HTTP/1.1` and the line break take 15 of them.
    let longest = 8192 - 15;
    let target = |length: usize| format!("/health/live?{}", "a".repeat(length - "/health/live?".len()));

    let fits = raw_get(server.address(), target(longest).as_bytes(), &[]).await;
    let over = raw_get(server.address(), target(longest + 1).as_bytes(), &[]).await;
    let post = raw_send(server.address(), "POST", target(longest).as_bytes(), &[]).await;

    assert_eq!((fits.status, over.status, post.status), (200, 414, 414));
    assert!(over.body.is_empty());
}

#[tokio::test]
async fn what_a_segment_holds_is_what_the_first_server_would_have_read_from_it() {
    let server = server_with(&[]).await;
    let said = |target: &'static str| {
        let address = server.address();
        async move {
            let answer = raw_get(address, target.as_bytes(), &[]).await;
            assert_eq!(answer.status, 200, "{target}");
            serde_json::from_slice::<Value>(&answer.body).unwrap()["what"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };

    for (target, what) in [
        ("/api/test/echo/plain", "plain"),
        ("/api/test/echo/pl%61in", "plain"),
        ("/api/test/echo/a%2fb", "a%2fb"),
        ("/api/test/echo/a%2Fb", "a%2Fb"),
        ("/api/test/echo/a%25b", "a%b"),
        ("/api/test/echo/a%252fb", "a%2fb"),
        ("/api/test/echo/a%20b%3Fc%23d", "a b?c#d"),
        ("/api/test/echo/a%5Cb", "a\\b"),
        ("/api/test/echo/a+b", "a+b"),
        ("/api/test/echo/a%2bb", "a+b"),
        ("/api/test/echo/%C3%A9", "\u{e9}"),
        ("/api/test/echo/%c3%a9", "\u{e9}"),
        ("/api/test/echo/x%c3", "x%c3"),
        ("/api/test/echo/x%ff%c0%af", "x%ff%c0%af"),
        ("/api/test/echo/x%e2%82", "x%e2%82"),
        ("/api/test/echo/x%e2%82%ac", "x\u{20ac}"),
        ("/api/test/echo/x%zz", "x%zz"),
        ("/api/test/echo/a%7e", "a~"),
        ("/api/test/echo/a/", "a"),
        ("/api/test/echo/..%2fa", "..%2fa"),
    ] {
        assert_eq!(said(target).await, what, "{target}");
    }
    // A segment that is not there is not a segment.
    let missing = raw_get(server.address(), b"/api/test/echo/", &[]).await;
    assert_eq!(missing.status, 404);
}

/// The one the first server let through: its gate decides on the path as
/// written, and its router on the path without the slash.
const THE_GATE: &[&str] = &[
    "/api/auth/login",
    "/api/auth/login/",
    "/api/./auth/login",
    "/api/x/../auth/login",
    "/health/../api/auth/login",
    "/api/%61uth/%6cogin",
    "/api/auth/login/.",
    "/api/auth/login?x=1",
];

#[tokio::test]
async fn the_client_gate_judges_every_spelling_of_a_gated_route_as_the_route_it_is() {
    let server = server_with(&[]).await;

    for target in THE_GATE {
        let refused = raw_send(server.address(), "POST", target.as_bytes(), &[]).await;
        let admitted = raw_send(
            server.address(),
            "POST",
            target.as_bytes(),
            &[("X-Stellarr-Client", "web"), ("X-Stellarr-Client-Version", "2.0.0")],
        )
        .await;

        // A client the gate does not know is refused; one it knows reaches the
        // handler (which has nothing to read).
        assert_eq!(refused.status, 403, "{target}");
        assert_ne!(admitted.status, 403, "{target}");
        assert_ne!(admitted.status, 404, "{target}");
    }
    // What is not the route is not gated and is not served.
    for target in ["/api/auth/login//", "/api/auth/login%2f", "/api/auth/log%69n%2f"] {
        let answer = raw_send(server.address(), "POST", target.as_bytes(), &[]).await;
        assert_eq!(answer.status, 404, "{target}");
    }
}

#[tokio::test]
async fn every_spelling_of_a_limited_route_counts_against_its_limit() {
    let server = server_with(&[
        PRODUCTION[0],
        PRODUCTION[1],
        ("RateLimiting__Login__PermitLimit", "4"),
    ])
    .await;
    let client = [("X-Stellarr-Client", "web"), ("X-Stellarr-Client-Version", "2.0.0")];

    let mut statuses = Vec::new();
    for target in [
        "/api/auth/login",
        "/api/auth/login/",
        "/api/%61uth/login",
        "/api/./auth/login/.",
        "/api/auth/login",
        "/api/auth/login/",
    ] {
        let body = r#"{"username":"x","password":"y"}"#;
        let request = format!(
            "POST {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\n\r\n{body}",
            client.iter().map(|(n, v)| format!("{n}: {v}\r\n")).collect::<String>(),
            body.len()
        );
        statuses.push(crate::support::raw_request(server.address(), request.as_bytes()).await.status);
    }

    assert_eq!(statuses, [401, 401, 401, 401, 429, 429]);
}

#[tokio::test]
async fn every_spelling_of_a_route_that_needs_a_session_needs_one() {
    let server = server_with(&[]).await;

    for target in [
        "/api/test/private",
        "/api/test/private/",
        "/api/test/./private",
        "/api/test/x/../private/",
        "/api/%74est/%70rivate",
        "/api/test/private/.",
        "/api/test/private?a=b",
    ] {
        let answer = raw_get(server.address(), target.as_bytes(), &[]).await;
        assert_eq!(answer.status, 401, "{target}");
        assert!(answer.body.is_empty(), "{target}");
    }
    for target in ["/api/test/private//", "/api/test/private%2f", "/api/test/private/..%2f"] {
        assert_eq!(raw_get(server.address(), target.as_bytes(), &[]).await.status, 404, "{target}");
    }
}

#[tokio::test]
async fn a_refusal_of_the_path_is_not_counted_or_logged_as_a_request() {
    let server = server_with(&[]).await;
    let before = server.client().get("/api/test/counts").await.json::<Value>();

    let refused = raw_get(server.address(), b"/health/live%00", &[]).await;
    let after = server.client().get("/api/test/counts").await.json::<Value>();

    assert_eq!(refused.status, 400);
    assert_eq!(refused.header("connection"), Some("close"));
    // The first `counts` is in the second answer; the refusal is not.
    assert_eq!(after["requests"].as_u64(), before["requests"].as_u64().map(|n| n + 1));
    assert_eq!(after["clientErrors"], before["clientErrors"]);
}

/// Not part of the ordinary run: it asks the first server, which the
/// integrator's `oracle` script starts in a container, the same questions.
///
/// ```sh
/// oracle start paths 5192 Production
/// ORACLE_ADDRESS=127.0.0.1:5192 cargo test -p stellarr-api --test api \
///     pipeline::paths_are_read_as_the_first_server_reads_them -- --ignored --nocapture
/// oracle stop paths
/// ```
#[tokio::test]
#[ignore = "needs the first server: see the doc comment"]
async fn paths_are_read_as_the_first_server_reads_them() {
    let oracle: std::net::SocketAddr = std::env::var("ORACLE_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:5192".into())
        .parse()
        .expect("an address");
    let server = server_with(&[]).await;
    let mut different = Vec::new();
    let long = [b"/health/live?".as_slice(), &vec![b'a'; 9000]].concat();
    let mut cases: Vec<(String, Vec<u8>, &str, Option<&str>)> = SPELLINGS
        .iter()
        .map(|(target, _)| (String::from_utf8_lossy(target).into_owned(), target.to_vec(), "GET", None))
        .collect();
    cases.push(("a request line over 8 KiB".into(), long, "GET", None));
    // Not read as a fragment by the server's HTTP library; here it is dropped
    // and the rest is served, there it is refused.
    cases.push(("a fragment".into(), b"/health/live#frag".to_vec(), "GET", Some("fragment")));
    cases.push(("a backslash".into(), b"/health\\live".to_vec(), "GET", None));
    // The first server's gate is bypassed by a slash at the end of the path
    // (it judges the path as written and routes the path without it); here
    // the gate and the router read the same path.
    cases.push(("the first server's gate, slash at the end".into(), b"/api/session/".to_vec(), "GET", Some("gate")));
    // Letter case: routes are matched with it here.
    cases.push(("letter case".into(), b"/HEALTH/live".to_vec(), "GET", Some("case")));

    for (label, target, method, known) in cases {
        let theirs = raw_send(oracle, method, &target, &[]).await;
        let ours = raw_send(server.address(), method, &target, &[]).await;
        let describe = |answer: &crate::support::Answer| {
            format!("{} {:?} {}", answer.status, answer.header("content-type"), answer.body.len())
        };
        // The first server writes its JSON in chunks and this one with a length.
        let same = theirs.status == ours.status && theirs.body == ours.body
            || (theirs.status == 200 && ours.status == 200);
        match (same, known) {
            (true, _) => println!("same      {label}"),
            (false, Some(why)) => println!("known     {label} ({why}: {} / {})", describe(&theirs), describe(&ours)),
            (false, None) => {
                println!("DIFFERENT {label}: first server {}, this one {}", describe(&theirs), describe(&ours));
                different.push(label);
            }
        }
    }
    assert!(different.is_empty(), "{different:?}");
}

#[tokio::test]
async fn what_path_and_query_accept() {
    let server = server_with(&[]).await;
    let client = server.client();

    // A name given twice is an error for a struct field, and is kept, in
    // order, by a list of pairs; a name nobody asked for is ignored.
    assert_eq!(client.get("/api/test/search?limit=1&limit=2").await.status, 400);
    assert_eq!(client.get("/api/test/search?limit=1&other=2").await.json::<Value>(), json!({ "limit": 1 }));
    assert_eq!(
        client.get("/api/test/pairs?a=1&b=&a=2&c").await.json::<Value>(),
        json!({ "pairs": [["a", "1"], ["b", ""], ["a", "2"], ["c", ""]] })
    );
    assert_eq!(
        client.get("/api/test/pairs?a=x%20y+z%2Bw").await.json::<Value>(),
        json!({ "pairs": [["a", "x y z+w"]] })
    );
    // A segment is never empty.
    assert_eq!(raw_get(server.address(), b"/api/test/echo//", &[]).await.status, 404);
    assert_eq!(raw_get(server.address(), b"/api/test/echo/a//", &[]).await.status, 404);
}
