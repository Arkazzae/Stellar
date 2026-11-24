//! What a signed-in person does to the security of their own account: the
//! security page (the second step's methods, recovery codes, remembered
//! browsers, the open sessions and the list of events), setting and changing
//! the password, and changing the mail address.
//!
//! `GET /api/me/security`, `GET /api/me/security/events`, `POST
//! /api/me/security/mfa/enroll/start`, `POST /api/me/security/mfa/reverify/start`,
//! `POST /api/me/security/mfa/{id}/remove`, `POST
//! /api/me/security/recovery/regenerate`, `DELETE /api/me/security/trusted/{id}`,
//! `GET /api/me/security/sessions`, `DELETE /api/me/security/sessions/{id}`,
//! `POST /api/me/security/sessions/revoke-others`, `POST
//! /api/me/security/password`, `POST /api/me/password`, `POST /api/me/email` and
//! `POST /api/auth/email/confirm`.
//!
//! Everything that changes the second step goes through `auth::ceremony`; this
//! group decides only what to answer. Ending a session also tells the hub, so
//! that the connections it held are closed at once and not at the watcher's
//! next look.

mod email_change;
mod methods;
mod page;

pub(crate) use page::{linked_identities, recent_events};
mod password;
mod providers;
mod sessions;
#[cfg(test)]
mod tests;

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::auth::flow::{self, Context};
use crate::auth::security_cookies;
use crate::http::{ApiResult, Endpoints, Origin, RateLimitPolicy};
use crate::state::AppState;

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/me/security", page::read)
        .authorized()
        .get("/api/me/security/events", page::events)
        .authorized()
        .post("/api/me/security/mfa/enroll/start", methods::start_enrollment)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/me/security/mfa/reverify/start", methods::start_reverification)
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/me/security/mfa/{method_id}/remove", methods::remove)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/me/security/recovery/regenerate", methods::regenerate_recovery_codes)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .delete("/api/me/security/trusted/{device_id}", methods::forget_trusted_device)
        .authorized()
        .get("/api/me/security/sessions", sessions::list)
        .authorized()
        .delete("/api/me/security/sessions/{session_id}", sessions::revoke)
        .authorized()
        .post("/api/me/security/sessions/revoke-others", sessions::revoke_others)
        .authorized()
        .post("/api/me/security/password", password::set)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/me/password", password::change)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/me/email", email_change::start)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::AuthWrite)
        // The link may be opened on another device: the token alone proves the
        // new address.
        .post("/api/auth/email/confirm", email_change::confirm)
        .json_body()
        .rate_limit(RateLimitPolicy::AuthWrite)
}

/// Every answer of the routes under `/api/me/security` is kept out of caches,
/// a refusal as much as a page. An answer that sets the session cookie has
/// its own cache headers, which stand.
fn no_store(result: ApiResult<Response>) -> Response {
    let mut response = result.unwrap_or_else(IntoResponse::into_response);
    let headers = response.headers_mut();
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if !headers.contains_key(header::PRAGMA) {
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }
    response
}

/// What a request says about itself, as the second step's calls take it.
fn context<'a>(state: &AppState, origin: &'a Origin, trusted_device: Option<&'a str>) -> Context<'a> {
    Context {
        ip_address: &origin.address,
        user_agent: &origin.user_agent,
        https: origin.https,
        trusted_device_cookie: trusted_device,
        passkeys_available: flow::passkeys_available(&state.settings().operations.public_url),
    }
}

/// The remembered-device token of the browser, if it holds one.
fn trusted_device_of(origin: &Origin) -> Option<String> {
    security_cookies::read_trusted_device(&origin.headers, origin.https)
}
