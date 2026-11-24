//! What a signed-in person does about the identities their account signs in
//! through: starting a link, and taking one away.
//!
//! Linking adds a way in, and with a provider whose second step the instance
//! trusts, one that skips the local second step, so it asks for the same fresh
//! proof as adding a second-step method; so does unlinking, which is refused
//! when the identity is the last way in the account signs in.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;
use stellarr_oidc::form::encode_component;
use stellarr_oidc::ticket::{self, TicketKind};

use super::names::shown_name;
use super::pages::no_store;
use super::proof::Proof;
use super::single_sign_on::unix_now;
use super::start::actor_of;
use crate::auth::ways_in::{self, OwnedProviders};
use crate::auth::{Caller, events};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Origin, Path, Problem, json_response};
use crate::instance::users::User;
use crate::servers::Object;
use crate::state::AppState;
use crate::values::{Guid, Id};

/// What a change of an identity may ask for besides the session's own proof:
/// the account's current password.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    password: Option<String>,
}

type Request = Result<Option<Json<Object<Change>>>, ApiError>;

/// The password a body gives, none when there is no body. A body that is not
/// the right JSON is refused as the framework refuses it.
fn password_of(request: Request) -> Result<Option<String>, Box<Response>> {
    match request {
        Ok(request) => Ok(request.and_then(|Json(Object(change))| change.password)),
        Err(refusal) => Err(Box::new(refusal.into_response())),
    }
}

/// The refusal for a session that may not change the account's security now.
fn fresh_proof_needed(state: &AppState) -> ApiResult<ApiError> {
    let title = state.messages().get(MessageKey::SecurityFreshMfaRequired, &[])?;
    Ok(Problem::new(StatusCode::FORBIDDEN).title(title).into())
}

#[derive(Serialize)]
struct Started {
    url: String,
}

/// Checks the proof and answers with the address that starts the link, ticket
/// included.
pub async fn start_link(
    State(state): State<AppState>,
    caller: Caller,
    Path(provider_id): Path<String>,
    request: Request,
) -> Response {
    let password = match password_of(request) {
        Ok(password) => password,
        Err(refusal) => return *refusal,
    };
    no_store(linking(&state, caller, &provider_id, password).await.unwrap_or_else(IntoResponse::into_response))
}

async fn linking(
    state: &AppState,
    caller: Caller,
    provider_id: &str,
    password: Option<String>,
) -> ApiResult<Response> {
    let Some(provider) = state.settings().oidc.find(provider_id) else {
        return Err(ApiError::not_found());
    };
    let proof = Proof::check(state, caller, password.as_deref()).await?;
    let allowed = state
        .database()
        .read(move |connection| -> Result<Option<(bool, i64)>, DbError> {
            let Some(user) = User::find(connection, caller.user)? else {
                return Ok(None);
            };
            Ok(Some((proof.allows(connection, caller)?, user.session_version)))
        })
        .await?;
    let Some((allowed, version)) = allowed else {
        return Err(ApiError::not_found());
    };
    if !allowed {
        return Err(fresh_proof_needed(state)?);
    }
    let actor = actor_of(caller.user, caller.session, version)
        .ok_or_else(|| ApiError::internal("a security version beyond what a ticket holds"))?;
    let ticket = ticket::issue(
        state.single_sign_on().key(),
        TicketKind::Link,
        actor,
        &provider.id,
        unix_now(),
    )
    .ok_or_else(|| ApiError::internal("a ticket could not be made"))?;
    let url = format!("{}?ticket={}", provider.link_path(), encode_component(&ticket));
    Ok(json_response(StatusCode::OK, &Started { url }))
}

/// Takes an identity away, unless it is the last way the account signs in.
pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    origin: Origin,
    Path(Guid(identity)): Path<Guid>,
    request: Request,
) -> Response {
    let password = match password_of(request) {
        Ok(password) => password,
        Err(refusal) => return *refusal,
    };
    no_store(removal(&state, caller, &origin, identity, password).await.unwrap_or_else(IntoResponse::into_response))
}

enum Removed {
    Done,
    NoAccount,
    NotAllowed,
    NoIdentity,
    LastWayIn,
}

async fn removal(
    state: &AppState,
    caller: Caller,
    origin: &Origin,
    identity: Id,
    password: Option<String>,
) -> ApiResult<Response> {
    let proof = Proof::check(state, caller, password.as_deref()).await?;
    let providers = owned(state);
    let (state_in, origin_in) = (state.clone(), origin.clone());
    let removed = state
        .database()
        .write(move |tx| -> Result<Removed, DbError> {
            // The question and the removal are one transaction, so that two
            // removals at once cannot each count the other's way in as left.
            let Some(user) = User::find(tx, caller.user)? else {
                return Ok(Removed::NoAccount);
            };
            if !proof.allows(tx, caller)? {
                return Ok(Removed::NotAllowed);
            }
            let found: Option<(String, Option<String>)> = tx
                .query_row(
                    "SELECT \"Issuer\", \"ProviderId\" FROM \"ExternalIdentities\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2",
                    params![identity, user.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((issuer, provider_id)) = found else {
                return Ok(Removed::NoIdentity);
            };
            if !ways_in::remain(tx, &providers.view(), user.id, Some(identity), None)? {
                return Ok(Removed::LastWayIn);
            }
            tx.execute(
                "DELETE FROM \"ExternalIdentities\" WHERE \"Id\" = ?1",
                [identity],
            )?;
            let name = identity_name(&state_in, provider_id.as_deref().unwrap_or("default"), &issuer)
                .map_err(|error| DbError::Task(error.to_string()))?;
            let email = events::verified_email(tx, user.id)?;
            events::record(
                tx,
                user.id,
                events::IDENTITY_UNLINKED,
                &origin_in.address,
                &origin_in.user_agent,
                &name,
                email.as_deref(),
            )?;
            Ok(Removed::Done)
        })
        .await?;
    match removed {
        Removed::Done => Ok(StatusCode::NO_CONTENT.into_response()),
        Removed::NoAccount | Removed::NoIdentity => Err(ApiError::not_found()),
        Removed::NotAllowed => Err(fresh_proof_needed(state)?),
        Removed::LastWayIn => Err(ways_in::identity_refusal(state.messages())?.into()),
    }
}

/// The providers this instance has set up, as the question of the last way in
/// asks about them.
pub fn owned(state: &AppState) -> OwnedProviders {
    OwnedProviders {
        configured: state
            .settings()
            .oidc
            .providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect(),
        default_id: "default".to_string(),
    }
}

/// What a linked identity is called: its provider's name while that is set up,
/// else the name of its issuer.
pub fn identity_name(
    state: &AppState,
    provider: &str,
    issuer: &str,
) -> Result<String, stellarr_messages::MissingValue> {
    match state.settings().oidc.find(provider) {
        Some(found) => shown_name(found, state.messages()),
        None => Ok(issuer_name(issuer)),
    }
}

/// How a linked identity is named when its provider is no longer set up: the
/// host of its issuer, or the issuer as it was written when that is no
/// address.
pub fn issuer_name(issuer: &str) -> String {
    let Some((scheme, rest)) = issuer.split_once("://") else {
        return issuer.to_string();
    };
    let valid_scheme = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        return issuer.to_string();
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_and_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_and_port.starts_with('[') {
        host_and_port
            .split_once(']')
            .map_or(host_and_port, |(inside, _)| &host_and_port[..=inside.len()])
    } else {
        host_and_port.split(':').next().unwrap_or("")
    };
    host.to_ascii_lowercase()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issuer_is_named_by_its_host() {
        assert_eq!(issuer_name("https://Id.Example.test/realms/main"), "id.example.test");
        assert_eq!(issuer_name("https://user@id.example.test:8443/x"), "id.example.test");
        assert_eq!(issuer_name("https://[::1]:8443/x"), "[::1]");
        assert_eq!(issuer_name("not an address"), "not an address");
    }
}
