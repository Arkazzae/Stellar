//! What a role, or one member, may do beyond, or short of, what their roles
//! grant inside one channel or one section. Who may set it is decided by
//! ManageRoles and not by ManageChannels: it edits what people may do, as the
//! roles editor does. Member overrides have routes of their own beside the
//! role ones, so a list of role overrides stays a list of role overrides.
//! Administrator is server-wide only and never part of an override.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::announce::Announcement;
use super::{invalid, no_content, taken};
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Endpoints, Json, Path, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{self, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{self, ADMINISTRATOR, MANAGE_ROLES, contains};
use crate::permissions::roles::Member;
use crate::permissions::visibility::{self, ServerVisibility};
use crate::state::AppState;
use crate::values::Id;
use stellarr_messages::{MessageKey, Messages};

pub(super) fn routes() -> Endpoints {
    target::<false>("channels").merge(target::<true>("sections"))
}

/// The routes of one kind of place: `SECTION` is true for sections and false
/// for channels.
fn target<const SECTION: bool>(collection: &str) -> Endpoints {
    let base = format!("/api/admin/{collection}/{{targetId}}/permissions");
    Endpoints::new()
        .get(&base, list::<SECTION>)
        .authorized()
        .put(&format!("{base}/{{roleId}}"), set::<SECTION>)
        .json_body()
        .authorized()
        .delete(&format!("{base}/{{roleId}}"), clear::<SECTION>)
        .authorized()
        .get(&format!("{base}/members"), list_members::<SECTION>)
        .authorized()
        .put(&format!("{base}/members/{{userId}}"), set_member::<SECTION>)
        .json_body()
        .authorized()
        .delete(&format!("{base}/members/{{userId}}"), clear_member::<SECTION>)
        .authorized()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OverrideRequest {
    allow: Option<Vec<String>>,
    deny: Option<Vec<String>>,
}

/// What one role is allowed and denied on top of its own permissions.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleOverrideDto {
    role_id: Id,
    allow: Vec<&'static str>,
    deny: Vec<&'static str>,
}

/// What one member is allowed and denied on top of what their roles give them
/// there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberOverrideDto {
    user_id: Id,
    allow: Vec<&'static str>,
    deny: Vec<&'static str>,
}

fn target_type(section: bool) -> i64 {
    i64::from(section)
}

fn scope(section: bool, server: Id, target: Id) -> Scope {
    if section {
        Scope::section(server, target)
    } else {
        Scope::channel(server, target)
    }
}

/// The answer for a request that may not go on: 404 for a channel or section
/// that is not on the server, or that the caller does not see, which is looked
/// for first, then 403 for a caller who may not manage roles.
fn refuse(
    db: &Connection,
    section: bool,
    caller: Caller,
    server: Id,
    target: Id,
) -> Result<(), ApiError> {
    let exists = if section {
        visibility::section_listed(db, caller.user, server, target)?
    } else {
        visibility::find(db, caller.user, server, target, None)?.is_some()
    };
    if !exists {
        return Err(ApiError::not_found());
    }
    if contains(authorization::server_wide(db, caller.user, server)?, MANAGE_ROLES) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}

/// The two masks of a request: known names, none both allowed and denied, and
/// never Administrator.
fn masks(request: &OverrideRequest, messages: &Messages) -> Result<(i64, i64), ApiError> {
    let parse = |names: &Option<Vec<String>>| {
        privileges::parse_names(names.iter().flatten().map(String::as_str))
    };
    let (Some(allow), Some(deny)) = (parse(&request.allow), parse(&request.deny)) else {
        return Err(invalid("permissions", messages.get(MessageKey::PermissionUnknown, &[])?));
    };
    if allow & deny != privileges::NONE {
        return Err(invalid(
            "permissions",
            messages.get(MessageKey::PermissionAllowedAndDenied, &[])?,
        ));
    }
    if (allow | deny) & ADMINISTRATOR != privileges::NONE {
        return Err(invalid(
            "permissions",
            messages.get(MessageKey::PermissionServerWideOnly, &[])?,
        ));
    }
    Ok((allow, deny))
}

fn target_name(db: &Connection, section: bool, target: Id) -> rusqlite::Result<String> {
    db.query_row(
        if section {
            "SELECT \"Name\" FROM \"Sections\" WHERE \"Id\" = ?1"
        } else {
            "SELECT \"Name\" FROM \"Channels\" WHERE \"Id\" = ?1"
        },
        [target],
        |row| row.get(0),
    )
}

fn mask_change(field: &str, before: i64, after: i64) -> Option<AuditChange> {
    let added: Vec<String> = privileges::to_names(after & !before).into_iter().map(str::to_string).collect();
    let removed: Vec<String> = privileges::to_names(before & !after).into_iter().map(str::to_string).collect();
    (!added.is_empty() || !removed.is_empty()).then(|| AuditChange {
        field: field.to_string(),
        added: Some(added),
        removed: Some(removed),
        ..AuditChange::default()
    })
}

/// Where a change is made and by whom, for the line it leaves in the audit log.
#[derive(Clone, Copy)]
struct Setting<'a> {
    messages: &'a Messages,
    caller: Caller,
    server: Id,
    section: bool,
    target: Id,
}

/// What an override held before and holds now.
struct Change {
    before: (i64, i64),
    after: (i64, i64),
}

/// A line for the audit log: which role, where, and what it was allowed and
/// denied.
fn record_role(db: &Connection, setting: &Setting<'_>, role: Id, change: Change) -> Result<(), ApiError> {
    let Setting { messages, caller, server, section, target } = *setting;
    if change.before == change.after {
        return Ok(());
    }
    let (role_name, role_color): (String, String) = db.query_row(
        "SELECT \"Name\", \"Color\" FROM \"ServerRoles\" WHERE \"Id\" = ?1",
        [role],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let name = target_name(db, section, target)?;
    audit::record(
        db,
        server,
        caller.user,
        &AuditEvent {
            action: if section { "section.permissions" } else { "channel.permissions" }.to_string(),
            sentence: messages.get(
                if section {
                    MessageKey::AuditSectionPermissions
                } else {
                    MessageKey::AuditChannelPermissions
                },
                &[("role", &role_name), ("name", &name)],
            )?,
            target_type: Some(if section { "section" } else { "channel" }.to_string()),
            target_id: Some(target),
            target_name: Some(name),
            changes: audit::only([
                Some(AuditChange {
                    field: "role".to_string(),
                    to: Some(role_name),
                    to_color: Some(role_color),
                    ..AuditChange::default()
                }),
                mask_change("allow", change.before.0, change.after.0),
                mask_change("deny", change.before.1, change.after.1),
            ]),
        },
    )?;
    Ok(())
}

/// A line for the audit log: which member, where, and what they were allowed
/// and denied.
fn record_member(db: &Connection, setting: &Setting<'_>, member: Id, change: Change) -> Result<(), ApiError> {
    let Setting { messages, caller, server, section, target } = *setting;
    if change.before == change.after {
        return Ok(());
    }
    let member_name: String = db.query_row(
        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
        [member],
        |row| row.get(0),
    )?;
    let name = target_name(db, section, target)?;
    audit::record(
        db,
        server,
        caller.user,
        &AuditEvent {
            action: if section {
                "section.permissions.member"
            } else {
                "channel.permissions.member"
            }
            .to_string(),
            sentence: messages.get(
                if section {
                    MessageKey::AuditSectionMemberPermissions
                } else {
                    MessageKey::AuditChannelMemberPermissions
                },
                &[("member", &member_name), ("name", &name)],
            )?,
            target_type: Some(if section { "section" } else { "channel" }.to_string()),
            target_id: Some(target),
            target_name: Some(name),
            changes: audit::only([
                Some(AuditChange {
                    field: "member".to_string(),
                    to: Some(member_name),
                    ..AuditChange::default()
                }),
                mask_change("allow", change.before.0, change.after.0),
                mask_change("deny", change.before.1, change.after.1),
            ]),
        },
    )?;
    Ok(())
}

async fn list<const SECTION: bool>(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(target): Path<Id>,
) -> ApiResult<Response> {
    let rows = state
        .database()
        .read(move |db| -> Result<Vec<RoleOverrideDto>, ApiError> {
            refuse(db, SECTION, caller, server, target)?;
            let mut statement = db.prepare(
                "SELECT \"Id\", \"AllowMask\", \"DenyMask\", \"RoleId\", \"ServerId\", \"TargetId\", \"TargetType\"
                 FROM \"PermissionOverrides\" WHERE \"TargetType\" = ?1 AND \"TargetId\" = ?2",
            )?;
            let rows = statement
                .query_map(params![target_type(SECTION), target], |row| {
                    Ok(RoleOverrideDto {
                        role_id: row.get(3)?,
                        allow: privileges::to_names(row.get(1)?),
                        deny: privileges::to_names(row.get(2)?),
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })
        .await?;
    Ok(json_response(StatusCode::OK, &rows))
}

async fn list_members<const SECTION: bool>(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(target): Path<Id>,
) -> ApiResult<Response> {
    let rows = state
        .database()
        .read(move |db| -> Result<Vec<MemberOverrideDto>, ApiError> {
            refuse(db, SECTION, caller, server, target)?;
            let mut statement = db.prepare(
                "SELECT \"Id\", \"AllowMask\", \"DenyMask\", \"ServerId\", \"TargetId\", \"TargetType\", \"UserId\"
                 FROM \"MemberPermissionOverrides\" WHERE \"ServerId\" = ?1 AND \"TargetType\" = ?2 AND \"TargetId\" = ?3",
            )?;
            let rows = statement
                .query_map(params![server, target_type(SECTION), target], |row| {
                    Ok(MemberOverrideDto {
                        user_id: row.get(6)?,
                        allow: privileges::to_names(row.get(1)?),
                        deny: privileges::to_names(row.get(2)?),
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })
        .await?;
    Ok(json_response(StatusCode::OK, &rows))
}

async fn set<const SECTION: bool>(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((target, role)): Path<(Id, Id)>,
    Json(request): Json<OverrideRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let (dto, announcement) = state
        .database()
        .write(move |tx| -> Result<(RoleOverrideDto, Announcement), ApiError> {
            refuse(tx, SECTION, caller, server, target)?;
            let before = ServerVisibility::load(tx, server)?;
            let known = tx
                .query_row(
                    "SELECT 1 FROM \"ServerRoles\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                    params![role, server],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !known {
                return Err(ApiError::not_found());
            }
            let (allow, deny) = masks(&request, &messages)?;
            if !authorization::can_change_override(
                tx,
                caller.user,
                &scope(SECTION, server, target),
                role,
                allow,
                deny,
            )? {
                return Err(ApiError::forbidden());
            }

            let current: Option<(Id, i64, i64)> = tx
                .query_row(
                    "SELECT \"Id\", \"AllowMask\", \"DenyMask\" FROM \"PermissionOverrides\"
                     WHERE \"TargetType\" = ?1 AND \"TargetId\" = ?2 AND \"RoleId\" = ?3",
                    params![target_type(SECTION), target, role],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            record_role(
                tx,
                &Setting { messages: &messages, caller, server, section: SECTION, target },
                role,
                Change {
                    before: current.map_or((0, 0), |(_, allow, deny)| (allow, deny)),
                    after: (allow, deny),
                },
            )?;
            if allow == privileges::NONE && deny == privileges::NONE {
                if let Some((id, _, _)) = current {
                    tx.execute("DELETE FROM \"PermissionOverrides\" WHERE \"Id\" = ?1", [id])?;
                }
            } else if let Some((id, _, _)) = current {
                tx.execute(
                    "UPDATE \"PermissionOverrides\" SET \"AllowMask\" = ?1, \"DenyMask\" = ?2 WHERE \"Id\" = ?3",
                    params![allow, deny, id],
                )?;
            } else {
                tx.execute(
                    "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\")
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![Id::random(), server, target_type(SECTION), target, role, allow, deny],
                )
                .map_err(taken)?;
            }
            let announcement = Announcement::views_after(tx, server, &before)?;
            Ok((
                RoleOverrideDto {
                    role_id: role,
                    allow: privileges::to_names(allow),
                    deny: privileges::to_names(deny),
                },
                announcement,
            ))
        })
        .await?;
    announcement.publish(&state, server);
    Ok(json_response(StatusCode::OK, &dto))
}

async fn clear<const SECTION: bool>(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((target, role)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let announcement = state
        .database()
        .write(move |tx| -> Result<Announcement, ApiError> {
            refuse(tx, SECTION, caller, server, target)?;
            let before = ServerVisibility::load(tx, server)?;
            let known = tx
                .query_row(
                    "SELECT 1 FROM \"ServerRoles\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                    params![role, server],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !known {
                return Err(ApiError::not_found());
            }
            if !authorization::can_change_override(
                tx,
                caller.user,
                &scope(SECTION, server, target),
                role,
                privileges::NONE,
                privileges::NONE,
            )? {
                return Err(ApiError::forbidden());
            }
            let cleared: Option<(Id, i64, i64)> = tx
                .query_row(
                    "SELECT \"Id\", \"AllowMask\", \"DenyMask\" FROM \"PermissionOverrides\"
                     WHERE \"TargetType\" = ?1 AND \"TargetId\" = ?2 AND \"RoleId\" = ?3",
                    params![target_type(SECTION), target, role],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((id, allow, deny)) = cleared {
                tx.execute("DELETE FROM \"PermissionOverrides\" WHERE \"Id\" = ?1", [id])?;
                record_role(
                tx,
                &Setting { messages: &messages, caller, server, section: SECTION, target },
                role,
                    Change {
                        before: (allow, deny),
                        after: (0, 0),
                    },
                )?;
            }
            Ok(Announcement::views_after(tx, server, &before)?)
        })
        .await?;
    announcement.publish(&state, server);
    Ok(no_content())
}

async fn set_member<const SECTION: bool>(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((target, user)): Path<(Id, Id)>,
    Json(request): Json<OverrideRequest>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let (dto, announcement) = state
        .database()
        .write(move |tx| -> Result<(MemberOverrideDto, Announcement), ApiError> {
            refuse(tx, SECTION, caller, server, target)?;
            let before = ServerVisibility::load(tx, server)?;
            let Some(member) = Member::load(tx, server, user)? else {
                return Err(ApiError::not_found());
            };
            let (allow, deny) = masks(&request, &messages)?;
            // The owner may do everything everywhere; an override would say
            // otherwise.
            if member.role.is_owner {
                return Err(ApiError::conflict());
            }
            if !authorization::can_change_member_override(
                tx,
                caller.user,
                &scope(SECTION, server, target),
                user,
                allow,
                deny,
            )? {
                return Err(ApiError::forbidden());
            }

            let current: Option<(Id, i64, i64)> = member_override(tx, server, SECTION, target, user)?;
            record_member(
                tx,
                &Setting { messages: &messages, caller, server, section: SECTION, target },
                user,
                Change {
                    before: current.map_or((0, 0), |(_, allow, deny)| (allow, deny)),
                    after: (allow, deny),
                },
            )?;
            if allow == privileges::NONE && deny == privileges::NONE {
                if let Some((id, _, _)) = current {
                    tx.execute("DELETE FROM \"MemberPermissionOverrides\" WHERE \"Id\" = ?1", [id])?;
                }
            } else if let Some((id, _, _)) = current {
                tx.execute(
                    "UPDATE \"MemberPermissionOverrides\" SET \"AllowMask\" = ?1, \"DenyMask\" = ?2 WHERE \"Id\" = ?3",
                    params![allow, deny, id],
                )?;
            } else {
                tx.execute(
                    "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\")
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![Id::random(), server, target_type(SECTION), target, user, allow, deny],
                )
                .map_err(taken)?;
            }
            let announcement = Announcement::views_after(tx, server, &before)?;
            Ok((
                MemberOverrideDto {
                    user_id: user,
                    allow: privileges::to_names(allow),
                    deny: privileges::to_names(deny),
                },
                announcement,
            ))
        })
        .await?;
    announcement.publish(&state, server);
    Ok(json_response(StatusCode::OK, &dto))
}

async fn clear_member<const SECTION: bool>(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((target, user)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let announcement = state
        .database()
        .write(move |tx| -> Result<Announcement, ApiError> {
            refuse(tx, SECTION, caller, server, target)?;
            let before = ServerVisibility::load(tx, server)?;
            let Some(member) = Member::load(tx, server, user)? else {
                return Err(ApiError::not_found());
            };
            if member.role.is_owner {
                return Err(ApiError::conflict());
            }
            if !authorization::can_change_member_override(
                tx,
                caller.user,
                &scope(SECTION, server, target),
                user,
                privileges::NONE,
                privileges::NONE,
            )? {
                return Err(ApiError::forbidden());
            }
            if let Some((id, allow, deny)) = member_override(tx, server, SECTION, target, user)? {
                tx.execute("DELETE FROM \"MemberPermissionOverrides\" WHERE \"Id\" = ?1", [id])?;
                record_member(
                tx,
                &Setting { messages: &messages, caller, server, section: SECTION, target },
                user,
                    Change {
                        before: (allow, deny),
                        after: (0, 0),
                    },
                )?;
            }
            Ok(Announcement::views_after(tx, server, &before)?)
        })
        .await?;
    announcement.publish(&state, server);
    Ok(no_content())
}

fn member_override(
    db: &Connection,
    server: Id,
    section: bool,
    target: Id,
    user: Id,
) -> rusqlite::Result<Option<(Id, i64, i64)>> {
    db.query_row(
        "SELECT \"Id\", \"AllowMask\", \"DenyMask\" FROM \"MemberPermissionOverrides\"
         WHERE \"ServerId\" = ?1 AND \"TargetType\" = ?2 AND \"TargetId\" = ?3 AND \"UserId\" = ?4",
        params![server, target_type(section), target, user],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
}
