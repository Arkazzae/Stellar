//! The second half of a sign-in that has a second step: the challenge a
//! person waits on, giving it up, answering it with a code, a remembered
//! browser or a recovery code, and asking for the code to be mailed.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::common::{
    Seen, body_of, challenge_id, challenge_problem, empty_code, method_unavailable, no_store,
    ok_json, passkeys_unavailable, problem, signed_in, too_many_failures, wrong_code,
};
use super::link::{self, Waiting};
use crate::auth::ceremony::{self, Answer, Answered, Pending};
use crate::auth::email_send::Sent;
use crate::auth::flow;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem, with_cookie};
use crate::state::AppState;

/// The challenge the browser holds, for the page that resumes it.
pub async fn pending(State(state): State<AppState>, origin: Origin) -> ApiResult<Response> {
    let seen = Seen::of(&state, origin);
    let Some((id, _)) = &seen.preauth else {
        return challenge_problem(&state, flow::ChallengeStatus::Missing);
    };
    let id = id.to_string();
    let context = seen.context();
    match ceremony::pending(&state, &seen.ask(&context, &id)).await? {
        Pending::Challenge(body) => Ok(ok_json(&*body)),
        Pending::Invalid(status) => {
            let response = challenge_problem(&state, status)?;
            Ok(with_cookie(response, &flow::clear_preauth_cookie(seen.origin.https)))
        }
    }
}

/// Gives the challenge up, whoever holds it, and takes its cookie away.
pub async fn cancel(State(state): State<AppState>, origin: Origin) -> ApiResult<Response> {
    let seen = Seen::of(&state, origin);
    if let Some((id, _)) = &seen.preauth {
        let id = id.to_string();
        let context = seen.context();
        ceremony::cancel(&state, &seen.ask(&context, &id)).await?;
    }
    let response = no_store(StatusCode::NO_CONTENT.into_response());
    Ok(with_cookie(response, &flow::clear_preauth_cookie(seen.origin.https)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest {
    challenge_id: Option<String>,
    method_id: Option<String>,
    code: Option<String>,
    #[serde(default)]
    remember_device: bool,
    #[serde(default)]
    use_trusted_device: bool,
}

/// Answers the challenge with a code of one of the account's methods, with a
/// recovery code, or with the browser's remembered device.
pub async fn verify(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<VerifyRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let answer = if request.use_trusted_device {
        Answer::TrustedDevice
    } else {
        Answer::Code {
            method: request.method_id.as_deref(),
            code: request.code.as_deref().unwrap_or(""),
        }
    };
    let waiting = Waiting::of(&seen.origin);
    let answered = ceremony::answer(
        &state,
        &seen.ask(&context, challenge),
        answer,
        request.remember_device,
        move |tx, completed| link::pending(tx, completed, &waiting),
    )
    .await?;
    match answered {
        Answered::SignedIn { completed, hook } => {
            finished(&state, &seen, *completed, hook).await
        }
        Answered::Refused => wrong_code(&state),
        Answered::TooManyFailures => too_many_failures(&state),
        Answered::MethodUnavailable => method_unavailable(&state),
        Answered::EmptyCode => empty_code(&state),
        Answered::PasskeysUnavailable => passkeys_unavailable(&state),
        Answered::Invalid(status) => challenge_problem(&state, status),
    }
}

/// The answer to a sign-in that was completed, with how linking an identity
/// came out when the sign-in asked for one: its cookie goes with the rest, and
/// a link that failed to be written is told as a link that failed.
pub(super) async fn finished(
    state: &AppState,
    seen: &Seen,
    completed: crate::auth::ceremony::Completed,
    hook: Result<Option<&'static str>, DbError>,
) -> ApiResult<Response> {
    let asked = completed.pending_identity.is_some();
    let identity_link = match hook {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(target: "Stellarr.Api.Security", "linking an identity after a sign-in failed: {error}");
            // The writes of the link were undone, the identity's removal with
            // them: it is removed again, so that a copy of its cookie opens
            // nothing.
            if let Some(expected) = completed.pending_identity {
                link::forget(state, &Waiting::of(&seen.origin), expected).await?;
            }
            asked.then_some(link::FAILED)
        }
    };
    let own = if asked {
        vec![link::clear_cookie(seen.origin.https)]
    } else {
        Vec::new()
    };
    signed_in(state, completed, identity_link, own).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailCodeRequest {
    challenge_id: Option<String>,
    method_id: Option<String>,
}

/// What the client is told once a mailed code is on its way.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeSent {
    status: &'static str,
    message: String,
    retry_after_seconds: i64,
}

/// Mails the code of the second step to the address of one of the account's
/// mail methods.
pub async fn send_login_code(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<EmailCodeRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let ask = seen.ask(&context, challenge);
    let sent = ceremony::send_login_code(&state, &ask, request.method_id.as_deref()).await?;
    code_sent(&state, sent).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailStartRequest {
    challenge_id: Option<String>,
    #[allow(dead_code)]
    name: Option<String>,
}

/// Mails the code that proves the account's address, to add it as a second
/// step.
pub async fn start_email(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<EmailStartRequest>, ApiError>,
) -> ApiResult<Response> {
    let request = match body_of(request, &origin, true) {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let seen = Seen::of(&state, origin);
    let challenge = challenge_id(request.challenge_id.as_deref());
    let context = seen.context();
    let ask = seen.ask(&context, challenge);
    let sent = ceremony::send_enrollment_code(&state, &ask).await?;
    code_sent(&state, sent).await
}

/// The answer to a request for a mailed code.
async fn code_sent(state: &AppState, sent: Sent) -> ApiResult<Response> {
    match sent {
        Sent::Delivered {
            address,
            retry_after_seconds,
        } => {
            let message = state
                .messages()
                .get(MessageKey::SecurityEmailCodeSent, &[("email", &address)])?;
            Ok(ok_json(&CodeSent {
                status: "sent",
                message,
                retry_after_seconds,
            }))
        }
        Sent::Cooldown { retry_after_seconds } => {
            let title = state.messages().get(
                MessageKey::SecurityEmailCooldown,
                &[("seconds", &retry_after_seconds)],
            )?;
            Ok(retry(
                no_store(
                    Problem::new(StatusCode::TOO_MANY_REQUESTS)
                        .title(title)
                        .with("retryAfterSeconds", retry_after_seconds)
                        .into_response(),
                ),
                retry_after_seconds,
            ))
        }
        Sent::Limit => {
            let title = state.messages().get(MessageKey::SecurityEmailSendLimit, &[])?;
            Ok(retry(
                no_store(
                    Problem::new(StatusCode::TOO_MANY_REQUESTS)
                        .title(title)
                        .with("retryAfterSeconds", 3600)
                        .into_response(),
                ),
                3600,
            ))
        }
        Sent::DeliveryFailed => problem(state, StatusCode::BAD_GATEWAY, MessageKey::SecurityDeliveryFailed),
        Sent::MethodUnavailable => method_unavailable(state),
        Sent::MailUnavailable => problem(state, StatusCode::CONFLICT, MessageKey::SecurityEmailUnavailable),
        Sent::Invalid(status) => challenge_problem(state, status),
    }
}

fn retry(mut response: Response, seconds: i64) -> Response {
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}
