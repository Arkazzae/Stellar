//! Signing in with a passkey alone: the passkey names the account and, with
//! user verification required, proves both the device and the person holding
//! it, so the session counts as having passed a second step.

use std::time::Duration;

use axum::extract::State;
use axum::response::Response;
use serde::{Deserialize, Serialize};

use super::common::{Seen, body_of, no_store, ok_json, passkey_rejected, passkeys_unavailable, with_cookies};
use super::wire;
use crate::auth::ceremony::{self, PasskeySignedIn};
use crate::auth::tokens;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, with_cookie};
use crate::instance::users::User;
use crate::state::AppState;

#[derive(Serialize)]
struct Available {
    available: bool,
}

/// Whether the instance can offer passkeys at all.
pub async fn available(State(state): State<AppState>) -> ApiResult<Response> {
    Ok(ok_json(&Available {
        available: super::common::relying_party(&state).is_some(),
    }))
}

/// The options of a sign-in with no password. They name no passkey: the
/// browser offers the ones it holds for this site.
pub async fn options(State(state): State<AppState>, origin: Origin) -> ApiResult<Response> {
    let seen = Seen::of(&state, origin);
    let Some(relying_party) = &seen.relying_party else {
        return passkeys_unavailable(&state);
    };
    let options = ceremony::passkey_sign_in_options(&state, relying_party, seen.origin.https).await?;
    Ok(with_cookie(
        super::passkey::options_answer(options.json),
        &options.cookie,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInRequest {
    response: wire::Raw,
    keep_signed_in: Option<bool>,
}

/// Signs in with the browser's answer to the options.
pub async fn sign_in(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<SignInRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let Ok(response) = wire::assertion(request.response.as_deref()) else {
        return Err(ApiError::bad_request());
    };
    let seen = Seen::of(&state, origin);
    let Some(relying_party) = &seen.relying_party else {
        return passkeys_unavailable(&state);
    };
    let cookie = crate::auth::security_cookies::read_passkey(&seen.origin.headers, seen.origin.https);
    let context = seen.context();
    // Left out, the session is kept; only an answer that says no is short.
    let keep_signed_in = request.keep_signed_in != Some(false);
    let signed = ceremony::passkey_sign_in(
        &state,
        &context,
        relying_party,
        cookie.as_ref(),
        response.as_ref(),
        keep_signed_in,
    )
    .await?;
    match signed {
        PasskeySignedIn::SignedIn(completed) => {
            let id = completed.user;
            let user = state
                .database()
                .read(move |connection| -> Result<Option<User>, DbError> {
                    Ok(User::find(connection, id)?)
                })
                .await?
                .ok_or_else(|| ApiError::internal("the account of a finished sign-in is gone"))?;
            Ok(with_cookies(ok_json(&user.dto("online")), &completed.cookies, &[]))
        }
        PasskeySignedIn::Rejected { cookie } => {
            // The same answer for every failure, slowed a little like the
            // password's.
            tokio::time::sleep(Duration::from_millis(
                u64::from(tokens::random_below(100)) + 80,
            ))
            .await;
            Ok(with_cookie(no_store(passkey_rejected(&state)?), &cookie))
        }
    }
}
