//! Making, renaming and removing a section.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::params;
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::names::{is_valid_section_name, trim};
use super::{Answer, say, Refuse, SectionDto, created, path_id};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, Json, Path, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_CHANNELS;
use crate::permissions::visibility;
use crate::state::AppState;
use crate::values::Id;

#[derive(Deserialize)]
pub struct CreateSection {
    name: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameSection {
    name: Option<String>,
}

const INVALID_NAME: Refuse = Refuse::Invalid("name", MessageKey::SectionNameInvalid);

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<CreateSection>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<SectionDto>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_CHANNELS)? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            let name = trim(request.name.as_deref().unwrap_or_default());
            if !is_valid_section_name(name) {
                return Ok(Answer::Refused(INVALID_NAME));
            }
            let last: Option<i64> = tx.query_row(
                "SELECT MAX(\"Position\") FROM \"Sections\" WHERE \"ServerId\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            let section = SectionDto {
                id: Id::random(),
                name: name.to_string(),
                position: last.unwrap_or(-1) + 1,
            };
            tx.execute(
                "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, ?3, ?4)",
                params![section.id, server, section.name, section.position],
            )?;
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "section.create".to_string(),
                    sentence: say(&messages, MessageKey::AuditSectionCreated, &[("name", &section.name)])?,
                    target_type: Some("section".to_string()),
                    target_id: Some(section.id),
                    target_name: Some(section.name.clone()),
                    changes: Vec::new(),
                },
            )?;
            Ok(Answer::Done(section))
        })
        .await?;
    match outcome {
        Answer::Refused(refusal) => Err(refusal.into_error(&messages)?),
        Answer::Done(section) => Ok(created(
            format!("/api/admin/sections/{}", section.id),
            &section,
        )),
    }
}

pub async fn rename(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(section_id): Path<String>,
    Json(request): Json<RenameSection>,
) -> ApiResult<Response> {
    let section_id = path_id(&section_id)?;
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<SectionDto>, DbError> {
            // A section that is not listed for the caller does not exist.
            if !visibility::section_listed(tx, user, server, section_id)? {
                return Ok(Answer::Refused(Refuse::NotFound));
            }
            let (mut name, position): (String, i64) = tx.query_row(
                "SELECT \"Name\", \"Position\" FROM \"Sections\" WHERE \"Id\" = ?1",
                [section_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if !allows(
                tx,
                user,
                &Scope::section(server, section_id),
                MANAGE_CHANNELS,
            )? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            let wanted = request.name.as_deref().map(trim);
            if let Some(wanted) = wanted
                && !is_valid_section_name(wanted)
            {
                return Ok(Answer::Refused(INVALID_NAME));
            }
            let before = name.clone();
            if let Some(wanted) = wanted {
                name = wanted.to_string();
            }
            if before != name {
                tx.execute(
                    "UPDATE \"Sections\" SET \"Name\" = ?2 WHERE \"Id\" = ?1",
                    params![section_id, name],
                )?;
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "section.update".to_string(),
                        sentence: say(&messages, MessageKey::AuditSectionRenamed, &[("name", &name)])?,
                        target_type: Some("section".to_string()),
                        target_id: Some(section_id),
                        target_name: Some(name.clone()),
                        changes: vec![AuditChange {
                            field: "name".to_string(),
                            from: Some(before),
                            to: Some(name.clone()),
                            ..AuditChange::default()
                        }],
                    },
                )?;
            }
            Ok(Answer::Done(SectionDto {
                id: section_id,
                name,
                position,
            }))
        })
        .await?;
    match outcome {
        Answer::Refused(refusal) => Err(refusal.into_error(&messages)?),
        Answer::Done(section) => Ok(json_response(StatusCode::OK, &section)),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(section_id): Path<String>,
) -> ApiResult<Response> {
    let section_id = path_id(&section_id)?;
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<()>, DbError> {
            if !visibility::section_listed(tx, user, server, section_id)? {
                return Ok(Answer::Refused(Refuse::NotFound));
            }
            let name: String = tx.query_row(
                "SELECT \"Name\" FROM \"Sections\" WHERE \"Id\" = ?1",
                [section_id],
                |row| row.get(0),
            )?;
            if !allows(
                tx,
                user,
                &Scope::section(server, section_id),
                MANAGE_CHANNELS,
            )? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            // Channels are not tied to a section by a foreign key, so that
            // the section is empty and its removal are one statement.
            let deleted = tx.execute(
                "DELETE FROM \"Sections\" WHERE \"Id\" = ?1
                 AND NOT EXISTS (SELECT 1 FROM \"Channels\" WHERE \"SectionId\" = \"Sections\".\"Id\")",
                [section_id],
            )?;
            if deleted == 0 {
                return Ok(Answer::Refused(Refuse::Conflict));
            }
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "section.delete".to_string(),
                    sentence: say(&messages, MessageKey::AuditSectionDeleted, &[("name", &name)])?,
                    target_type: Some("section".to_string()),
                    target_id: Some(section_id),
                    target_name: Some(name),
                    changes: Vec::new(),
                },
            )?;
            Ok(Answer::Done(()))
        })
        .await?;
    match outcome {
        Answer::Refused(refusal) => Err(refusal.into_error(&messages)?),
        Answer::Done(()) => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
