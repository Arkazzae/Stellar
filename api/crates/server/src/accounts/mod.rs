//! Founding the instance, registering, signing in and out, and reading who is
//! signed in.
//!
//! `GET /api/auth/registration`, `POST /api/auth/register`, `POST
//! /api/auth/login`, `POST /api/auth/logout`, `GET /api/session` and `GET
//! /api/me`.

mod login;
pub(crate) mod me;
mod register;

use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::Response;
use serde::Serialize;

use crate::database::DbError;
use crate::http::{ApiResult, Endpoints, MaybeCaller, RateLimitPolicy, json_response};
use crate::instance::users::{User, UserDto};
use crate::state::AppState;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/auth/registration", registration)
        .post("/api/auth/register", register::register)
        .json_body()
        .rate_limit(RateLimitPolicy::AuthWrite)
        .post("/api/auth/login", login::login)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/logout", login::logout)
        .authorized()
        .get("/api/session", session)
        .get("/api/me", me::read)
        .authorized()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Registration {
    setup_required: bool,
}

/// Whether the instance waits for its owner. It does until it has an account,
/// whatever happens to its servers later.
async fn registration(State(state): State<AppState>) -> ApiResult<Response> {
    let setup_required = state
        .database()
        .read(|connection| -> Result<bool, DbError> {
            Ok(connection.query_row("SELECT NOT EXISTS (SELECT 1 FROM \"Users\")", [], |row| row.get(0))?)
        })
        .await?;
    let mut response = json_response(axum::http::StatusCode::OK, &Registration { setup_required });
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[derive(Serialize)]
struct SessionBody {
    user: Option<UserDto>,
}

/// Open to everyone: "nobody is signed in" is an answer, not an error.
async fn session(State(state): State<AppState>, MaybeCaller(caller): MaybeCaller) -> ApiResult<Response> {
    let Some(caller) = caller else {
        return Ok(json_response(axum::http::StatusCode::OK, &SessionBody { user: None }));
    };
    let user = state
        .database()
        .read(move |connection| -> Result<_, DbError> { Ok(User::find(connection, caller.user)?) })
        .await?;
    let user = user.map(|user| {
        let presence = state.presence_of(user.id);
        user.dto(&presence)
    });
    Ok(json_response(axum::http::StatusCode::OK, &SessionBody { user }))
}
