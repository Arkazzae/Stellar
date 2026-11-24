//! The ways in that are not a password alone, and the way back for a password
//! that is forgotten: the second half of a sign-in (a code from an app, a
//! mailed code, a recovery code, a passkey, a remembered browser), setting a
//! second step up when the instance demands one, signing in with a passkey
//! alone, redeeming the credential an operator hands over after a recovery, and
//! resetting a forgotten password by mail.
//!
//! Every route here answers the person who holds a challenge, and a challenge
//! belongs to the browser it was made for: the cookie that binds it and the id
//! the client sends must agree. What each route does with a challenge is one
//! call of `auth::ceremony`, which owns the transaction; what is left to a
//! route is to read the request, answer from the outcome, and set the cookies.

mod common;
mod enroll;
mod link;
mod mfa;
mod passkey;
mod passkey_sign_in;
mod password_reset;
mod recovery;
mod wire;

use crate::http::{Endpoints, RateLimitPolicy};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .get("/api/auth/mfa/pending", mfa::pending)
        .rate_limit(RateLimitPolicy::LoginPoll)
        .post("/api/auth/mfa/cancel", mfa::cancel)
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/auth/mfa/verify", mfa::verify)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/mfa/email/send", mfa::send_login_code)
        .json_body()
        .rate_limit(RateLimitPolicy::AuthWrite)
        .post("/api/auth/mfa/enroll/totp/start", enroll::start_totp)
        .json_body()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/auth/mfa/enroll/totp/confirm", enroll::confirm_totp)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/mfa/enroll/email/start", mfa::start_email)
        .json_body()
        .rate_limit(RateLimitPolicy::AuthWrite)
        .post("/api/auth/mfa/enroll/email/confirm", enroll::confirm_email)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/mfa/passkey/options", passkey::options)
        .json_body()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/auth/mfa/passkey/verify", passkey::verify)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/mfa/enroll/passkey/start", enroll::start_passkey)
        .json_body()
        .rate_limit(RateLimitPolicy::SecurityWrite)
        .post("/api/auth/mfa/enroll/passkey/confirm", enroll::confirm_passkey)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .get("/api/auth/passkey", passkey_sign_in::available)
        .post("/api/auth/passkey/options", passkey_sign_in::options)
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/passkey/login", passkey_sign_in::sign_in)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/recovery/redeem", recovery::redeem)
        .json_body()
        .rate_limit(RateLimitPolicy::Login)
        .post("/api/auth/password/forgot", password_reset::forgot)
        .json_body()
        .rate_limit(RateLimitPolicy::AuthWrite)
        .post("/api/auth/password/reset", password_reset::reset)
        .json_body()
        .rate_limit(RateLimitPolicy::AuthWrite)
}
