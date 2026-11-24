//! What is done with messages that already exist, and the other ways of
//! reading them: reactions, pins, threads, and searching a channel.
//!
//! `PUT` and `DELETE /api/messages/{messageId}/reactions/{emoji}`, `GET
//! /api/channels/{channelId}/pins`, `PUT` and `DELETE
//! /api/messages/{messageId}/pin`, `GET /api/channels/{channelId}/threads`
//! and `.../threads/{rootId}`, and `GET /api/channels/{channelId}/messages/search`
//! and `.../messages/index`.
//!
//! A message is written by [`crate::messages::dto`], as everywhere.

mod cursor;
mod index;
mod ordinal_case;
mod pins;
mod reactions;
mod search;
mod text;
mod threads;

use crate::http::Endpoints;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .put("/api/messages/{messageId}/reactions/{emoji}", reactions::add)
        .authorized()
        .delete("/api/messages/{messageId}/reactions/{emoji}", reactions::remove)
        .authorized()
        .get("/api/channels/{channelId}/pins", pins::list)
        .authorized()
        .put("/api/messages/{messageId}/pin", pins::pin)
        .authorized()
        .delete("/api/messages/{messageId}/pin", pins::unpin)
        .authorized()
        .get("/api/channels/{channelId}/threads", threads::list)
        .authorized()
        .get("/api/channels/{channelId}/threads/{rootId}", threads::read)
        .authorized()
        .get("/api/channels/{channelId}/messages/search", index::search)
        .authorized()
        .get("/api/channels/{channelId}/messages/index", index::page)
        .authorized()
}
