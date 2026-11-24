//! Path and query parameters, with the refusals the clients know.
//!
//! Handlers take these instead of axum's own extractors of the same names,
//! whose refusal is a plain-text sentence. Together with [`super::Json`] they
//! are the only ways a handler reads a request: path, query and body go
//! through these, and nothing else.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use super::error::ApiError;

/// A value taken from the path, such as `Path(id): Path<Id>` for
/// `/api/channels/{id}`. A segment that is not a value of its type is a path
/// that does not exist: 404 with no body, as it was where the first server
/// typed a segment (`{id:guid}`).
pub struct Path<T>(pub T);

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for Path<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Path<T>, ApiError> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Path(value))
            .map_err(|_| ApiError::not_found())
    }
}

/// The query string as a struct. One that does not fit is a bad request with
/// no body.
pub struct Query<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for Query<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Query<T>, ApiError> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Query(value))
            .map_err(|_| ApiError::bad_request())
    }
}
