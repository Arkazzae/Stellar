//! Opening the conversation with somebody.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::http::lenient::{Malformed, Members};
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Json, json_response};
use crate::social::{announce, is_blocked, pair};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

#[derive(Serialize)]
struct Opened {
    id: String,
}

/// Opens the conversation with somebody, or returns the one that is there: the
/// pair has a single conversation, also when both ask at once.
pub async fn open(
    State(state): State<AppState>,
    caller: Caller,
    Json(body): Json<Members>,
) -> ApiResult<Response> {
    // `userId` is not optional: a missing one is the empty id, which is nobody.
    let other = match body.get::<Id>("userId") {
        Ok(Some(id)) => id,
        Ok(None) if body.is_null("userId") => return Err(Malformed.into()),
        Ok(None) => Id::from_bytes([0; 16]),
        Err(malformed) => return Err(malformed.into()),
    };
    let me = caller.user;
    let (id, created) = state
        .database()
        .write(move |tx| -> Result<(Id, bool), ApiError> {
            if me == other || !crate::social::is_reachable(tx, other)? {
                return Err(ApiError::not_found());
            }
            if is_blocked(tx, me, other)? {
                return Err(ApiError::forbidden());
            }
            let (a, b) = pair(me, other);
            let existing: Option<Id> = tx
                .query_row(
                    "SELECT \"Id\" FROM \"DirectConversations\" WHERE \"UserAId\" = ?1 AND \"UserBId\" = ?2",
                    params![a, b],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok((id, false));
            }
            let id = Id::random();
            tx.execute(
                "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\",
                     \"EncryptionRequestedById\", \"EncryptionRequestedAt\", \"EncryptionEnabledAt\", \"CurrentEpochId\")
                 VALUES (?1, ?2, ?3, ?4, 'Off', NULL, NULL, NULL, NULL)",
                params![id, a, b, Timestamp::now()],
            )?;
            Ok((id, true))
        })
        .await?;
    if created {
        announce(&state, &[me, other]);
    }
    Ok(json_response(StatusCode::OK, &Opened { id: id.to_string() }))
}
