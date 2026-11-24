//! What is done with messages that already exist, and the other ways of
//! reading them: reactions, pins, threads, and searching a channel.
//!
//! `PUT` and `DELETE /api/messages/{messageId}/reactions/{emoji}`, `GET
//! /api/channels/{channelId}/pins`, `PUT` and `DELETE
//! /api/messages/{messageId}/pin`, `GET /api/channels/{channelId}/threads`
//! and `.../threads/{rootId}`, and `GET /api/channels/{channelId}/messages/search`
//! and `.../messages/index`.
//!
//! [`wire`] writes a message as the clients read it; the route that posts,
//! edits and pages messages answers with the same JSON.

mod binding;
mod cursor;
mod index;
mod pins;
mod reactions;
mod search;
mod text;
mod threads;
pub mod wire;

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
