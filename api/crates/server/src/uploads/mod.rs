//! Uploading files: attachments for messages, the pictures of people and of
//! servers, and the encrypted bytes of end-to-end encrypted conversations.
//!
//! `POST /api/uploads` (also at `/api/uploads/images`, which clients from
//! before uploads took more than images still use), `POST
//! /api/uploads/encrypted`, `POST` and `DELETE /api/me/images/{kind}` and
//! `POST` and `DELETE /api/admin/server/icon`.
//!
//! The form is read while it arrives (see [`crate::http::multipart`]): each
//! file goes to a staging folder as it comes, nothing over its limit is
//! written, and only when the whole form is in is it decided what the answer
//! is, in the order the first server decided it. A file is then checked
//! against the type it claims, room is reserved for it
//! ([`crate::admission`]), and it is moved into the store before its row is
//! written; a row that cannot be written takes its objects out again.

mod attachments;
mod files;
mod form;
mod icon;
mod inspect;
mod numbers;
mod profile;
mod refusals;
#[cfg(test)]
mod witness;

use crate::http::{Endpoints, RateLimitPolicy};

pub use attachments::{Attachment, compact, month, queue_deletion, retire_if_unreferenced, roll_back};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/uploads", files::upload)
        .authorized()
        .rate_limit(RateLimitPolicy::Uploads)
        .post("/api/uploads/images", files::upload)
        .authorized()
        .rate_limit(RateLimitPolicy::Uploads)
        .post("/api/uploads/encrypted", files::encrypted)
        .authorized()
        .rate_limit(RateLimitPolicy::Uploads)
        .post("/api/me/images/{kind}", profile::upload)
        .authorized()
        .rate_limit(RateLimitPolicy::Uploads)
        .delete("/api/me/images/{kind}", profile::remove)
        .authorized()
        .post("/api/admin/server/icon", icon::upload)
        .authorized()
        .rate_limit(RateLimitPolicy::Uploads)
        .delete("/api/admin/server/icon", icon::remove)
        .authorized()
}
