//! The security page's state and the list of security events.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::{no_store, providers, trusted_device_of};
use crate::auth::policy::{FRESH_SECOND_STEP_WINDOW, SecurityPolicy};
use crate::auth::{Caller, fresh, recovery_codes, trusted_devices};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Origin, json_response};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// How many events the page shows, and how many the list of them does.
const PAGE_EVENTS: i64 = 20;
const LISTED_EVENTS: i64 = 100;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    requirement: &'static str,
    mfa_required: bool,
    fresh_mfa: bool,
    mfa_evidence: Option<String>,
    mfa_completed_at: Option<Timestamp>,
    trusted_device_available: bool,
    methods: Vec<Method>,
    remaining_recovery_codes: i64,
    trusted_devices: Vec<Device>,
    recent_events: Vec<Event>,
    fresh_mfa_window_seconds: u64,
    trusted_device_days: i64,
    email_code_available: bool,
    email_resend_cooldown_seconds: i64,
    security_recovery_required: bool,
    passkey_available: bool,
    identities: Vec<Identity>,
    identity_providers: Vec<Provider>,
    has_password: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Method {
    id: Id,
    r#type: String,
    name: String,
    created_at: Timestamp,
    last_used_at: Option<Timestamp>,
    email_address: Option<String>,
    pending: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    id: Id,
    name: String,
    device: String,
    created_at: Timestamp,
    expires_at: Timestamp,
    last_used_at: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    id: Id,
    kind: String,
    detail: String,
    ip_address: String,
    device: String,
    created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    id: Id,
    provider_id: Option<String>,
    name: String,
    created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    id: String,
    display_name: String,
}

pub async fn read(State(state): State<AppState>, caller: Caller, origin: Origin) -> Response {
    no_store(page(&state, caller, &origin).await)
}

pub async fn events(State(state): State<AppState>, caller: Caller) -> Response {
    no_store(list_events(&state, caller).await)
}

async fn list_events(state: &AppState, caller: Caller) -> ApiResult<Response> {
    let events = state
        .database()
        .read(move |connection| events_of(connection, caller.user, LISTED_EVENTS))
        .await?;
    Ok(json_response(StatusCode::OK, &events))
}

fn events_of(connection: &Connection, user: Id, limit: i64) -> rusqlite::Result<Vec<Event>> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Kind\", \"Detail\", \"IpAddress\", \"Device\", \"CreatedAt\" FROM \"SecurityEvents\"
         WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\" DESC LIMIT ?2",
    )?;
    statement
        .query_map(params![user, limit], |row| {
            Ok(Event {
                id: row.get(0)?,
                kind: row.get(1)?,
                detail: row.get(2)?,
                ip_address: row.get(3)?,
                device: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect()
}

async fn page(state: &AppState, caller: Caller, origin: &Origin) -> ApiResult<Response> {
    let trusted_token = trusted_device_of(origin);
    let public_url = state.settings().operations.public_url.clone();
    let configured = providers::configured(state);
    let found = state
        .database()
        .read(move |connection| -> Result<Option<Page>, DbError> {
            let now = Timestamp::now();
            let Some(user) = crate::instance::users::User::find(connection, caller.user)? else {
                return Ok(None);
            };
            let policy = SecurityPolicy::load(connection)?;
            let methods = confirmed_methods(connection, user.id)?;
            let current = fresh::current_session(connection, &user, caller.session, now)?;
            let evidence: Option<String> = match current {
                Some(_) => connection.query_row(
                    "SELECT \"MfaEvidence\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
                    [caller.session],
                    |row| row.get(0),
                )?,
                None => None,
            };
            let completed = current.as_ref().and_then(|session| session.mfa_completed_at);
            let required = !methods.is_empty() || policy.is_required_at_login(connection, user.id)?;
            let trusted = trusted_devices::find(connection, user.id, trusted_token.as_deref(), now)?.is_some();
            let devices = trusted_devices::list(connection, user.id, now)?
                .into_iter()
                .map(|device| Device {
                    id: device.id,
                    name: device.name,
                    device: device.device,
                    created_at: device.created_at,
                    expires_at: device.expires_at,
                    last_used_at: device.last_used_at,
                })
                .collect();
            let email_code_available = methods.iter().any(|method| method.r#type == "email");
            let owned = providers::owned_of(&configured);
            Ok(Some(Page {
                requirement: policy.mfa_requirement,
                mfa_required: required,
                fresh_mfa: completed.is_some_and(|at| at >= now.minus(FRESH_SECOND_STEP_WINDOW)),
                mfa_evidence: evidence,
                mfa_completed_at: completed,
                trusted_device_available: trusted,
                methods,
                remaining_recovery_codes: recovery_codes::remaining(connection, user.id)?,
                trusted_devices: devices,
                recent_events: events_of(connection, user.id, PAGE_EVENTS)?,
                fresh_mfa_window_seconds: FRESH_SECOND_STEP_WINDOW.as_secs(),
                trusted_device_days: policy.trusted_device_days_clamped(),
                email_code_available,
                email_resend_cooldown_seconds: policy.email_code_cooldown_seconds,
                security_recovery_required: user.security_recovery_required,
                passkey_available: crate::auth::flow::passkeys_available(&public_url),
                identities: identities(connection, user.id, &owned, &configured)?,
                identity_providers: configured
                    .iter()
                    .map(|provider| Provider { id: provider.id.clone(), display_name: provider.display_name.clone() })
                    .collect(),
                has_password: user.password_hash.is_some(),
            }))
        })
        .await?;
    let page = found.ok_or_else(ApiError::not_found)?;
    Ok(json_response(StatusCode::OK, &page))
}

fn confirmed_methods(connection: &Connection, user: Id) -> rusqlite::Result<Vec<Method>> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Type\", \"Name\", \"CreatedAt\", \"LastUsedAt\", \"EmailAddress\" FROM \"MfaMethods\"
         WHERE \"UserId\" = ?1 AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL ORDER BY \"CreatedAt\"",
    )?;
    statement
        .query_map([user], |row| {
            let kind: String = row.get(1)?;
            let address: Option<String> = row.get(5)?;
            Ok(Method {
                id: row.get(0)?,
                email_address: if kind == "email" { address } else { None },
                r#type: kind,
                name: row.get(2)?,
                created_at: row.get(3)?,
                last_used_at: row.get(4)?,
                pending: false,
            })
        })?
        .collect()
}

/// The identities linked to the account, named for the provider that is
/// configured for them or, failing that, the host of their issuer.
fn identities(
    connection: &Connection,
    user: Id,
    owned: &crate::auth::ways_in::OwnedProviders,
    configured: &[providers::Provider],
) -> rusqlite::Result<Vec<Identity>> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"ProviderId\", \"Issuer\", \"CreatedAt\" FROM \"ExternalIdentities\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\"",
    )?;
    let providers = owned.view();
    statement
        .query_map([user], |row| {
            let linked_through: Option<String> = row.get(1)?;
            let issuer: String = row.get(2)?;
            let provider = providers.of(linked_through.as_deref()).to_string();
            Ok(Identity {
                id: row.get(0)?,
                name: providers::identity_name(configured, &provider, &issuer),
                provider_id: Some(provider),
                created_at: row.get(3)?,
            })
        })?
        .collect()
}
