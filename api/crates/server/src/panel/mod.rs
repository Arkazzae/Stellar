//! What every route of the administration panel (`/api/cockpit`) stands on.
//!
//! The panel is the operator's: only the owner of the instance server passes
//! (`permissions::membership::InstanceOwner`), and where the account's
//! policy asks for a second step, only a session that has passed one. A
//! change that could hand the instance over, or take its backups away, asks
//! more: a second step passed in the last five minutes
//! ([`FreshInstanceOwner`]).
//!
//! A route of the panel is declared with `.authorized()` (never
//! `.fresh_second_step()`, which would answer a caller who is not even the
//! owner that a second step is missing), and its handler asks for the gate as
//! an argument, before any body:
//!
//! ```ignore
//! async fn read(State(state): State<AppState>, InstanceOwner(server): InstanceOwner) -> ... {}
//! async fn change(State(state): State<AppState>, caller: Caller, FreshInstanceOwner(server): FreshInstanceOwner, body: Json<..>) -> ... {}
//! ```
//!
//! What a caller who fails two checks is told is the order of the arguments:
//! first that the request is from somebody, then whether they own the
//! instance, then the second step, then the body. That is the order the first
//! server's filters ran in.

pub mod operations;

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
