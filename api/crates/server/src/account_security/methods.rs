//! The second step as the security page changes it: starting a set-up or a
//! second proof from the session, taking a method away, new recovery codes and
//! forgetting a remembered browser.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::{context, fresh_proof_needed, no_store, providers, trusted_device_of};
use crate::auth::ceremony::{self, Changed, FromSession, Removed, Started};
use crate::auth::{Caller, ways_in};
use crate::http::{ApiError, ApiResult, Json, Origin, Path, Problem, json_response, with_cookie, with_session_cookie};
use crate::servers::Object;
use crate::state::AppState;
use crate::values::{Guid, Id};

/// What a change of the second step may ask for besides the session's own
/// proof: the account's current password.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proof {
    password: Option<String>,
}

// Written by hand so that the password is never in what is printed.
impl std::fmt::Debug for Proof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proof").finish_non_exhaustive()
    }
}

/// A body that has to be there. One that is JSON but not the right JSON is
/// refused as the framework refuses it, before the page's headers are set; one
/// that is missing, or is `null`, is refused by the route itself, with them.
pub(super) fn required<T>(request: Result<Option<Json<Object<T>>>, ApiError>) -> Result<T, Box<Response>> {
    match request {
        Ok(Some(Json(Object(body)))) => Ok(body),
        Ok(None) => Err(Box::new(no_store(Err(ApiError::bad_request())))),
        Err(refusal) => Err(Box::new(refusal.into_response())),
    }
}

pub async fn start_enrollment(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    request: Result<Option<Json<Object<Proof>>>, ApiError>,
) -> Response {
    let password = match request {
        Ok(request) => request.and_then(|Json(Object(request))| request.password),
        Err(refusal) => return refusal.into_response(),
    };
    no_store(start(&state, caller, &origin, FromSession::Enrollment, password.as_deref()).await)
}

pub async fn start_reverification(State(state): State<AppState>, caller: Caller, origin: Origin) -> Response {
    no_store(start(&state, caller, &origin, FromSession::Reverification, None).await)
}

/// Hands a challenge over to the routes that answer it, with the cookie that
/// binds it to this browser.
async fn start(
    state: &AppState,
    caller: Caller,
    origin: &Origin,
    kind: FromSession,
    password: Option<&str>,
) -> ApiResult<Response> {
    let trusted = trusted_device_of(origin);
    let context = context(state, origin, trusted.as_deref());
    match ceremony::start_from_session(state, caller, &context, kind, password).await? {
        Started::Challenge { body, cookie } => Ok(with_cookie(json_response(StatusCode::OK, &body), &cookie)),
        Started::NotAllowed => Err(fresh_proof_needed(state)?),
        Started::NoMethods => Err(method_unavailable(state, StatusCode::CONFLICT)?),
    }
}

fn method_unavailable(state: &AppState, status: StatusCode) -> ApiResult<ApiError> {
    let title = state.messages().get(MessageKey::SecurityMethodUnavailable, &[])?;
    Ok(Problem::new(status).title(title).into())
}

pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Path(Guid(method)): Path<Guid>,
    request: Result<Option<Json<Object<Proof>>>, ApiError>,
) -> Response {
    let request = match required(request) {
        Ok(request) => request,
        Err(refusal) => return *refusal,
    };
    no_store(removal(&state, caller, &origin, method, request.password.as_deref()).await)
}

async fn removal(
    state: &AppState,
    caller: Caller,
    origin: &Origin,
    method: Id,
    password: Option<&str>,
) -> ApiResult<Response> {
    let trusted = trusted_device_of(origin);
    let context = context(state, origin, trusted.as_deref());
    let providers = providers::owned_of(&providers::configured(state));
    match ceremony::remove_method(state, caller, &context, method, &providers, password).await? {
        // The session that asked stays, on the account's new version.
        Removed::Done { cookie } => Ok(with_session_cookie(StatusCode::NO_CONTENT.into_response(), &cookie)),
        Removed::NotAllowed => Err(fresh_proof_needed(state)?),
        Removed::NotFound => {
            let title = state.messages().get(MessageKey::SecurityMethodNotFound, &[])?;
            Err(Problem::new(StatusCode::NOT_FOUND).title(title).into())
        }
        Removed::LastWayIn => Err(ways_in::passkey_refusal(state.messages())?.into()),
        Removed::RequiredByPolicy => Err(method_unavailable(state, StatusCode::CONFLICT)?),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryCodes {
    recovery_codes: Vec<String>,
}

pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    request: Result<Option<Json<Object<Proof>>>, ApiError>,
) -> Response {
    let request = match required(request) {
        Ok(request) => request,
        Err(refusal) => return *refusal,
    };
    no_store(regenerate(&state, caller, &origin, request.password.as_deref()).await)
}

async fn regenerate(state: &AppState, caller: Caller, origin: &Origin, password: Option<&str>) -> ApiResult<Response> {
    let trusted = trusted_device_of(origin);
    let context = context(state, origin, trusted.as_deref());
    match ceremony::regenerate_recovery_codes(state, caller, &context, password).await? {
        Changed::Done(recovery_codes) => Ok(json_response(StatusCode::OK, &RecoveryCodes { recovery_codes })),
        Changed::NotAllowed => Err(fresh_proof_needed(state)?),
        // With no second step there is nothing to give codes for.
        Changed::NotFound => Err(method_unavailable(state, StatusCode::CONFLICT)?),
    }
}

pub async fn forget_trusted_device(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Path(Guid(device)): Path<Guid>,
) -> Response {
    no_store(forget(&state, caller, &origin, device).await)
}

async fn forget(state: &AppState, caller: Caller, origin: &Origin, device: Id) -> ApiResult<Response> {
    let context = context(state, origin, None);
    if ceremony::forget_trusted_device(state, caller, &context, device).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ApiError::not_found())
    }
}
