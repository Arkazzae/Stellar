//! The two things only the owner of a server may do: hand it on, and delete
//! it. Neither can be undone.

use axum::extract::State;
use axum::http::StatusCode;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use stellarr_messages::MessageKey;

use crate::auth::{Caller, fresh};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Problem};
use crate::permissions::audit::{self, AuditEvent};
use crate::permissions::membership::{self, ActiveServer};
use crate::permissions::ownership;
use crate::permissions::roles::Member;
use crate::permissions::access_changed::{self, ViewChange};
use crate::servers::Object;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// Refuses, with the problem the clients answer by asking for the proof, a
/// session that has not passed the account's second step lately.
async fn require_fresh(state: &AppState, caller: Caller) -> ApiResult<()> {
    let fresh = state
        .database()
        .read(move |db| -> Result<bool, DbError> {
            Ok(fresh::has_fresh_second_step(
                db,
                caller.user,
                Some(caller.session),
                Timestamp::now(),
            )?)
        })
        .await?;
    if fresh {
        Ok(())
    } else {
        Err(fresh::refusal(state.messages())?.into())
    }
}

/// Who the server goes to. A body that names nobody names the empty id, which
/// is nobody's.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    #[serde(default = "nobody")]
    user_id: Id,
}

fn nobody() -> Id {
    Id::from_bytes([0; 16])
}

/// The owner sees every channel and the former owner may not, so whoever sees
/// something else afterwards is told, as after any change of roles.
///
/// An account with a second step must have passed it in this session in the
/// last few minutes. The check comes after the person is known to be a member
/// of the server and before any other refusal, as the first server's filter
/// did, so that a caller who fails two checks is told of the second step.
pub async fn transfer(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    request: Option<Json<Object<Transfer>>>,
) -> ApiResult<StatusCode> {
    // One change of who owns a server at a time: the slot is freed and then
    // given, and the database alone cannot keep two of them from interleaving.
    require_fresh(&state, caller).await?;
    // The first server looked for the body only after this check, so a body
    // that is not there at all is told of the second step first.
    let Some(Json(Object(request))) = request else {
        return Err(ApiError::bad_request());
    };
    let _turn = state.mutation_gate().enter().await;
    let successor = request.user_id;
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<Vec<Id>, ApiError>, DbError> {
                let view = ViewChange::begin(tx, server)?;
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
                Ok(Ok(view.finish(tx)?))
            }
        })
        .await?;
    let changed = outcome?;
    access_changed::publish(state.events(), server, changed);
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the server and everything it holds. The instance server cannot be
/// deleted at all: it carries the instance's settings, and owning it is what
/// opens the operator's panel. Without it, whoever owned the next server, or
/// created one, would run the instance.
///
/// Guarded by a fresh second step, like a hand-over, before anything else is
/// looked at.
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<StatusCode> {
    require_fresh(&state, caller).await?;
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
