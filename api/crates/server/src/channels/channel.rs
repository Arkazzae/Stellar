//! Making, changing and removing a channel.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, params};
use serde::Deserialize;
use stellarr_messages::MessageKey;

use super::names::{
    MAX_HIDE_AFTER, MAX_SLOW_MODE, MAX_TOPIC, MAX_USER_LIMIT, is_valid_channel_name, length,
    normalize_channel, trim,
};
use super::{Answer, say, ChannelDto, Refuse, created, kind_name, path_id};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, Json, Path, json_response};
use crate::permissions::audit::{self, AuditChange, AuditEvent};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_CHANNELS;
use crate::permissions::visibility::{self, ChannelKind};
use crate::state::AppState;
use crate::values::Id;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannel {
    name: Option<String>,
    kind: Option<String>,
    section_id: Option<Id>,
}

/// Every field is optional; one left out keeps its value.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannel {
    name: Option<String>,
    topic: Option<String>,
    slow_mode_seconds: Option<i32>,
    hide_after_inactivity_hours: Option<i32>,
    user_limit: Option<i32>,
}

/// A channel's row.
struct Row {
    id: Id,
    name: String,
    kind: ChannelKind,
    position: i64,
    topic: String,
    slow_mode_seconds: i64,
    hide_after_inactivity_hours: i64,
    user_limit: i64,
    section: Option<Id>,
}

impl Row {
    fn load(db: &Connection, id: Id) -> rusqlite::Result<Row> {
        db.query_row(
            "SELECT \"Id\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\"
             FROM \"Channels\" WHERE \"Id\" = ?1",
            [id],
            |row| {
                Ok(Row {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    position: row.get(3)?,
                    topic: row.get(4)?,
                    slow_mode_seconds: row.get(5)?,
                    hide_after_inactivity_hours: row.get(6)?,
                    user_limit: row.get(7)?,
                    section: row.get(8)?,
                })
            },
        )
    }

    fn dto(&self) -> ChannelDto {
        ChannelDto {
            id: self.id,
            name: self.name.clone(),
            kind: kind_name(self.kind),
            position: self.position,
            topic: self.topic.clone(),
            slow_mode_seconds: self.slow_mode_seconds,
            hide_after_inactivity_hours: self.hide_after_inactivity_hours,
            user_limit: self.user_limit,
            section_id: self.section,
        }
    }
}

pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Json(request): Json<CreateChannel>,
) -> ApiResult<Response> {
    let messages = *state.messages();
    let name = normalize_channel(request.name.as_deref());
    let kind = match request.kind.as_deref() {
        Some("text") => Some(ChannelKind::Text),
        Some("voice") => Some(ChannelKind::Voice),
        _ => None,
    };
    let (true, Some(kind)) = (is_valid_channel_name(&name), kind) else {
        return Err(Refuse::Invalid("channel", MessageKey::ChannelNameOrKindInvalid)
            .into_error(&messages)?);
    };
    let user = caller.user;
    let section = request.section_id;
    let kind_text = kind_name(kind);

    let outcome = state
        .database()
        .try_write(move |tx| -> Result<Answer<ChannelDto>, DbError> {
            // A name another channel has is taken, whether or not the caller
            // sees that channel.
            let taken: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Name\" = ?2)",
                params![server, name],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Answer::Refused(Refuse::Conflict));
            }
            // A section that is not listed for the caller is one that does
            // not exist.
            if let Some(section) = section
                && !visibility::section_listed(tx, user, server, section)?
            {
                return Ok(Answer::Refused(Refuse::NotFound));
            }
            // Inside a section, that section's override decides who may add to it.
            let scope = match section {
                Some(section) => Scope::section(server, section),
                None => Scope::server(server),
            };
            if !allows(tx, user, &scope, MANAGE_CHANNELS)? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            let last: Option<i64> = tx.query_row(
                "SELECT MAX(\"Position\") FROM \"Channels\" WHERE \"ServerId\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            let id = Id::random();
            let position = last.unwrap_or(-1) + 1;
            tx.execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
                 VALUES (?1, ?2, ?3, ?4, ?5, '', 0, 0, 0, ?6)",
                params![id, server, name, kind, position, section],
            )?;
            let section_name: Option<String> = match section {
                Some(section) => Some(tx.query_row(
                    "SELECT \"Name\" FROM \"Sections\" WHERE \"Id\" = ?1",
                    [section],
                    |row| row.get(0),
                )?),
                None => None,
            };
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "channel.create".to_string(),
                    sentence: say(&messages, MessageKey::AuditChannelCreated, &[("name", &name)])?,
                    target_type: Some("channel".to_string()),
                    target_id: Some(id),
                    target_name: Some(name.clone()),
                    changes: audit::only([
                        Some(AuditChange {
                            field: "kind".to_string(),
                            to: Some(kind_text.to_string()),
                            ..AuditChange::default()
                        }),
                        section_name.map(|section| AuditChange {
                            field: "section".to_string(),
                            to: Some(section),
                            ..AuditChange::default()
                        }),
                    ]),
                },
            )?;
            Ok(Answer::Done(Row::load(tx, id)?.dto()))
        })
        .await?;
    match outcome {
        // A name that another request took first.
        None => Err(Refuse::Conflict.into_error(&messages)?),
        Some(Answer::Refused(refusal)) => Err(refusal.into_error(&messages)?),
        Some(Answer::Done(channel)) => Ok(created(
            format!("/api/admin/channels/{}", channel.id),
            &channel,
        )),
    }
}

pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel_id): Path<String>,
    Json(request): Json<UpdateChannel>,
) -> ApiResult<Response> {
    let channel_id = path_id(&channel_id)?;
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .try_write(move |tx| -> Result<Answer<ChannelDto>, DbError> {
            // Managing a channel starts with seeing it.
            let Some(place) = visibility::find(tx, user, server, channel_id, None)? else {
                return Ok(Answer::Refused(Refuse::NotFound));
            };
            if !allows(tx, user, &Scope::channel(server, place.id), MANAGE_CHANNELS)? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            let mut channel = Row::load(tx, channel_id)?;

            let name = request
                .name
                .as_deref()
                .map(|name| normalize_channel(Some(name)));
            if let Some(name) = &name {
                if !is_valid_channel_name(name) {
                    return Ok(Answer::Refused(Refuse::Invalid(
                        "name",
                        MessageKey::ChannelNameInvalid,
                    )));
                }
                let taken: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Id\" != ?2 AND \"Name\" = ?3)",
                    params![server, channel.id, name],
                    |row| row.get(0),
                )?;
                if taken {
                    return Ok(Answer::Refused(Refuse::Conflict));
                }
            }

            let topic = request.topic.as_deref().map(trim);
            let in_range = |value: Option<i32>, most: i64| {
                value.is_none_or(|value| (0..=most).contains(&i64::from(value)))
            };
            if !(topic.is_none_or(|topic| length(topic) <= MAX_TOPIC)
                && in_range(request.slow_mode_seconds, MAX_SLOW_MODE)
                && in_range(request.hide_after_inactivity_hours, MAX_HIDE_AFTER)
                && in_range(request.user_limit, MAX_USER_LIMIT))
            {
                return Ok(Answer::Refused(Refuse::Invalid(
                    "channel",
                    MessageKey::ChannelSettingInvalid,
                )));
            }
            let text_only = request.topic.is_some() || request.slow_mode_seconds.is_some();
            let voice_only = request.user_limit.is_some();
            if match channel.kind {
                ChannelKind::Text => voice_only,
                ChannelKind::Voice => text_only,
            } {
                return Ok(Answer::Refused(Refuse::Invalid(
                    "channel",
                    MessageKey::ChannelSettingWrongKind,
                )));
            }

            let before = (
                channel.name.clone(),
                channel.topic.clone(),
                channel.slow_mode_seconds,
                channel.hide_after_inactivity_hours,
                channel.user_limit,
            );
            if let Some(name) = name {
                channel.name = name;
            }
            if let Some(topic) = topic {
                channel.topic = topic.to_string();
            }
            if let Some(value) = request.slow_mode_seconds {
                channel.slow_mode_seconds = i64::from(value);
            }
            if let Some(value) = request.hide_after_inactivity_hours {
                channel.hide_after_inactivity_hours = i64::from(value);
            }
            if let Some(value) = request.user_limit {
                channel.user_limit = i64::from(value);
            }
            tx.execute(
                "UPDATE \"Channels\" SET \"Name\" = ?2, \"Topic\" = ?3, \"SlowModeSeconds\" = ?4, \"HideAfterInactivityHours\" = ?5, \"UserLimit\" = ?6
                 WHERE \"Id\" = ?1",
                params![
                    channel.id,
                    channel.name,
                    channel.topic,
                    channel.slow_mode_seconds,
                    channel.hide_after_inactivity_hours,
                    channel.user_limit,
                ],
            )?;
            let number = |value: i64| value.to_string();
            let changes = audit::only([
                AuditChange::changed("name", Some(&before.0), Some(&channel.name)),
                AuditChange::changed("topic", Some(&before.1), Some(&channel.topic)),
                AuditChange::changed(
                    "slowMode",
                    Some(&number(before.2)),
                    Some(&number(channel.slow_mode_seconds)),
                ),
                AuditChange::changed(
                    "hideAfter",
                    Some(&number(before.3)),
                    Some(&number(channel.hide_after_inactivity_hours)),
                ),
                AuditChange::changed(
                    "userLimit",
                    Some(&number(before.4)),
                    Some(&number(channel.user_limit)),
                ),
            ]);
            if !changes.is_empty() {
                audit::record(
                    tx,
                    server,
                    user,
                    &AuditEvent {
                        action: "channel.update".to_string(),
                        sentence: say(&messages, MessageKey::AuditChannelUpdated, &[("name", &channel.name)])?,
                        target_type: Some("channel".to_string()),
                        target_id: Some(channel.id),
                        target_name: Some(channel.name.clone()),
                        changes,
                    },
                )?;
            }
            Ok(Answer::Done(channel.dto()))
        })
        .await?;
    match outcome {
        None => Err(Refuse::Conflict.into_error(&messages)?),
        Some(Answer::Refused(refusal)) => Err(refusal.into_error(&messages)?),
        Some(Answer::Done(channel)) => Ok(json_response(StatusCode::OK, &channel)),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(channel_id): Path<String>,
) -> ApiResult<Response> {
    let channel_id = path_id(&channel_id)?;
    let messages = *state.messages();
    let user = caller.user;
    let outcome = state
        .database()
        .write(move |tx| -> Result<Answer<()>, DbError> {
            let Some(channel) = visibility::find(tx, user, server, channel_id, None)? else {
                return Ok(Answer::Refused(Refuse::NotFound));
            };
            if !allows(
                tx,
                user,
                &Scope::channel(server, channel.id),
                MANAGE_CHANNELS,
            )? {
                return Ok(Answer::Refused(Refuse::Forbidden));
            }
            // Counting and deleting are one statement, so that the last
            // channel of a kind is never taken.
            let deleted = tx.execute(
                "DELETE FROM \"Channels\" WHERE \"Id\" = ?1
                 AND (SELECT COUNT(*) FROM \"Channels\" other WHERE other.\"ServerId\" = ?2 AND other.\"Kind\" = ?3) > 1",
                params![channel.id, server, channel.kind],
            )?;
            if deleted == 0 {
                return Ok(Answer::Refused(Refuse::Conflict));
            }
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "channel.delete".to_string(),
                    sentence: say(&messages, MessageKey::AuditChannelDeleted, &[("name", &channel.name)])?,
                    target_type: Some("channel".to_string()),
                    target_id: Some(channel.id),
                    target_name: Some(channel.name.clone()),
                    changes: Vec::new(),
                },
            )?;
            Ok(Answer::Done(()))
        })
        .await?;
    match outcome {
        Answer::Refused(refusal) => Err(refusal.into_error(&messages)?),
        Answer::Done(()) => {
            // Whoever sat in the channel is taken out of it, now that it is gone.
            crate::voice::unseat(&state, server, Some(vec![channel_id]))
                .await
                .announce(&state)
                .await;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
    }
}
