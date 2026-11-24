//! What happens when a provider sends a person back.
//!
//! The answer comes as a post from another site (or a get, from a provider
//! that ignores how it was asked to answer), so it carries no session cookie:
//! what ties it to the browser that asked is the cookie set when the
//! sign-in began, and what it is for is in the sealed state it returns.
//!
//! Every way this can fail ends the same way for the person: a redirect to the
//! page that says roughly why. Which page depends on what the round trip was
//! for, and that is only known once the state has been opened. The provider's
//! own refusal reads as the person cancelling when the *query* of the request
//! says `access_denied`, as the first server read it, so an answer that was
//! posted says "failed" for a person who cancelled.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value};
use stellarr_oidc::flow::{self, Actor, Answer, Callback, Flow, Purpose};
use stellarr_oidc::idtoken::{self, Expect, IdToken, IdTokenError};
use stellarr_oidc::options::Provider;
use stellarr_oidc::profile::Profile;
use stellarr_oidc::token::{self, TokenError};

use super::linking::{self, Linked, Requester};
use super::pages;
use super::pending::{self, Pending};
use super::single_sign_on::{Failed, Metadata, unix_now};
use crate::auth::flow::{self as sign_in, Context, Options, Start};
use crate::auth::policy::SecurityPolicy;
use crate::auth::{events, security_cookies, session};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Origin, Path, with_cookie, with_session_cookie};
use crate::instance::users::User;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The most of a posted answer that is read: a state is under four kibibytes
/// and a code under eight.
const MAX_ANSWER: usize = 64 * 1024;

pub async fn callback_default(
    State(state): State<AppState>,
    origin: Origin,
    request: Request,
) -> ApiResult<Response> {
    let provider = state.settings().oidc.find("default").cloned();
    receive(state, origin, provider, request).await
}

/// The callback of a provider named in the settings. The original provider
/// has the address above and no other, whatever its id is called.
pub async fn callback_named(
    State(state): State<AppState>,
    origin: Origin,
    Path(id): Path<String>,
    request: Request,
) -> ApiResult<Response> {
    let provider = state
        .settings()
        .oidc
        .find(&id)
        .filter(|provider| provider.id != "default")
        .cloned();
    receive(state, origin, provider, request).await
}

async fn receive(
    state: AppState,
    origin: Origin,
    provider: Option<Provider>,
    request: Request,
) -> ApiResult<Response> {
    // No route here at all for a provider that is not set up.
    let Some(provider) = provider else {
        return Err(ApiError::not_found());
    };
    let message = Message::read(request).await;
    let reply = answer(&state, &origin, &provider, message).await;
    Ok(reply.into_response())
}

/// What the request said, and whether its query said the person cancelled.
struct Message {
    /// The form or the query that holds the provider's answer, if the request
    /// can carry one.
    form: Option<String>,
    cancelled: bool,
}

impl Message {
    async fn read(request: Request) -> Message {
        let (parts, body) = request.into_parts();
        let query = parts.uri.query().unwrap_or("").to_string();
        let cancelled = says_denied(&query);
        let form = if parts.method == Method::GET {
            Some(query)
        } else if parts.method == Method::POST && is_form(&parts.headers) {
            crate::http::read_all(&parts.headers, body, MAX_ANSWER)
                .await
                .ok()
                .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok())
        } else {
            None
        };
        Message { form, cancelled }
    }
}

fn is_form(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| {
            media
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

/// Whether the query holds `error` exactly once and as `access_denied`.
fn says_denied(query: &str) -> bool {
    let mut values = query
        .split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .filter(|(name, _)| stellarr_oidc::form::unescape(&name.replace('+', " ")) == "error")
        .map(|(_, value)| stellarr_oidc::form::unescape(&value.replace('+', " ")));
    matches!((values.next(), values.next()), (Some(only), None) if only == "access_denied")
}

/// Where to send the browser, with the cookies to set on the way.
struct Reply {
    location: String,
    /// A cookie that is not the session's, in the order to set them.
    cookies: Vec<String>,
    session_cookie: Option<String>,
}

impl Reply {
    fn to(location: String) -> Reply {
        Reply {
            location,
            cookies: Vec::new(),
            session_cookie: None,
        }
    }

    fn with(mut self, cookie: String) -> Reply {
        self.cookies.push(cookie);
        self
    }

    fn into_response(self) -> Response {
        let mut response = pages::redirect(&self.location);
        for cookie in &self.cookies {
            response = with_cookie(response, cookie);
        }
        match &self.session_cookie {
            Some(cookie) => with_session_cookie(response, cookie),
            None => response,
        }
    }
}

/// Where a round trip that did not work ends, for what it was for.
fn failure(flow: Option<&Flow>, cancelled: bool) -> String {
    match flow.map(|flow| &flow.purpose) {
        Some(Purpose::Link(_)) => pages::link_result(if cancelled { "cancelled" } else { "failed" }),
        Some(Purpose::Reauthenticate(_)) => {
            pages::reauthentication_page(false, flow.map(|flow| flow.return_url.as_str()))
        }
        Some(Purpose::SignIn) | None => pages::login_failed(cancelled),
    }
}

async fn answer(state: &AppState, origin: &Origin, provider: &Provider, message: Message) -> Reply {
    let cancelled = message.cancelled;
    let Some(form) = message.form else {
        return Reply::to(failure(None, cancelled));
    };
    let Ok(callback) = Callback::parse(&form) else {
        return Reply::to(failure(None, cancelled));
    };
    let state_text = callback.state.clone().unwrap_or_default();
    let binding = pages::read_binding(&origin.headers, &state_text).unwrap_or_default();
    let sso = state.single_sign_on();
    let accepted = flow::accept(
        sso.key(),
        sso.guard(),
        &provider.id,
        &binding,
        &callback,
        unix_now(),
    );
    let answered = match accepted {
        Ok(answered) => answered,
        Err(refused) => {
            tracing::info!("an answer from \"{}\" was refused: {refused}", provider.id);
            return Reply::to(failure(None, cancelled));
        }
    };
    // The state is used up from here on, whatever comes of it.
    let clear = pages::clear_binding(&state_text, provider);
    let reply = match answered {
        Answer::Refused { error, flow, .. } => {
            tracing::info!("\"{}\" refused the sign-in: {error}", provider.id);
            Reply::to(failure(Some(&flow), cancelled))
        }
        Answer::Code { code, flow } => {
            match proceed(state, origin, provider, &code, &flow).await {
                Ok(reply) => reply,
                Err(failed) => {
                    tracing::warn!(
                        "a sign-in through \"{}\" did not work out: {}",
                        provider.id,
                        failed.0
                    );
                    Reply::to(failure(Some(&flow), cancelled))
                }
            }
        }
    };
    reply.with(clear)
}

/// Redeems the code, checks the token and does what the round trip was for.
async fn proceed(
    state: &AppState,
    origin: &Origin,
    provider: &Provider,
    code: &str,
    flow: &Flow,
) -> Result<Reply, Failed> {
    let sso = state.single_sign_on();
    let metadata = sso.metadata(provider).await?;
    let request = token::token_request(&metadata.discovery, provider, flow, code);
    let (status, body) = sso.redeem(&request).await?;
    let tokens = token::parse_token_response(status, &body).map_err(|error: TokenError| Failed(error.to_string()))?;

    let (metadata, identity) = verified(state, provider, flow, &tokens.id_token, &tokens.access_token, metadata).await?;
    let issuer = identity.stored_issuer().to_string();
    let subject = identity.subject.clone();
    let requester = Requester {
        ip_address: origin.address.clone(),
        user_agent: origin.user_agent.clone(),
    };
    match &flow.purpose {
        Purpose::Link(actor) => {
            let linked = link_identity(state, &requester, provider, actor, &issuer, &subject).await?;
            Ok(Reply::to(pages::link_result(linked.word())))
        }
        // No route starts such a round trip yet, so no state asks for one.
        Purpose::Reauthenticate(_) => Err(Failed("a fresh sign-in is not offered".into())),
        Purpose::SignIn => {
            sign_in_or_keep(state, origin, provider, flow, &metadata, &tokens.access_token, &identity, &issuer, &subject)
                .await
        }
    }
}

/// The ID token, checked against the provider's keys, which are read again
/// once when it names one they lack.
async fn verified(
    state: &AppState,
    provider: &Provider,
    flow: &Flow,
    id_token: &str,
    access_token: &str,
    metadata: Metadata,
) -> Result<(Metadata, IdToken), Failed> {
    let check = |metadata: &Metadata| {
        let expect = Expect::for_flow(
            &metadata.discovery,
            provider.client_id(),
            flow,
            Some(access_token),
            unix_now(),
        );
        idtoken::verify(id_token, &metadata.keys, &expect)
    };
    match check(&metadata) {
        Ok(identity) => Ok((metadata, identity)),
        Err(IdTokenError::UnknownKeyId) => {
            let again = state
                .single_sign_on()
                .metadata_again(provider, &metadata)
                .await?
                .ok_or_else(|| Failed("the token names a key the provider does not publish".into()))?;
            let identity = check(&again).map_err(|error| Failed(error.to_string()))?;
            Ok((again, identity))
        }
        Err(error) => Err(Failed(error.to_string())),
    }
}

/// What the provider says about the person at its profile endpoint, when it
/// has one.
async fn profile_of(
    state: &AppState,
    metadata: &Metadata,
    access_token: &str,
    subject: &str,
) -> Result<Option<Map<String, Value>>, Failed> {
    let request = token::userinfo_request(&metadata.discovery, Some(access_token))
        .map_err(|error| Failed(error.to_string()))?;
    let Some(request) = request else {
        return Ok(None);
    };
    let (status, content_type, body) = state.single_sign_on().profile(&request).await?;
    token::parse_userinfo(status, content_type.as_deref(), &body, subject)
        .map(Some)
        .map_err(|error| Failed(error.to_string()))
}

// ---- the account a link was started for ----

/// The account a round trip from the settings was started for. The provider's
/// answer is a cross-site post without the session cookie, so the session in
/// the sealed state is checked here instead: still active, on the account's
/// current security version, and the account neither disabled nor waiting for
/// an operator's recovery.
fn asking_account(
    connection: &rusqlite::Connection,
    actor: &Actor,
    now: Timestamp,
) -> Result<Option<User>, DbError> {
    let (Some(user_id), Some(session_id)) = (
        crate::values::parse_guid(&actor.user_id),
        crate::values::parse_guid(&actor.session_id),
    ) else {
        return Ok(None);
    };
    let Some(user) = User::find(connection, user_id)? else {
        return Ok(None);
    };
    let version = i64::from(actor.security_version);
    let active: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL
                        AND \"ExpiresAt\" > ?3 AND \"AbsoluteExpiresAt\" > ?3 AND \"SecurityVersion\" = ?4)",
        params![session_id, user_id, now, version],
        |row| row.get(0),
    )?;
    Ok((active
        && !user.is_disabled
        && !user.security_recovery_required
        && user.session_version == version)
        .then_some(user))
}

/// Links the identity to the account the link was started for, once the
/// session named in the sealed state is found still able to ask.
async fn link_identity(
    state: &AppState,
    requester: &Requester,
    provider: &Provider,
    actor: &Actor,
    issuer: &str,
    subject: &str,
) -> Result<Linked, Failed> {
    let (requester, provider, actor) = (requester.clone(), provider.clone(), actor.clone());
    let (issuer, subject) = (issuer.to_string(), subject.to_string());
    state
        .database()
        .write(move |tx| -> Result<Linked, DbError> {
            let Some(user) = asking_account(tx, &actor, Timestamp::now())? else {
                return Ok(Linked::Failed);
            };
            linking::link(tx, &requester, user.id, &provider, &issuer, &subject)
        })
        .await
        .map_err(|error| Failed(error.to_string()))
}

// ---- signing in ----

/// What the write that decides a sign-in came to.
enum Signed {
    /// The account is gone, disabled, or the provider's profile did not come.
    Refused,
    Recovery,
    SecondStep { enrollment: bool, cookie: String },
    Session { user: User, id: Id },
}

#[allow(clippy::too_many_arguments)]
async fn sign_in_or_keep(
    state: &AppState,
    origin: &Origin,
    provider: &Provider,
    flow: &Flow,
    metadata: &Metadata,
    access_token: &str,
    identity: &IdToken,
    issuer: &str,
    subject: &str,
) -> Result<Reply, Failed> {
    let (linked, known) = {
        let (issuer, subject) = (issuer.to_string(), subject.to_string());
        state
            .database()
            .read(move |connection| -> Result<(bool, Option<User>), DbError> {
                let owner: Option<Id> = connection
                    .query_row(
                        "SELECT \"UserId\" FROM \"ExternalIdentities\" WHERE \"Issuer\" = ?1 AND \"Subject\" = ?2",
                        params![issuer, subject],
                        |row| row.get(0),
                    )
                    .optional()?;
                match owner {
                    Some(owner) => Ok((true, User::find(connection, owner)?)),
                    None => Ok((false, None)),
                }
            })
            .await
            .map_err(|error| Failed(error.to_string()))?
    };
    if !linked {
        return keep_unlinked(state, origin, provider, flow, metadata, access_token, identity, issuer, subject).await;
    }
    let Some(user) = known.filter(|user| !user.is_disabled) else {
        return Err(Failed("the account of the identity is not available".into()));
    };
    // The provider's profile is asked for before a session is made, as its
    // answer decides whether the sign-in counts.
    let profile_ok = profile_of(state, metadata, access_token, subject).await.is_ok();
    let second_step_at_provider =
        provider.passed_second_step(&identity.amr(), identity.auth_time().as_deref(), unix_now());
    let signed = write_sign_in(state, origin, flow, user.id, second_step_at_provider, profile_ok).await?;
    match signed {
        Signed::Refused => Err(Failed("the sign-in could not be made".into())),
        Signed::Recovery => Ok(Reply::to(pages::recovery_page(flow.short_lived))),
        Signed::SecondStep { enrollment, cookie } => {
            Ok(Reply::to(pages::second_step_page(enrollment, &flow.return_url)).with(cookie))
        }
        Signed::Session { user, id } => {
            let cookie = session::sign_in_cookie(
                state.sealer(),
                &state.settings().environment,
                origin.https,
                user.id,
                user.session_version,
                id,
                flow.short_lived,
            );
            let mut reply = Reply::to(flow.return_url.clone());
            reply.session_cookie = Some(cookie);
            Ok(reply)
        }
    }
}

async fn write_sign_in(
    state: &AppState,
    origin: &Origin,
    flow: &Flow,
    user_id: Id,
    second_step_at_provider: bool,
    profile_ok: bool,
) -> Result<Signed, Failed> {
    let (state_in, origin_in) = (state.clone(), origin.clone());
    let short_lived = flow.short_lived;
    state
        .database()
        .write(move |tx| -> Result<Signed, DbError> {
            // Read again here, where nothing can change it.
            let Some(user) = User::find(tx, user_id)?.filter(|user| !user.is_disabled) else {
                return Ok(Signed::Refused);
            };
            let trusted = security_cookies::read_trusted_device(&origin_in.headers, origin_in.https);
            let context = Context {
                ip_address: &origin_in.address,
                user_agent: &origin_in.user_agent,
                https: origin_in.https,
                trusted_device_cookie: trusted.as_deref(),
                passkeys_available: sign_in::passkeys_available(&state_in.settings().operations.public_url),
            };
            let policy = SecurityPolicy::load(tx)?;
            let started = sign_in::start(
                tx,
                &context,
                state_in.messages(),
                &user,
                "oidc",
                &policy,
                Options {
                    short_lived,
                    second_step_at_provider,
                    ..Options::default()
                },
            )?;
            match started {
                Start::RecoveryRequired => Ok(Signed::Recovery),
                Start::Challenge { body, cookie } => {
                    let enrollment = serde_json::to_value(&body)
                        .ok()
                        .and_then(|value| value.get("status").and_then(Value::as_str).map(str::to_string))
                        .is_some_and(|status| status == "mfa_enrollment_required");
                    Ok(Signed::SecondStep { enrollment, cookie })
                }
                Start::Session(_) if !profile_ok => Ok(Signed::Refused),
                Start::Session(new) => {
                    new.insert(tx)?;
                    let email = events::verified_email(tx, user.id)?;
                    events::record(
                        tx,
                        user.id,
                        events::LOGIN_SUCCEEDED,
                        &origin_in.address,
                        &origin_in.user_agent,
                        "oidc",
                        email.as_deref(),
                    )?;
                    Ok(Signed::Session { user, id: new.id })
                }
            }
        })
        .await
        .map_err(|error| Failed(error.to_string()))
}

/// Keeps the identity for this browser and sends it to the sign-in page, which
/// says nobody here is linked to it and what can be done. Nothing is signed
/// in.
#[allow(clippy::too_many_arguments)]
async fn keep_unlinked(
    state: &AppState,
    origin: &Origin,
    provider: &Provider,
    flow: &Flow,
    metadata: &Metadata,
    access_token: &str,
    identity: &IdToken,
    issuer: &str,
    subject: &str,
) -> Result<Reply, Failed> {
    let mut profile = Profile::from_claims(&identity.claims);
    if let Some(info) = profile_of(state, metadata, access_token, subject).await? {
        profile.apply_userinfo(&info);
    }
    let waiting = Pending {
        id: Id::random(),
        provider_id: provider.id.clone(),
        issuer: issuer.to_string(),
        subject: subject.to_string(),
        email_verified: profile.email.is_some() && profile.email_verified,
        email: profile.email,
        display_name: profile.name,
        preferred_username: profile.preferred_username,
        amr: profile.amr,
        auth_time: profile.auth_time,
        short_lived: flow.short_lived,
    };
    let previous = pending::held(&origin.headers, origin.https).map(|(id, _)| id);
    let https = origin.https;
    let kept = state
        .database()
        .write(move |tx| -> Result<pending::Kept, DbError> { pending::keep(tx, &waiting, previous, https) })
        .await
        .map_err(|error| Failed(error.to_string()))?;
    Ok(Reply::to(pages::unlinked_page(&flow.return_url, flow.short_lived)).with(kept.cookie))
}
