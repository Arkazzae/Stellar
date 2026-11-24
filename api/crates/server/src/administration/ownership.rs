//! The two things only the owner of a server may do: hand it on, and delete
//! it. Neither can be undone.

use axum::extract::State;
use axum::http::StatusCode;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Json, Problem};
use crate::permissions::audit::{self, AuditEvent};
use crate::permissions::membership::{self, ActiveServer};
use crate::permissions::ownership;
use crate::permissions::roles::Member;
use crate::permissions::visibility::ServerVisibility;
use crate::state::AppState;
use crate::values::Id;

/// Who the server goes to. A body that names nobody names the empty id, which
/// is nobody's.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    user_id: Option<Id>,
}

/// Tells the people whose view of the server changed that what they see has.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessChanged {
    server_id: Id,
}

/// The owner sees every channel and the former owner may not, so whoever sees
/// something else afterwards is told, as after any change of roles.
///
/// Where the account's policy asks for a second step made in the last few
/// minutes, this route is to be guarded by it; that guard is not part of this
/// route yet.
pub async fn transfer(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<Transfer>,
) -> ApiResult<StatusCode> {
    // One change of who owns a server at a time: the slot is freed and then
    // given, and the database alone cannot keep two of them from interleaving.
    let _turn = state.mutation_gate().enter().await;
    let successor = request.user_id.unwrap_or(Id::from_bytes([0; 16]));
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<Vec<Id>, ApiError>, DbError> {
                let before = ServerVisibility::load(tx, server)?;
                let Some(owner) = Member::load(tx, server, caller.user)?.filter(Member::is_owner)
                else {
                    return Ok(Err(ApiError::forbidden()));
                };
                let target = Member::load(tx, server, successor)?.filter(|m| m.user != caller.user);
                let Some(target) = target else {
                    let message = state
                        .messages()
                        .get(MessageKey::TransferTargetInvalid, &[])
                        .map_err(|e| DbError::Task(e.to_string()))?;
                    return Ok(Err(
                        Problem::validation(vec![("userId".into(), vec![message])]).into(),
                    ));
                };
                ownership::hand_over(tx, server, &owner, &target)?;
                let name: String = tx.query_row(
                    "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                    [target.user],
                    |row| row.get(0),
                )?;
                audit::record(
                    tx,
                    server,
                    caller.user,
                    &AuditEvent {
                        action: "server.transfer".into(),
                        sentence: state
                            .messages()
                            .get(MessageKey::AuditOwnershipTransferredTo, &[("name", &name)])
                            .map_err(|e| DbError::Task(e.to_string()))?,
                        target_type: Some("member".into()),
                        target_id: Some(target.user),
                        target_name: Some(name),
                        changes: Vec::new(),
                    },
                )?;
                let after = ServerVisibility::load(tx, server)?;
                Ok(Ok(ServerVisibility::changed(tx, &before, &after)?))
            }
        })
        .await?;
    let changed = outcome?;
    if !changed.is_empty() {
        state.events().publish(Event::new(
            Audience::Followers {
                server,
                users: changed,
            },
            "ChannelAccessChanged",
            &AccessChanged { server_id: server },
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the server and everything it holds. The instance server cannot be
/// deleted at all: it carries the instance's settings, and owning it is what
/// opens the operator's panel. Without it, whoever owned the next server, or
/// created one, would run the instance.
///
/// Where the account's policy asks for a second step made in the last few
/// minutes, this route is to be guarded by it; that guard is not part of this
/// route yet.
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<StatusCode> {
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<(), ApiError>, DbError> {
                let owns: bool = tx
                    .query_row(
                        "SELECT r.\"IsOwner\" FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
                         WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2",
                        params![server, caller.user],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(false);
                if !owns {
                    return Ok(Err(ApiError::forbidden()));
                }
                if membership::find_administered_server(tx, caller.user)? == Some(server) {
                    let title = state
                        .messages()
                        .get(MessageKey::InstanceServerCannotBeDeleted, &[])
                        .map_err(|e| DbError::Task(e.to_string()))?;
                    return Ok(Err(Problem::new(StatusCode::CONFLICT).title(title).into()));
                }
                if ownership::delete_server(tx, server)? {
                    Ok(Ok(()))
                } else {
                    Ok(Err(ApiError::not_found()))
                }
            }
        })
        .await?;
    outcome?;
    Ok(StatusCode::NO_CONTENT)
}
