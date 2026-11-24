//! The invites of a server: listing, making, revoking, and sending one to a
//! member of the instance.

use aws_lc_rs::rand;
use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, ClientInfo, Json, Path, Problem, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{self, Refusal, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{CREATE_INVITE, MANAGE_SERVER};
use crate::permissions::visibility;
use crate::servers::{invitation_arrived, invitations_changed};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const INVITE_CODE_BYTES: usize = 8;
const INVITE_MAX_USES: i64 = 25;
const INVITE_LIFETIME_DAYS: u64 = 7;

/// An invite as the people who run the server see it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteDto {
    id: Id,
    url: String,
    created_by_name: String,
    uses: i64,
    max_uses: Option<i64>,
    /// Spelled by the caller: an invite just made says it is UTC, one read
    /// back from the database does not.
    expires_at: Option<String>,
    channel_id: Option<Id>,
    channel_name: Option<String>,
}

/// Where an invite link leads: the instance's public address, with the scheme
/// and port browsers reach it at. The request alone loses the port, as nginx
/// hands the API the host without it, and behind another proxy it may not be
/// the public address at all. Without a public address the request's is the
/// best there is.
fn invite_origin(state: &AppState, client: &ClientInfo, headers: &HeaderMap) -> String {
    let public = &state.settings().operations.public_url;
    if !public.is_empty() {
        return public.clone();
    }
    let host = client.host.clone().unwrap_or_else(|| {
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string()
    });
    format!("{}://{host}", client.scheme)
}

/// The server's invites that can still be used, newest first. One to a
/// channel the caller does not see is listed without the channel: the invite is
/// theirs to revoke, the channel not theirs to know about.
pub async fn list(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Extension(client): Extension<ClientInfo>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let origin = invite_origin(&state, &client, &headers);
    let found = state
        .database()
        .read(move |db| -> Result<Option<Vec<InviteDto>>, DbError> {
            if !authorization::allows(db, caller.user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(None);
            }
            let now = Timestamp::now();
            let mut statement = db.prepare(
                "SELECT i.\"Id\", i.\"Code\", u.\"DisplayName\", i.\"Uses\", i.\"MaxUses\", i.\"ExpiresAt\", i.\"ChannelId\", c.\"Name\"
                 FROM \"ServerInvites\" i
                 JOIN \"Users\" u ON u.\"Id\" = i.\"CreatedById\"
                 LEFT JOIN \"Channels\" c ON c.\"Id\" = i.\"ChannelId\"
                 WHERE i.\"ServerId\" = ?1
                   AND (i.\"ExpiresAt\" IS NULL OR i.\"ExpiresAt\" > ?2)
                   AND (i.\"MaxUses\" IS NULL OR i.\"Uses\" < i.\"MaxUses\")
                 ORDER BY i.\"CreatedAt\" DESC",
            )?;
            let rows = statement
                .query_map(params![server, now], |row| {
                    Ok((
                        row.get::<_, Id>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<Timestamp>>(5)?,
                        row.get::<_, Option<Id>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let view = visibility::of_member(db, caller.user, server)?;
            Ok(Some(
                rows.into_iter()
                    .map(|(id, code, by, uses, max_uses, expires_at, channel, name)| {
                        let hidden = match (&view, channel) {
                            (Some(view), Some(channel)) => !view.channels.contains(&channel),
                            _ => false,
                        };
                        let (channel_id, channel_name) = if hidden { (None, None) } else { (channel, name) };
                        InviteDto {
                            id,
                            url: format!("{origin}/invite/{code}"),
                            created_by_name: by,
                            uses,
                            max_uses,
                            expires_at: expires_at.map(Timestamp::to_unspecified_wire_text),
                            channel_id,
                            channel_name,
                        }
                    })
                    .collect(),
            ))
        })
        .await?;
    match found {
        Some(invites) => Ok(json_response(StatusCode::OK, &invites)),
        None => Err(ApiError::forbidden()),
    }
}

/// What a person asks for when they make an invite.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvite {
    channel_id: Option<Id>,
}

/// Making an invite is its own permission, decided where the invite leads: in
/// the channel for an invite to a channel, so its overrides count, and
/// server-wide otherwise.
pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Extension(client): Extension<ClientInfo>,
    headers: HeaderMap,
    request: Option<Json<CreateInvite>>,
) -> ApiResult<Response> {
    let origin = invite_origin(&state, &client, &headers);
    let wanted = request.and_then(|Json(request)| request.channel_id);
    let made = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<InviteDto, Refused>, DbError> {
                let messages = state.messages();
                let text = |key: MessageKey| messages.get(key, &[]).map_err(|e| DbError::Task(e.to_string()));
                // A channel the caller does not see is as unknown as one that
                // is not there.
                let mut channel: Option<(Id, String)> = None;
                if let Some(id) = wanted {
                    match visibility::find(tx, caller.user, server, id, None)? {
                        Some(place) => channel = Some((place.id, place.name)),
                        None => return Ok(Err(Refused::UnknownChannel(text(MessageKey::ChannelUnknown)?))),
                    }
                }
                let scope = match &channel {
                    Some((id, _)) => Scope::channel(server, *id),
                    None => Scope::server(server),
                };
                if !authorization::allows(tx, caller.user, &scope, CREATE_INVITE)? {
                    return Ok(Err(Refused::Denied(
                        Refusal::problem(messages, MessageKey::CreateInviteDenied, CREATE_INVITE)
                            .map_err(|e| DbError::Task(e.to_string()))?,
                    )));
                }
                let now = Timestamp::now();
                let id = Id::random();
                let code = new_code();
                let expires_at = now.plus(std::time::Duration::from_secs(INVITE_LIFETIME_DAYS * 86_400));
                tx.execute(
                    "INSERT INTO \"ServerInvites\" (\"Id\", \"ServerId\", \"CreatedById\", \"ChannelId\", \"Code\", \"Uses\", \"MaxUses\", \"ExpiresAt\", \"CreatedAt\")
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
                    params![id, server, caller.user, channel.as_ref().map(|c| c.0), code, INVITE_MAX_USES, expires_at, now],
                )?;
                let sentence = match &channel {
                    None => text(MessageKey::AuditInviteCreated)?,
                    Some((_, name)) => messages
                        .get(MessageKey::AuditInviteCreatedForChannel, &[("channel", name)])
                        .map_err(|e| DbError::Task(e.to_string()))?,
                };
                audit::record(
                    tx,
                    server,
                    caller.user,
                    &AuditEvent {
                        action: "invite.create".into(),
                        sentence,
                        target_type: Some("invite".into()),
                        target_id: Some(id),
                        target_name: Some(code.clone()),
                        changes: channel
                            .as_ref()
                            .map(|(_, name)| {
                                vec![AuditChange {
                                    field: "channel".into(),
                                    to: Some(name.clone()),
                                    ..AuditChange::default()
                                }]
                            })
                            .unwrap_or_default(),
                    },
                )?;
                let by: String = tx.query_row(
                    "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                    [caller.user],
                    |row| row.get(0),
                )?;
                Ok(Ok(
                    InviteDto {
                        id,
                        url: format!("{origin}/invite/{code}"),
                        created_by_name: by,
                        uses: 0,
                        max_uses: Some(INVITE_MAX_USES),
                        expires_at: Some(expires_at.to_wire_text()),
                        channel_id: channel.as_ref().map(|c| c.0),
                        channel_name: channel.map(|c| c.1),
                    },
                ))
            }
        })
        .await?;
    match made {
        Ok(invite) => {
            let mut response = json_response(StatusCode::CREATED, &invite);
            if let Ok(location) = HeaderValue::from_str(&format!("/api/admin/invites/{}", invite.id)) {
                response.headers_mut().insert(header::LOCATION, location);
            }
            Ok(response)
        }
        Err(Refused::UnknownChannel(message)) => {
            Err(Problem::validation(vec![("channelId".into(), vec![message])]).into())
        }
        Err(Refused::Denied(problem)) => Err(problem.into()),
    }
}

enum Refused {
    UnknownChannel(String),
    Denied(Problem),
}

/// A code nobody can guess: eight random bytes, in lower case hexadecimal.
fn new_code() -> String {
    let mut bytes = [0u8; INVITE_CODE_BYTES];
    rand::fill(&mut bytes).expect("the system random number generator failed");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Takes an invite back. People who were sent it lose it from their inbox with
/// it.
pub async fn revoke(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(invite): Path<Id>,
) -> ApiResult<StatusCode> {
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<Vec<Id>, ApiError>, DbError> {
                if !authorization::allows(tx, caller.user, &Scope::server(server), MANAGE_SERVER)? {
                    return Ok(Err(ApiError::forbidden()));
                }
                let found: Option<String> = tx
                    .query_row(
                        "SELECT \"Code\" FROM \"ServerInvites\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                        params![invite, server],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(code) = found else {
                    return Ok(Err(ApiError::not_found()));
                };
                let mut statement = tx.prepare(
                    "SELECT \"UserId\" FROM \"UserNotifications\"
                     WHERE \"ServerId\" = ?1 AND \"Source\" = 'system' AND \"TargetInviteCode\" = ?2 ORDER BY \"Sequence\"",
                )?;
                let mut recipients: Vec<Id> = Vec::new();
                for user in statement
                    .query_map(params![server, code], |row| row.get::<_, Id>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                {
                    if !recipients.contains(&user) {
                        recipients.push(user);
                    }
                }
                drop(statement);
                tx.execute(
                    "DELETE FROM \"UserNotifications\" WHERE \"ServerId\" = ?1 AND \"Source\" = 'system' AND \"TargetInviteCode\" = ?2",
                    params![server, code],
                )?;
                tx.execute("DELETE FROM \"ServerInvites\" WHERE \"Id\" = ?1", [invite])?;
                audit::record(
                    tx,
                    server,
                    caller.user,
                    &AuditEvent {
                        action: "invite.revoke".into(),
                        sentence: state
                            .messages()
                            .get(MessageKey::AuditInviteRevoked, &[])
                            .map_err(|e| DbError::Task(e.to_string()))?,
                        target_type: Some("invite".into()),
                        target_id: Some(invite),
                        target_name: Some(code),
                        changes: Vec::new(),
                    },
                )?;
                Ok(Ok(recipients))
            }
        })
        .await?;
    for user in outcome? {
        state.events().publish(invitations_changed(user, server));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The answer of a member who is a member already.
#[derive(Serialize)]
struct AlreadyMember {
    error: &'static str,
}

/// Sends an invite to one person of the instance, as a notice in their inbox
/// that stays until they answer it.
pub async fn send(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path((invite, user)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    let outcome = state
        .database()
        .write(move |tx| -> Result<Result<Option<i64>, Response>, DbError> {
            if !authorization::allows(tx, caller.user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(Err(ApiError::forbidden().into_response()));
            }
            let now = Timestamp::now();
            let found = tx
                .query_row(
                    "SELECT \"Code\", \"ExpiresAt\", \"Uses\", \"MaxUses\" FROM \"ServerInvites\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                    params![invite, server],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<Timestamp>>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )
                .optional()?;
            let person: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Users\" WHERE \"Id\" = ?1 AND \"Origin\" IS NULL)",
                [user],
                |row| row.get(0),
            )?;
            let usable = found
                .filter(|(_, expires_at, uses, max_uses)| {
                    !expires_at.is_some_and(|at| at <= now) && !max_uses.is_some_and(|max| max <= *uses)
                })
                .map(|(code, ..)| code);
            let Some(code) = usable.filter(|_| person) else {
                return Ok(Err(ApiError::not_found().into_response()));
            };
            let member: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2)",
                params![server, user],
                |row| row.get(0),
            )?;
            if member {
                return Ok(Err(json_response(
                    StatusCode::CONFLICT,
                    &AlreadyMember {
                        error: "already_member",
                    },
                )));
            }
            let banned: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ServerBans\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2)",
                params![server, user],
                |row| row.get(0),
            )?;
            if banned {
                return Ok(Err(ApiError::forbidden().into_response()));
            }
            let sent: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"UserNotifications\"
                   WHERE \"UserId\" = ?1 AND \"ServerId\" = ?2 AND \"Source\" = 'system' AND \"TargetInviteCode\" = ?3)",
                params![user, server, code],
                |row| row.get(0),
            )?;
            if sent {
                return Ok(Ok(None));
            }
            let server_name: String = tx.query_row(
                "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            let actor_name: String = tx.query_row(
                "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
                [caller.user],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO \"UserNotifications\" (\"Id\", \"ServerId\", \"UserId\", \"Source\", \"SourcePackageId\", \"SourcePackageName\",
                     \"SourceModuleId\", \"SourceModuleName\", \"Title\", \"Body\", \"Tone\", \"TargetChannelId\", \"TargetInviteCode\", \"CreatedAt\", \"ReadAt\")
                 VALUES (?1, ?2, ?3, 'system', 'stellarr.server-invite', ?4, 'server-invite', ?5, 'Server invitation', ?6, 'info', NULL, ?7, ?8, NULL)",
                params![
                    Id::random(),
                    server,
                    user,
                    server_name,
                    actor_name,
                    format!("{actor_name} invited you to {server_name}."),
                    code,
                    now
                ],
            )?;
            Ok(Ok(Some(tx.last_insert_rowid())))
        })
        .await?;
    match outcome {
        Ok(Some(sequence)) => state
            .events()
            .publish(invitation_arrived(user, server, sequence)),
        Ok(None) => {}
        Err(response) => return Ok(response),
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
