//! A server's name, description and whether anybody may join it.

use axum::extract::State;
use rusqlite::params;
use serde::Deserialize;
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Problem};
use crate::instance::servers::{Server, ServerDto};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{self, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::state::AppState;
use super::Object;

const MAX_NAME_LENGTH: usize = 80;
const MAX_DESCRIPTION_LENGTH: usize = 280;

/// A change to the server profile; a property left out stays as it is.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateServer {
    name: Option<String>,
    description: Option<String>,
    is_public: Option<bool>,
}

fn invalid(field: &str, message: String) -> ApiError {
    Problem::validation(vec![(field.to_string(), vec![message])]).into()
}

fn visibility(is_public: bool) -> &'static str {
    if is_public { "public" } else { "invite-only" }
}

pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(Object(request)): Json<Object<UpdateServer>>,
) -> ApiResult<axum::response::Response> {
    let messages = state.messages();
    let name = request.name.as_deref().map(str::trim);
    let description = request.description.as_deref().map(str::trim);
    // A person who may not manage the server is told so before anything about
    // their request, which is why the check comes inside the transaction with
    // the rest.
    let denied_or_invalid = |db_allowed: bool| -> Result<Option<ApiError>, ApiError> {
        if !db_allowed {
            return Ok(Some(ApiError::forbidden()));
        }
        if let Some(name) = name
            && !(1..=MAX_NAME_LENGTH).contains(&stellarr_unicode::utf16::length(name))
        {
            return Ok(Some(invalid(
                "name",
                messages.get(MessageKey::ServerNameLength, &[])?,
            )));
        }
        if let Some(description) = description
            && stellarr_unicode::utf16::length(description) > MAX_DESCRIPTION_LENGTH
        {
            return Ok(Some(invalid(
                "description",
                messages.get(MessageKey::ServerDescriptionLength, &[])?,
            )));
        }
        Ok(None)
    };
    let allowed = state
        .database()
        .read(move |db| -> Result<bool, DbError> {
            Ok(authorization::allows(
                db,
                caller.user,
                &Scope::server(server),
                MANAGE_SERVER,
            )?)
        })
        .await?;
    if let Some(refusal) = denied_or_invalid(allowed)? {
        return Err(refusal);
    }
    let name = name.map(str::to_string);
    let description = description.map(str::to_string);
    let made_public = request.is_public;
    let updated = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Option<Server>, DbError> {
                let messages = state.messages();
                // Read again where nothing can change it: the person may have
                // lost the permission, or the server may be gone.
                if !authorization::allows(tx, caller.user, &Scope::server(server), MANAGE_SERVER)? {
                    return Ok(None);
                }
                let Some(before) = Server::find(tx, server)? else {
                    return Ok(None);
                };
                let new_name = name.unwrap_or_else(|| before.name.clone());
                let new_description = description.unwrap_or_else(|| before.description.clone());
                let new_public = made_public.unwrap_or(before.is_public);
                tx.execute(
                    "UPDATE \"Servers\" SET \"Name\" = ?1, \"Description\" = ?2, \"IsPublic\" = ?3 WHERE \"Id\" = ?4",
                    params![new_name, new_description, new_public, server],
                )?;
                let profile = audit::only([
                    AuditChange::changed("name", Some(&before.name), Some(&new_name)),
                    AuditChange::changed(
                        "description",
                        Some(&before.description),
                        Some(&new_description),
                    ),
                ]);
                if !profile.is_empty() {
                    audit::record(
                        tx,
                        server,
                        caller.user,
                        &AuditEvent {
                            action: "server.update".into(),
                            sentence: messages.get(MessageKey::AuditServerUpdated, &[])
                                .map_err(|e| DbError::Task(e.to_string()))?,
                            target_type: Some("server".into()),
                            target_id: Some(server),
                            target_name: Some(new_name.clone()),
                            changes: profile,
                        },
                    )?;
                }
                // Visibility is a switch of its own in the settings, so it is
                // a line of its own.
                if before.is_public != new_public {
                    let key = if new_public {
                        MessageKey::AuditServerMadePublic
                    } else {
                        MessageKey::AuditServerMadeInviteOnly
                    };
                    audit::record(
                        tx,
                        server,
                        caller.user,
                        &AuditEvent {
                            action: "server.visibility".into(),
                            sentence: messages.get(key, &[]).map_err(|e| DbError::Task(e.to_string()))?,
                            target_type: Some("server".into()),
                            target_id: Some(server),
                            target_name: Some(new_name.clone()),
                            changes: vec![AuditChange {
                                field: "visibility".into(),
                                from: Some(visibility(before.is_public).into()),
                                to: Some(visibility(new_public).into()),
                                ..AuditChange::default()
                            }],
                        },
                    )?;
                }
                Ok(Server::find(tx, server)?)
            }
        })
        .await?;
    let Some(updated) = updated else {
        return Err(ApiError::forbidden());
    };
    let dto: ServerDto = updated.dto();
    state.events().publish(Event::new(
        Audience::Server(server),
        "ServerUpdated",
        &dto,
    ));
    Ok(crate::http::json_response(axum::http::StatusCode::OK, &dto))
}
