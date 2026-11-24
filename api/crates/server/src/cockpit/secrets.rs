//! The secrets the operator sets in the panel. They only go in: a read says
//! whether each is set and where its value comes from, never what it is.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::{invalid, bind};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path, json_response};
use crate::panel::{self, FreshInstanceOwner};
use crate::permissions::membership::InstanceOwner;
use crate::values::Timestamp;
use crate::secrets::{self, ResolvedSecrets, SecretDefinition};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretDto {
    name: &'static str,
    configured: bool,
    source: &'static str,
    variable: &'static str,
    updated_at: Option<Timestamp>,
}

fn describe(definition: &SecretDefinition, resolved: &ResolvedSecrets) -> SecretDto {
    let status = resolved.status(definition);
    SecretDto {
        name: status.name,
        configured: status.configured,
        source: status.source,
        variable: status.variable,
        updated_at: status.updated_at,
    }
}

pub(super) async fn list(
    State(state): State<AppState>,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<Vec<SecretDto>, DbError> {
            let resolved = secrets::read(db, &reading.settings().secrets, reading.sealer())?;
            Ok(secrets::ALL.iter().map(|d| describe(d, &resolved)).collect())
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

/// A body that carries a secret is never printed.
#[derive(Deserialize)]
struct Value {
    value: Option<String>,
}

pub(super) async fn set(
    State(state): State<AppState>,
    Path(name): Path<String>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<Value>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let Some(definition) = secrets::find(&name) else {
        return Err(ApiError::not_found());
    };
    let messages = *state.messages();
    let value = body.value.unwrap_or_default();
    if !secrets::is_valid_value(&value) {
        return Err(invalid(
            "value",
            messages.get(MessageKey::CockpitSecretValueInvalid, &[("max", &secrets::MAX_LENGTH)])?,
        ));
    }
    let writing = state.clone();
    let answer = state
        .database()
        .write(move |tx| -> Result<SecretDto, ApiError> {
            secrets::set(tx, writing.sealer(), definition, &value)?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(MessageKey::AuditSecretSet, &[("name", &definition.name)])?,
            )?;
            let resolved = secrets::read(tx, &writing.settings().secrets, writing.sealer())?;
            Ok(describe(definition, &resolved))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

pub(super) async fn clear(
    State(state): State<AppState>,
    Path(name): Path<String>,
    caller: crate::auth::Caller,
    FreshInstanceOwner(server): FreshInstanceOwner,
) -> ApiResult<Response> {
    let Some(definition) = secrets::find(&name) else {
        return Err(ApiError::not_found());
    };
    let messages = *state.messages();
    let writing = state.clone();
    let answer = state
        .database()
        .write(move |tx| -> Result<SecretDto, ApiError> {
            secrets::clear(tx, definition)?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(MessageKey::AuditSecretCleared, &[("name", &definition.name)])?,
            )?;
            let resolved = secrets::read(tx, &writing.settings().secrets, writing.sealer())?;
            Ok(describe(definition, &resolved))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}
