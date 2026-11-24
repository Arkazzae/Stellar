//! WebAuthn for Stellarr: the options a browser is given, and the checks on
//! what it sends back. No I/O and no storage: challenges, sessions and
//! credentials stay with the application, which hands in what it kept.
//!
//! What is supported is exactly what the application asks for:
//! - algorithms: EdDSA (Ed25519), ES256, ES384, ES512, RS256, RS384, RS512,
//!   PS256, PS384 and PS512, in that order of preference ([`Algorithm`]);
//! - attestation not requested; `none` and `packed` self attestation are
//!   accepted, every other statement is refused;
//! - user verification required in every ceremony;
//! - a discoverable credential preferred, one origin (or a few) per relying
//!   party, no cross-origin ceremonies, no extensions requested.
//!
//! # What the application must do itself
//!
//! This crate checks one answer against what it is given. These parts stay
//! with the caller:
//! - A credential id must be unique across all accounts. Keep a unique index
//!   and make the check at registration atomic with the insert, because a
//!   sign-in finds the credential by its id alone.
//! - A challenge is used once and expires. Delete or mark it on the first
//!   attempt, right or wrong, and refuse an expired one before calling in.
//! - The stored signature count is replaced with a compare-and-set on the
//!   count that was read, so that two simultaneous sign-ins with the same
//!   count cannot both pass.
//! - For sign-in without a user name, set `require_user_handle` and take the
//!   allowed credential list from the options that were stored (an empty list
//!   when they named none).
//! - Keep the account's credentials apart: the stored credential handed to
//!   [`verify_assertion`] must already be the one that belongs to the account
//!   being signed in.
//!
//! Signatures and hashes come from `aws-lc-rs`; [`cbor`] and [`cose`] are
//! the crate's own readers.

mod authdata;
mod base64url;
pub mod cbor;
mod ceremony;
mod client_data;
pub mod cose;
mod error;
pub mod options;
mod relying_party;

pub mod assertion;
pub mod registration;

pub use assertion::{
    Assertion, AssertionBody, AssertionRequest, AssertionResponse, StoredCredential,
    verify_assertion,
};
pub use cose::Algorithm;
pub use error::Error;
pub use options::{
    Challenge, CredentialDescriptor, Options, Transport, User, assertion_options, creation_options,
};
pub use registration::{AttestationBody, Registration, RegistrationResponse, verify_registration};
pub use relying_party::RelyingParty;

/// Base64url without padding, as every binary member of the options and
/// answers is written.
pub mod b64 {
    pub use crate::base64url::{decode, encode};
}
