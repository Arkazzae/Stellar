//! A response that sets or takes away the session cookie.

use axum::http::{HeaderValue, header};
use axum::response::Response;

/// An answer that sets a session cookie is kept out of every cache, as the
/// cookie handler of the first server did.
pub fn keep_out_of_caches(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache,no-store"),
    );
    headers.insert(
        header::EXPIRES,
        HeaderValue::from_static("Thu, 01 Jan 1970 00:00:00 GMT"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

/// Adds a `Set-Cookie` line that signs the person in or out to a response.
pub fn with_session_cookie(mut response: Response, line: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(line) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    keep_out_of_caches(&mut response);
    response
}

/// Adds a `Set-Cookie` line of a cookie that is not the session's, with none
/// of the cache headers.
pub fn with_cookie(mut response: Response, line: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(line) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}
