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

/// A body that must be a JSON object. A list is no object, though serde would
/// read one into a struct whose members all have defaults.
pub struct Object<T>(pub T);

impl<'de, T: serde::de::DeserializeOwned> serde::Deserialize<'de> for Object<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Object<T>, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if !value.is_object() {
            return Err(serde::de::Error::custom("not an object"));
        }
        T::deserialize(value)
            .map(Object)
            .map_err(serde::de::Error::custom)
    }
}

/// An id in a path, in any spelling the first server's routes took for one:
/// anything else is a route that does not exist.
pub fn guid(text: &str) -> crate::http::ApiResult<crate::values::Id> {
    crate::permissions::membership::parse_guid(text).ok_or_else(crate::http::ApiError::not_found)
}

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
