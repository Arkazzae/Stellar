//! A person's inbox: what was addressed to them, newest first.
//!
//! `GET /api/notifications`, `PUT /api/notifications/{id}/read`, `POST
//! /api/notifications/read-all` and `DELETE /api/notifications/{id}`. Entries
//! are written by the groups they come from ([`add`], [`announce`]).
//!
//! The inbox is private to its owner, so no cache may keep a copy: every
//! answer from the point the server is known to be the person's own carries
//! `Cache-Control: no-store`.

mod entry;
mod page;

pub use entry::{Entry, NewEntry, add, announce, announce_invitations_changed};

use crate::http::Endpoints;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/notifications", page::list)
        .authorized()
        .put("/api/notifications/{notificationId}/read", page::mark_read)
        .authorized()
        .post("/api/notifications/read-all", page::mark_all_read)
        .authorized()
        .delete("/api/notifications/{notificationId}", page::delete)
        .authorized()
}
