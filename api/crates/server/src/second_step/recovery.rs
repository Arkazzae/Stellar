//! Exchanging the credential an operator hands over after recovering an
//! account for the challenge that lets the person set a new second step up.
//! The result is access to set one up, never a session.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::common::{Seen, body_of, ok_json, problem};
use crate::auth::ceremony::{self, Redeemed};
use crate::http::{ApiError, ApiResult, Json, Origin, with_cookie};
use crate::state::AppState;

/// A credential is made of at least this many characters and at most this
/// many, counted in UTF-16 units.
const LENGTH: std::ops::RangeInclusive<usize> = 16..=512;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedeemRequest {
    credential: Option<String>,
    keep_signed_in: Option<bool>,
}

/// Redeems the credential the operator's command printed.
pub async fn redeem(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<RedeemRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let credential = request.credential.as_deref().unwrap_or("").trim();
    if !LENGTH.contains(&credential.encode_utf16().count()) {
        return invalid(&state);
    }
    let seen = Seen::of(&state, origin);
    let context = seen.context();
    // Left out, the session that follows is kept; only an answer that says no
    // is short.
    let keep_signed_in = request.keep_signed_in != Some(false);
    match ceremony::redeem_recovery_credential(&state, &context, credential, keep_signed_in).await? {
        Redeemed::Challenge { body, cookie } => Ok(with_cookie(ok_json(&*body), &cookie)),
        Redeemed::Invalid => invalid(&state),
    }
}

fn invalid(state: &AppState) -> ApiResult<Response> {
    problem(
        state,
        StatusCode::BAD_REQUEST,
        MessageKey::SecurityRecoveryCredentialInvalid,
    )
}
