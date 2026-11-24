//! The notification defaults of a server.

use axum::extract::State;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Problem};
use crate::permissions::authorization::{self, Scope};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::permissions::visibility::{self, MemberView};
use crate::state::AppState;
use crate::values::Id;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    all_messages: bool,
    mentions_only: bool,
    voice_join_sound: bool,
    muted_channel_ids: Vec<Option<String>>,
    server_muted: bool,
}

/// What a request may change; a property left out stays as it is.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    all_messages: Option<bool>,
    mentions_only: Option<bool>,
    voice_join_sound: Option<bool>,
    muted_channel_ids: Option<Vec<Id>>,
    server_muted: Option<bool>,
}

struct Stored {
    all_messages: bool,
    mentions_only: bool,
    voice_join_sound: bool,
    muted_json: String,
    server_muted: bool,
}

/// The defaults of a server. Older servers have none yet; they get the column
/// defaults the first time somebody looks.
fn settings_of(db: &Connection, server: Id) -> Result<Stored, DbError> {
    db.execute(
        "INSERT OR IGNORE INTO \"ServerAdminSettings\" (\"ServerId\", \"AllMessages\", \"MentionsOnly\", \"VoiceJoinSound\", \"MutedChannelIdsJson\", \"ServerMuted\")
         VALUES (?1, 0, 1, 1, '[]', 0)",
        [server],
    )?;
    Ok(db.query_row(
        "SELECT \"AllMessages\", \"MentionsOnly\", \"VoiceJoinSound\", \"MutedChannelIdsJson\", \"ServerMuted\" FROM \"ServerAdminSettings\" WHERE \"ServerId\" = ?1",
        [server],
        |row| {
            Ok(Stored {
                all_messages: row.get(0)?,
                mentions_only: row.get(1)?,
                voice_join_sound: row.get(2)?,
                muted_json: row.get(3)?,
                server_muted: row.get(4)?,
            })
        },
    )?)
}

/// The ids of the stored list. A damaged list must not make the settings
/// unreadable, so it reads as empty.
fn muted_of(json: &str) -> Vec<Option<String>> {
    serde_json::from_str::<Vec<Option<String>>>(json).unwrap_or_default()
}

fn seen(view: &MemberView, text: &Option<String>) -> bool {
    text.as_deref()
        .and_then(crate::permissions::membership::parse_guid)
        .is_some_and(|id| view.channels.contains(&id))
}

/// The muted channels the caller does not see stay muted, unnamed.
fn dto(stored: &Stored, view: Option<&MemberView>) -> SettingsDto {
    let mut muted = muted_of(&stored.muted_json);
    if let Some(view) = view {
        muted.retain(|id| seen(view, id));
    }
    SettingsDto {
        all_messages: stored.all_messages,
        mentions_only: stored.mentions_only,
        voice_join_sound: stored.voice_join_sound,
        muted_channel_ids: muted,
        server_muted: stored.server_muted,
    }
}

pub async fn read(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Json<SettingsDto>> {
    let found = state
        .database()
        .write(move |tx| -> Result<Option<SettingsDto>, DbError> {
            if !authorization::allows(tx, caller.user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(None);
            }
            let stored = settings_of(tx, server)?;
            let view = visibility::of_member(tx, caller.user, server)?;
            Ok(Some(dto(&stored, view.as_ref())))
        })
        .await?;
    found.map(Json).ok_or_else(ApiError::forbidden)
}

pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<UpdateSettings>,
) -> ApiResult<Json<SettingsDto>> {
    let outcome = state
        .database()
        .write({
            let state = state.clone();
            move |tx| -> Result<Result<SettingsDto, ApiError>, DbError> {
                if !authorization::allows(tx, caller.user, &Scope::server(server), MANAGE_SERVER)? {
                    return Ok(Err(ApiError::forbidden()));
                }
                // The caller names the channels they see; one they do not is as
                // unknown as one that is not there, and those they do not see
                // keep what they were, since the caller could not have named
                // them.
                let mut muted: Option<Vec<Id>> = None;
                if let Some(requested) = &request.muted_channel_ids {
                    let mut distinct: Vec<Id> = Vec::new();
                    for id in requested {
                        if !distinct.contains(id) {
                            distinct.push(*id);
                        }
                    }
                    muted = Some(distinct);
                }
                let view = visibility::of_member(tx, caller.user, server)?;
                if let Some(muted) = &muted {
                    let mut known = 0;
                    for id in muted {
                        known += tx.query_row(
                            "SELECT COUNT(*) FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Id\" = ?2",
                            params![server, id],
                            |row| row.get::<_, i64>(0),
                        )?;
                    }
                    let hidden = view
                        .as_ref()
                        .is_some_and(|view| muted.iter().any(|id| !view.channels.contains(id)));
                    if known as usize != muted.len() || hidden {
                        let message = state
                            .messages()
                            .get(MessageKey::ChannelUnknown, &[])
                            .map_err(|e| DbError::Task(e.to_string()))?;
                        return Ok(Err(
                            Problem::validation(vec![("mutedChannelIds".into(), vec![message])]).into(),
                        ));
                    }
                }
                let stored = settings_of(tx, server)?;
                if let (Some(list), Some(view)) = (&mut muted, &view) {
                    for id in muted_of(&stored.muted_json) {
                        let parsed = id.as_deref().and_then(crate::permissions::membership::parse_guid);
                        if let Some(parsed) = parsed
                            && !view.channels.contains(&parsed)
                        {
                            list.push(parsed);
                        }
                    }
                }
                let saved = Stored {
                    all_messages: request.all_messages.unwrap_or(stored.all_messages),
                    mentions_only: request.mentions_only.unwrap_or(stored.mentions_only),
                    voice_join_sound: request.voice_join_sound.unwrap_or(stored.voice_join_sound),
                    muted_json: match &muted {
                        Some(list) => serde_json::to_string(
                            &list.iter().map(Id::to_string).collect::<Vec<_>>(),
                        )
                        .map_err(|e| DbError::Task(e.to_string()))?,
                        None => stored.muted_json,
                    },
                    server_muted: request.server_muted.unwrap_or(stored.server_muted),
                };
                tx.execute(
                    "UPDATE \"ServerAdminSettings\" SET \"AllMessages\" = ?1, \"MentionsOnly\" = ?2, \"VoiceJoinSound\" = ?3, \"MutedChannelIdsJson\" = ?4, \"ServerMuted\" = ?5
                     WHERE \"ServerId\" = ?6",
                    params![
                        saved.all_messages,
                        saved.mentions_only,
                        saved.voice_join_sound,
                        saved.muted_json,
                        saved.server_muted,
                        server
                    ],
                )?;
                Ok(Ok(dto(&saved, view.as_ref())))
            }
        })
        .await?;
    outcome.map(Json)
}
