//! `GET /api/me/join-sound`: every sound this account may announce its
//! arrivals with, grouped by the server it belongs to.
//!
//! Deliberately not the active server's soundboard: the choice is personal and
//! travels with the person into every call, so the whole membership is the
//! catalogue. A server with no sounds is left out; a selection whose server
//! has since been left is still reported, because saying nothing would look
//! like the setting had cleared itself.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::auth::Caller;
use crate::database::DbError;
use crate::http::{ApiResult, json_response};
use crate::soundboard::{SoundDto, seconds};
use crate::state::AppState;
use crate::values::Id;

/// The most of a clip that a room ever hears. A greeting may be recognisable
/// but must not hold the floor, so the audible window starts where the clip
/// starts and stops here whatever the clip does.
pub const MAX_SECONDS: f64 = 5.0;

/// Where the audible window of one soundboard clip ends, in seconds.
pub fn window_end(trim_start: f64, trim_end: f64) -> f64 {
    trim_end.min(trim_start + MAX_SECONDS)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerSounds {
    server_id: Id,
    server_name: String,
    icon_url: Option<String>,
    sounds: Vec<SoundDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Library {
    /// The audible window, so the picker can preview exactly what is heard.
    #[serde(serialize_with = "seconds")]
    max_seconds: f64,
    selected_id: Option<Id>,
    servers: Vec<ServerSounds>,
}

pub async fn library(State(state): State<AppState>, caller: Caller) -> ApiResult<Response> {
    let user = caller.user;
    let library = state
        .database()
        .read(move |db| -> Result<Library, DbError> {
            let mut statement = db.prepare(
                "SELECT s.\"Id\", s.\"Name\", s.\"IconUrl\" FROM \"Servers\" s
                 WHERE s.\"Id\" IN (SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"UserId\" = ?1)
                 ORDER BY s.\"Name\"",
            )?;
            let servers: Vec<(Id, String, Option<String>)> = statement
                .query_map([user], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?;
            let mut offered = Vec::new();
            for (server, name, icon) in servers {
                let mut sounds = crate::soundboard::sounds_of(db, server)?;
                if sounds.is_empty() {
                    continue;
                }
                // The same window the room is given, so that what somebody
                // auditions in their settings is what a channel hears.
                for sound in &mut sounds {
                    sound.trim_end_seconds = window_end(sound.trim_start_seconds, sound.trim_end_seconds);
                }
                offered.push(ServerSounds { server_id: server, server_name: name, icon_url: icon, sounds });
            }
            let selected: Option<Option<Id>> = db
                .query_row(
                    "SELECT \"JoinSoundId\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
                    [user],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(Library { max_seconds: MAX_SECONDS, selected_id: selected.flatten(), servers: offered })
        })
        .await?;
    Ok(json_response(StatusCode::OK, &library))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_is_heard_for_five_seconds_at_most() {
        assert_eq!(window_end(1.0, 12.0), 6.0);
        assert_eq!(window_end(1.0, 3.0), 3.0);
        assert_eq!(window_end(0.0, 5.0), 5.0);
    }
}
