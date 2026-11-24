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
use crate::values::Id;

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
/// of the server and before other checks so that the second-step refusal
/// takes precedence.
pub async fn transfer(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    request: Option<Json<Object<Transfer>>>,
) -> ApiResult<StatusCode> {
    // One change of who owns a server at a time: the slot is freed and then
    // given, and the database alone cannot keep two of them from interleaving.
    fresh::require(&state, caller, None).await?;
    // Reject an absent body only after the second-step check.
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
                let name: String = crate::instance::users::User::display_name(tx, target.user)?;
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
    // The former owner may no longer hold what lets them sit where they do.
    crate::voice::enforce(&state, server).await;
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
    fresh::require(&state, caller, None).await?;
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<Vec<Id>, ApiError>, DbError> {
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
                // The voice rooms the server has, read before they go with it.
                let rooms = crate::voice::voice_channels_of(tx, server)?;
                if ownership::delete_server(tx, server)? {
                    Ok(Ok(rooms))
                } else {
                    Ok(Err(ApiError::not_found()))
                }
            }
        })
        .await?;
    let rooms = outcome?;
    // Whoever sat in one of its voice rooms is taken out of it.
    crate::voice::unseat(&state, server, Some(rooms))
        .await
        .announce(&state)
        .await;
    Ok(StatusCode::NO_CONTENT)
}
