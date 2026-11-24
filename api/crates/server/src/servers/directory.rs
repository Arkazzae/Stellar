//! The servers a person is in, the public ones they could join, making a new
//! one, and joining a public one.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};

use super::{announce, join};
use super::join::Entry;
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::instance::servers::{self, Server, ServerDto};
use crate::instance::starter::{self, ServerRequest};
use crate::instance::users::User;
use crate::instance::welcome::{self, Welcome};
use crate::state::AppState;
use crate::values::Timestamp;

/// How many public servers one look at the directory shows.
const DISCOVER_PAGE_SIZE: i64 = 100;

/// The servers the caller is in, the one they joined first at the top.
pub async fn list(State(state): State<AppState>, caller: Caller) -> ApiResult<Json<Vec<ServerDto>>> {
    let found = state
        .database()
        .read(move |db| -> Result<Vec<ServerDto>, DbError> {
            let mut statement = db.prepare(&format!(
                "SELECT {} FROM \"ServerMembers\" m JOIN \"Servers\" s ON s.\"Id\" = m.\"ServerId\"
                 WHERE m.\"UserId\" = ?1 ORDER BY m.\"JoinedAt\"",
                prefixed("s")
            ))?;
            let rows = statement
                .query_map([caller.user], Server::from_row)?
                .collect::<rusqlite::Result<Vec<Server>>>()?;
            Ok(rows.iter().map(Server::dto).collect())
        })
        .await?;
    Ok(Json(found))
}

/// The public servers the caller is neither in nor banned from, newest first.
pub async fn discover(State(state): State<AppState>, caller: Caller) -> ApiResult<Json<Vec<ServerDto>>> {
    let found = state
        .database()
        .read(move |db| -> Result<Vec<ServerDto>, DbError> {
            let mut statement = db.prepare(&format!(
                "SELECT {} FROM \"Servers\" s
                 WHERE s.\"IsPublic\" = 1
                   AND NOT EXISTS (SELECT 1 FROM \"ServerMembers\" m WHERE m.\"ServerId\" = s.\"Id\" AND m.\"UserId\" = ?1)
                   AND NOT EXISTS (SELECT 1 FROM \"ServerBans\" b WHERE b.\"ServerId\" = s.\"Id\" AND b.\"UserId\" = ?1)
                 ORDER BY s.\"CreatedAt\" DESC LIMIT ?2",
                prefixed("s")
            ))?;
            let rows = statement
                .query_map(params![caller.user, DISCOVER_PAGE_SIZE], Server::from_row)?
                .collect::<rusqlite::Result<Vec<Server>>>()?;
            Ok(rows.iter().map(Server::dto).collect())
        })
        .await?;
    Ok(Json(found))
}

/// The server columns, each qualified by the alias of its table.
fn prefixed(alias: &str) -> String {
    servers::COLUMNS
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A new server with its two system roles, its starter channels and its
/// notification defaults, owned by the caller, who is greeted in its first
/// channel.
pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Json(request): Json<ServerRequest>,
) -> ApiResult<Response> {
    let starter = starter::prepare(&request, state.messages())?;
    let (server, welcome) = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<(Server, Option<Welcome>), DbError> {
                let messages = state.messages();
                let now = Timestamp::now();
                let added = starter::add(tx, &starter, messages, now)?;
                crate::permissions::ownership::admit(tx, added.id, caller.user, added.owner_role, false, Some(1), now)?;
                let owner = User::find(tx, caller.user)?
                    .ok_or_else(|| DbError::Task("the signed-in account is not there".into()))?;
                let welcome = welcome::create(tx, added.id, &owner, now, messages)?;
                let server = Server::find(tx, added.id)?
                    .ok_or_else(|| DbError::Task("the server just written is not there".into()))?;
                Ok((server, welcome))
            }
        })
        .await?;
    announce::welcomed(&state, welcome).await;
    let mut response = json_response(StatusCode::CREATED, &server.dto());
    if let Ok(location) = HeaderValue::from_str(&format!("/api/servers/{}", server.id)) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

/// Joins a public server through its own door.
pub async fn join_public(
    State(state): State<AppState>,
    caller: Caller,
    Path(server): Path<String>,
) -> ApiResult<Json<ServerDto>> {
    let server = super::guid(&server)?;
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Option<(Server, Entry)>, DbError> {
                let found = tx
                    .query_row(
                        &format!(
                            "SELECT {} FROM \"Servers\" WHERE \"Id\" = ?1 AND \"IsPublic\" = 1",
                            servers::COLUMNS
                        ),
                        [server],
                        Server::from_row,
                    )
                    .optional()?;
                let Some(found) = found else {
                    return Ok(None);
                };
                let entry = join::enter(tx, found.id, caller.user, Timestamp::now(), state.messages(), None)?;
                Ok(Some((found, entry)))
            }
        })
        .await?;
    let Some((server, entry)) = outcome else {
        return Err(ApiError::not_found());
    };
    join::publish(&state, caller.user, server.id, join::joined(entry, &state)?).await;
    Ok(Json(server.dto()))
}
