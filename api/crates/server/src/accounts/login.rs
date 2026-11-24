//! Signing in with a password, and signing out.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::register::trusted_device_name;
use crate::auth::flow::{self, Context, Options, Start};
use crate::auth::policy::SecurityPolicy;
use crate::auth::{Caller, cookies, events, password, session};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Problem, json_response, with_cookie, with_session_cookie};
use crate::identity_providers::{self as providers, Linked, Requester};
use crate::instance::users::{User, UserDto};
use crate::state::AppState;
use crate::values::Timestamp;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    username: Option<String>,
    password: Option<String>,
    keep_signed_in: Option<bool>,
    #[serde(default)]
    link_pending: bool,
}

/// A sign-in that asked to link the identity waiting for this browser, and how
/// the link came out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkedSignIn {
    user: UserDto,
    identity_link: &'static str,
}

#[derive(Serialize)]
struct Status {
    status: &'static str,
}

/// Answers the routes of `/api/auth` that must never be stored.
fn no_store(mut response: Response) -> Response {
    use axum::http::{HeaderValue, header};
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

pub async fn login(
    State(state): State<AppState>,
    origin: Origin,
    request: Result<Json<LoginRequest>, ApiError>,
) -> ApiResult<Response> {
    let Json(request) = match request {
        Ok(request) => request,
        // A request that names no content type is turned away by the route,
        // which has already marked its answer as not to be stored.
        Err(refusal) if !origin.headers.contains_key(axum::http::header::CONTENT_TYPE) => {
            return Ok(no_store(refusal.into_response()));
        }
        Err(refusal) => return Err(refusal),
    };
    // Either the user name or the e-mail address, in any letter case.
    let identifier = crate::instance::names::to_upper(request.username.as_deref().unwrap_or("").trim());
    let user = {
        let identifier = identifier.clone();
        state
            .database()
            .read(move |connection| -> Result<_, DbError> { Ok(User::find_by_login(connection, &identifier)?) })
            .await?
    };
    let known_id = user.as_ref().map(|user| user.id);
    // Every refusal does the same work: the password is derived whether or
    // not there is an account that may sign in, and whether or not it has a
    // usable hash, and one write follows. How long the answer takes then says
    // nothing about which refusal it was. The random wait below only blurs
    // what is left.
    let candidate = request.password.clone().unwrap_or_default();
    let usable = user.as_ref().filter(|user| !user.is_disabled).and_then(|user| user.password_hash.clone());
    let accepted = tokio::task::spawn_blocking(move || password::verify(&candidate, usable.as_deref()))
        .await
        .map_err(ApiError::internal)?;
    let Some(user) = user.filter(|user| accepted && !user.is_disabled) else {
        return refuse(&state, &origin, known_id).await;
    };

    let short_lived = request.keep_signed_in == Some(false);
    let trusted_device_cookie = cookies::read(&origin.headers, trusted_device_name(origin.https));
    // The identity to link is the one this browser waits with now; the link
    // itself waits for the whole sign-in, second step included.
    let held = providers::held_pending(&origin.headers, origin.https);
    let pending_identity = if request.link_pending {
        let looked = held.clone();
        state
            .database()
            .read(move |connection| -> Result<_, DbError> { Ok(providers::find_pending(connection, looked.as_ref())?.map(|waiting| waiting.id)) })
            .await?
    } else {
        None
    };
    let link_pending = request.link_pending;
    let (write_state, write_origin, write_user) = (state.clone(), origin.clone(), user.clone());
    let (started, linked) = state
        .database()
        .write(move |tx| -> Result<(Start, Option<Linked>), DbError> {
            let messages = write_state.messages();
            let context = Context {
                ip_address: &write_origin.address,
                user_agent: &write_origin.user_agent,
                https: write_origin.https,
                trusted_device_cookie: trusted_device_cookie.as_deref(),
                passkeys_available: flow::passkeys_available(&write_state.settings().operations.public_url),
            };
            let policy = SecurityPolicy::load(tx)?;
            let start = flow::start(tx, &context, messages, &write_user, "password", &policy, Options { short_lived, pending_identity, ..Options::default() })?;
            let mut linked = None;
            if let Start::Session(session) = &start {
                session.insert(tx)?;
                let email = events::verified_email(tx, write_user.id)?;
                events::record(
                    tx,
                    write_user.id,
                    events::LOGIN_SUCCEEDED,
                    &write_origin.address,
                    &write_origin.user_agent,
                    "password",
                    email.as_deref(),
                )?;
                if link_pending {
                    // Once the session exists, so that a refusal never undoes
                    // the sign-in; the waiting identity is used up either way.
                    let requester = Requester { ip_address: write_origin.address.clone(), user_agent: write_origin.user_agent.clone() };
                    linked = Some(providers::link_pending(tx, &requester, write_user.id, pending_identity, held.as_ref(), |id| {
                        write_state.settings().oidc.find(id).cloned()
                    })?);
                }
            }
            Ok((start, linked))
        })
        .await?;

    let response = match started {
        Start::RecoveryRequired => json_response(StatusCode::OK, &Status { status: "security_recovery_required" }),
        Start::Challenge { body, cookie } => with_cookie(json_response(StatusCode::OK, &body), &cookie),
        Start::Session(new) => {
            let cookie = session::sign_in_cookie(
                state.sealer(),
                &state.settings().environment,
                origin.https,
                user.id,
                user.session_version,
                new.id,
                short_lived,
            );
            if request.link_pending {
                // Whatever came of the link, the waiting identity is used up.
                let pending = providers::clear_pending_cookie(origin.https);
                let word = linked.unwrap_or(Linked::Failed).word();
                let body = json_response(StatusCode::OK, &LinkedSignIn { user: user.dto("online"), identity_link: word });
                return Ok(with_cookie(with_session_cookie(no_store(body), &cookie), &pending));
            }
            // The cookie's own cache headers come after, and win.
            return Ok(with_session_cookie(no_store(json_response(StatusCode::OK, &user.dto("online"))), &cookie));
        }
    };
    Ok(no_store(response))
}

/// The 401 of a sign-in that did not work. The failure of an account that
/// exists is recorded; for one that does not, the same transaction is made and
/// writes nothing.
async fn refuse(state: &AppState, origin: &Origin, known: Option<crate::values::Id>) -> ApiResult<Response> {
    let origin = origin.clone();
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            match known {
                Some(user) => events::record_wrong_password(tx, user, &origin.address, &origin.user_agent),
                None => Ok(()),
            }
        })
        .await?;
    password::wrong_password_wait().await;
    let title = state.messages().get(MessageKey::SignInFailed, &[])?;
    Ok(no_store(Problem::new(StatusCode::UNAUTHORIZED).title(title).into_response()))
}

pub async fn logout(State(state): State<AppState>, origin: Origin, caller: Caller) -> ApiResult<Response> {
    let (user, session) = (caller.user, caller.session);
    let now = Timestamp::now();
    let origin_for_event = origin.clone();
    state
        .database()
        .write(move |tx| -> Result<(), DbError> {
            if session::revoke(tx, session, now)? {
                // Signing out is the person's own doing: the first server
                // sent no alert for it, and the event is written without an
                // address to tell.
                events::record(
                    tx,
                    user,
                    events::SESSION_REVOKED,
                    &origin_for_event.address,
                    &origin_for_event.user_agent,
                    "logout",
                    None,
                )?;
            }
            Ok(())
        })
        .await?;
    // Whatever the session had open is closed now.
    state.events().session_ended(user, session);
    let cookie = session::sign_out_cookie(&state.settings().environment, origin.https);
    Ok(with_session_cookie(no_store(StatusCode::NO_CONTENT.into_response()), &cookie))
}
