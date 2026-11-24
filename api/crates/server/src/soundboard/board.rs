//! The order of the sounds and the two switches of the soundboard.

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::params;
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::{SettingsDto, announce_changed, no_content};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Json, Problem, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::servers::Object;
use crate::state::AppState;
use crate::values::Id;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reorder {
    sound_ids: Option<Vec<Id>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    everyone_can_play: Option<bool>,
    rate_limited: Option<bool>,
}

enum Order {
    Forbidden,
    Incomplete,
    Done,
}

/// Every sound of the server, each once, in the order they are to have.
pub async fn reorder(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(Object(request)): Json<Object<Reorder>>,
) -> ApiResult<Response> {
    let user = caller.user;
    let order = request.sound_ids.unwrap_or_default();
    let outcome = state
        .database()
        .write(move |tx| -> Result<Order, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(Order::Forbidden);
            }
            let mut statement =
                tx.prepare("SELECT \"Id\" FROM \"SoundboardSounds\" WHERE \"ServerId\" = ?1")?;
            let held: HashSet<Id> = statement
                .query_map([server], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            drop(statement);
            let listed: HashSet<Id> = order.iter().copied().collect();
            if listed.len() != order.len() || listed != held {
                return Ok(Order::Incomplete);
            }
            for (position, id) in order.iter().enumerate() {
                tx.execute(
                    "UPDATE \"SoundboardSounds\" SET \"Position\" = ?1 WHERE \"Id\" = ?2",
                    params![position as i64, id],
                )?;
            }
            Ok(Order::Done)
        })
        .await?;
    match outcome {
        Order::Forbidden => Err(ApiError::forbidden()),
        Order::Incomplete => {
            let text = state.messages().get(MessageKey::SoundOrderIncomplete, &[])?;
            Err(Problem::validation(vec![("soundIds".to_string(), vec![text])]).into())
        }
        Order::Done => {
            announce_changed(&state, server);
            Ok(no_content())
        }
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Changes the switches that are named; the rest stay as they are.
pub async fn settings(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(Object(request)): Json<Object<Settings>>,
) -> ApiResult<Response> {
    let user = caller.user;
    let sentence = state.messages().get(MessageKey::AuditSoundboardSettings, &[])?;
    let changed = state
        .database()
        .write(move |tx| -> Result<Option<SettingsDto>, DbError> {
            if !allows(tx, user, &Scope::server(server), MANAGE_SERVER)? {
                return Ok(None);
            }
            let (everyone, limited): (bool, bool) = tx.query_row(
                "SELECT \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let now_everyone = request.everyone_can_play.unwrap_or(everyone);
            let now_limited = request.rate_limited.unwrap_or(limited);
            tx.execute(
                "UPDATE \"Servers\" SET \"SoundboardEveryoneCanPlay\" = ?1, \"SoundboardRateLimited\" = ?2 WHERE \"Id\" = ?3",
                params![now_everyone, now_limited, server],
            )?;
            let changes = audit::only([
                AuditChange::changed(
                    "everyoneCanPlay",
                    Some(on_off(everyone)),
                    Some(on_off(now_everyone)),
                ),
                AuditChange::changed(
                    "rateLimited",
                    Some(on_off(limited)),
                    Some(on_off(now_limited)),
                ),
            ]);
            if !changes.is_empty() {
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "soundboard.settings".to_string(),
                        sentence,
                        changes,
                        ..AuditEvent::default()
                    },
                )?;
            }
            Ok(Some(SettingsDto { everyone_can_play: now_everyone, rate_limited: now_limited }))
        })
        .await?;
    let Some(settings) = changed else {
        return Err(ApiError::forbidden());
    };
    announce_changed(&state, server);
    Ok(json_response(StatusCode::OK, &settings))
}
