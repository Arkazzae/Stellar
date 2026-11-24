//! Listing, creating, changing and deleting the roles of a server.


use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{Connection, params};
use serde::Deserialize;

use super::announce::Announcement;
use super::{Chosen, RoleDto, RoleRow, no_content, taken};
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization;
use crate::permissions::membership::ActiveServer;
use crate::permissions::ownership;
use crate::permissions::privileges::{self, MANAGE_ROLES, contains};
use crate::permissions::roles::{HeldRoleTable, Role};
use crate::permissions::access_changed::ViewChange;
use crate::state::AppState;
use crate::values::Id;
use stellarr_messages::{MessageKey, Messages};

const MAX_NAME_LENGTH: usize = 64;
const MAX_COLOR_LENGTH: usize = 48;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RoleRequest {
    name: Option<String>,
    color: Option<String>,
    permissions: Option<Vec<String>>,
}

/// A role as a request may make it: the name and color trimmed and within
/// their lengths, and every permission named one that exists.
pub(crate) struct Draft {
    pub(crate) name: String,
    pub(crate) color: String,
    pub(crate) permissions: i64,
}

use stellarr_unicode::utf16::length;

pub(crate) fn validate(name: &str, color: &str, permissions: Option<&[String]>) -> Option<Draft> {
    let name = name.trim();
    let color = color.trim();
    let permissions = privileges::parse_names(
        permissions
            .unwrap_or_default()
            .iter()
            .map(String::as_str),
    )?;
    ((1..=MAX_NAME_LENGTH).contains(&length(name)) && (1..=MAX_COLOR_LENGTH).contains(&length(color)))
        .then(|| Draft {
            name: name.to_string(),
            color: color.to_string(),
            permissions,
        })
}

pub(crate) fn invalid_role(messages: &Messages) -> Result<ApiError, stellarr_messages::MissingValue> {
    Ok(crate::http::Problem::invalid_field("role", messages.get(MessageKey::RoleInvalid, &[])?))
}

fn name_taken(db: &Connection, server: Id, name: &str, except: Option<Id>) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"Name\" = ?2 AND (?3 IS NULL OR \"Id\" != ?3))",
        params![server, name, except],
        |row| row.get(0),
    )
}

/// The caller must hold ManageRoles server-wide to go on at all.
fn require_manage_roles(db: &Connection, caller: Caller, server: Id) -> Result<(), ApiError> {
    if contains(authorization::server_wide(db, caller.user, server)?, MANAGE_ROLES) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}

pub(super) async fn list(State(state): State<AppState>, ActiveServer(server): ActiveServer) -> ApiResult<Response> {
    let roles = state
        .database()
        .read(move |db| -> Result<Vec<RoleDto>, ApiError> {
            let rows = RoleRow::all(db, server)?;
            let table = HeldRoleTable::load(db, server)?;
            Ok(rows
                .iter()
                .map(|row| row.dto(table.count_holders(row.role.id)))
                .collect())
        })
        .await?;
    Ok(json_response(StatusCode::OK, &roles))
}

pub(super) async fn create(
    State(state): State<AppState>,
    caller: Caller,
    chosen: Chosen,
    Json(request): Json<RoleRequest>,
) -> ApiResult<Response> {
    let server = chosen.server()?;
    // Check for fresh second-step proof after body parsing and membership
    // checks, ahead of the remaining refusals.
    crate::auth::fresh::require(&state, caller, Some(server)).await?;
    let messages = *state.messages();
    let created = state
        .database()
        .write(move |tx| -> Result<RoleDto, ApiError> {
            require_manage_roles(tx, caller, server)?;
            let Some(draft) = validate(
                request.name.as_deref().unwrap_or_default(),
                request.color.as_deref().unwrap_or_default(),
                request.permissions.as_deref(),
            ) else {
                return Err(invalid_role(&messages)?);
            };
            if name_taken(tx, server, &draft.name, None)? {
                return Err(ApiError::conflict());
            }
            if !authorization::can_create_role(tx, caller.user, server, draft.permissions)? {
                return Err(ApiError::forbidden());
            }

            let id = Id::random();
            let position = ownership::place_new_role(tx, server)?;
            Role {
                id,
                server,
                position,
                is_system: false,
                is_owner: false,
                permissions: draft.permissions,
            }
            .insert(tx, &draft.name, &draft.color)
            .map_err(taken)?;
            let added = privileges::to_names(draft.permissions);
            audit::record(
                tx,
                server,
                caller.user,
                &AuditEvent {
                    action: "role.create".to_string(),
                    sentence: messages.get(MessageKey::AuditRoleCreated, &[("name", &draft.name)])?,
                    target_type: Some("role".to_string()),
                    target_id: Some(id),
                    target_name: Some(draft.name.clone()),
                    changes: audit::only([
                        Some(AuditChange {
                            field: "color".to_string(),
                            to: Some(draft.color.clone()),
                            to_color: Some(draft.color.clone()),
                            ..AuditChange::default()
                        }),
                        (draft.permissions != privileges::NONE).then(|| {
                            AuditChange::added(
                                "permissions",
                                added.iter().map(|name| name.to_string()).collect(),
                            )
                        }),
                    ]),
                },
            )?;
            Ok(RoleDto {
                id,
                name: draft.name,
                color: draft.color,
                permissions: privileges::to_names(draft.permissions),
                member_count: 0,
                is_system: false,
                is_owner: false,
            })
        })
        .await?;
    let location = format!("/api/admin/roles/{}", created.id);
    let mut response = json_response(StatusCode::CREATED, &created);
    if let Ok(location) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

pub(super) async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Path(role_id): Path<Id>,
    chosen: Chosen,
    Json(request): Json<RoleRequest>,
) -> ApiResult<Response> {
    let server = chosen.server()?;
    // Check for fresh second-step proof after body parsing and membership
    // checks, ahead of the remaining refusals.
    crate::auth::fresh::require(&state, caller, Some(server)).await?;
    let messages = *state.messages();
    let (dto, announcement) = state
        .database()
        .write(move |tx| -> Result<(RoleDto, Announcement), ApiError> {
            require_manage_roles(tx, caller, server)?;
            let Some(role) = RoleRow::find(tx, server, role_id)? else {
                return Err(ApiError::not_found());
            };
            if role.role.is_owner {
                return Err(ApiError::conflict());
            }
            let held_names: Vec<String> = privileges::to_names(role.role.permissions)
                .into_iter()
                .map(str::to_string)
                .collect();
            let Some(draft) = validate(
                request.name.as_deref().unwrap_or(&role.name),
                request.color.as_deref().unwrap_or(&role.color),
                Some(request.permissions.as_deref().unwrap_or(&held_names)),
            ) else {
                return Err(invalid_role(&messages)?);
            };
            if name_taken(tx, server, &draft.name, Some(role.role.id))? {
                return Err(ApiError::conflict());
            }
            if !authorization::can_edit_role(tx, caller.user, &role.role, draft.permissions)? {
                return Err(ApiError::forbidden());
            }

            // Read once the change may go on, before anything is written.
            let before = ViewChange::begin(tx, server)?;
            tx.execute(
                "UPDATE \"ServerRoles\" SET \"Name\" = ?1, \"Color\" = ?2, \"Permissions\" = ?3 WHERE \"Id\" = ?4",
                params![draft.name, draft.color, draft.permissions, role.role.id],
            )
            .map_err(taken)?;
            let granted = names(draft.permissions & !role.role.permissions);
            let revoked = names(role.role.permissions & !draft.permissions);
            let changes = audit::only([
                AuditChange::changed("name", Some(&role.name), Some(&draft.name)),
                (role.color != draft.color).then(|| AuditChange {
                    field: "color".to_string(),
                    from: Some(role.color.clone()),
                    to: Some(draft.color.clone()),
                    from_color: Some(role.color.clone()),
                    to_color: Some(draft.color.clone()),
                    ..AuditChange::default()
                }),
                (!granted.is_empty() || !revoked.is_empty()).then(|| AuditChange {
                    field: "permissions".to_string(),
                    added: Some(granted),
                    removed: Some(revoked),
                    ..AuditChange::default()
                }),
            ]);
            if !changes.is_empty() {
                audit::record(
                    tx,
                    server,
                    caller.user,
                    &AuditEvent {
                        action: "role.update".to_string(),
                        sentence: messages.get(MessageKey::AuditRoleUpdated, &[("name", &draft.name)])?,
                        target_type: Some("role".to_string()),
                        target_id: Some(role.role.id),
                        target_name: Some(draft.name.clone()),
                        changes,
                    },
                )?;
            }

            let table = HeldRoleTable::load(tx, server)?;
            let holders = table.holders(role.role.id);
            let announcement = Announcement::after(tx, server, &before, &holders)?;
            let dto = RoleRow {
                role: Role {
                    permissions: draft.permissions,
                    ..role.role
                },
                name: draft.name,
                color: draft.color,
            }
            .dto(table.count_holders(role.role.id));
            Ok((dto, announcement))
        })
        .await?;
    announcement.publish(&state, server).await;
    Ok(json_response(StatusCode::OK, &dto))
}

fn names(mask: i64) -> Vec<String> {
    privileges::to_names(mask)
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub(super) async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path(role_id): Path<Id>,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    // Check for fresh second-step proof after body parsing and membership
    // checks, ahead of the remaining refusals.
    crate::auth::fresh::require(&state, caller, Some(server)).await?;
    let messages = *state.messages();
    let announcement = state
        .database()
        .write(move |tx| -> Result<Announcement, ApiError> {
            require_manage_roles(tx, caller, server)?;
            let Some(role) = RoleRow::find(tx, server, role_id)? else {
                return Err(ApiError::not_found());
            };
            if role.role.is_system {
                return Err(ApiError::conflict());
            }
            // Its holders keep the other roles they hold, and the plain member
            // role, rather than nothing.
            let mut system: Vec<RoleRow> = RoleRow::all(tx, server)?
                .into_iter()
                .filter(|row| row.role.is_system && !row.role.is_owner)
                .collect();
            if system.len() != 1 {
                return Err(ApiError::internal(
                    "a server must have exactly one role for plain members",
                ));
            }
            let fallback = system.remove(0);
            if !authorization::can_delete_role(tx, caller.user, &role.role, &fallback.role)? {
                return Err(ApiError::forbidden());
            }

            let before = ViewChange::begin(tx, server)?;
            let holders = HeldRoleTable::load(tx, server)?.holders(role.role.id);
            // The top role is moved off the role first: a member may not point
            // at a role that is gone. The rows of those holding it go with it.
            ownership::rank_again(tx, server, Some(role.role.id))?;
            tx.execute("DELETE FROM \"ServerRoles\" WHERE \"Id\" = ?1", [role.role.id])?;
            audit::record(
                tx,
                server,
                caller.user,
                &AuditEvent {
                    action: "role.delete".to_string(),
                    sentence: messages.get(MessageKey::AuditRoleDeleted, &[("name", &role.name)])?,
                    target_type: Some("role".to_string()),
                    target_id: Some(role.role.id),
                    target_name: Some(role.name.clone()),
                    changes: if holders.is_empty() {
                        Vec::new()
                    } else {
                        vec![AuditChange {
                            field: "holders".to_string(),
                            to: Some(holders.len().to_string()),
                            ..AuditChange::default()
                        }]
                    },
                },
            )?;
            Ok(Announcement::after(tx, server, &before, &holders)?)
        })
        .await?;
    announcement.publish(&state, server).await;
    Ok(no_content())
}
