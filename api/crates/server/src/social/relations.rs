//! Ending a friendship, blocking and unblocking.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::params;

use super::{GuidSegment, announce, pair};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path};
use crate::state::AppState;
use crate::values::Timestamp;

pub async fn remove_friend(
    State(state): State<AppState>,
    caller: Caller,
    Path(GuidSegment(other)): Path<GuidSegment>,
) -> ApiResult<Response> {
    let me = caller.user;
    let (a, b) = pair(me, other);
    let exists: bool = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok(db.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Friendships\" WHERE \"UserAId\" = ?1 AND \"UserBId\" = ?2)",
                params![a, b],
                |row| row.get(0),
            )?)
        })
        .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "DELETE FROM \"Friendships\" WHERE \"UserAId\" = ?1 AND \"UserBId\" = ?2",
                params![a, b],
            )?;
            Ok(())
        })
        .await?;
    announce(&state, &[me, other]);
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Blocks somebody: the friendship and any pending request between the two go,
/// in both directions, and a call between them ends. Blocking again changes
/// nothing.
pub async fn block(
    State(state): State<AppState>,
    caller: Caller,
    Path(GuidSegment(other)): Path<GuidSegment>,
) -> ApiResult<Response> {
    let me = caller.user;
    if me == other {
        return Err(ApiError::not_found());
    }
    let (a, b) = pair(me, other);
    let done = state
        .database()
        .write(move |tx| -> Result<bool, DbError> {
            let person: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Users\" WHERE \"Id\" = ?1 AND \"Origin\" IS NULL)",
                [other],
                |row| row.get(0),
            )?;
            if !person {
                return Ok(false);
            }
            tx.execute(
                "DELETE FROM \"Friendships\" WHERE \"UserAId\" = ?1 AND \"UserBId\" = ?2",
                params![a, b],
            )?;
            tx.execute(
                "DELETE FROM \"FriendRequests\"
                 WHERE (\"SenderId\" = ?1 AND \"ReceiverId\" = ?2) OR (\"SenderId\" = ?2 AND \"ReceiverId\" = ?1)",
                params![me, other],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
                params![me, other, Timestamp::now()],
            )?;
            Ok(true)
        })
        .await?;
    if !done {
        return Err(ApiError::not_found());
    }
    // OPEN POINT: the first server also ends a direct voice call between the
    // two here (`EndDirectCallAsync`): it empties the private room, tells the
    // seated connections `DirectCallDeclined` {channelId, byUserId} and sends
    // the room's voice snapshot. Voice rooms are not ported yet, so the call
    // is left running; this is the one place to add that.
    announce(&state, &[me, other]);
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn unblock(
    State(state): State<AppState>,
    caller: Caller,
    Path(GuidSegment(other)): Path<GuidSegment>,
) -> ApiResult<Response> {
    let me = caller.user;
    let exists: bool = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok(db.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"UserBlocks\" WHERE \"BlockerId\" = ?1 AND \"BlockedId\" = ?2)",
                params![me, other],
                |row| row.get(0),
            )?)
        })
        .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            tx.execute(
                "DELETE FROM \"UserBlocks\" WHERE \"BlockerId\" = ?1 AND \"BlockedId\" = ?2",
                params![me, other],
            )?;
            Ok(())
        })
        .await?;
    announce(&state, &[me, other]);
    Ok(StatusCode::NO_CONTENT.into_response())
}
