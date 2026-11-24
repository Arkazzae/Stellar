//! The account's signed-in devices: listing them, and ending one or all the
//! others. A session that ends here is also shut on the hub, so that what it
//! had open does not outlive it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::params;
use serde::Serialize;

use super::no_store;
use crate::auth::{Caller, events, session};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Origin, Path, json_response};
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

/// The most sessions the list shows.
const LISTED: i64 = 100;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnSession {
    id: Id,
    device: String,
    ip_address: String,
    created_at: Timestamp,
    last_seen_at: Timestamp,
    expires_at: Timestamp,
    primary_source: String,
    mfa_evidence: Option<String>,
    current: bool,
}

pub async fn list(State(state): State<AppState>, caller: Caller) -> Response {
    no_store(listing(&state, caller).await)
}

async fn listing(state: &AppState, caller: Caller) -> ApiResult<Response> {
    let sessions = state
        .database()
        .read(move |connection| -> rusqlite::Result<Vec<OwnSession>> {
            let mut statement = connection.prepare(
                "SELECT \"Id\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\", \"PrimarySource\", \"MfaEvidence\"
                 FROM \"UserSessions\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2
                 ORDER BY \"LastSeenAt\" DESC LIMIT ?3",
            )?;
            statement
                .query_map(params![caller.user, Timestamp::now(), LISTED], |row| {
                    let id: Id = row.get(0)?;
                    Ok(OwnSession {
                        id,
                        device: row.get(1)?,
                        ip_address: row.get(2)?,
                        created_at: row.get(3)?,
                        last_seen_at: row.get(4)?,
                        expires_at: row.get(5)?,
                        primary_source: row.get(6)?,
                        mfa_evidence: row.get(7)?,
                        current: id == caller.session,
                    })
                })?
                .collect()
        })
        .await?;
    Ok(json_response(StatusCode::OK, &sessions))
}

pub async fn revoke(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Path(Guid(target)): Path<Guid>,
) -> Response {
    no_store(revoking(&state, caller, &origin, target).await)
}

/// Ends one of the account's other sessions. The current one ends by signing
/// out, so that this cannot strand the person who asks.
async fn revoking(state: &AppState, caller: Caller, origin: &Origin, target: Id) -> ApiResult<Response> {
    if target == caller.session {
        return Err(ApiError::conflict());
    }
    let origin = origin.clone();
    let ended = state
        .database()
        .write(move |tx| -> Result<bool, DbError> {
            let own: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL)",
                params![target, caller.user],
                |row| row.get(0),
            )?;
            if !own || !session::revoke(tx, target, Timestamp::now())? {
                return Ok(false);
            }
            events::record(tx, caller.user, events::SESSION_REVOKED, &origin.address, &origin.user_agent, "device", None)?;
            Ok(true)
        })
        .await?;
    if !ended {
        return Err(ApiError::not_found());
    }
    state.events().session_ended(caller.user, target);
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn revoke_others(State(state): State<AppState>, caller: Caller, origin: Origin) -> Response {
    no_store(revoking_others(&state, caller, &origin).await)
}

/// Ends every session of the account but the one that asks. Nothing else
/// about the account changes: its remembered browsers and its other sessions'
/// versions stay.
async fn revoking_others(state: &AppState, caller: Caller, origin: &Origin) -> ApiResult<Response> {
    let origin = origin.clone();
    let ended = state
        .database()
        .write(move |tx| -> Result<Vec<Id>, DbError> {
            let others: Vec<Id> = {
                let mut statement = tx.prepare(
                    "SELECT \"Id\" FROM \"UserSessions\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"Id\" <> ?2",
                )?;
                statement
                    .query_map(params![caller.user, caller.session], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            if others.is_empty() {
                return Ok(others);
            }
            let now = Timestamp::now();
            for other in &others {
                session::revoke(tx, *other, now)?;
            }
            events::record(tx, caller.user, events::SESSION_REVOKED, &origin.address, &origin.user_agent, "others", None)?;
            Ok(others)
        })
        .await?;
    for session in ended {
        state.events().session_ended(caller.user, session);
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
