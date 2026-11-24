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
use super::forwarded::ClientInfo;

/// A value taken from the path, such as `Path(id): Path<Id>` for
/// `/api/channels/{id}`. A segment that is not a value of its type is a path
/// that does not exist and returns 404 with no body.
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

/// The person a request is from. A handler takes `Caller` on a route that is
/// `.authorized()`, where there always is one.
impl<S: Send + Sync> FromRequestParts<S> for crate::auth::Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<crate::auth::Caller, ApiError> {
        parts
            .extensions
            .get::<crate::auth::Caller>()
            .copied()
            .ok_or_else(ApiError::unauthorized)
    }
}

/// The person a request is from, if there is one, for a route that answers
/// those who are not signed in too.
pub struct MaybeCaller(pub Option<crate::auth::Caller>);

impl<S: Send + Sync> FromRequestParts<S> for MaybeCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<MaybeCaller, ApiError> {
        Ok(MaybeCaller(
            parts.extensions.get::<crate::auth::Caller>().copied(),
        ))
    }
}

/// What a request says about where it came from: the address the proxies
/// vouch for, whether it came over HTTPS, and the browser's name for itself.
#[derive(Clone)]
pub struct Origin {
    pub address: String,
    pub https: bool,
    /// The `User-Agent`, without surrounding whitespace.
    pub user_agent: String,
    /// Every header of the request, for the ones a handler reads itself.
    pub headers: axum::http::HeaderMap,
}

/// The headers carry the cookies, so only where the request came from is shown.
impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Origin")
            .field("address", &self.address)
            .field("https", &self.https)
            .finish_non_exhaustive()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Origin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Origin, ApiError> {
        let client = parts.extensions.get::<super::ClientInfo>();
        Ok(Origin {
            address: client.map_or_else(
                || "unknown".to_string(),
                |client| client.address.to_string(),
            ),
            https: client.is_some_and(ClientInfo::is_https),
            user_agent: parts
                .headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .trim()
                .to_string(),
            headers: parts.headers.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_printed_of_an_origin_holds_neither_its_cookies_nor_its_browser() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "session=sekrit".parse().unwrap(),
        );
        let origin = Origin {
            address: "1.2.3.4".into(),
            https: true,
            user_agent: "agent".into(),
            headers,
        };

        let shown = format!("{origin:?}");

        assert!(
            shown.contains("1.2.3.4") && !shown.contains("sekrit") && !shown.contains("agent"),
            "{shown}"
        );
    }
}
