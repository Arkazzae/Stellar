//! A route that asks for a fresh second step, tested with routes that exist
//! only here, and a sign-in the test writes itself: no group of endpoints that
//! signs people in or sets a second step up is needed to know what the
//! pipeline does with a session that passed one a while ago.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::params;
use serde::Deserialize;
use serde_json::Value;
use stellarr_api::auth::session::{NewSession, sign_in_cookie};
use stellarr_api::http::{ApiResult, Endpoints, Query, with_session_cookie};
use stellarr_api::state::AppState;
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, Server};

fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/test/session", session)
        .get("/api/test/guarded", guarded)
        .fresh_second_step()
        .get("/api/test/open", guarded)
        .authorized()
}

#[derive(Deserialize)]
struct Wanted {
    /// Whether the account has a second step.
    method: bool,
    /// How long ago the session passed it, in seconds, if it did.
    passed: Option<u64>,
    /// What the session records as having stood in for one.
    evidence: Option<String>,
}

/// Writes an account and a session as asked, and signs the caller in.
async fn session(State(state): State<AppState>, Query(wanted): Query<Wanted>) -> ApiResult<Response> {
    let now = Timestamp::now();
    let user = Id::random();
    let mut session = NewSession::new(user, "Test".into(), "127.0.0.1".into(), "password", 1, false);
    session.mfa_completed_at = wanted
        .passed
        .map(|seconds| now.minus(std::time::Duration::from_secs(seconds)));
    session.mfa_evidence = wanted.evidence;
    let id = session.id;
    state
        .database()
        .write(move |tx| -> Result<(), stellarr_api::database::DbError> {
            tx.execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\",
                     \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, ?1, ?1, 'Test', 'x', 0, 1, 0, 'c', ?2)",
                params![user, now],
            )?;
            if wanted.method {
                tx.execute(
                    "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\")
                     VALUES (?1, ?2, 'totp', 'Phone', ?3, ?3)",
                    params![Id::random(), user, now],
                )?;
            }
            session.insert(tx)
        })
        .await?;
    let cookie = sign_in_cookie(state.sealer(), &state.settings().environment, false, user, 1, id, false);
    Ok(with_session_cookie(StatusCode::NO_CONTENT.into_response(), &cookie))
}

async fn guarded() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn signed_in(server: &Server, query: &str) -> Client {
    let client = server.client();
    let response = client.get(&format!("/api/test/session?{query}")).await;
    assert_eq!(response.status, 204, "{}", response.text());
    client
}

async fn server() -> Server {
    Server::in_process(routes(), &[]).await
}

#[tokio::test]
async fn nobody_signed_in_is_refused_before_the_second_step_is_looked_at() {
    let server = server().await;

    let response = server.client().get("/api/test/guarded").await;

    assert_eq!(response.status, 401);
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn an_account_without_a_second_step_has_nothing_to_prove() {
    let server = server().await;
    let client = signed_in(&server, "method=false").await;

    assert_eq!(client.get("/api/test/guarded").await.status, 204);
}

#[tokio::test]
async fn a_session_that_passed_the_second_step_lately_is_let_through() {
    let server = server().await;
    let client = signed_in(&server, "method=true&passed=60").await;

    assert_eq!(client.get("/api/test/guarded").await.status, 204);
}

#[tokio::test]
async fn a_session_that_passed_it_long_ago_or_never_is_refused_with_the_code_the_clients_know() {
    let server = server().await;
    for query in [
        "method=true&passed=600",
        "method=true",
        // A remembered device is not a second step passed.
        "method=true&evidence=trusted_device",
    ] {
        let client = signed_in(&server, query).await;

        let response = client.get("/api/test/guarded").await;

        assert_eq!(response.status, 403, "{query}");
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        // As the first server answers it, member by member, in this order.
        assert!(
            response.text().starts_with(
                "{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.4\",\"title\":\"This needs a second step from the last 5 minutes. Confirm it and try again.\",\"status\":403,\"code\":\"fresh-mfa-required\",\"traceId\":\"00-"
            ),
            "{}",
            response.text()
        );
        let body: Value = response.json();
        assert_eq!(body["code"], "fresh-mfa-required", "{query}");
        assert_eq!(body["status"], 403);
        assert_eq!(
            body["title"],
            "This needs a second step from the last 5 minutes. Confirm it and try again."
        );
        assert_eq!(body["type"], "https://tools.ietf.org/html/rfc9110#section-15.5.4");
        // The route that only asks for a session is not refused.
        assert_eq!(client.get("/api/test/open").await.status, 204, "{query}");
    }
}
