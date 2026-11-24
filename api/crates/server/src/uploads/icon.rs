//! `POST` and `DELETE /api/admin/server/icon`: the picture of the server.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::attachments::{Attachment, compact, retire_if_unreferenced, roll_back};
use super::files::{created, read};
use super::form::Slot;
use super::inspect::{self, file_name_of, keep_image};
use super::refusals::{Texts, no_room, refuse};
use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, json_response};
use crate::permissions::audit::{self, AuditEvent};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const MAX_ICON_BYTES: i64 = 4 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IconDto {
    icon_url: Option<String>,
}

/// What members are told when the server changes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerDto {
    id: Id,
    name: String,
    description: String,
    icon_url: Option<String>,
    created_at: Timestamp,
    is_public: bool,
}

fn server_dto(db: &rusqlite::Connection, server: Id) -> rusqlite::Result<ServerDto> {
    db.query_row(
        "SELECT \"Name\", \"Description\", \"IconUrl\", \"CreatedAt\", \"IsPublic\" FROM \"Servers\" WHERE \"Id\" = ?1",
        [server],
        |row| {
            Ok(ServerDto {
                id: server,
                name: row.get(0)?,
                description: row.get(1)?,
                icon_url: row.get(2)?,
                created_at: row.get(3)?,
                is_public: row.get(4)?,
            })
        },
    )
}

pub async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    request: Request,
) -> ApiResult<Response> {
    let keeps = |content_type: &str| keep_image(content_type, MAX_ICON_BYTES);
    let slots = [Slot { name: "file", keeps: &keeps }];
    let mut received = read(&state, request, &slots).await?;
    let file = received.files.remove(0).ok_or_else(ApiError::bad_request)?;
    let user = caller.user;
    if !may_manage(&state, user, server).await? {
        return Err(ApiError::forbidden());
    }
    let spec = match inspect::image(&file, MAX_ICON_BYTES) {
        Ok(spec) => spec,
        Err(refused) => {
            let texts = Texts {
                unsupported_title: MessageKey::ServerIconTypeUnsupported,
                unsupported_detail: MessageKey::ImageTypesAllowed,
                too_large_title: MessageKey::ServerIconTooLarge,
                too_large_detail: &|_| MessageKey::ServerIconMaximum,
                mismatch_title: MessageKey::ServerIconTypeMismatch,
            };
            return Err(refuse(&state, &refused, &texts)?);
        }
    };
    let lease = match state
        .admission()
        .enter(state.database(), user, file.length as i64)
        .await?
    {
        Ok(lease) => lease,
        Err(why) => return Err(no_room(&state, why)?),
    };
    let id = Id::random();
    let key = format!(
        "servers/{}/icon/{}{}",
        compact(server),
        compact(id),
        spec.extension
    );
    let mut source = file.open().await.map_err(ApiError::internal)?;
    state
        .storage()
        .put(&key, &mut source)
        .await
        .map_err(ApiError::internal)?;
    let attachment = Attachment {
        id,
        uploaded_by: user,
        object_key: key.clone(),
        file_name: file_name_of(&file.file_name).to_string(),
        content_type: spec.content_type.to_string(),
        size: file.length as i64,
        width: None,
        height: None,
        duration_seconds: None,
        is_voice_message: false,
        poster: None,
        preview: None,
        placeholder: None,
        is_encrypted: false,
    };
    let url = format!("/assets/{id}");
    let new_url = url.clone();
    let sentence = state.messages().get(MessageKey::AuditServerIconChanged, &[])?;
    let written = state
        .database()
        .write(move |tx| -> Result<(Option<String>, ServerDto), DbError> {
            attachment.insert(tx)?;
            let (previous, name): (Option<String>, String) = tx.query_row(
                "SELECT \"IconUrl\", \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            tx.execute(
                "UPDATE \"Servers\" SET \"IconUrl\" = ?1 WHERE \"Id\" = ?2",
                params![new_url, server],
            )?;
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "server.icon".to_string(),
                    sentence,
                    target_type: Some("server".to_string()),
                    target_id: Some(server),
                    target_name: Some(name),
                    changes: Vec::new(),
                },
            )?;
            Ok((previous, server_dto(tx, server)?))
        })
        .await;
    let (previous, dto) = match written {
        Ok(done) => done,
        Err(error) => {
            roll_back(&state, vec![key]).await;
            return Err(error.into());
        }
    };
    drop(lease);
    retire_if_unreferenced(&state, previous, None).await;
    state
        .events()
        .publish(Event::new(Audience::Server(server), "ServerUpdated", &dto));
    Ok(created(url.clone(), &IconDto { icon_url: Some(url) }))
}

pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let user = caller.user;
    if !may_manage(&state, user, server).await? {
        return Err(ApiError::forbidden());
    }
    let sentence = state.messages().get(MessageKey::AuditServerIconRemoved, &[])?;
    let (previous, dto) = state
        .database()
        .write(move |tx| -> Result<(Option<String>, ServerDto), DbError> {
            let (previous, name): (Option<String>, String) = tx.query_row(
                "SELECT \"IconUrl\", \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            tx.execute(
                "UPDATE \"Servers\" SET \"IconUrl\" = NULL WHERE \"Id\" = ?1",
                [server],
            )?;
            if previous.is_some() {
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "server.icon.remove".to_string(),
                        sentence,
                        target_type: Some("server".to_string()),
                        target_id: Some(server),
                        target_name: Some(name),
                        changes: Vec::new(),
                    },
                )?;
            }
            Ok((previous, server_dto(tx, server)?))
        })
        .await?;
    retire_if_unreferenced(&state, previous, None).await;
    state
        .events()
        .publish(Event::new(Audience::Server(server), "ServerUpdated", &dto));
    Ok(json_response(StatusCode::OK, &IconDto { icon_url: None }))
}

async fn may_manage(state: &AppState, user: Id, server: Id) -> Result<bool, ApiError> {
    Ok(state
        .database()
        .read(move |db| -> Result<bool, DbError> {
            Ok(allows(db, user, &Scope::server(server), MANAGE_SERVER)?)
        })
        .await?)
}
