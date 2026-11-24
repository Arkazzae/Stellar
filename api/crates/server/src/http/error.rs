//! What a handler returns when it cannot give the answer that was asked for.
//!
//! A handler returns `ApiResult<T>`. `?` turns the failures it meets (the
//! database, a message with a missing value) into a 500 problem, and the
//! helpers below make the answers the clients know:
//!
//! - `ApiError::not_found()`, `forbidden()`, `conflict()`, `unauthorized()`
//!   and `bad_request()` answer with the status and an empty body;
//! - `ApiError::from(Problem)` answers with a problem the client can read.
//!
//! Which of the two a route uses is part of what the web client sees: keep
//! whichever the route has always done.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use stellarr_messages::MissingValue;
use tracing::error;

use super::problem::Problem;
use super::trace;
use crate::database::DbError;

pub type ApiResult<T = Response> = Result<T, ApiError>;

#[derive(Debug)]
pub enum ApiError {
    /// A problem with a body.
    Problem(Problem),
    /// A bare status and no body.
    Status(StatusCode),
    /// The request declared a body over the ceiling. The connection is closed
    /// after the answer, since the rest of the body is not read.
    TooLarge,
    /// Something the server got wrong. The cause is logged, never sent.
    Internal(String),
}

impl ApiError {
    pub fn status(status: StatusCode) -> ApiError {
        ApiError::Status(status)
    }

    pub fn bad_request() -> ApiError {
        ApiError::Status(StatusCode::BAD_REQUEST)
    }

    pub fn unauthorized() -> ApiError {
        ApiError::Status(StatusCode::UNAUTHORIZED)
    }

    pub fn forbidden() -> ApiError {
        ApiError::Status(StatusCode::FORBIDDEN)
    }

    pub fn not_found() -> ApiError {
        ApiError::Status(StatusCode::NOT_FOUND)
    }

    pub fn conflict() -> ApiError {
        ApiError::Status(StatusCode::CONFLICT)
    }

    pub fn internal(cause: impl std::fmt::Display) -> ApiError {
        ApiError::Internal(cause.to_string())
    }
}

impl From<Problem> for ApiError {
    fn from(problem: Problem) -> ApiError {
        ApiError::Problem(problem)
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> ApiError {
        ApiError::Internal(error.to_string())
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> ApiError {
        ApiError::Internal(format!("database: {error}"))
    }
}

impl From<MissingValue> for ApiError {
    fn from(error: MissingValue) -> ApiError {
        ApiError::Internal(error.to_string())
    }
}

/// The 500 a failed request gets: the standard problem, which says nothing of
/// the cause, and headers that keep it out of every cache.
pub fn internal_error_response() -> Response {
    let mut response = Problem::new(StatusCode::INTERNAL_SERVER_ERROR).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache,no-store"),
    );
    headers.insert(header::EXPIRES, HeaderValue::from_static("-1"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Problem(problem) => problem.into_response(),
            ApiError::Status(status) => {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = status;
                response
            }
            ApiError::TooLarge => {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
                response
                    .headers_mut()
                    .insert(header::CONNECTION, HeaderValue::from_static("close"));
                response
            }
            ApiError::Internal(cause) => {
                error!(trace_id = %trace::current(), "request failed: {cause}");
                internal_error_response()
            }
        }
    }
}
