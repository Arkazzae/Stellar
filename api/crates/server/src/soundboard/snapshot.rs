//! `GET /api/soundboard`: the sounds in their order, the server's switches, and
//! what the caller may do with them.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::Connection;
use serde::Serialize;

use super::{SettingsDto, SoundDto, can_play, url_of};
use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, json_response};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::state::AppState;
use crate::values::Id;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    sounds: Vec<SoundDto>,
    settings: SettingsDto,
    can_manage: bool,
    can_play: bool,
}

/// The server's sounds as clients read them, in the order they are shown.
/// A sound with no measured length is as long as its trim says.
pub(crate) fn sounds_of(db: &Connection, server: Id) -> rusqlite::Result<Vec<SoundDto>> {
    let mut statement = db.prepare(
        "SELECT s.\"Id\", s.\"Name\", s.\"Glyph\", s.\"AttachmentId\", a.\"DurationSeconds\", s.\"TrimStartSeconds\", s.\"TrimEndSeconds\"
         FROM \"SoundboardSounds\" s JOIN \"ImageAttachments\" a ON a.\"Id\" = s.\"AttachmentId\"
         WHERE s.\"ServerId\" = ?1 ORDER BY s.\"Position\", s.\"CreatedAt\"",
    )?;
    statement
        .query_map([server], |row| {
            let attachment: Id = row.get(3)?;
            let duration: Option<f64> = row.get(4)?;
            let end: f64 = row.get(6)?;
            Ok(SoundDto {
                id: row.get(0)?,
                name: row.get(1)?,
                glyph: row.get(2)?,
                url: url_of(attachment),
                duration_seconds: duration.unwrap_or(end),
                trim_start_seconds: row.get(5)?,
                trim_end_seconds: end,
            })
        })?
        .collect()
}

pub async fn read(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let user = caller.user;
    let snapshot = state
        .database()
        .read(move |db| -> Result<Snapshot, DbError> {
            let (everyone, limited): (bool, bool) = db.query_row(
                "SELECT \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok(Snapshot {
                sounds: sounds_of(db, server)?,
                settings: SettingsDto { everyone_can_play: everyone, rate_limited: limited },
                can_manage: allows(db, user, &Scope::server(server), MANAGE_SERVER)?,
                can_play: can_play(db, user, server, everyone)?,
            })
        })
        .await
        .map_err(ApiError::from)?;
    Ok(json_response(StatusCode::OK, &snapshot))
}
