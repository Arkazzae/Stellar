//! What every route of the second step does the same way: reading who is
//! asking, turning an outcome of the ceremony into the answer the web client
//! knows, and keeping an answer out of caches.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use stellarr_messages::MessageKey;
use stellarr_webauthn::RelyingParty;

use crate::auth::ceremony::{Ask, Completed};
use crate::auth::flow::{self, ChallengeStatus, Context};
use crate::auth::security_cookies;
use crate::database::DbError;
use crate::http::{
    ApiError, ApiResult, Json, Origin, Problem, json_response, with_cookie, with_session_cookie,
};
use crate::instance::users::{User, UserDto};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The relying party of the instance, which is also the whole of the decision
/// whether passkeys are offered: the one place it is made, so that what a
/// challenge says about passkeys and what the passkey routes do agree.
pub(super) fn relying_party(state: &AppState) -> Option<RelyingParty> {
    RelyingParty::from_public_url(&state.settings().operations.public_url)
}

/// Who is asking, in a form that outlives the borrow of the request.
pub(super) struct Seen {
    pub origin: Origin,
    trusted_device: Option<String>,
    /// The id and secret in the browser's challenge cookie.
    pub preauth: Option<(Id, String)>,
    pub relying_party: Option<RelyingParty>,
}

impl Seen {
    pub fn of(state: &AppState, origin: Origin) -> Seen {
        Seen {
            trusted_device: security_cookies::read_trusted_device(&origin.headers, origin.https),
            preauth: flow::read_preauth(&origin.headers, origin.https),
            relying_party: relying_party(state),
            origin,
        }
    }

    pub fn context(&self) -> Context<'_> {
        Context {
            ip_address: &self.origin.address,
            user_agent: &self.origin.user_agent,
            https: self.origin.https,
            trusted_device_cookie: self.trusted_device.as_deref(),
            passkeys_available: self.relying_party.is_some(),
        }
    }

    pub fn ask<'a>(&'a self, context: &'a Context<'a>, challenge_id: &'a str) -> Ask<'a> {
        Ask {
            context,
            preauth: self.preauth.as_ref(),
            challenge_id,
        }
    }
}

/// The challenge id a client wrote, as the ceremony reads it (it knows every
/// spelling of an id); a body without one names no challenge.
pub(super) fn challenge_id(sent: Option<&str>) -> &str {
    sent.unwrap_or("")
}

/// Keeps an answer out of every cache, as every route under `/api/auth/mfa`
/// and its neighbours does.
pub(super) fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

/// A JSON answer that is never stored.
pub(super) fn ok_json<T: Serialize>(value: &T) -> Response {
    no_store(json_response(StatusCode::OK, value))
}

/// The answer to a route that reads a body, when the body cannot be read: a
/// request that names no content type is turned away by the route with the
/// answer already marked as not to be stored.
pub(super) fn body_of<T>(
    request: Result<Json<T>, ApiError>,
    origin: &Origin,
    stored: bool,
) -> Result<T, Box<Response>> {
    match request {
        Ok(Json(value)) => Ok(value),
        Err(refusal) => {
            let response = refusal.into_response();
            Err(Box::new(
                if !stored || origin.headers.contains_key(header::CONTENT_TYPE) {
                    response
                } else {
                    no_store(response)
                },
            ))
        }
    }
}

pub(super) fn text(state: &AppState, key: MessageKey) -> ApiResult<String> {
    Ok(state.messages().get(key, &[])?)
}

/// A problem with the sentence of `key`.
pub(super) fn problem(state: &AppState, status: StatusCode, key: MessageKey) -> ApiResult<Response> {
    Ok(no_store(
        Problem::new(status).title(text(state, key)?).into_response(),
    ))
}

/// A challenge that cannot be used: out of attempts is a 429, anything else a
/// 401.
pub(super) fn challenge_problem(state: &AppState, status: ChallengeStatus) -> ApiResult<Response> {
    if status == ChallengeStatus::AttemptLimit {
        problem(
            state,
            StatusCode::TOO_MANY_REQUESTS,
            MessageKey::SecurityTooManyAttempts,
        )
    } else {
        problem(
            state,
            StatusCode::UNAUTHORIZED,
            MessageKey::SecurityChallengeInvalid,
        )
    }
}

pub(super) fn wrong_code(state: &AppState) -> ApiResult<Response> {
    problem(
        state,
        StatusCode::UNAUTHORIZED,
        MessageKey::SecurityCodeInvalid,
    )
}

pub(super) fn too_many_failures(state: &AppState) -> ApiResult<Response> {
    problem(
        state,
        StatusCode::TOO_MANY_REQUESTS,
        MessageKey::SecurityTooManyAttempts,
    )
}

pub(super) fn method_unavailable(state: &AppState) -> ApiResult<Response> {
    problem(
        state,
        StatusCode::CONFLICT,
        MessageKey::SecurityMethodUnavailable,
    )
}

pub(super) fn passkeys_unavailable(state: &AppState) -> ApiResult<Response> {
    problem(
        state,
        StatusCode::CONFLICT,
        MessageKey::SecurityPasskeyUnavailable,
    )
}

pub(super) fn passkey_rejected(state: &AppState) -> ApiResult<Response> {
    problem(
        state,
        StatusCode::UNAUTHORIZED,
        MessageKey::SecurityPasskeyRejected,
    )
}

/// The refusal of a code that was not sent: a validation problem on `code`.
pub(super) fn empty_code(state: &AppState) -> ApiResult<Response> {
    Ok(no_store(
        Problem::validation(vec![(
            "code".to_string(),
            vec![text(state, MessageKey::SecurityCodeInvalid)?],
        )])
        .into_response(),
    ))
}

/// The refusal of a name that is too long: a validation problem on `name`.
pub(super) fn invalid_name(state: &AppState) -> ApiResult<Response> {
    Ok(no_store(
        Problem::validation(vec![(
            "name".to_string(),
            vec![text(state, MessageKey::SecurityMethodNameInvalid)?],
        )])
        .into_response(),
    ))
}

/// What the client is told when a sign-in is finished.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignedIn {
    status: &'static str,
    user: UserDto,
    recovery_codes: Option<Vec<String>>,
    trusted_device_expires_at: Option<Timestamp>,
    identity_link: Option<&'static str>,
}

/// The answer to a sign-in that was completed: the account, the recovery codes
/// to show once, when a remembered device ends, and how linking an identity
/// came out, with the cookies the ceremony handed back in its order and then
/// the lines of the route's own.
pub(super) async fn signed_in(
    state: &AppState,
    completed: Completed,
    identity_link: Option<&'static str>,
    own_cookies: Vec<String>,
) -> ApiResult<Response> {
    let id = completed.user;
    let user = state
        .database()
        .read(move |connection| -> Result<Option<User>, DbError> { Ok(User::find(connection, id)?) })
        .await?
        .ok_or_else(|| ApiError::internal("the account of a finished sign-in is gone"))?;
    let body = SignedIn {
        status: "ok",
        user: user.dto("online"),
        recovery_codes: completed.recovery_codes,
        trusted_device_expires_at: completed.trusted_device_expires_at,
        identity_link,
    };
    Ok(with_cookies(
        ok_json(&body),
        &completed.cookies,
        &own_cookies,
    ))
}

/// Sets the cookies of a finished sign-in in the order given. The first is
/// the session's, which keeps the answer out of every cache in its own way.
pub(super) fn with_cookies(mut response: Response, ceremony: &[String], own: &[String]) -> Response {
    for (index, line) in ceremony.iter().chain(own).enumerate() {
        response = if index == 0 {
            with_session_cookie(response, line)
        } else {
            with_cookie(response, line)
        };
    }
    response
}
