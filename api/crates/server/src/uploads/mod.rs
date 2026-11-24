//! Uploading files: attachments for messages, the pictures of people and of
//! servers, and the encrypted bytes of end-to-end encrypted conversations.
//!
//! `POST /api/uploads/images` (the name is from before uploads took more than
//! images; it is the one address for a file), `POST /api/uploads/encrypted`, `POST` and `DELETE /api/me/images/{kind}` and
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

use crate::http::{Endpoints, RateLimitPolicy};

pub use attachments::{Attachment, compact, month, retire_if_unreferenced, roll_back};
// What the groups that keep a file of their own (a custom emoji) borrow from
// here, so that they take the form, the checks and the refusals the same way.
pub(crate) use attachments::number as json_number;
pub(crate) use files::read as read_form;
pub(crate) use form::Slot;
pub(crate) use inspect::{Failure, image as inspect_image, keep_image};
pub(crate) use profile::broadcast_user_updated;
pub(crate) use refusals::{no_room, problem};

pub fn routes() -> Endpoints {
    Endpoints::new()
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
