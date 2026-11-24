//! The password: the first one of an account made through a provider, and a
//! change of the one an account has.
//!
//! Checking a password and making the hash of a new one each take time, so
//! both are done before the transaction that writes, on a thread that may
//! block; the transaction then relies on the check only if the stored hash is
//! still the one it was made against.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::{fresh_second_step_refusal, no_store};
use crate::auth::{Caller, events, fresh, password, recovery};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem, json_response, with_session_cookie};
use crate::state::AppState;
use crate::values::Timestamp;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRequest {
    new_password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequest {
    current_password: Option<String>,
    new_password: Option<String>,
}

#[derive(Serialize)]
struct Outcome {
    result: &'static str,
}

fn outcome(result: &'static str) -> Response {
    json_response(StatusCode::OK, &Outcome { result })
}

/// The stored hash of the account: none when there is no such account, and
/// none inside when the account has no password.
fn stored_hash(connection: &Connection, user: crate::values::Id) -> rusqlite::Result<Option<Option<String>>> {
    connection
        .query_row("SELECT \"PasswordHash\" FROM \"Users\" WHERE \"Id\" = ?1", [user], |row| row.get(0))
        .optional()
}

async fn hashed(new: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || password::hash(&new)).await.map_err(ApiError::internal)
}

// ---- the first password ----

pub async fn set(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    request: Result<Option<Json<SetRequest>>, ApiError>,
) -> Response {
    let request = match super::methods::required(request) {
        Ok(request) => request,
        Err(refusal) => return *refusal,
    };
    no_store(first(&state, caller, &origin, request.new_password).await)
}

/// What asking for a first password came to, as far as the account goes.
enum FirstCheck {
    Missing,
    AlreadySet,
    ProofNeeded,
    Allowed,
}

/// An account without a password has only the proof the session itself gives:
/// a second step passed lately if it has one, a sign-in lately if it has not.
fn check_first(connection: &Connection, caller: Caller) -> Result<FirstCheck, DbError> {
    Ok(match stored_hash(connection, caller.user)? {
        None => FirstCheck::Missing,
        Some(Some(_)) => FirstCheck::AlreadySet,
        Some(None) => {
            if fresh::can_change_security(connection, caller.user, Some(caller.session), || false, Timestamp::now())? {
                FirstCheck::Allowed
            } else {
                FirstCheck::ProofNeeded
            }
        }
    })
}

fn refuse_first(state: &AppState, check: FirstCheck) -> ApiResult<()> {
    match check {
        FirstCheck::Allowed => Ok(()),
        FirstCheck::Missing => Err(ApiError::not_found()),
        FirstCheck::AlreadySet => {
            let title = state.messages().get(MessageKey::SecurityPasswordAlreadySet, &[])?;
            Err(Problem::new(StatusCode::CONFLICT).title(title).into())
        }
        FirstCheck::ProofNeeded => Err(fresh_second_step_refusal(state)?),
    }
}

/// The text for a new password the policy refuses, if it does.
fn violation(state: &AppState, new: Option<&str>) -> ApiResult<Option<String>> {
    let messages = state.messages();
    Ok(if password::is_too_short(new) {
        Some(messages.get(MessageKey::PasswordTooShort, &[("min", &password::MIN_LENGTH)])?)
    } else if password::is_too_long(new) {
        Some(messages.get(MessageKey::PasswordTooLong, &[("max", &password::MAX_LENGTH)])?)
    } else {
        None
    })
}

async fn first(state: &AppState, caller: Caller, origin: &Origin, new: Option<String>) -> ApiResult<Response> {
    if let Some(text) = violation(state, new.as_deref())? {
        return Err(Problem::validation(vec![("newPassword".into(), vec![text])]).into());
    }
    let new = new.unwrap_or_default();
    // A look first, so that a refusal costs no hashing.
    let seen = state.database().read(move |connection| check_first(connection, caller)).await?;
    refuse_first(state, seen)?;
    let hash = hashed(new).await?;
    let origin = origin.clone();
    let checked = state
        .database()
        .write(move |tx| -> Result<FirstCheck, DbError> {
            let check = check_first(tx, caller)?;
            if !matches!(check, FirstCheck::Allowed) {
                return Ok(check);
            }
            tx.execute("UPDATE \"Users\" SET \"PasswordHash\" = ?2 WHERE \"Id\" = ?1", params![caller.user, hash])?;
            let alert = events::verified_email(tx, caller.user)?;
            events::record(
                tx,
                caller.user,
                events::PASSWORD_CHANGED,
                &origin.address,
                &origin.user_agent,
                "password set",
                alert.as_deref(),
            )?;
            Ok(FirstCheck::Allowed)
        })
        .await?;
    refuse_first(state, checked)?;
    // Nothing about the sessions already open was in doubt, so they stay.
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- changing it ----

pub async fn change(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Json(request): Json<ChangeRequest>,
) -> ApiResult<Response> {
    if password::is_too_short(request.new_password.as_deref()) {
        return Ok(outcome("too-short"));
    }
    if password::is_too_long(request.new_password.as_deref()) {
        let text = state.messages().get(MessageKey::NewPasswordTooLong, &[("max", &password::MAX_LENGTH)])?;
        return Err(Problem::validation(vec![("newPassword".into(), vec![text])]).into());
    }
    let new = request.new_password.unwrap_or_default();

    // Asked before the password is looked at, so that a stolen session cannot
    // be used to try passwords on an account that has a second step.
    let (is_fresh, stored) = state
        .database()
        .read(move |connection| -> Result<(bool, Option<Option<String>>), DbError> {
            let fresh = fresh::has_fresh_second_step(connection, caller.user, Some(caller.session), Timestamp::now())?;
            Ok((fresh, stored_hash(connection, caller.user)?))
        })
        .await?;
    let stored = stored.ok_or_else(|| ApiError::internal("the signed-in account is not there"))?;
    if !is_fresh {
        return Ok(outcome("second-step-required"));
    }
    let Some(current) = request.current_password else {
        return Ok(outcome("wrong-current"));
    };
    let (candidate, against) = (current.clone(), stored.clone());
    let proves = tokio::task::spawn_blocking(move || password::verify(&candidate, against.as_deref()))
        .await
        .map_err(ApiError::internal)?;
    if !proves {
        return Ok(outcome("wrong-current"));
    }
    if current == new {
        return Ok(outcome("same-as-current"));
    }
    let hash = hashed(new).await?;

    let (write_state, origin) = (state.clone(), origin.clone());
    let done = state
        .database()
        .write(move |tx| -> Result<Result<String, &'static str>, DbError> {
            let now = Timestamp::now();
            if !fresh::has_fresh_second_step(tx, caller.user, Some(caller.session), now)? {
                return Ok(Err("second-step-required"));
            }
            // Somebody changed it between the check and now: what was given is
            // no longer the current password.
            if stored_hash(tx, caller.user)? != Some(stored) {
                return Ok(Err("wrong-current"));
            }
            tx.execute("UPDATE \"Users\" SET \"PasswordHash\" = ?2 WHERE \"Id\" = ?1", params![caller.user, hash])?;
            // A new version ends every session but this one, every remembered
            // browser and every approved handoff that was not used.
            let version = recovery::credentials_changed(tx, caller.user, Some(caller.session), now)?;
            let short_lived: bool = tx.query_row(
                "SELECT \"ShortLived\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
                [caller.session],
                |row| row.get(0),
            )?;
            let alert = events::verified_email(tx, caller.user)?;
            events::record(
                tx,
                caller.user,
                events::PASSWORD_CHANGED,
                &origin.address,
                &origin.user_agent,
                "password changed",
                alert.as_deref(),
            )?;
            // This request's own cookie carries the old version.
            Ok(Ok(crate::auth::session::sign_in_cookie(
                write_state.sealer(),
                &write_state.settings().environment,
                origin.https,
                caller.user,
                version,
                caller.session,
                short_lived,
            )))
        })
        .await?;
    Ok(match done {
        Ok(cookie) => with_session_cookie(outcome("changed"), &cookie),
        Err(result) => outcome(result),
    })
}
