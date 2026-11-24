//! The servers an instance holds: the ones a person is in and the public ones
//! they could join, making a new one, joining one, the page an invite link
//! opens, and a server's own profile.
//!
//! `GET /api/servers`, `GET /api/servers/discover`, `POST /api/servers`, `POST
//! /api/servers/{id}/join`, `GET /api/invites/{code}`, `POST
//! /api/invites/{code}/join` and `PATCH /api/admin/server`.

mod announce;
mod directory;
mod invites;
mod join;
mod profile;

use crate::http::{Endpoints, RateLimitPolicy};

pub use announce::{invitation_arrived, invitations_changed};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/servers", directory::list)
        .authorized()
        .get("/api/servers/discover", directory::discover)
        .authorized()
        .post("/api/servers", directory::create)
        .json_body()
        .authorized()
        .post("/api/servers/{server_id}/join", directory::join_public)
        .authorized()
        // Read by the page an invite link opens, before anybody signs in.
        .get("/api/invites/{code}", invites::preview)
        .rate_limit(RateLimitPolicy::InvitePreview)
        .post("/api/invites/{code}/join", invites::join)
        .authorized()
        .patch("/api/admin/server", profile::update)
        .json_body()
        .authorized()
}
