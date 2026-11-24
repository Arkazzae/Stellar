//! Removing people from a server: kicking, which they can undo by joining
//! again, and banning, which stays until a moderator lifts it.
//!
//! `GET /api/admin/bans`, `POST /api/admin/members/{userId}/ban`,
//! `DELETE /api/admin/bans/{userId}` and `DELETE /api/admin/members/{userId}`.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::auth::Caller;
use crate::roles::Chosen;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Endpoints, Json, Path, Problem, json_response, no_content};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{self, MemberAuthorization};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{MANAGE_BANS, contains};
use crate::state::AppState;
use crate::values::{Id, Timestamp};
use stellarr_messages::MessageKey;

const MAX_REASON_LENGTH: usize = 500;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/admin/bans", list_bans)
        .authorized()
        .post("/api/admin/members/{userId}/ban", ban)
        .json_body()
        .authorized()
        .delete("/api/admin/bans/{userId}", unban)
        .authorized()
        .delete("/api/admin/members/{userId}", kick)
        .authorized()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BanRequest {
    /// Shown to moderators in the ban list; without one a standard text is
    /// stored.
    reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BanDto {
    user_id: Id,
    display_name: String,
    banned_by_name: String,
    reason: String,
    banned_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberRemoved {
    server_id: Id,
    user_id: Id,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessRevoked {
    server_id: Id,
    kind: &'static str,
}

/// The refusal of a moderation check: the person is not a member, or the
/// caller may not act on them.
fn refusal(decision: &MemberAuthorization) -> Option<ApiError> {
    match decision {
        MemberAuthorization::Allowed { .. } => None,
        MemberAuthorization::NotFound => Some(ApiError::not_found()),
        MemberAuthorization::Forbidden => Some(ApiError::forbidden()),
    }
}

fn may_manage_bans(db: &Connection, caller: Caller, server: Id) -> Result<(), ApiError> {
    if contains(authorization::server_wide(db, caller.user, server)?, MANAGE_BANS) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}

async fn list_bans(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let bans = state
        .database()
        .read(move |db| -> Result<Vec<BanDto>, ApiError> {
            may_manage_bans(db, caller, server)?;
            let mut statement = db.prepare(
                "SELECT b.\"UserId\", u.\"DisplayName\", m.\"DisplayName\", b.\"Reason\", b.\"BannedAt\"
                 FROM \"ServerBans\" b
                 INNER JOIN \"Users\" u ON b.\"UserId\" = u.\"Id\"
                 INNER JOIN \"Users\" m ON b.\"BannedById\" = m.\"Id\"
                 WHERE b.\"ServerId\" = ?1 ORDER BY b.\"BannedAt\" DESC",
            )?;
            let bans = statement
                .query_map([server], |row| {
                    Ok(BanDto {
                        user_id: row.get(0)?,
                        display_name: row.get(1)?,
                        banned_by_name: row.get(2)?,
                        reason: row.get(3)?,
                        banned_at: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(bans)
        })
        .await?;
    Ok(json_response(StatusCode::OK, &bans))
}

async fn ban(
    State(state): State<AppState>,
    caller: Caller,
    Path(user): Path<Id>,
    chosen: Chosen,
    Json(request): Json<BanRequest>,
) -> ApiResult<Response> {
    let server = chosen.server()?;
    let messages = *state.messages();
    let reason = request.reason.as_deref().unwrap_or_default().trim().to_string();
    if stellarr_unicode::utf16::length(&reason) > MAX_REASON_LENGTH {
        return Err(Problem::validation(vec![(
            "reason".to_string(),
            vec![messages.get(MessageKey::BanReasonTooLong, &[("max", &MAX_REASON_LENGTH)])?],
        )])
        .into());
    }
    let banned = state
        .database()
        .write(move |tx| -> Result<BanDto, ApiError> {
            let decision = authorization::ban_member(tx, caller.user, server, user)?;
            if let Some(refused) = refusal(&decision) {
                return Err(refused);
            }
            let name = crate::instance::users::User::display_name(tx, user)?;
            let by = crate::instance::users::User::display_name(tx, caller.user)?;
            let stored = if reason.is_empty() {
                messages.get(MessageKey::NoBanReason, &[])?
            } else {
                reason.clone()
            };
            let now = Timestamp::now();
            tx.execute(
                "INSERT INTO \"ServerBans\" (\"ServerId\", \"UserId\", \"BannedById\", \"Reason\", \"BannedAt\") VALUES (?1, ?2, ?3, ?4, ?5)",
                params![server, user, caller.user, stored, now],
            )
            .map_err(taken)?;
            crate::permissions::ownership::remove_member(tx, server, user)?;
            audit::record(
                tx,
                server,
                caller.user,
                &AuditEvent {
                    action: "member.ban".to_string(),
                    sentence: messages.get(MessageKey::AuditMemberBanned, &[("name", &name)])?,
                    target_type: Some("member".to_string()),
                    target_id: Some(user),
                    target_name: Some(name.clone()),
                    changes: if reason.is_empty() {
                        Vec::new()
                    } else {
                        vec![AuditChange {
                            field: "reason".to_string(),
                            to: Some(reason.clone()),
                            ..AuditChange::default()
                        }]
                    },
                },
            )?;
            Ok(BanDto {
                user_id: user,
                display_name: name,
                banned_by_name: by,
                reason: stored,
                banned_at: now,
            })
        })
        .await?;
    announce_removal(&state, server, user, "banned").await;
    let mut response = json_response(StatusCode::CREATED, &banned);
    if let Ok(location) = HeaderValue::from_str(&format!("/api/admin/bans/{user}")) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}

async fn unban(
    State(state): State<AppState>,
    caller: Caller,
    Path(user): Path<Id>,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let messages = *state.messages();
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            may_manage_bans(tx, caller, server)?;
            // Lifting a ban lets the person join again; it does not make them
            // a member.
            let removed = tx.execute(
                "DELETE FROM \"ServerBans\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
                params![server, user],
            )?;
            if removed == 0 {
                return Err(ApiError::not_found());
            }
            let name = crate::instance::users::User::display_name(tx, user)?;
            audit::record(
                tx,
                server,
                caller.user,
                &AuditEvent {
                    action: "member.unban".to_string(),
                    sentence: messages.get(MessageKey::AuditMemberUnbanned, &[("name", &name)])?,
                    target_type: Some("member".to_string()),
                    target_id: Some(user),
                    target_name: Some(name),
                    changes: Vec::new(),
                },
            )?;
            Ok(())
        })
        .await?;
    Ok(no_content())
}

async fn kick(
    State(state): State<AppState>,
    caller: Caller,
    Path(user): Path<Id>,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let messages = *state.messages();
    state
        .database()
        .write(move |tx| -> Result<(), ApiError> {
            let decision = authorization::kick_member(tx, caller.user, server, user)?;
            if let Some(refused) = refusal(&decision) {
                return Err(refused);
            }
            let name = crate::instance::users::User::display_name(tx, user)?;
            crate::permissions::ownership::remove_member(tx, server, user)?;
            audit::record(
                tx,
                server,
                caller.user,
                &AuditEvent {
                    action: "member.kick".to_string(),
                    sentence: messages.get(MessageKey::AuditMemberKicked, &[("name", &name)])?,
                    target_type: Some("member".to_string()),
                    target_id: Some(user),
                    target_name: Some(name),
                    changes: Vec::new(),
                },
            )?;
            Ok(())
        })
        .await?;
    announce_removal(&state, server, user, "kicked").await;
    Ok(no_content())
}

/// Tells the server that the member is gone and the member that they were
/// removed, then takes their connections out of the server's groups. They hear
/// about it through their own user group, which outlives the membership.
///
/// Their voice seats go first (with theirs go the seats of anybody else who sits
/// where they may not), with the media sessions they hold, and the rooms are
/// told last, when the person is already out of the server's groups.
async fn announce_removal(state: &AppState, server: Id, user: Id, kind: &'static str) {
    let unseated = crate::voice::unseat(state, server, None).await;
    state.events().publish(Event::new(
        Audience::Server(server),
        "ServerMemberRemoved",
        &MemberRemoved {
            server_id: server,
            user_id: user,
        },
    ));
    state.events().publish(Event::new(
        Audience::Users(vec![user]),
        "ServerAccessRevoked",
        &AccessRevoked {
            server_id: server,
            kind,
        },
    ));
    state.events().server_left(user, server);
    unseated.announce(state).await;
}

/// A unique value that was taken is a conflict.
fn taken(error: rusqlite::Error) -> ApiError {
    let error = DbError::from(error);
    if error.is_unique_violation() {
        ApiError::conflict()
    } else {
        ApiError::from(error)
    }
}
