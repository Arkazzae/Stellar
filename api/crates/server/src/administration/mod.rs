//! What the people who run a server do beyond its profile: invite people, read
//! the audit log, set the notification defaults, and the two things only the
//! owner may do, handing the server on and deleting it.
//!
//! Everything is under `/api/admin`, about the server the request names with
//! its `X-Stellarr-Server` header, else the first the caller joined.

mod audit;
mod invites;
mod notifications;
mod ownership;

use crate::http::Endpoints;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/admin/invites", invites::list)
        .authorized()
        .post("/api/admin/invites", invites::create)
        .json_body()
        .authorized()
        .delete("/api/admin/invites/{invite_id}", invites::revoke)
        .authorized()
        .post(
            "/api/admin/invites/{invite_id}/members/{user_id}",
            invites::send,
        )
        .authorized()
        .get("/api/admin/audit", audit::list)
        .authorized()
        .get("/api/admin/notifications", notifications::read)
        .authorized()
        .patch("/api/admin/notifications", notifications::update)
        .json_body()
        .authorized()
        .post("/api/admin/server/transfer", ownership::transfer)
        .json_body()
        .authorized()
        .delete("/api/admin/server", ownership::delete)
        .authorized()
}
