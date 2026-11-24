//! Channel messages: posting, rewriting, removing, and reading a channel's
//! transcript back page by page and as a journal of changes.
//!
//! What a message looks like as JSON is in [`dto`]: the routes of reactions,
//! pins, threads and search read the same rows and show them through
//! [`dto::load`].

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use stellarr_messages::{MessageKey, Messages, MissingValue, Value};

use crate::database::DbError;
use crate::http::{ApiError, Endpoints, Problem};

pub mod cursor;
pub mod dto;
mod edit;
mod history;
pub mod mentions;
pub mod post;

/// The most a message may say, in UTF-16 units: room for a pasted listing, yet
/// bounded for storage, realtime fan-out and transcript rendering.
pub const MAX_CONTENT_UNITS: usize = 20_000;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/channels/{channelId}/messages", post::create)
        .authorized()
        .json_body()
        .patch("/api/messages/{messageId}", edit::edit)
        .authorized()
        .json_body()
        .delete("/api/messages/{messageId}", edit::delete)
        .authorized()
        .get("/api/channels/{channelId}/messages", history::history)
        .authorized()
        .get("/api/channels/{channelId}/messages/sync", history::sync)
        .authorized()
}

/// Why a step inside a transaction stopped: an answer to give, or a wait in
/// slow mode, which also sets a header.
#[derive(Debug)]
pub enum Stop {
    Answer(ApiError),
    /// The author must wait this many whole seconds.
    SlowMode(i64),
}

impl Stop {
    pub fn not_found() -> Stop {
        Stop::Answer(ApiError::not_found())
    }

    /// The answer to the client, in the language of the instance.
    pub fn into_response(self, messages: &Messages) -> Response {
        match self {
            Stop::Answer(error) => error.into_response(),
            Stop::SlowMode(seconds) => {
                let built = (|| -> Result<Problem, MissingValue> {
                    Ok(Problem::new(StatusCode::TOO_MANY_REQUESTS)
                        .title(messages.get(MessageKey::SlowModeActive, &[])?)
                        .detail(messages.get(MessageKey::SlowModeWait, &[("seconds", &seconds)])?))
                })();
                match built {
                    Ok(problem) => {
                        let mut response = problem.into_response();
                        if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                            response.headers_mut().insert(header::RETRY_AFTER, value);
                        }
                        response
                    }
                    Err(error) => ApiError::from(error).into_response(),
                }
            }
        }
    }
}

impl From<ApiError> for Stop {
    fn from(error: ApiError) -> Stop {
        Stop::Answer(error)
    }
}

impl From<DbError> for Stop {
    fn from(error: DbError) -> Stop {
        Stop::Answer(error.into())
    }
}

impl From<rusqlite::Error> for Stop {
    fn from(error: rusqlite::Error) -> Stop {
        Stop::Answer(error.into())
    }
}

impl From<MissingValue> for Stop {
    fn from(error: MissingValue) -> Stop {
        Stop::Answer(error.into())
    }
}

/// A refusal of one field of the request, with the text in the language of the
/// instance.
pub fn field_problem(
    field: &str,
    messages: &Messages,
    key: MessageKey,
    values: &[(&str, Value<'_>)],
) -> ApiError {
    match messages.get(key, values) {
        Ok(text) => Problem::invalid_field(field, text),
        Err(error) => error.into(),
    }
}

/// Trims Unicode white space from both ends.
pub fn trimmed(text: &str) -> &str {
    text.trim_matches(char::is_whitespace)
}

pub(crate) use stellarr_unicode::utf16::length as units;
