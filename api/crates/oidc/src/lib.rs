//! OpenID Connect relying party: the parts of single sign-on that are
//! decisions, with no network and no storage.
//!
//! The application makes the HTTP calls with its own client and hands over
//! the bodies it got; this crate says what to send and whether what came back
//! can be trusted. A sign-in goes:
//!
//! 1. [`discovery::parse`] and [`jwks::parse`] read what the provider
//!    publishes.
//! 2. [`flow::start`] builds the redirect and the browser binding to keep.
//! 3. [`flow::accept`] opens the callback; [`token::token_request`] redeems
//!    its code; [`token::parse_token_response`] reads the answer.
//! 4. [`idtoken::verify`] checks the ID token; [`token::userinfo_request`]
//!    and [`token::parse_userinfo`] add the profile; [`profile::Profile`]
//!    holds what the application reads.
//!
//! Every function that depends on time takes `now` in Unix seconds.

pub mod b64;
pub mod discovery;
pub mod flow;
pub mod form;
pub mod idtoken;
pub mod jwks;
pub mod jws;
pub mod options;
pub mod profile;
pub mod sealed;
pub mod ticket;
pub mod token;

/// The largest document read from a provider: discovery, key set, token
/// response or userinfo.
pub const MAX_DOCUMENT: usize = 512 * 1024;
