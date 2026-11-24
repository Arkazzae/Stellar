//! Web Push, without a network: everything a server needs to notify a browser
//! except the HTTP request itself.
//!
//! - [`ValidatedSubscription`] checks what a browser hands in when it
//!   subscribes.
//! - [`VapidKeys`] holds the server's identity (RFC 8292) and signs the
//!   `Authorization` header.
//! - [`encrypt`] seals a message for one browser (RFC 8291, RFC 8188
//!   `aes128gcm`).
//! - [`build_request`] puts those together into a [`PushRequest`] the
//!   application sends with its own HTTP client.
//! - [`classify`] reads the push service's answer.
//!
//! A stored subscription should be validated again before every delivery, not
//! only when it was saved: a row that was altered or is old must not make the
//! application contact an internal address.
//!
//! The application is responsible for the transport: no redirects, no cookies,
//! no proxy, a socket opened only to a public address, and a deadline. It also
//! asks for the answer as it is and decodes none of it: the status is all that
//! is read, so a push service cannot make the server spend memory or time on
//! a compressed body (the outbound client's `decompress(false)`). Failing
//! to connect or to answer in time is a retry, which is not a status and so is
//! not classified here.

mod address;
pub mod base64url;
mod encrypt;
mod outcome;
mod p256;
mod request;
mod subscription;
mod vapid;

#[cfg(test)]
mod testing;

pub use address::{is_public_v4, is_public_v6};
pub use encrypt::{EncryptError, HEADER_LEN, MAX_BODY, MAX_PLAINTEXT, RECORD_SIZE, encrypt};
pub use outcome::{Dispatch, MAX_RETRY_AFTER, Outcome, classify, parse_retry_after};
pub use request::{
    BuildError, MAX_PAYLOAD_BYTES, PushPayload, PushRequest, SendOptions, TIME_TO_LIVE_SECONDS,
    Urgency, build_request,
};
pub use subscription::{
    AUTH_MAX_LENGTH, AUTH_SECRET_LEN, ENDPOINT_MAX_LENGTH, P256DH_MAX_LENGTH,
    ValidatedSubscription, normalize_endpoint,
};
pub use vapid::{GeneratedKeys, TOKEN_LIFETIME_SECONDS, VapidError, VapidKeys, generate_keys};
