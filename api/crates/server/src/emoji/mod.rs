//! The custom emoji of a server: the list, uploading one, deleting one, and
//! the image behind each.
//!
//! `GET` and `POST /api/emoji`, `DELETE /api/emoji/{id}` and `GET
//! /api/emoji/{id}/image`.
//!
//! An emoji is an image kept under a key of its own, outside the attachments,
//! so that no message can reach it and deleting one never depends on how many
//! messages mention it. Its row's trigger queues the image for deletion.

mod image;
mod upload;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use stellarr_messages::MessageKey;

use crate::auth::Caller;
use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::http::{ApiError, ApiResult, Endpoints, Path, RateLimitPolicy, json_response};
use crate::permissions::audit::{self, AuditEvent};
use crate::permissions::authorization::{Scope, allows};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::MANAGE_SERVER;
use crate::state::AppState;
use crate::values::{Guid, Id};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/emoji", list)
        .authorized()
        .post("/api/emoji", upload::upload)
        .authorized()
        .rate_limit(RateLimitPolicy::Uploads)
        .delete("/api/emoji/{emoji_id}", remove)
        .authorized()
        .get("/api/emoji/{emoji_id}/image", image::image)
        .authorized()
}

/// An emoji as a client reads it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmojiDto {
    id: Id,
    /// The code with its colons, as a message spells it.
    shortcode: String,
    /// Where the image is served from; clients draw it in place of a character.
    glyph: String,
}

impl EmojiDto {
    pub(crate) fn new(id: Id, server: Id, code: &str) -> EmojiDto {
        EmojiDto {
            id,
            shortcode: format!(":{code}:"),
            glyph: format!("/api/emoji/{id}/image?serverId={server}"),
        }
    }
}

/// Tells the members following the server that its emoji changed.
fn announce_changed(state: &AppState, server: Id) {
    /// Nothing but the fact: the clients ask for the list again.
    #[derive(Serialize)]
    struct Nothing {}
    state
        .events()
        .publish(Event::new(Audience::Server(server), "EmojiChanged", &Nothing {}));
}

pub(super) fn may_manage(db: &rusqlite::Connection, user: Id, server: Id) -> rusqlite::Result<bool> {
    allows(db, user, &Scope::server(server), MANAGE_SERVER)
}

async fn list(
    State(state): State<AppState>,
    _caller: Caller,
    ActiveServer(server): ActiveServer,
) -> ApiResult<Response> {
    let emoji = state
        .database()
        .read(move |db| -> Result<Vec<EmojiDto>, DbError> {
            let mut statement = db.prepare(
                "SELECT \"Id\", \"Shortcode\" FROM \"ServerEmoji\" WHERE \"ServerId\" = ?1 ORDER BY \"CreatedAt\", \"Shortcode\"",
            )?;
            let rows = statement
                .query_map([server], |row| {
                    let id: Id = row.get(0)?;
                    let code: String = row.get(1)?;
                    Ok(EmojiDto::new(id, server, &code))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(json_response(StatusCode::OK, &emoji))
}

enum Removal {
    Forbidden,
    Missing,
    Done,
}

async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    ActiveServer(server): ActiveServer,
    Path(Guid(emoji)): Path<Guid>,
) -> ApiResult<Response> {
    let user = caller.user;
    let state_for_text = state.clone();
    let outcome = state
        .database()
        .write(move |tx| -> Result<Removal, DbError> {
            if !may_manage(tx, user, server)? {
                return Ok(Removal::Forbidden);
            }
            let code: Option<String> = tx
                .query_row(
                    "SELECT \"Shortcode\" FROM \"ServerEmoji\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
                    params![emoji, server],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(code) = code else {
                return Ok(Removal::Missing);
            };
            // The row's trigger queues the image for deletion, in this
            // transaction.
            tx.execute("DELETE FROM \"ServerEmoji\" WHERE \"Id\" = ?1", [emoji])?;
            let sentence = state_for_text
                .messages()
                .get(MessageKey::AuditEmojiDeleted, &[("name", &code)])
                .map_err(|error| DbError::Task(error.to_string()))?;
            audit::record(
                tx,
                server,
                user,
                &AuditEvent {
                    action: "emoji.delete".to_string(),
                    sentence,
                    target_type: Some("emoji".to_string()),
                    target_id: Some(emoji),
                    target_name: Some(code),
                    changes: Vec::new(),
                },
            )?;
            Ok(Removal::Done)
        })
        .await?;
    match outcome {
        Removal::Forbidden => Err(ApiError::forbidden()),
        Removal::Missing => Err(ApiError::not_found()),
        Removal::Done => {
            announce_changed(&state, server);
            let mut response = Response::new(axum::body::Body::empty());
            *response.status_mut() = StatusCode::NO_CONTENT;
            Ok(response)
        }
    }
}
