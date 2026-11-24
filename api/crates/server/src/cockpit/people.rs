//! The accounts and roles of the instance server: who they are, making an
//! account, changing a person's role or status, and the roles themselves.

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::{MessageKey, Messages};

use super::{Plain, invalid, length, no_content, nobody, bind, taken};
use crate::auth::password;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Path, Problem, json_response};
use crate::panel::{self};
use crate::permissions::membership::InstanceOwner;
use crate::permissions::access_changed::{self, ViewChange};
use crate::permissions::ownership;
use crate::permissions::privileges;
use crate::permissions::roles::{self, HeldRoleTable, Role};
use crate::state::AppState;
use crate::values::{Guid, Id, Timestamp};

const MAX_ROLE_NAME_LENGTH: usize = 64;
const MAX_ROLE_COLOR_LENGTH: usize = 48;

/// A role as the panel lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RoleDto {
    id: Id,
    name: String,
    color: String,
    permissions: Vec<&'static str>,
    member_count: usize,
    is_system: bool,
    is_owner: bool,
}

/// A role with the two columns the rules do not read.
struct RoleRow {
    role: Role,
    name: String,
    color: String,
}

const ROLE_COLUMNS: &str =
    "\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\"";

impl RoleRow {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoleRow> {
        Ok(RoleRow {
            role: Role {
                id: row.get(0)?,
                server: row.get(1)?,
                permissions: row.get(4)?,
                position: row.get(5)?,
                is_system: row.get(6)?,
                is_owner: row.get(7)?,
            },
            name: row.get(2)?,
            color: row.get(3)?,
        })
    }

    fn find(db: &Connection, server: Id, id: Id) -> rusqlite::Result<Option<RoleRow>> {
        db.query_row(
            &format!("SELECT {ROLE_COLUMNS} FROM \"ServerRoles\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2"),
            params![id, server],
            RoleRow::read,
        )
        .optional()
    }

    fn dto(&self, member_count: usize) -> RoleDto {
        RoleDto {
            id: self.role.id,
            name: self.name.clone(),
            color: self.color.clone(),
            permissions: privileges::to_names(self.role.permissions),
            member_count,
            is_system: self.role.is_system,
            is_owner: self.role.is_owner,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserDto {
    id: Id,
    username: String,
    display_name: String,
    email: Option<String>,
    avatar_color: String,
    avatar_url: Option<String>,
    presence: String,
    realtime_connections: usize,
    role_id: Id,
    role_name: String,
    is_disabled: bool,
    joined_at: Plain,
    created_at: Plain,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UsersDto {
    users: Vec<UserDto>,
    roles: Vec<RoleDto>,
    session_registry_available: bool,
}

pub(super) async fn list(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<UsersDto, DbError> {
            let mut statement = db.prepare(
                "SELECT u.\"Id\", u.\"Username\", u.\"DisplayName\", u.\"Email\", u.\"AvatarColor\", u.\"AvatarUrl\", m.\"RoleId\",
                        r.\"Name\", u.\"IsDisabled\", m.\"JoinedAt\", u.\"CreatedAt\"
                 FROM \"ServerMembers\" m
                 JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                 JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
                 WHERE m.\"ServerId\" = ?1 ORDER BY u.\"DisplayName\"",
            )?;
            let users = statement
                .query_map([server], |row| {
                    let id: Id = row.get(0)?;
                    Ok(UserDto {
                        id,
                        username: row.get(1)?,
                        display_name: row.get(2)?,
                        email: row.get(3)?,
                        avatar_color: row.get(4)?,
                        avatar_url: row.get(5)?,
                        presence: reading.presence().public_presence_of(id, None),
                        realtime_connections: reading.presence().connection_ids(id).len(),
                        role_id: row.get(6)?,
                        role_name: row.get(7)?,
                        is_disabled: row.get(8)?,
                        joined_at: Plain(row.get(9)?),
                        created_at: Plain(row.get(10)?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = db.prepare(&format!(
                "SELECT {ROLE_COLUMNS} FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\""
            ))?;
            let rows = statement
                .query_map([server], RoleRow::read)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let held = HeldRoleTable::load(db, server)?;
            Ok(UsersDto {
                users,
                roles: rows
                    .iter()
                    .map(|row| row.dto(held.count_holders(row.role.id)))
                    .collect(),
                session_registry_available: true,
            })
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateUser {
    username: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    password: Option<String>,
    #[serde(default = "nobody")]
    role_id: Id,
}

/// A user name's characters, one UTF-16 unit at a time as the first server
/// read them: a letter or a digit, or one of three marks. A character outside
/// the basic plane is two units, neither of which is a letter.
fn is_username(text: &str) -> bool {
    (2..=32).contains(&length(text))
        && text.encode_utf16().all(|unit| {
            char::from_u32(u32::from(unit)).is_some_and(|c| {
                stellarr_unicode::is_letter_or_digit(c) || matches!(c, '_' | '.' | '-')
            })
        })
}

/// The hue of a new account's colour, from its id: the first server's hash
/// of an identifier, taken modulo 360. Accounts made before keep what this
/// gave them.
fn avatar_hue(id: Id) -> u32 {
    let bytes = id.as_bytes();
    let word = |from: usize| {
        u32::from_le_bytes([bytes[from], bytes[from + 1], bytes[from + 2], bytes[from + 3]])
    };
    let first = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let second = u32::from(u16::from_be_bytes([bytes[4], bytes[5]]))
        | u32::from(u16::from_be_bytes([bytes[6], bytes[7]])) << 16;
    (first ^ second ^ word(8) ^ word(12)) % 360
}

pub(super) async fn create(
    State(state): State<AppState>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<CreateUser>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    let username = body.username.as_deref().unwrap_or("").trim().to_string();
    let display_name = body.display_name.as_deref().unwrap_or("").trim().to_string();
    let email = body
        .email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty())
        .map(str::to_string);
    let valid = is_username(&username)
        && (1..=64).contains(&length(&display_name))
        && !password::is_too_short(body.password.as_deref())
        && !password::is_too_long(body.password.as_deref())
        && email
            .as_deref()
            .is_none_or(|email| super::address::try_create(email).is_some());
    if !valid {
        return Err(Problem::validation(vec![(
            "user".into(),
            vec![messages.get(MessageKey::CockpitUserInvalid, &[])?],
        )])
        .into());
    }
    let normalized_username = crate::instance::names::to_upper(&username);
    let normalized_email = email.as_deref().map(crate::instance::names::to_upper);
    let given = body.password.unwrap_or_default();
    let hashed = tokio::task::spawn_blocking(move || password::hash(&given))
        .await
        .map_err(ApiError::internal)?;
    let role_id = body.role_id;
    let id = Id::random();
    let written = {
        let display_name = display_name.clone();
        state
            .database()
            .write(move |tx| -> Result<(), ApiError> {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM \"Users\" WHERE \"NormalizedUsername\" = ?1 OR (?2 IS NOT NULL AND \"NormalizedEmail\" = ?2))",
                    params![normalized_username, normalized_email],
                    |row| row.get(0),
                )?;
                if exists {
                    return Err(ApiError::conflict());
                }
                let role: Option<Id> = tx
                    .query_row(
                        "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2 AND \"IsOwner\" = 0",
                        params![role_id, server],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(role) = role else {
                    return Err(invalid(
                        "roleId",
                        messages.get(MessageKey::CockpitRoleMustNotBeOwner, &[])?,
                    ));
                };
                let base = roles::find_member_role(tx, server)?;
                let now = Timestamp::now();
                tx.execute(
                    "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\", \"NormalizedEmail\",
                         \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8, ?9)",
                    params![
                        id,
                        username,
                        normalized_username,
                        display_name,
                        hashed,
                        email,
                        normalized_email,
                        format!("oklch(0.58 0.14 {})", avatar_hue(id)),
                        now
                    ],
                )
                .map_err(taken)?;
                ownership::admit(tx, server, id, role, Some(role) == base.map(|b| b.id), None, now)?;
                panel::record(
                    tx,
                    server,
                    caller.user,
                    messages.get(MessageKey::AuditAccountCreated, &[("name", &display_name)])?,
                )?;
                Ok(())
            })
            .await
    };
    written?;
    let mut response = json_response(StatusCode::CREATED, &serde_json::json!({ "id": id.to_string() }));
    if let Ok(location) = HeaderValue::from_str(&format!("/api/cockpit/users/{id}")) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleChoice {
    #[serde(default = "nobody")]
    role_id: Id,
}

/// Tells each of `users` who is a member what they now hold server-wide.
fn notify(state: &AppState, server: Id, users: &[Id], held: &HeldRoleTable) {
    let members: std::collections::HashSet<Id> = held.members().collect();
    let mut seen = std::collections::HashSet::new();
    for user in users {
        if seen.insert(*user) && members.contains(user) {
            state.events().publish(Event::new(
                Audience::Users(vec![*user]),
                "PermissionsChanged",
                &PermissionsChanged {
                    server_id: server,
                    permissions: privileges::to_names(held.permissions_of(*user)),
                },
            ));
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionsChanged {
    server_id: Id,
    permissions: Vec<&'static str>,
}

pub(super) async fn set_role(
    State(state): State<AppState>,
    Path(Guid(user)): Path<Guid>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<RoleChoice>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    if user == caller.user {
        return Err(ApiError::conflict());
    }
    let messages = *state.messages();
    let wanted = body.role_id;
    let done = state
        .database()
        .write(move |tx| -> Result<(Vec<Id>, HeldRoleTable), ApiError> {
            let member = roles::Member::load(tx, server, user)?;
            let role = RoleRow::find(tx, server, wanted)?.filter(|row| !row.role.is_owner);
            let (Some(member), Some(role)) = (member, role) else {
                return Err(ApiError::not_found());
            };
            if member.is_owner() {
                return Err(ApiError::conflict());
            }
            let Some(base) = roles::find_member_role(tx, server)? else {
                return Err(Problem::new(StatusCode::SERVICE_UNAVAILABLE)
                    .title(messages.get(MessageKey::ServerHasNoMemberRole, &[])?)
                    .into());
            };
            // Exactly this role, or the base role alone, as in the role
            // editor of a client that knows one role per member.
            let view = ViewChange::begin(tx, server)?;
            ownership::set_roles(tx, server, user, &[role.role], &base)?;
            let name: String = tx.query_row(
                "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| row.get(0),
            )?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(
                    MessageKey::AuditRoleChanged,
                    &[("name", &name), ("role", &role.name)],
                )?,
            )?;
            let changed = view.finish(tx)?;
            Ok((changed, HeldRoleTable::load(tx, server)?))
        })
        .await?;
    let (changed, held) = done;
    access_changed::publish(state.events(), server, changed);
    notify(&state, server, &[user], &held);
    Ok(no_content())
}

#[derive(Deserialize)]
struct StatusChoice {
    #[serde(default)]
    disabled: bool,
}

pub(super) async fn set_status(
    State(state): State<AppState>,
    Path(Guid(user)): Path<Guid>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<StatusChoice>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    if user == caller.user {
        return Err(Problem::new(StatusCode::CONFLICT)
            .title(messages.get(MessageKey::CockpitCannotDisableSelf, &[])?)
            .into());
    }
    let disabled = body.disabled;
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            let found: Option<(String, bool, bool)> = tx
                .query_row(
                    "SELECT u.\"DisplayName\", u.\"IsDisabled\", r.\"IsOwner\" FROM \"ServerMembers\" m
                     JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\" JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
                     WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2",
                    params![server, user],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((name, was_disabled, owner)) = found else {
                return Err(ApiError::not_found());
            };
            if owner {
                return Err(Problem::new(StatusCode::CONFLICT)
                    .title(messages.get(MessageKey::CockpitCannotDisableOwner, &[])?)
                    .into());
            }
            if was_disabled == disabled {
                return Ok(());
            }
            // A new session version turns every cookie of the account away,
            // and the revoked rows keep the session list honest about it.
            tx.execute(
                "UPDATE \"Users\" SET \"IsDisabled\" = ?2, \"SessionVersion\" = \"SessionVersion\" + 1 WHERE \"Id\" = ?1",
                params![user, disabled],
            )?;
            if disabled {
                tx.execute(
                    "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
                    params![user, Timestamp::now()],
                )?;
            }
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(
                    if disabled {
                        MessageKey::AuditAccountDisabled
                    } else {
                        MessageKey::AuditAccountEnabled
                    },
                    &[("name", &name)],
                )?,
            )?;
            Ok(())
        })
        .await?;
    Ok(no_content())
}

#[derive(Deserialize)]
struct RoleRequest {
    name: Option<String>,
    color: Option<String>,
    permissions: Option<Vec<Option<String>>>,
}

/// A role as a request may make it: name and colour trimmed and within their
/// lengths, and every permission named one that exists.
struct Draft {
    name: String,
    color: String,
    permissions: i64,
}

fn validate(name: &str, color: &str, permissions: &[Option<String>]) -> Option<Draft> {
    let (name, color) = (name.trim(), color.trim());
    // A `null` among the names is a name that is none of them.
    let permissions = privileges::parse_names(
        permissions
            .iter()
            .map(|name| name.as_deref().unwrap_or("\u{0}")),
    )?;
    ((1..=MAX_ROLE_NAME_LENGTH).contains(&length(name))
        && (1..=MAX_ROLE_COLOR_LENGTH).contains(&length(color)))
    .then(|| Draft {
        name: name.to_string(),
        color: color.to_string(),
        permissions,
    })
}

fn invalid_role(messages: &Messages) -> Result<ApiError, stellarr_messages::MissingValue> {
    Ok(invalid("role", messages.get(MessageKey::RoleInvalid, &[])?))
}

pub(super) async fn create_role(
    State(state): State<AppState>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<RoleRequest>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    let created = state
        .database()
        .write(move |tx| -> Result<RoleDto, ApiError> {
            let Some(draft) = validate(
                body.name.as_deref().unwrap_or(""),
                body.color.as_deref().unwrap_or(""),
                body.permissions.as_deref().unwrap_or_default(),
            ) else {
                return Err(invalid_role(&messages)?);
            };
            let used: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"Name\" = ?2)",
                params![server, draft.name],
                |row| row.get(0),
            )?;
            if used {
                return Err(ApiError::conflict());
            }
            let id = Id::random();
            let position = ownership::place_new_role(tx, server)?;
            tx.execute(
                "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0)",
                params![id, server, draft.name, draft.color, draft.permissions, position],
            )
            .map_err(taken)?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(MessageKey::AuditRoleCreated, &[("name", &draft.name)])?,
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
    let mut response = json_response(StatusCode::CREATED, &created);
    if let Ok(location) = HeaderValue::from_str(&format!("/api/cockpit/roles/{}", created.id)) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

pub(super) async fn update_role(
    State(state): State<AppState>,
    Path(Guid(role_id)): Path<Guid>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<RoleRequest>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    let (dto, changed, holders, held) = state
        .database()
        .write(move |tx| -> Result<_, ApiError> {
            let Some(row) = RoleRow::find(tx, server, role_id)? else {
                return Err(ApiError::not_found());
            };
            if row.role.is_owner {
                return Err(ApiError::conflict());
            }
            let current: Vec<Option<String>> = privileges::to_names(row.role.permissions)
                .into_iter()
                .map(|name| Some(name.to_string()))
                .collect();
            let Some(draft) = validate(
                body.name.as_deref().unwrap_or(&row.name),
                body.color.as_deref().unwrap_or(&row.color),
                body.permissions.as_deref().unwrap_or(&current),
            ) else {
                return Err(invalid_role(&messages)?);
            };
            let used: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"Id\" != ?2 AND \"Name\" = ?3)",
                params![server, role_id, draft.name],
                |row| row.get(0),
            )?;
            if used {
                return Err(ApiError::conflict());
            }
            let view = ViewChange::begin(tx, server)?;
            tx.execute(
                "UPDATE \"ServerRoles\" SET \"Name\" = ?1, \"Color\" = ?2, \"Permissions\" = ?3 WHERE \"Id\" = ?4",
                params![draft.name, draft.color, draft.permissions, role_id],
            )
            .map_err(taken)?;
            panel::record(
                tx,
                server,
                caller.user,
                messages.get(MessageKey::AuditRoleUpdated, &[("name", &draft.name)])?,
            )?;
            let changed = view.finish(tx)?;
            let held = HeldRoleTable::load(tx, server)?;
            let updated = RoleRow {
                role: Role {
                    permissions: draft.permissions,
                    ..row.role
                },
                name: draft.name,
                color: draft.color,
            };
            let dto = updated.dto(held.count_holders(role_id));
            let holders = held.holders(role_id);
            Ok((dto, changed, holders, held))
        })
        .await?;
    access_changed::publish(state.events(), server, changed);
    notify(&state, server, &holders, &held);
    Ok(json_response(StatusCode::OK, &dto))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_name_is_two_to_thirty_two_units_of_letters_digits_and_three_marks() {
        for good in ["ab", "Maya.Lin", "a_b-c", "zażółć", "٣٤", "日本"] {
            assert!(is_username(good), "{good}");
        }
        for bad in ["a", "", &"a".repeat(33), "a b", "a@b", "a+b", "\u{1d4d0}\u{1d4d1}", "a\u{301}b"] {
            assert!(!is_username(bad), "{bad}");
        }
        // A letter outside the basic plane is two units that are not letters.
        assert!(!is_username("\u{10400}\u{10401}"));
    }

    #[test]
    fn the_hue_is_the_hash_code_of_the_guid() {
        for (id, hue) in [
            ("2bb89c37-c5f0-436a-84b7-a23c5b25ee9e", 272),
            ("049083c7-a28f-4b91-b21b-2a0920dc1686", 146),
        ] {
            assert_eq!(avatar_hue(Id::parse(id).unwrap()), hue, "{id}");
        }
    }
}
