//! Signing in, signing up and proving oneself again through an identity
//! provider the operator has set up (OpenID Connect), and linking such an
//! identity to an account.
//!
//! A provider only proves who a person is: an account is signed in when that
//! identity was linked to it, by the account itself or by an operator, never
//! by matching a mail address. The result is the same revocable session a
//! password sign-in gets, and the instance's own second step is asked for
//! where its policy asks, unless the operator trusts the provider's.
//!
//! An identity no account is linked to waits ten minutes for the browser it
//! came back to (see [`pending`]), and the sign-in page offers to link it to
//! an account signed in with a password, or to make an account from it with an
//! invite.
//!
//! `GET /api/auth/oidc`, `GET` and `DELETE /api/auth/oidc/pending`, the
//! `login` and `link` routes of the original provider and of each named one
//! (`/api/auth/oidc/{id}/login`), the callback each provider is given, and the
//! two routes of `/api/me/security/identities` that start a link and take an
//! identity away. Starting a fresh sign-in at a provider as proof for a
//! sensitive change (`reauthenticate`) is the same round trip with another
//! ticket; what it ends in is recorded on the session by
//! `ceremony::reauthenticated_by_provider`.

mod callback;
mod identities;
mod linking;
mod names;
mod pages;
mod pending;
mod plain_login;
mod single_sign_on;
mod start;

pub use identities::{identity_name, issuer_name, owned};
pub use linking::{Linked, Requester, link_pending, stage};
pub use names::{recorded_name, recorded_name_of, shown_name, shown_name_of};
pub use pending::{
    Held, Pending, clear_cookie as clear_pending_cookie, find as find_pending, held as held_pending,
    take as take_pending,
};
pub use single_sign_on::SingleSignOn;

pub use linking::sign_up_allowed;

use crate::http::{Endpoints, RateLimitPolicy};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/auth/oidc", start::status)
        .get("/api/auth/oidc/pending", start::pending_read)
        .rate_limit(RateLimitPolicy::LoginPoll)
        .delete("/api/auth/oidc/pending", start::pending_drop)
        .rate_limit(RateLimitPolicy::AuthWrite)
        .get("/api/auth/oidc/login", start::login_default)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/oidc/{id}/login", start::login_named)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/oidc/link", start::link_default)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/oidc/{id}/link", start::link_named)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/oidc/reauthenticate", start::reauthenticate_default)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/oidc/{id}/reauthenticate", start::reauthenticate_named)
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/oidc/callback", callback::callback_default)
        .post("/api/auth/oidc/callback", callback::callback_default)
        .put("/api/auth/oidc/callback", callback::callback_default)
        .patch("/api/auth/oidc/callback", callback::callback_default)
        .delete("/api/auth/oidc/callback", callback::callback_default)
        .get("/api/auth/oidc/{id}/callback", callback::callback_named)
        .post("/api/auth/oidc/{id}/callback", callback::callback_named)
        .put("/api/auth/oidc/{id}/callback", callback::callback_named)
        .patch("/api/auth/oidc/{id}/callback", callback::callback_named)
        .delete("/api/auth/oidc/{id}/callback", callback::callback_named)
        .post("/api/me/security/identities/{provider_id}/link", identities::start_link)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/me/security/identities/{provider_id}/reauthenticate", identities::start_reauthentication)
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/me/security/identities/{identity_id}/remove", identities::remove)
        .json_body()
        .authorized()
        .rate_limit(RateLimitPolicy::SecurityWrite)
}
