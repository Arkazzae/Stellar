//! The channels of a server and the sections they sit in: making, ordering,
//! changing and removing them, the marker of how far a person has read, and
//! the two hub calls about a text channel.
//!
//! `POST /api/admin/channels`, `PUT /api/admin/channels/order`,
//! `PATCH` and `DELETE /api/admin/channels/{id}`, the same four under
//! `/api/admin/sections`, and `PUT /api/channels/{id}/read`. The hub's
//! `JoinChannel` and `SendTyping` are in [`hub`].
//!
//! A channel the caller does not see does not exist for them: every route
//! answers as for an id that was never made. Who is told of a change is
//! decided when the event is sent, from what each person sees then
//! ([`layout`]).

mod channel;
pub mod hub;
mod layout;
mod names;
mod order;
mod read;
mod section;

use axum::http::StatusCode;
use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use crate::http::{ApiError, Endpoints, Problem};
use crate::permissions::visibility::ChannelKind;
use crate::values::Id;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/admin/channels", channel::create)
        .authorized()
        .json_body()
        .put("/api/admin/channels/order", order::reorder_channels)
        .authorized()
        .json_body()
        .patch("/api/admin/channels/{channelId}", channel::update)
        .authorized()
        .json_body()
        .delete("/api/admin/channels/{channelId}", channel::delete)
        .authorized()
        .post("/api/admin/sections", section::create)
        .authorized()
        .json_body()
        .put("/api/admin/sections/order", order::reorder_sections)
        .authorized()
        .json_body()
        .patch("/api/admin/sections/{sectionId}", section::rename)
        .authorized()
        .json_body()
        .delete("/api/admin/sections/{sectionId}", section::delete)
        .authorized()
        .put("/api/channels/{channelId}/read", read::mark_read)
        .authorized()
}

/// What a channel looks like to the clients.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelDto {
    id: Id,
    name: String,
    kind: &'static str,
    position: i64,
    topic: String,
    slow_mode_seconds: i64,
    hide_after_inactivity_hours: i64,
    user_limit: i64,
    section_id: Option<Id>,
}

/// What a section looks like to the clients.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SectionDto {
    id: Id,
    name: String,
    position: i64,
}

fn kind_name(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Text => "text",
        ChannelKind::Voice => "voice",
    }
}

/// How a request that went no further is answered. A handler's closure
/// decides it inside its transaction and the handler turns it into an
/// answer, so that nothing was written when it is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refuse {
    /// 404, with no body: it does not exist for the caller.
    NotFound,
    /// 403, with no body.
    Forbidden,
    /// 409, with no body.
    Conflict,
    /// 400 with the validation problem: the field and what is wrong with it.
    Invalid(&'static str, MessageKey),
    /// 403 with the code clients ask for a second step by: a change of what
    /// somebody may do on the instance's own server, from a session whose
    /// second step is not recent.
    SecondStep,
}

impl Refuse {
    fn into_error(self, messages: &Messages) -> Result<ApiError, stellarr_messages::MissingValue> {
        Ok(match self {
            Refuse::NotFound => ApiError::not_found(),
            Refuse::Forbidden => ApiError::forbidden(),
            Refuse::Conflict => ApiError::conflict(),
            Refuse::SecondStep => ApiError::from(crate::auth::fresh::refusal(messages)?),
            Refuse::Invalid(field, key) => ApiError::from(Problem::validation(vec![(
                field.to_string(),
                vec![messages.get(key, &[])?],
            )])),
        })
    }
}

/// What a closure of a handler comes to: done, or refused.
enum Answer<T> {
    Done(T),
    Refused(Refuse),
}

fn created(location: &str, body: &impl Serialize) -> axum::response::Response {
    let mut response = crate::http::json_response(StatusCode::CREATED, body);
    if let Ok(value) = axum::http::HeaderValue::from_str(location) {
        response
            .headers_mut()
            .insert(axum::http::header::LOCATION, value);
    }
    response
}

/// A path's id the way the first server's route constraint read one, which is
/// as leniently as any other GUID text; one that is not an id is a path that
/// does not exist.
fn path_id(text: &str) -> Result<Id, ApiError> {
    crate::permissions::membership::parse_guid(text).ok_or_else(ApiError::not_found)
}

/// A sentence for the audit log. A text with a value missing is a fault in
/// the code, answered as any other failure of the request.
fn say(
    messages: &Messages,
    key: MessageKey,
    values: &[(&str, stellarr_messages::Value<'_>)],
) -> Result<String, crate::database::DbError> {
    messages
        .get(key, values)
        .map_err(|error| crate::database::DbError::Task(error.to_string()))
}
