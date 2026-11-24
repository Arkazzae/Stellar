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

use crate::http::{Endpoints, RateLimitPolicy, no_store};

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
