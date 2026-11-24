//! Signing the desktop application in through the system browser, where the
//! single sign-on provider works.
//!
//! The application starts a request and keeps its secret. The person opens the
//! approval page in a browser where they are (or become) signed in, and
//! approves with a button, a form post bound to them, their browser session
//! and the request. The application then trades the request id and its secret
//! for a session of its own, so the browser never learns the secret and the
//! application never sees the browser's cookie.
//!
//! `POST /api/auth/desktop/browser/start` and `GET` and `POST
//! /api/auth/desktop/browser/verify/{id}` are here, and the last step, `POST
//! /api/auth/desktop/browser/exchange`, which the application polls.

mod approval;
mod exchange;
mod form;
mod page;
mod same_origin;
mod start;

use axum::http::{HeaderValue, header};
use axum::response::Response;

use crate::http::{Endpoints, RateLimitPolicy};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/auth/desktop/browser/start", start::start)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/desktop/browser/verify/{request_id}", approval::show)
        .authorized()
        .post("/api/auth/desktop/browser/verify/{request_id}", approval::approve)
        .authorized()
        .post("/api/auth/desktop/browser/exchange", exchange::exchange)
        .json_body()
        .rate_limit(RateLimitPolicy::LoginPoll)
}

/// Every answer of this route is kept out of every cache.
fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}
