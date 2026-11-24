//! A server's soundboard: the catalogue of sounds, their order, and the two
//! rules that decide who may play them.
//!
//! `GET /api/soundboard`, and under `/api/admin/soundboard`: `POST /sounds`,
//! `PATCH` and `DELETE /sounds/{id}`, `PUT /order` and `PATCH /settings`.
//!
//! A sound is an uploaded audio file plus a trim, which never alters the
//! stored file. Playing one in a voice room is a method of the hub; it reads
//! what is kept here (the server's two switches and the sound's trim) and asks
//! [`can_play`] who may.

mod board;
mod format;
mod snapshot;
mod sounds;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use rusqlite::Connection;
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;

pub(crate) use snapshot::sounds_of;

use crate::events::{Audience, Event};
use crate::http::{Endpoints, json_response};
use crate::permissions::authorization::server_wide;
use crate::permissions::privileges::{MANAGE_MESSAGES, MANAGE_SERVER, MODERATE_MEMBERS};
use crate::state::AppState;
use crate::values::Id;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/soundboard", snapshot::read)
        .authorized()
        .post("/api/admin/soundboard/sounds", sounds::create)
        .json_body()
        .authorized()
        .patch("/api/admin/soundboard/sounds/{sound_id}", sounds::update)
        .json_body()
        .authorized()
        .delete("/api/admin/soundboard/sounds/{sound_id}", sounds::remove)
        .authorized()
        .put("/api/admin/soundboard/order", board::reorder)
        .json_body()
        .authorized()
        .patch("/api/admin/soundboard/settings", board::settings)
        .json_body()
        .authorized()
}

/// What a person needs on a server to play its sounds when it keeps the
/// soundboard to staff.
pub const STAFF: i64 = MANAGE_SERVER | MANAGE_MESSAGES | MODERATE_MEMBERS;

/// Whether the member may play the server's sounds under its rule. A seat in
/// the room and the cooldown are the hub's to check, for each play.
pub fn can_play(
    db: &Connection,
    user: Id,
    server: Id,
    everyone_can_play: bool,
) -> rusqlite::Result<bool> {
    Ok(everyone_can_play || server_wide(db, user, server)? & STAFF != 0)
}

/// A sound as a client reads it. The times are written the way the first
/// server wrote a number: `2`, not `2.0`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundDto {
    pub id: Id,
    pub name: String,
    pub glyph: String,
    pub url: String,
    #[serde(serialize_with = "seconds")]
    pub duration_seconds: f64,
    #[serde(serialize_with = "seconds")]
    pub trim_start_seconds: f64,
    #[serde(serialize_with = "seconds")]
    pub trim_end_seconds: f64,
}

/// The server's two switches.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub everyone_can_play: bool,
    pub rate_limited: bool,
}

/// A number of seconds as the first server wrote it. A negative zero is
/// written with its sign, which is how a trim of `-0` comes back.
pub(crate) fn seconds<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if *value == 0.0 && value.is_sign_negative() {
        return match RawValue::from_string("-0".to_string()) {
            Ok(raw) => raw.serialize(serializer),
            Err(error) => Err(serde::ser::Error::custom(error)),
        };
    }
    crate::uploads::json_number(&Some(*value), serializer)
}

/// The sound's address: the clip is an upload of its own.
pub(crate) fn url_of(attachment: Id) -> String {
    format!("/assets/{attachment}")
}

/// A `201` that says where the new thing is.
fn created(location: String, body: &impl Serialize) -> Response {
    let mut response = json_response(StatusCode::CREATED, body);
    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

fn no_content() -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}

/// Tells the members following the server that its soundboard changed.
fn announce_changed(state: &AppState, server: Id) {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Changed {
        server_id: Id,
    }
    state.events().publish(Event::new(
        Audience::Server(server),
        "SoundboardChanged",
        &Changed { server_id: server },
    ));
}
