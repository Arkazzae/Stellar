//! Setting a second step up when the instance asks for one: an authenticator
//! app, the address of the account, or a passkey. Each ends the sign-in it
//! was set up for, with the recovery codes of an account that had none.

use axum::extract::State;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stellarr_messages::MessageKey;

use super::common::{
    Seen, body_of, challenge_id, challenge_problem, invalid_name, method_unavailable, ok_json,
    passkey_rejected, passkeys_unavailable, text, too_many_failures, wrong_code,
};
use super::link::{self, Waiting};
use super::mfa::{Purposes, finished, unavailable_passkeys, unusable_for};
use super::wire;
use crate::auth::ceremony::{
    self, Enrolled, PasskeyStarted, TotpConfirmation, TotpStarted,
};
use crate::auth::mfa;
use crate::http::{ApiError, ApiResult, Json, Origin};
use crate::state::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpStartRequest {
    challenge_id: Option<String>,
    name: Option<String>,
}

/// What a person is given to set an authenticator app up.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Enrollment {
    method_id: String,
    r#type: &'static str,
    secret: String,
    otp_auth_uri: String,
}

/// Starts adding an authenticator app: its secret is shown once.
pub async fn start_totp(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<TotpStartRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let started =
        ceremony::start_totp(&state, &seen.ask(&context, &challenge), request.name.as_deref())
            .await?;
    match started {
        TotpStarted::Started { method, secret, uri } => Ok(ok_json(&Enrollment {
            method_id: method.to_string(),
            r#type: "totp",
            secret,
            otp_auth_uri: uri,
        })),
        TotpStarted::InvalidName => invalid_name(&state),
        TotpStarted::Invalid(status) => challenge_problem(&state, status),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpConfirmRequest {
    challenge_id: Option<String>,
    method_id: Option<String>,
    code: Option<String>,
    name: Option<String>,
    #[serde(default)]
    remember_device: bool,
}

/// Confirms the authenticator app with its first code.
pub async fn confirm_totp(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<TotpConfirmRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let waiting = Waiting::of(&seen.origin);
    let enrolled = ceremony::confirm_totp(
        &state,
        &seen.ask(&context, &challenge),
        TotpConfirmation {
            method: request.method_id.as_deref().and_then(mfa::parse_method_id),
            name: request.name.as_deref(),
            code: request.code.as_deref().unwrap_or(""),
        },
        request.remember_device,
        move |tx, completed| link::pending(tx, completed, &waiting),
    )
    .await?;
    enrolled_answer(&state, &seen, enrolled, Rejection::Code).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailConfirmRequest {
    challenge_id: Option<String>,
    code: Option<String>,
    name: Option<String>,
    #[serde(default)]
    remember_device: bool,
}

/// Adds the account's address as a second step with the code that was mailed
/// to it.
pub async fn confirm_email(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<EmailConfirmRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let waiting = Waiting::of(&seen.origin);
    let enrolled = ceremony::confirm_email(
        &state,
        &seen.ask(&context, &challenge),
        request.name.as_deref(),
        request.code.as_deref().unwrap_or(""),
        request.remember_device,
        move |tx, completed| link::pending(tx, completed, &waiting),
    )
    .await?;
    enrolled_answer(&state, &seen, enrolled, Rejection::Code).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyStartRequest {
    challenge_id: Option<String>,
}

/// The options for adding a passkey, which the browser signs.
pub async fn start_passkey(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<PasskeyStartRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let Some(relying_party) = &seen.relying_party else {
        return unavailable_passkeys(&state, &seen, &challenge, Purposes::Enrolment).await;
    };
    match ceremony::start_passkey(&state, &seen.ask(&context, &challenge), relying_party).await? {
        PasskeyStarted::Options(json) => Ok(super::passkey::options_answer(json)),
        PasskeyStarted::Invalid(status) => challenge_problem(&state, status),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyConfirmRequest {
    challenge_id: Option<String>,
    name: Option<String>,
    response: Option<Value>,
    #[serde(default)]
    remember_device: bool,
}

/// Adds a passkey with the browser's answer to the options.
pub async fn confirm_passkey(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<PasskeyConfirmRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let Ok(response) = wire::registration(request.response.as_ref()) else {
        return Err(ApiError::bad_request());
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let Some(relying_party) = &seen.relying_party else {
        // The challenge and then the name are looked at before the
        // instance's passkeys are.
        if let Some(refusal) =
            unusable_for(&state, &seen.ask(&context, &challenge), Purposes::Enrolment).await?
        {
            return Ok(refusal);
        }
        let fallback = text(&state, MessageKey::SecurityDefaultPasskeyName)?;
        if mfa::method_name(request.name.as_deref(), &fallback).is_none() {
            return invalid_name(&state);
        }
        return passkeys_unavailable(&state);
    };
    let waiting = Waiting::of(&seen.origin);
    let enrolled = ceremony::confirm_passkey(
        &state,
        &seen.ask(&context, &challenge),
        relying_party,
        request.name.as_deref(),
        response.as_ref(),
        request.remember_device,
        move |tx, completed| link::pending(tx, completed, &waiting),
    )
    .await?;
    enrolled_answer(&state, &seen, enrolled, Rejection::Passkey).await
}

/// Which sentence a wrong proof is refused with.
#[derive(Clone, Copy)]
enum Rejection {
    Code,
    Passkey,
}

async fn enrolled_answer(
    state: &AppState,
    seen: &Seen,
    enrolled: Enrolled<Option<&'static str>>,
    rejection: Rejection,
) -> ApiResult<Response> {
    match enrolled {
        Enrolled::Done { completed, hook } => finished(state, seen, *completed, hook).await,
        Enrolled::Refused => match rejection {
            Rejection::Code => wrong_code(state),
            Rejection::Passkey => passkey_rejected(state),
        },
        Enrolled::TooManyFailures => too_many_failures(state),
        Enrolled::Unavailable => method_unavailable(state),
        Enrolled::InvalidName => invalid_name(state),
        Enrolled::Invalid(status) => challenge_problem(state, status),
    }
}
