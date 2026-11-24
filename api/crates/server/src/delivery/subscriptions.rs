//! The three routes under `/api/me` that belong to delivery: what the
//! instance offers, adding a browser and removing it.
//!
//! These routes belong to the person, not to the server they happen to be
//! looking at, so none of them asks for a membership.

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_webpush::{ValidatedSubscription, normalize_endpoint};

use crate::auth::{Caller, events};
use crate::database::DbError;
use crate::http::{
    ApiError, ApiResult, DEFAULT_JSON_BODY_BYTES, Problem, json_response, read_all,
};
use crate::mail::setup::{self, MailSetup};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// A person's browsers beyond this many are refused.
const MAX_PER_PERSON: i64 = 10;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    email_available: bool,
    email_verified: bool,
    push_available: bool,
    application_server_key: Option<String>,
}

#[derive(Serialize)]
struct Saved {
    id: Id,
}

#[derive(Deserialize)]
struct Keys {
    p256dh: Option<String>,
    auth: Option<String>,
}

#[derive(Deserialize)]
struct SaveBody {
    endpoint: Option<String>,
    keys: Option<Keys>,
}

#[derive(Deserialize)]
struct RemoveBody {
    endpoint: Option<String>,
}

/// The refusal of a subscription that is not valid. Its text is the first
/// server's, in English whatever the instance's language is.
fn invalid() -> ApiError {
    Problem::validation(vec![(
        "subscription".to_string(),
        vec!["The push subscription endpoint or its keys are not valid.".to_string()],
    )])
    .into()
}

/// A body that is missing or `null` is "no request", which is a refusal of the
/// subscription like any other; one that is not JSON of the right shape is a
/// bare 400.
async fn read_body<T: serde::de::DeserializeOwned>(request: Request) -> Result<Option<T>, ApiError> {
    let (parts, body) = request.into_parts();
    let bytes = read_all(&parts.headers, body, DEFAULT_JSON_BODY_BYTES).await?;
    if bytes.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice::<Option<T>>(&bytes).map_err(|_| ApiError::bad_request())
}

/// Whether the session row still exists, belongs to the account and has not
/// ended. Authentication has checked the cookie; this is the row itself.
fn session_is_active(
    db: &Connection,
    user: Id,
    session: Id,
    now: Timestamp,
) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"UserSessions\"
             WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?3
               AND (\"AbsoluteExpiresAt\" IS NULL OR \"AbsoluteExpiresAt\" > ?3))",
        params![session, user, now],
        |row| row.get(0),
    )
}

pub async fn capabilities(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let reading = state.clone();
    let found = state
        .database()
        .read(move |db| -> Result<Option<(bool, bool)>, DbError> {
            if !session_is_active(db, caller.user, caller.session, Timestamp::now())? {
                return Ok(None);
            }
            let mail = matches!(
                setup::find(db, reading.settings(), reading.sealer())?,
                MailSetup::Ready(_)
            );
            let verified = events::verified_email(db, caller.user)?.is_some();
            Ok(Some((mail, verified)))
        })
        .await?;
    let Some((email_available, email_verified)) = found else {
        return Err(ApiError::unauthorized());
    };
    let vapid = super::vapid::keys(&state.settings().push);
    let mut response = json_response(
        StatusCode::OK,
        &Capabilities {
            email_available,
            email_verified,
            push_available: vapid.is_some(),
            application_server_key: vapid.map(|keys| keys.public_key()),
        },
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

enum Outcome {
    SignedOut,
    Invalid,
    Conflict,
    Saved(Id),
}

pub async fn save(State(state): State<AppState>, caller: Caller, request: Request) -> ApiResult<Response> {
    let body: Option<SaveBody> = read_body(request).await?;
    let keys = body.as_ref().and_then(|body| body.keys.as_ref());
    let subscription = ValidatedSubscription::validate(
        body.as_ref().and_then(|body| body.endpoint.as_deref()),
        keys.and_then(|keys| keys.p256dh.as_deref()),
        keys.and_then(|keys| keys.auth.as_deref()),
    );
    let outcome = state
        .database()
        .write(move |tx| -> Result<Outcome, DbError> {
            let now = Timestamp::now();
            if !session_is_active(tx, caller.user, caller.session, now)? {
                return Ok(Outcome::SignedOut);
            }
            // An ended sign-in is told so before the subscription is judged.
            let Some(subscription) = subscription else {
                return Ok(Outcome::Invalid);
            };
            // The whole of it is one transaction that holds the write lock, so
            // two requests for one endpoint cannot both find it free.
            let existing: Option<(Id, Id, Id)> = tx
                .query_row(
                    "SELECT \"Id\", \"UserId\", \"SessionId\" FROM \"PushSubscriptions\" WHERE \"Endpoint\" = ?1",
                    [subscription.endpoint()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((id, owner, session)) = existing {
                // An endpoint belongs to the account that created it. It may
                // move to a newer sign-in of that account only once the old
                // one has ended: a browser that signs in again after a
                // revocation still receives its own notifications.
                if owner != caller.user {
                    return Ok(Outcome::Conflict);
                }
                if session != caller.session {
                    if session_is_active(tx, owner, session, now)? {
                        return Ok(Outcome::Conflict);
                    }
                    tx.execute(
                        "UPDATE \"PushSubscriptions\" SET \"SessionId\" = ?1 WHERE \"Id\" = ?2",
                        params![caller.session, id],
                    )?;
                }
                tx.execute(
                    "UPDATE \"PushSubscriptions\" SET \"P256dh\" = ?1, \"Auth\" = ?2 WHERE \"Id\" = ?3",
                    params![subscription.p256dh_text(), subscription.auth_text(), id],
                )?;
                return Ok(Outcome::Saved(id));
            }
            let owned: i64 = tx.query_row(
                "SELECT COUNT(*) FROM \"PushSubscriptions\" WHERE \"UserId\" = ?1",
                [caller.user],
                |row| row.get(0),
            )?;
            if owned >= MAX_PER_PERSON {
                return Ok(Outcome::Conflict);
            }
            let id = Id::random();
            tx.execute(
                "INSERT INTO \"PushSubscriptions\" (\"Id\", \"UserId\", \"SessionId\", \"Endpoint\", \"P256dh\", \"Auth\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    caller.user,
                    caller.session,
                    subscription.endpoint(),
                    subscription.p256dh_text(),
                    subscription.auth_text(),
                    now
                ],
            )?;
            Ok(Outcome::Saved(id))
        })
        .await?;
    match outcome {
        Outcome::SignedOut => Err(ApiError::unauthorized()),
        Outcome::Invalid => Err(invalid()),
        Outcome::Conflict => Err(ApiError::conflict()),
        Outcome::Saved(id) => Ok(json_response(StatusCode::OK, &Saved { id })),
    }
}

pub async fn remove(State(state): State<AppState>, caller: Caller, request: Request) -> ApiResult<Response> {
    let body: Option<RemoveBody> = read_body(request).await?;
    let endpoint = body
        .and_then(|body| body.endpoint)
        .and_then(|endpoint| normalize_endpoint(&endpoint));
    let signed_in = state
        .database()
        .write(move |tx| -> Result<Option<bool>, DbError> {
            if !session_is_active(tx, caller.user, caller.session, Timestamp::now())? {
                return Ok(None);
            }
            let Some(endpoint) = endpoint else {
                return Ok(Some(false));
            };
            tx.execute(
                "DELETE FROM \"PushSubscriptions\" WHERE \"Endpoint\" = ?1 AND \"UserId\" = ?2 AND \"SessionId\" = ?3",
                params![endpoint, caller.user, caller.session],
            )?;
            Ok(Some(true))
        })
        .await?;
    // "Removed" and "not yours" are one answer: the second tells nobody which
    // account owns the endpoint.
    match signed_in {
        None => Err(ApiError::unauthorized()),
        Some(false) => Err(invalid()),
        Some(true) => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
