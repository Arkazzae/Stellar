//! The routes that send a person to a provider, and the two that say what is
//! on offer and what waits for this browser.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Serialize;
use stellarr_oidc::flow::{self, Actor, Purpose, Start};
use stellarr_oidc::options::Provider;
use stellarr_oidc::ticket::{self, TicketKind};

use super::linking::{providers_allowing, sign_up_allowed};
use super::names::{display_name_or, shown_name};
use super::pages::{self, no_store, redirect, reauthentication_page};
use super::pending;
use super::single_sign_on::unix_now;
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{
    ApiError, ApiResult, ClientInfo, MaybeCaller, Origin, Path, QueryValues, json_response,
    with_cookie,
};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// What a provider is to the sign-in page.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Offered {
    id: String,
    display_name: String,
    login_url: String,
    sign_up: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    enabled: bool,
    display_name: String,
    providers: Vec<Offered>,
}

/// The providers the sign-in page offers, each with its button's name and
/// address, and whether it may create accounts for people with an invite.
/// `enabled` and `displayName` describe the first one, as clients from before
/// several providers read them.
pub async fn status(State(state): State<AppState>) -> ApiResult<Response> {
    let allowing = state
        .database()
        .read(|connection| -> Result<_, DbError> { Ok(providers_allowing(connection)?) })
        .await?;
    let messages = state.messages();
    let oidc = &state.settings().oidc;
    let mut providers = Vec::new();
    for provider in &oidc.providers {
        providers.push(Offered {
            id: provider.id.clone(),
            display_name: shown_name(provider, messages)?,
            login_url: provider.login_path(),
            sign_up: allowing.contains(&provider.id),
        });
    }
    let display_name = match providers.first() {
        Some(first) => first.display_name.clone(),
        None => display_name_or(&oidc.display_name, messages)?,
    };
    let body = Status {
        enabled: !providers.is_empty(),
        display_name,
        providers,
    };
    Ok(no_store(json_response(StatusCode::OK, &body)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderName {
    id: String,
    display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingBody {
    provider: ProviderName,
    display_name: Option<String>,
    email: Option<String>,
    username: String,
    sign_up: bool,
}

/// The identity a provider confirmed that this browser waits with, as the
/// sign-in page shows it: whose it is at which provider, and a login to
/// suggest for a new account. It says nothing about the accounts here, not
/// even whether the address belongs to one. 404 when there is none, it ran
/// out or its provider is no longer set up.
pub async fn pending_read(State(state): State<AppState>, origin: Origin) -> ApiResult<Response> {
    let held = pending::held(&origin.headers, origin.https);
    let found = state
        .database()
        .read(move |connection| -> Result<_, DbError> {
            let Some(waiting) = pending::find(connection, held.as_ref())? else {
                return Ok(None);
            };
            let allowed = sign_up_allowed(connection, &waiting.provider_id)?;
            Ok(Some((waiting, allowed)))
        })
        .await?;
    let provider = found
        .as_ref()
        .and_then(|(waiting, _)| state.settings().oidc.find(&waiting.provider_id));
    let (Some((waiting, allowed)), Some(provider)) = (&found, provider) else {
        return Ok(no_store(ApiError::not_found().into_response()));
    };
    let body = PendingBody {
        provider: ProviderName {
            id: provider.id.clone(),
            display_name: shown_name(provider, state.messages())?,
        },
        display_name: waiting.display_name.clone(),
        email: waiting.email.clone(),
        username: waiting.suggested_username(),
        sign_up: *allowed,
    };
    Ok(no_store(json_response(StatusCode::OK, &body)))
}

/// Forgets the identity this browser waits with: the visitor went back to the
/// sign-in page.
pub async fn pending_drop(State(state): State<AppState>, origin: Origin) -> ApiResult<Response> {
    let held = pending::held(&origin.headers, origin.https);
    state
        .database()
        .write(move |tx| -> Result<(), DbError> { pending::forget(tx, held.as_ref()) })
        .await?;
    let response = no_store(StatusCode::NO_CONTENT.into_response());
    Ok(with_cookie(response, &pending::clear_cookie(origin.https)))
}

// ---- starting a round trip ----

pub async fn login_default(
    state: State<AppState>,
    client: Extension<ClientInfo>,
    headers: HeaderMap,
    query: QueryValues,
) -> ApiResult<Response> {
    let first = state.0.settings().oidc.providers.first().cloned();
    login(state.0, client.0, headers, query, first).await
}

pub async fn login_named(
    state: State<AppState>,
    client: Extension<ClientInfo>,
    headers: HeaderMap,
    Path(id): Path<String>,
    query: QueryValues,
) -> ApiResult<Response> {
    let found = state.0.settings().oidc.find(&id).cloned();
    login(state.0, client.0, headers, query, found).await
}

async fn login(
    state: AppState,
    client: ClientInfo,
    headers: HeaderMap,
    query: QueryValues,
    provider: Option<Provider>,
) -> ApiResult<Response> {
    // A value that is not a flag is refused before anything else.
    let keep_signed_in = match flag(&query, "keepSignedIn") {
        Ok(flag) => flag,
        Err(refusal) => return Ok(no_store(refusal.into_response())),
    };
    let Some(provider) = provider else {
        return Ok(no_store(ApiError::not_found().into_response()));
    };
    let return_url = query.text("returnUrl");
    let response = challenge(
        &state,
        &client,
        &headers,
        &provider,
        return_url.as_deref(),
        keep_signed_in == Some(false),
        Purpose::SignIn,
    )
    .await?;
    Ok(no_store(response))
}

/// Sends a signed-in account to the provider to link itself. The ticket must
/// name this provider and this very session; the provider's answer then links
/// instead of signing in.
pub async fn link_default(
    state: State<AppState>,
    client: Extension<ClientInfo>,
    headers: HeaderMap,
    caller: MaybeCaller,
    query: QueryValues,
) -> ApiResult<Response> {
    let first = state.0.settings().oidc.providers.first().cloned();
    ticketed(state.0, client.0, headers, caller.0, query, first, TicketKind::Link).await
}

pub async fn link_named(
    state: State<AppState>,
    client: Extension<ClientInfo>,
    headers: HeaderMap,
    caller: MaybeCaller,
    Path(id): Path<String>,
    query: QueryValues,
) -> ApiResult<Response> {
    let found = state.0.settings().oidc.find(&id).cloned();
    ticketed(state.0, client.0, headers, caller.0, query, found, TicketKind::Link).await
}

/// Sends a signed-in account to a provider it is linked to, to sign in there
/// again as the proof a sensitive change asks for. The ticket is bound like a
/// link's; the answer acts on this session and comes back to `returnUrl` on
/// this origin, through the page that says how it went.
pub async fn reauthenticate_default(
    state: State<AppState>,
    client: Extension<ClientInfo>,
    headers: HeaderMap,
    caller: MaybeCaller,
    query: QueryValues,
) -> ApiResult<Response> {
    let first = state.0.settings().oidc.providers.first().cloned();
    ticketed(state.0, client.0, headers, caller.0, query, first, TicketKind::Reauthenticate).await
}

pub async fn reauthenticate_named(
    state: State<AppState>,
    client: Extension<ClientInfo>,
    headers: HeaderMap,
    caller: MaybeCaller,
    Path(id): Path<String>,
    query: QueryValues,
) -> ApiResult<Response> {
    let found = state.0.settings().oidc.find(&id).cloned();
    ticketed(state.0, client.0, headers, caller.0, query, found, TicketKind::Reauthenticate).await
}

async fn ticketed(
    state: AppState,
    client: ClientInfo,
    headers: HeaderMap,
    caller: Option<Caller>,
    query: QueryValues,
    provider: Option<Provider>,
    kind: TicketKind,
) -> ApiResult<Response> {
    let Some(provider) = provider else {
        return Ok(no_store(ApiError::not_found().into_response()));
    };
    let return_url = query.text("returnUrl");
    let reauthenticating = kind == TicketKind::Reauthenticate;
    let refused = || {
        let page = if reauthenticating {
            reauthentication_page(false, return_url.as_deref())
        } else {
            pages::link_result("failed")
        };
        no_store(redirect(&page))
    };
    let ticket = query.text("ticket").unwrap_or_default();
    let Ok(read) = ticket::read(state.single_sign_on().key(), kind, &ticket, unix_now()) else {
        return Ok(refused());
    };
    // The ticket names this provider and the very session presenting it.
    let fits = read.provider_id == provider.id
        && caller.is_some_and(|caller| {
            read.actor.user_id == compact(caller.user) && read.actor.session_id == compact(caller.session)
        });
    if !fits {
        return Ok(refused());
    }
    let (purpose, return_to) = if reauthenticating {
        (Purpose::Reauthenticate(read.actor), return_url)
    } else {
        (Purpose::Link(read.actor), Some(pages::link_result("linked")))
    };
    let response = challenge(&state, &client, &headers, &provider, return_to.as_deref(), false, purpose).await?;
    Ok(no_store(response))
}

/// An id as the tickets and states spell one: without hyphens.
pub fn compact(id: Id) -> String {
    id.to_string().replace('-', "")
}

pub fn actor_of(user: Id, session: Id, security_version: i64) -> Option<Actor> {
    Some(Actor {
        user_id: compact(user),
        session_id: compact(session),
        security_version: u32::try_from(security_version).ok()?,
    })
}

/// A query value read as `bool.TryParse` does, or nothing when there is none.
fn flag(query: &QueryValues, name: &str) -> ApiResult<Option<bool>> {
    let Some(text) = query.text(name).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if trimmed.eq_ignore_ascii_case("true") {
        Ok(Some(true))
    } else if trimmed.eq_ignore_ascii_case("false") {
        Ok(Some(false))
    } else {
        Err(ApiError::bad_request())
    }
}

/// The address the provider is to answer to: this server's, as the request
/// reached it.
fn redirect_uri(client: &ClientInfo, headers: &HeaderMap, provider: &Provider) -> Option<String> {
    let host = client.host.clone().or_else(|| {
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    })?;
    Some(format!("{}://{host}{}", client.scheme, provider.callback_path))
}

/// Builds the request to the provider and answers with the redirect to it,
/// and the cookie that ties the answer to this browser.
async fn challenge(
    state: &AppState,
    client: &ClientInfo,
    headers: &HeaderMap,
    provider: &Provider,
    return_url: Option<&str>,
    short_lived: bool,
    purpose: Purpose,
) -> ApiResult<Response> {
    let sso = state.single_sign_on();
    let metadata = sso.metadata(provider).await.map_err(|failed| {
        tracing::warn!("the provider \"{}\" cannot be reached: {}", provider.id, failed.0);
        ApiError::internal("the identity provider cannot be reached")
    })?;
    let redirect_uri = redirect_uri(client, headers, provider)
        .ok_or_else(|| ApiError::internal("the request names no host"))?;
    let now = unix_now();
    let started = flow::start(
        sso.key(),
        &Start {
            provider,
            discovery: &metadata.discovery,
            redirect_uri: &redirect_uri,
            purpose,
            return_url,
            short_lived,
            now,
        },
    )
    .map_err(ApiError::internal)?;
    let state_text = pages::state_of(&started.redirect_to)
        .ok_or_else(|| ApiError::internal("the request to the provider carries no state"))?;
    let expires = Timestamp::from_unix_seconds(now + flow::STATE_LIFETIME)
        .ok_or_else(|| ApiError::internal("the sign-in has no end"))?;
    let cookie = pages::set_binding(state_text, &started.browser_binding, expires, provider);
    Ok(with_cookie(redirect(&started.redirect_to), &cookie))
}

