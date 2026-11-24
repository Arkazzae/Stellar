//! Asking somebody to be friends, and answering: accepting, declining and
//! withdrawing.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::{announce, are_friends, is_blocked, pair, pair_key};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Path, Problem, json_response};
use crate::instance::names::to_upper;
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

/// The body of a request to befriend. Property names are read without regard
/// to case, and the last of a name given twice counts, as the first server
/// read them.
pub struct CreateRequest {
    username: Option<String>,
}

impl<'de> Deserialize<'de> for CreateRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<CreateRequest, D::Error> {
        struct Reader;
        impl<'de> Visitor<'de> for Reader {
            type Value = CreateRequest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<CreateRequest, A::Error> {
                let mut username = None;
                while let Some(key) = map.next_key::<String>()? {
                    if to_upper(&key) == "USERNAME" {
                        username = map.next_value::<Option<String>>()?;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(CreateRequest { username })
            }
        }
        deserializer
            .deserialize_map(Reader)
            .map_err(|e: D::Error| de::Error::custom(e))
    }
}

#[derive(Serialize)]
struct Answer {
    result: &'static str,
}

fn answer(result: &'static str) -> Response {
    json_response(StatusCode::OK, &Answer { result })
}

enum Outcome {
    Result(&'static str),
    Sent(Id, Id),
}

/// Always answers 200 with a result code, so that the form can say what
/// happened without telling a stranger which user names exist: an account
/// that takes no requests reads exactly like one that is not there.
///
/// Every check and the insert happen in one transaction, so a block set at the
/// same moment cannot slip a request in beside it.
pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Json(request): Json<CreateRequest>,
) -> ApiResult<Response> {
    let username = request
        .username
        .as_deref()
        .unwrap_or("")
        .trim()
        .trim_start_matches('@')
        .to_string();
    if username.is_empty() {
        let text = state.messages().get(MessageKey::FriendUsernameRequired, &[])?;
        return Err(Problem::validation(vec![("username".into(), vec![text])]).into());
    }
    let normalized = to_upper(&username);
    let me = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, DbError> {
            let target: Option<(Id, bool)> = tx
                .query_row(
                    "SELECT u.\"Id\", COALESCE(p.\"AllowFriendRequests\", 1) FROM \"Users\" u
                     LEFT JOIN \"UserPreferences\" p ON p.\"UserId\" = u.\"Id\"
                     WHERE u.\"NormalizedUsername\" = ?1 AND u.\"Origin\" IS NULL AND u.\"IsDisabled\" = 0",
                    [&normalized],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((target, _)) = target.filter(|(_, allows)| *allows) else {
                return Ok(Outcome::Result("unknown-user"));
            };
            if target == me {
                return Ok(Outcome::Result("self"));
            }
            if is_blocked(tx, me, target)? {
                return Ok(Outcome::Result("blocked"));
            }
            if are_friends(tx, me, target)? {
                return Ok(Outcome::Result("already-friends"));
            }
            let pending: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"FriendRequests\"
                                WHERE (\"SenderId\" = ?1 AND \"ReceiverId\" = ?2)
                                   OR (\"SenderId\" = ?2 AND \"ReceiverId\" = ?1))",
                params![me, target],
                |row| row.get(0),
            )?;
            if pending {
                return Ok(Outcome::Result("already-sent"));
            }
            tx.execute(
                "INSERT INTO \"FriendRequests\" (\"Id\", \"SenderId\", \"ReceiverId\", \"PairKey\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![Id::random(), me, target, pair_key(me, target), Timestamp::now()],
            )?;
            Ok(Outcome::Sent(me, target))
        })
        .await?;
    match outcome {
        Outcome::Result(result) => Ok(answer(result)),
        Outcome::Sent(from, to) => {
            announce(&state, &[from, to]);
            Ok(answer("sent"))
        }
    }
}

/// Accepting makes the friendship and ends the request together, or does
/// neither: a request answered at the same moment by the other side leaves
/// nothing to do, and the answer is still 204.
pub async fn accept(
    State(state): State<AppState>,
    caller: Caller,
    Path(Guid(request)): Path<Guid>,
) -> ApiResult<Response> {
    let me = caller.user;
    let found: Option<(Id, Id)> = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok(db
                .query_row(
                    "SELECT \"SenderId\", \"ReceiverId\" FROM \"FriendRequests\" WHERE \"Id\" = ?1 AND \"ReceiverId\" = ?2",
                    params![request, me],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?)
        })
        .await?;
    let Some((sender, receiver)) = found else {
        return Err(ApiError::not_found());
    };
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            let removed = tx.execute("DELETE FROM \"FriendRequests\" WHERE \"Id\" = ?1", [request])?;
            if removed == 1 && !are_friends(tx, sender, receiver)? {
                let (a, b) = pair(sender, receiver);
                tx.execute(
                    "INSERT INTO \"Friendships\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
                    params![Id::random(), a, b, Timestamp::now()],
                )?;
            }
            Ok(())
        })
        .await?;
    announce(&state, &[sender, receiver]);
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Declines a request the person received, or withdraws one they sent.
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path(Guid(request)): Path<Guid>,
) -> ApiResult<Response> {
    let me = caller.user;
    let found: Option<(Id, Id)> = state
        .database()
        .read(move |db| -> Result<_, DbError> {
            Ok(db
                .query_row(
                    "SELECT \"SenderId\", \"ReceiverId\" FROM \"FriendRequests\"
                     WHERE \"Id\" = ?1 AND (\"SenderId\" = ?2 OR \"ReceiverId\" = ?2)",
                    params![request, me],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?)
        })
        .await?;
    let Some((sender, receiver)) = found else {
        return Err(ApiError::not_found());
    };
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            tx.execute("DELETE FROM \"FriendRequests\" WHERE \"Id\" = ?1", [request])?;
            Ok(())
        })
        .await?;
    announce(&state, &[sender, receiver]);
    Ok(StatusCode::NO_CONTENT.into_response())
}
