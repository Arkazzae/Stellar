//! What every route of the administration panel (`/api/cockpit`) stands on.
//!
//! The panel is the operator's: only the owner of the instance server passes
//! (`permissions::membership::InstanceOwner`), and where the account's
//! policy asks for a second step, only a session that has passed one. A
//! change that could hand the instance over, or take its backups away, asks
//! more: a second step passed in the last five minutes
//! ([`FreshInstanceOwner`]).
//!
//! In short: for a route without a JSON body the gate comes first; for one
//! with a body the body is bound first and the gate second (see below).
//!
//! A route of the panel is declared with `.authorized()` (never
//! `.fresh_second_step()`, which would answer a caller who is not even the
//! owner that a second step is missing), and its handler asks for the gate as
//! an argument:
//!
//! ```ignore
//! async fn read(State(state): State<AppState>, InstanceOwner(server): InstanceOwner) -> ... {}
//! async fn download(State(state): State<AppState>, FreshInstanceOwner(server): FreshInstanceOwner, ...) -> ... {}
//! ```
//!
//! What a caller who fails two checks is told is the order of the checks, and
//! it is the first server's: that the request is from somebody (401), whether
//! they own the instance (403, or `cockpit-mfa-required` for the owner whose
//! policy asks a second step of the panel), the second step passed lately
//! (`fresh-mfa-required`), then what the route itself refuses.
//!
//! A route that reads a JSON body is different in one way: the first server
//! bound the body before any of its filters ran, so a body that is not JSON
//! (400), or a request that does not say it is JSON (415), is refused the same
//! for anybody, before the gate. Such a route is `.json_body()` and its
//! handler calls [`bind_then_gate`] with the request instead of taking the
//! gate as arguments.

pub mod operations;

use axum::extract::Request;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use rusqlite::Connection;
use stellarr_messages::MessageKey;

use crate::auth::{Caller, fresh};
use crate::database::DbError;
use crate::http::ApiError;
use crate::permissions::audit::{self, AuditEvent};
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The owner of the instance server, who passed a second step in the last
/// five minutes if the account has one. Anybody else is refused as
/// [`InstanceOwner`] refuses them, and an owner without a fresh step with the
/// problem the clients answer by asking for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshInstanceOwner(pub Id);

impl FromRequestParts<AppState> for FreshInstanceOwner {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<FreshInstanceOwner, ApiError> {
        let InstanceOwner(server) = InstanceOwner::from_request_parts(parts, state).await?;
        let caller = *parts
            .extensions
            .get::<Caller>()
            .ok_or_else(ApiError::unauthorized)?;
        let fresh = state
            .database()
            .read(move |db| -> Result<bool, DbError> {
                Ok(fresh::has_fresh_second_step(
                    db,
                    caller.user,
                    Some(caller.session),
                    Timestamp::now(),
                )?)
            })
            .await?;
        if fresh {
            Ok(FreshInstanceOwner(server))
        } else {
            Err(fresh::refusal(state.messages())?.into())
        }
    }
}

/// Writes the audit line of a change the operator made, in the transaction of
/// the change. The sentence is in the language of the instance.
pub fn record(db: &Connection, server: Id, actor: Id, sentence: String) -> rusqlite::Result<()> {
    audit::record(
        db,
        server,
        actor,
        &AuditEvent {
            action: "operator".to_string(),
            sentence,
            ..AuditEvent::default()
        },
    )?;
    Ok(())
}

/// The sentence of the audit line for a change of the panel's settings.
pub fn settings_updated(state: &AppState) -> Result<String, ApiError> {
    Ok(state
        .messages()
        .get(MessageKey::AuditSettingsUpdated, &[])?)
}

/// What a route of the panel that takes a JSON body is given: who is calling,
/// the instance server, and the body as the route read it.
pub struct Bound<T> {
    pub caller: Caller,
    pub server: Id,
    pub body: T,
}

/// Reads the JSON body of a request and then asks the gate, in that order.
///
/// The first server bound a route's body before any of its filters ran, so a
/// body that is not JSON, or has a member of the wrong kind, was a 400 for
/// anybody, owner or not, before it was asked who was calling. An empty body or
/// `null` bound as nothing and went on to the gate; the route itself then
/// said what it thought of nothing. The gate's answers come after the
/// binding: a caller who is not the owner, then one without a fresh second
/// step (when `fresh`), then the route.
///
/// The body is read as `T`: anything that is not a JSON object, or has a member
/// of the wrong kind or one named twice, is a 400; an empty body or `null`
/// binds as nothing, which the route itself answers (after the gate).
pub async fn bind_then_gate<T: serde::de::DeserializeOwned>(
    state: &AppState,
    request: Request,
    fresh: bool,
) -> Result<Bound<Option<T>>, ApiError> {
    let (mut parts, body) = request.into_parts();
    let bytes =
        crate::http::read_all(&parts.headers, body, crate::http::DEFAULT_JSON_BODY_BYTES).await?;
    let body = if bytes.is_empty() {
        None
    } else {
        // Only an object is a body; a list would bind to a struct by position.
        if !crate::http::is_object_or_null(&bytes) {
            return Err(ApiError::bad_request());
        }
        serde_json::from_slice::<Option<T>>(&bytes).map_err(|_| ApiError::bad_request())?
    };
    let caller = *parts
        .extensions
        .get::<Caller>()
        .ok_or_else(ApiError::unauthorized)?;
    let server = if fresh {
        FreshInstanceOwner::from_request_parts(&mut parts, state)
            .await?
            .0
    } else {
        InstanceOwner::from_request_parts(&mut parts, state)
            .await?
            .0
    };
    Ok(Bound {
        caller,
        server,
        body,
    })
}
