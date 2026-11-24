//! The members of a server and the roles each holds: listing them, giving one
//! member exactly one role, and setting the whole list of a member's roles.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::announce::Announcement;
use super::{Chosen, Guid, RoleRow, nil, no_content, no_member_role};
use crate::auth::Caller;
use crate::http::{ApiError, ApiResult, Json, Path, json_response};
use crate::instance::users::{User, UserDto};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization;
use crate::permissions::membership::ActiveServer;
use crate::permissions::ownership;
use crate::permissions::privileges::{MANAGE_MEMBERS, contains};
use crate::permissions::roles::{self, HeldRoleTable, Member, Role};
use crate::permissions::access_changed::ViewChange;
use crate::state::AppState;
use crate::values::{Id, Timestamp};
use stellarr_messages::MessageKey;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssignRequest {
    /// Left out, it is the empty id, which no role has.
    #[serde(default = "nil")]
    role_id: Id,
}

/// A member of the server. `role_id` is the role that ranks them, the highest
/// they hold or the base role; `role_ids` every role they hold besides the
/// base role, highest first.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberDto {
    user: UserDto,
    role_id: Id,
    joined_at: String,
    role_ids: Vec<Id>,
}

/// A member as read: the account, the role that ranks them, when they joined
/// and the roles they hold.
type MemberRow = (User, Id, Timestamp, Vec<Id>);

pub(super) async fn list(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let rows = state
        .database()
        .read(move |db| -> Result<Vec<MemberRow>, ApiError> {
            let mut statement = db.prepare(
                "SELECT m.\"UserId\", m.\"RoleId\", m.\"JoinedAt\" FROM \"ServerMembers\" m
                 INNER JOIN \"Users\" u ON m.\"UserId\" = u.\"Id\"
                 WHERE m.\"ServerId\" = ?1 ORDER BY u.\"DisplayName\"",
            )?;
            let members: Vec<(Id, Id, Timestamp)> = statement
                .query_map([server], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let table = HeldRoleTable::load(db, server)?;
            let mut rows = Vec::with_capacity(members.len());
            for (user, role, joined) in members {
                let account = User::find(db, user)?
                    .ok_or_else(|| ApiError::internal("a member has no account"))?;
                let held = table.of(user).iter().map(|role| role.id).collect();
                rows.push((account, role, joined, held));
            }
            Ok(rows)
        })
        .await?;
    let members: Vec<MemberDto> = rows
        .into_iter()
        .map(|(user, role_id, joined, role_ids)| MemberDto {
            user: user.dto(&state.presence().public_presence_of(user.id, Some(caller.user))),
            role_id,
            joined_at: joined.to_wire_text(),
            role_ids,
        })
        .collect();
    Ok(json_response(StatusCode::OK, &members))
}

/// The roles in one list and not the other, either way.
fn diff(before: &[Role], after: &[Role]) -> Vec<Role> {
    before
        .iter()
        .filter(|role| after.iter().all(|kept| kept.id != role.id))
        .chain(
            after
                .iter()
                .filter(|role| before.iter().all(|held| held.id != role.id)),
        )
        .copied()
        .collect()
}

fn display_name(db: &Connection, user: Id) -> rusqlite::Result<String> {
    db.query_row(
        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
        [user],
        |row| row.get(0),
    )
}

/// Everything the two routes read before they decide: the caller may manage
/// members and is not the member.
fn may_manage(db: &Connection, caller: Caller, server: Id, user: Id) -> Result<(), ApiError> {
    let held = authorization::server_wide(db, caller.user, server)?;
    if !contains(held, MANAGE_MEMBERS) || caller.user == user {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

/// Gives the member exactly one role, or none besides the base role when the
/// base role is named: what the role editor of a client that knows one role
/// per member asks for.
pub(super) async fn assign(
    State(state): State<AppState>,
    caller: Caller,
    Path(Guid(user)): Path<Guid>,
    chosen: Chosen,
    Json(request): Json<AssignRequest>,
) -> ApiResult<Response> {
    let server = chosen.server()?;
    // Before any other refusal, as the first server's filter was, but after the
    // person is known to be a member and the body has been read.
    super::require_fresh(&state, caller, server).await?;
    let messages = *state.messages();
    let announcement = state
        .database()
        .write(move |tx| -> Result<Announcement, ApiError> {
            may_manage(tx, caller, server, user)?;
            let member = Member::load(tx, server, user)?;
            let role = RoleRow::find(tx, server, request.role_id)?;
            let (Some(member), Some(role)) = (member, role) else {
                return Err(ApiError::not_found());
            };
            // The owner is neither demoted nor joined by anybody: handing over
            // the server is a transfer of its own.
            if member.role.is_owner || role.role.is_owner {
                return Err(ApiError::conflict());
            }
            let (current, base) = roles::held_of_member(tx, &member)?;
            let Some(base) = base else {
                return Err(no_member_role(&messages)?);
            };
            let wanted: Vec<Role> = if role.role.id == base.id { vec![] } else { vec![role.role] };
            // As when every member held one role, the role named and the one
            // the member ranks by are checked even when they stay.
            let mut checked: Vec<Role> = Vec::new();
            for one in diff(&current, &wanted)
                .into_iter()
                .chain([member.role, role.role])
            {
                if checked.iter().all(|seen| seen.id != one.id) {
                    checked.push(one);
                }
            }
            if !authorization::can_change_roles(tx, caller.user, &member, &checked)? {
                return Err(ApiError::forbidden());
            }

            let rows: HashMap<Id, RoleRow> = RoleRow::all(tx, server)?
                .into_iter()
                .map(|row| (row.role.id, row))
                .collect();
            let held_name = |id: Id| rows.get(&id).map(|row| row.name.clone()).unwrap_or_default();
            let top = rows.get(&member.role.id);
            let removed_besides_top: Vec<String> = current
                .iter()
                .filter(|held| held.id != member.role.id && held.id != role.role.id)
                .map(|held| held_name(held.id))
                .collect();
            if member.role.id != role.role.id || !removed_besides_top.is_empty() {
                let name = display_name(tx, user)?;
                audit::record(
                    tx,
                    server,
                    caller.user,
                    &AuditEvent {
                        action: "member.role".to_string(),
                        sentence: messages.get(
                            MessageKey::AuditRoleChanged,
                            &[("name", &name), ("role", &role.name)],
                        )?,
                        target_type: Some("member".to_string()),
                        target_id: Some(user),
                        target_name: Some(name),
                        changes: audit::only([
                            (member.role.id != role.role.id).then(|| AuditChange {
                                field: "role".to_string(),
                                from: top.map(|row| row.name.clone()),
                                to: Some(role.name.clone()),
                                from_color: top.map(|row| row.color.clone()),
                                to_color: Some(role.color.clone()),
                                ..AuditChange::default()
                            }),
                            (!removed_besides_top.is_empty()).then(|| AuditChange {
                                field: "roles".to_string(),
                                removed: Some(removed_besides_top),
                                ..AuditChange::default()
                            }),
                        ]),
                    },
                )?;
            }

            let before = ViewChange::begin(tx, server)?;
            ownership::set_roles(tx, server, user, &wanted, &base)?;
            Ok(Announcement::after(tx, server, &before, &[user])?)
        })
        .await?;
    announcement.publish(&state, server).await;
    Ok(no_content())
}

