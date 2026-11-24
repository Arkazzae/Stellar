//! Every server of the instance, whoever owns it: who is in it, handing it
//! to another of its members, and deleting it. The instance server is the
//! owner's own and is neither handed on nor deleted from here.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use super::{Plain, invalid, no_content, nobody, bind};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Path, Problem, json_response};
use crate::panel::{self, FreshInstanceOwner};
use crate::permissions::membership::InstanceOwner;
use crate::permissions::access_changed::{self, ViewChange};
use crate::permissions::ownership;
use crate::permissions::roles::Member;
use crate::state::AppState;
use crate::values::{Guid, Id};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Person {
    id: Id,
    username: String,
    display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerDto {
    id: Id,
    name: String,
    description: String,
    is_public: bool,
    created_at: Plain,
    is_instance: bool,
    owner: Option<Person>,
    member_count: i64,
    channel_count: i64,
}

/// Every server of the instance, oldest first, the instance server among them.
pub(super) async fn list(
    State(state): State<AppState>,
    InstanceOwner(instance): InstanceOwner,
) -> ApiResult<Response> {
    let answer = state
        .database()
        .read(move |db| -> Result<Vec<ServerDto>, DbError> {
            let mut statement = db.prepare(
                "SELECT s.\"Id\", s.\"Name\", s.\"Description\", s.\"IsPublic\", s.\"CreatedAt\",
                    (SELECT COUNT(*) FROM \"ServerMembers\" m WHERE m.\"ServerId\" = s.\"Id\"),
                    (SELECT COUNT(*) FROM \"Channels\" c WHERE c.\"ServerId\" = s.\"Id\")
                 FROM \"Servers\" s ORDER BY s.\"CreatedAt\", s.\"Id\"",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok(ServerDto {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        is_public: row.get(3)?,
                        created_at: Plain(row.get(4)?),
                        is_instance: false,
                        owner: None,
                        member_count: row.get(5)?,
                        channel_count: row.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut owner = db.prepare(
                "SELECT u.\"Id\", u.\"Username\", u.\"DisplayName\" FROM \"ServerMembers\" m
                 JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
                 WHERE m.\"ServerId\" = ?1 AND r.\"IsOwner\" = 1 LIMIT 1",
            )?;
            rows.into_iter()
                .map(|mut server| {
                    server.is_instance = server.id == instance;
                    server.owner = owner
                        .query_row([server.id], |row| {
                            Ok(Person {
                                id: row.get(0)?,
                                username: row.get(1)?,
                                display_name: row.get(2)?,
                            })
                        })
                        .optional()?;
                    Ok(server)
                })
                .collect()
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberDto {
    id: Id,
    username: String,
    display_name: String,
    is_owner: bool,
}

/// The members of one server, the people its ownership can go to.
pub(super) async fn members(
    State(state): State<AppState>,
    Path(Guid(server)): Path<Guid>,
    InstanceOwner(_): InstanceOwner,
) -> ApiResult<Response> {
    let answer = state
        .database()
        .read(move |db| -> Result<Option<Vec<MemberDto>>, DbError> {
            let there: bool = db.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Servers\" WHERE \"Id\" = ?1)",
                [server],
                |row| row.get(0),
            )?;
            if !there {
                return Ok(None);
            }
            let mut statement = db.prepare(
                "SELECT u.\"Id\", u.\"Username\", u.\"DisplayName\", r.\"IsOwner\" FROM \"ServerMembers\" m
                 JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\" JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
                 WHERE m.\"ServerId\" = ?1 ORDER BY u.\"DisplayName\", u.\"Username\"",
            )?;
            let rows = statement
                .query_map([server], |row| {
                    Ok(MemberDto {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        display_name: row.get(2)?,
                        is_owner: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some(rows))
        })
        .await?;
    answer
        .map(|members| json_response(StatusCode::OK, &members))
        .ok_or_else(ApiError::not_found)
}

/// The refusal of what the instance server cannot be: handed on would lock
/// the operator out of the panel.
fn is_instance(state: &AppState) -> Result<ApiError, stellarr_messages::MissingValue> {
    Ok(Problem::new(StatusCode::CONFLICT)
        .title(state.messages().get(MessageKey::CockpitServerIsInstance, &[])?)
        .into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Successor {
    #[serde(default = "nobody")]
    user_id: Id,
}

/// Hands a server to another of its members.
pub(super) async fn transfer(
    State(state): State<AppState>,
    Path(Guid(served)): Path<Guid>,
    request: Request,
) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server: instance,
        body,
    } = panel::bind_then_gate(&state, request, true, bind::<Successor>).await?;
    let Some(body) = body else {
        return Err(ApiError::bad_request());
    };
    if served == instance {
        return Err(is_instance(&state)?);
    }
    // One change of who owns a server at a time: the slot is freed and then
    // given, and the database alone cannot keep two of them from interleaving.
    let _turn = state.mutation_gate().enter().await;
    let messages = *state.messages();
    let successor = body.user_id;
    let changed = state
        .database()
        .write(move |tx| -> Result<Vec<Id>, ApiError> {
            let name: Option<String> = tx
                .query_row(
                    "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                    [served],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(name) = name else {
                return Err(ApiError::not_found());
            };
            let owner: Option<Id> = tx
                .query_row(
                    "SELECT m.\"UserId\" FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
                     WHERE m.\"ServerId\" = ?1 AND r.\"IsOwner\" = 1 LIMIT 1",
                    [served],
                    |row| row.get(0),
                )
                .optional()?;
            let owner = match owner {
                Some(user) => Member::load(tx, served, user)?,
                None => None,
            };
            let target = Member::load(tx, served, successor)?;
            let (Some(owner), Some(target)) = (owner, target) else {
                return Err(invalid(
                    "userId",
                    messages.get(MessageKey::TransferTargetInvalid, &[])?,
                ));
            };
            if target.user == owner.user {
                return Err(invalid(
                    "userId",
                    messages.get(MessageKey::TransferTargetInvalid, &[])?,
                ));
            }
            // The owner sees every channel and the former owner may not.
            let view = ViewChange::begin(tx, served)?;
            ownership::hand_over(tx, served, &owner, &target)?;
            let target_name: String = tx.query_row(
                "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                [target.user],
                |row| row.get(0),
            )?;
            panel::record(
                tx,
                served,
                caller.user,
                messages.get(MessageKey::AuditOwnershipTransferred, &[])?,
            )?;
            panel::record(
                tx,
                instance,
                caller.user,
                messages.get(
                    MessageKey::CockpitAuditServerTransferred,
                    &[("server", &name), ("name", &target_name)],
                )?,
            )?;
            Ok(view.finish(tx)?)
        })
        .await?;
    access_changed::publish(state.events(), served, changed);
    Ok(no_content())
}

/// Deletes a server other than the instance server, and notes it in the
/// instance's audit log.
pub(super) async fn delete(
    State(state): State<AppState>,
    Path(Guid(served)): Path<Guid>,
    FreshInstanceOwner(instance): FreshInstanceOwner,
    caller: crate::auth::Caller,
) -> ApiResult<Response> {
    if served == instance {
        return Err(is_instance(&state)?);
    }
    let _turn = state.mutation_gate().enter().await;
    let messages = *state.messages();
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            let name: Option<String> = tx
                .query_row(
                    "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                    [served],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(name) = name else {
                return Err(ApiError::not_found());
            };
            if !ownership::delete_server(tx, served)? {
                return Err(ApiError::not_found());
            }
            panel::record(
                tx,
                instance,
                caller.user,
                messages.get(MessageKey::CockpitAuditServerDeleted, &[("name", &name)])?,
            )?;
            Ok(())
        })
        .await?;
    Ok(no_content())
}
