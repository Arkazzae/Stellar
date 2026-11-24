//! An invite link: what the page it opens may show before anybody signs in,
//! and joining through it.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::extract::State;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use stellarr_messages::MessageKey;

use super::join::{self, Entry};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, MaybeCaller, Path, Problem, json_response};
use crate::instance::invites::{self, Invite};
use crate::instance::servers::Server;
use crate::permissions::visibility;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// Whether an invite may name the channel it points to to whoever holds it. A
/// member is told it when they see it. Anybody else is told it when they would
/// see it on joining, holding the base role alone, so that an invite to a
/// channel the newcomer cannot see lands them in the server without naming the
/// channel, before joining as after.
fn names_channel(db: &Connection, invite: &Invite, user: Option<Id>) -> rusqlite::Result<bool> {
    let Some(channel) = invite.channel else {
        return Ok(false);
    };
    if let Some(user) = user {
        let member: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2)",
            params![invite.server, user],
            |row| row.get(0),
        )?;
        if member {
            return visibility::views(db, user, channel);
        }
    }
    visibility::newcomer_views(db, channel)
}

fn channel_name(db: &Connection, channel: Id) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT \"Name\" FROM \"Channels\" WHERE \"Id\" = ?1",
        [channel],
        |row| row.get(0),
    )
    .optional()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    server: PreviewServer,
    channel: Option<PreviewChannel>,
    inviter: PreviewInviter,
    expires_at: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewServer {
    id: Id,
    name: String,
    description: String,
    icon_url: Option<String>,
}

#[derive(Serialize)]
struct PreviewChannel {
    id: Id,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewInviter {
    display_name: String,
    avatar_color: String,
    avatar_url: Option<String>,
}

/// Where an invite leads and who sent it. Every invite that cannot be used,
/// whether unknown, revoked, expired, used up or pointing at a deleted server,
/// gets the same answer, so that nobody trying codes learns which ones once
/// existed.
pub async fn preview(
    State(state): State<AppState>,
    MaybeCaller(caller): MaybeCaller,
    Path(code): Path<String>,
) -> ApiResult<Response> {
    let signed_in = caller.is_some();
    let user = caller.map(|caller| caller.user);
    let found = state
        .database()
        .read(move |db| -> Result<Option<Preview>, DbError> {
            let Some(invite) = invites::find_usable(db, &code, Timestamp::now())? else {
                return Ok(None);
            };
            let Some(server) = Server::find(db, invite.server)? else {
                return Ok(None);
            };
            let (display_name, avatar_color, avatar_url) = db.query_row(
                "SELECT \"DisplayName\", \"AvatarColor\", \"AvatarUrl\" FROM \"Users\" WHERE \"Id\" = ?1",
                [invite.created_by],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, Option<String>>(2)?)),
            )?;
            let channel = match invite.channel {
                Some(channel) if names_channel(db, &invite, user)? => channel_name(db, channel)?
                    .map(|name| PreviewChannel { id: channel, name }),
                _ => None,
            };
            // The images are served only to a session, so for anybody else
            // their addresses would be broken images.
            Ok(Some(Preview {
                server: PreviewServer {
                    id: server.id,
                    name: server.name,
                    description: server.description,
                    icon_url: server.icon_url.filter(|_| signed_in),
                },
                channel,
                inviter: PreviewInviter {
                    display_name,
                    avatar_color,
                    avatar_url: avatar_url.filter(|_| signed_in),
                },
                expires_at: invite.expires_at,
            }))
        })
        .await?;
    let mut response = match found {
        Some(preview) => json_response(StatusCode::OK, &preview),
        None => ApiError::from(
            Problem::new(StatusCode::NOT_FOUND)
                .title(state.messages().get(MessageKey::InviteUnusable, &[])?),
        )
        .into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// The server a person joined through an invite, and the channel the invite
/// points to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Joined {
    id: Id,
    name: String,
    description: String,
    icon_url: Option<String>,
    created_at: Timestamp,
    is_public: bool,
    channel_id: Option<Id>,
    channel_name: Option<String>,
}

/// What the write of a join brings out.
struct Taken {
    server: Server,
    entry: Entry,
    channel: Option<(Id, String)>,
}

/// Joins the server the invite leads to. The use of the invite is counted in
/// the transaction that adds the member, against the same moment that found it
/// usable, so that an invite with one use left lets one person in.
pub async fn join(
    State(state): State<AppState>,
    caller: Caller,
    Path(code): Path<String>,
) -> ApiResult<Response> {
    let taken = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Option<Taken>, DbError> {
                let now = Timestamp::now();
                let Some(invite) = invites::find_usable(tx, &code, now)? else {
                    return Ok(None);
                };
                let Some(server) = Server::find(tx, invite.server)? else {
                    return Ok(None);
                };
                let entry = join::enter(tx, server.id, caller.user, now, state.messages(), invite.channel)?;
                let mut channel = None;
                if let Entry::Joined { added, .. } = &entry {
                    if *added && !invites::record_use(tx, invite.id, now)? {
                        return Err(DbError::Task("an invite found usable could not be used".into()));
                    }
                    if let Some(id) = invite.channel
                        && names_channel(tx, &invite, Some(caller.user))?
                    {
                        channel = channel_name(tx, id)?.map(|name| (id, name));
                    }
                }
                Ok(Some(Taken { server, entry, channel }))
            }
        })
        .await?;
    let Some(taken) = taken else {
        return Err(ApiError::not_found());
    };
    let done = join::joined(taken.entry, &state)?;
    let server = taken.server;
    join::publish(&state, caller.user, server.id, done).await;
    let (channel_id, channel_name) = match taken.channel {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };
    Ok(json_response(
        StatusCode::OK,
        &Joined {
            id: server.id,
            name: server.name,
            description: server.description,
            icon_url: server.icon_url,
            created_at: server.created_at,
            is_public: server.is_public,
            channel_id,
            channel_name,
        },
    ))
}
