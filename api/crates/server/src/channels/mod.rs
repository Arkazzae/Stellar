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

use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use crate::http::{ApiError, Endpoints, Problem, created};
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
pub(crate) struct ChannelDto {
    pub(crate) id: Id,
    name: String,
    kind: &'static str,
    position: i64,
    topic: String,
    slow_mode_seconds: i64,
    hide_after_inactivity_hours: i64,
    user_limit: i64,
    section_id: Option<Id>,
}

impl ChannelDto {
    /// Whether people write in it, as against talk in it.
    pub(crate) fn is_text(&self) -> bool {
        self.kind == "text"
    }
}

/// What a section looks like to the clients.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SectionDto {
    pub(crate) id: Id,
    name: String,
    position: i64,
}

/// Every channel of a server in its order, whoever may see it: the caller
/// narrows the list to what a person sees.
pub(crate) fn channels_of(db: &rusqlite::Connection, server: Id) -> rusqlite::Result<Vec<ChannelDto>> {
    let mut statement = db.prepare(
        "SELECT \"Id\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\",
                \"UserLimit\", \"SectionId\"
         FROM \"Channels\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\"",
    )?;
    statement
        .query_map([server], |row| {
            Ok(ChannelDto {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: kind_name(row.get(2)?),
                position: row.get(3)?,
                topic: row.get(4)?,
                slow_mode_seconds: row.get(5)?,
                hide_after_inactivity_hours: row.get(6)?,
                user_limit: row.get(7)?,
                section_id: row.get(8)?,
            })
        })?
        .collect()
}

/// Every section of a server in its order.
pub(crate) fn sections_of(db: &rusqlite::Connection, server: Id) -> rusqlite::Result<Vec<SectionDto>> {
    let mut statement = db.prepare(
        "SELECT \"Id\", \"Name\", \"Position\" FROM \"Sections\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\"",
    )?;
    statement
        .query_map([server], |row| {
            Ok(SectionDto {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
            })
        })?
        .collect()
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

/// A path's id; one that is not an id is a path that does not exist.
fn path_id(text: &str) -> Result<Id, ApiError> {
    Id::parse(text).ok_or_else(ApiError::not_found)
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
