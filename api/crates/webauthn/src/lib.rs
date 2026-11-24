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
