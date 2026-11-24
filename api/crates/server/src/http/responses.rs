use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Serialize;

use super::json_response;

/// A created resource with its location, where it can be written as a header.
pub(crate) fn created(location: impl AsRef<str>, body: &impl Serialize) -> Response {
    let mut response = json_response(StatusCode::CREATED, body);
    if let Ok(value) = HeaderValue::from_str(location.as_ref()) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

pub(crate) fn no_content() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}

pub(crate) fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}
