//! The sign-in providers of the instance and whether each may create
//! accounts for people with an invite.
//!
//! Which providers exist is the environment's business (the registry in
//! `settings.oidc`); whether each may create accounts is the owner's, here, in
//! the row `identity_providers` reads when somebody signs up.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::{MessageKey, MissingValue};

use super::bind;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path, json_response};
use crate::panel::{self};
use crate::permissions::membership::InstanceOwner;
use crate::state::AppState;
use crate::values::Timestamp;

/// A provider the instance has set up.
pub(super) struct Registered {
    pub id: String,
    /// What the provider's button says: the operator's name for it, else the
    /// instance's word for the account that comes with it.
    pub name: String,
}

/// The providers in the order they are set up, and the id the one without a
/// name of its own goes by.
pub(super) struct Registry {
    pub providers: Vec<Registered>,
    pub default_id: String,
}

impl Registry {
    pub(super) fn find(&self, id: &str) -> Option<&Registered> {
        self.providers.iter().find(|provider| provider.id == id)
    }

    /// The ids, for the question whether an account keeps a way in.
    pub(super) fn owned(&self) -> crate::auth::ways_in::OwnedProviders {
        crate::auth::ways_in::OwnedProviders {
            configured: self.providers.iter().map(|p| p.id.clone()).collect(),
            default_id: self.default_id.clone(),
        }
    }
}

/// Asks the registry of providers the environment set up.
pub(super) fn registry(state: &AppState) -> Result<Registry, MissingValue> {
    let providers = state
        .settings()
        .oidc
        .providers
        .iter()
        .map(|provider| {
            Ok(Registered {
                id: provider.id.clone(),
                name: crate::identity_providers::shown_name(provider, state.messages())?,
            })
        })
        .collect::<Result<_, MissingValue>>()?;
    Ok(Registry {
        providers,
        default_id: crate::identity_providers::owned(state).default_id,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderDto {
    id: String,
    display_name: String,
    sign_up: bool,
}

pub async fn list(
    State(state): State<AppState>,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Response> {
    let registry = registry(&state)?;
    let answer = state
        .database()
        .read(move |db| -> Result<Vec<ProviderDto>, DbError> {
            let mut statement = db.prepare(
                "SELECT \"ProviderId\" FROM \"IdentityProviderSettings\" WHERE \"SignUpAllowed\" = 1",
            )?;
            let allowing: Vec<String> = statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(registry
                .providers
                .into_iter()
                .map(|provider| ProviderDto {
                    sign_up: allowing.contains(&provider.id),
                    id: provider.id,
                    display_name: provider.name,
                })
                .collect())
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignUpChoice {
    sign_up: Option<bool>,
}

/// Lets a provider create accounts for people with an invite, or stops it. A
/// new way to get an account on the instance is a security change, so it asks
/// for a fresh second step like the others, and is audited.
pub async fn set_sign_up(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<SignUpChoice>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let registry = registry(&state)?;
    let Some(found) = registry.find(&provider) else {
        return Err(ApiError::not_found());
    };
    let messages = *state.messages();
    let Some(sign_up) = body.sign_up else {
        return Err(super::invalid(
            "signUp",
            messages.get(MessageKey::CockpitProviderSignUpMissing, &[])?,
        ));
    };
    let (id, name) = (found.id.clone(), found.name.clone());
    let answer_id = id.clone();
    let answer_name = name.clone();
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            let was: bool = tx
                .query_row(
                    "SELECT \"SignUpAllowed\" FROM \"IdentityProviderSettings\" WHERE \"ProviderId\" = ?1",
                    [&id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if was != sign_up {
                panel::record(
                    tx,
                    server,
                    caller.user,
                    messages.get(
                        if sign_up {
                            MessageKey::AuditProviderSignUpAllowed
                        } else {
                            MessageKey::AuditProviderSignUpStopped
                        },
                        &[("provider", &name)],
                    )?,
                )?;
            }
            tx.execute(
                "INSERT INTO \"IdentityProviderSettings\" (\"ProviderId\", \"SignUpAllowed\", \"UpdatedAt\") VALUES (?1, ?2, ?3)
                 ON CONFLICT (\"ProviderId\") DO UPDATE SET \"SignUpAllowed\" = excluded.\"SignUpAllowed\", \"UpdatedAt\" = excluded.\"UpdatedAt\"",
                params![id, sign_up, Timestamp::now()],
            )
            .map_err(super::taken)?;
            Ok(())
        })
        .await?;
    Ok(json_response(
        StatusCode::OK,
        &ProviderDto {
            id: answer_id,
            display_name: answer_name,
            sign_up,
        },
    ))
}
