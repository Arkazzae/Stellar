//! A passkey or security key as the second step of a sign-in.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;

use super::common::{
    Seen, body_of, challenge_id, challenge_problem, method_unavailable, no_store, passkeys_unavailable,
    wrong_code,
};
use super::link::{self, Waiting};
use super::mfa::finished;
use super::wire;
use crate::auth::ceremony::{self, Answer, Answered, PasskeyOptions};
use crate::http::{ApiError, ApiResult, Json, Origin};
use crate::state::AppState;

/// What the browser is handed to sign with: the text as it was made, as
/// `application/json` with no character set.
pub(super) fn options_answer(json: String) -> Response {
    let mut response = Response::new(Body::from(json));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    no_store(response)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsRequest {
    challenge_id: Option<String>,
}

/// The options a passkey answers the challenge's second step with.
pub async fn options(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<OptionsRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    match ceremony::passkey_options(&state, &seen.ask(&context, challenge), seen.relying_party.as_ref()).await? {
        PasskeyOptions::Options(json) => Ok(options_answer(json)),
        PasskeyOptions::Unavailable => method_unavailable(&state),
        PasskeyOptions::PasskeysUnavailable => passkeys_unavailable(&state),
        PasskeyOptions::Invalid(status) => challenge_problem(&state, status),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest {
    challenge_id: Option<String>,
    response: Option<Value>,
    #[serde(default)]
    remember_device: bool,
}

/// Answers the challenge with a passkey.
pub async fn verify(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<VerifyRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let Ok(response) = wire::assertion(request.response.as_ref()) else {
        return Err(ApiError::bad_request());
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let waiting = Waiting::of(&state, &seen.origin);
    let answered = ceremony::answer(
        &state,
        &seen.ask(&context, challenge),
        match &seen.relying_party {
            Some(relying_party) => Answer::Passkey {
                relying_party,
                response: response.as_ref(),
            },
            None => Answer::PasskeysUnavailable,
        },
        request.remember_device,
        move |tx, completed| link::pending(tx, completed, &waiting),
    )
    .await?;
    match answered {
        Answered::SignedIn { completed, hook } => finished(&state, &seen, *completed, hook).await,
        Answered::Refused => wrong_code(&state),
        Answered::TooManyFailures => super::common::too_many_failures(&state),
        Answered::MethodUnavailable => method_unavailable(&state),
        Answered::EmptyCode => super::common::empty_code(&state),
        Answered::PasskeysUnavailable => passkeys_unavailable(&state),
        Answered::Invalid(status) => challenge_problem(&state, status),
    }
}
