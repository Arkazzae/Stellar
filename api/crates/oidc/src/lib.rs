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
//!
//! # What the application must do itself
//!
//! This crate decides whether an answer is genuine. It does not know accounts
//! or sessions, so the application must:
//!
//! - **Link and reauthentication.** [`flow::accept`] returns the account that
//!   asked (the [`flow::Actor`] in the flow). The provider's answer is a
//!   cross-site post that carries no session cookie, so before acting the
//!   application must check again that this session is still active, its
//!   security version unchanged, the account neither disabled nor waiting for
//!   recovery, and the provider the same as the one the request was for.
//! - **Tickets.** A [`ticket::Ticket`]'s account and provider must be compared
//!   with the session presenting it and with the route it is presented to. A
//!   ticket can be read more than once within its five minutes.
//! - **The binding cookie.** The provider answers with a cross-site post, so
//!   the cookie that carries [`flow::Started::browser_binding`] must be
//!   `SameSite=None; Secure` (and `HttpOnly`), and cleared once the answer is
//!   handled.
//! - **Replay.** [`flow::ReplayGuard`] lives in memory, so a restart forgets
//!   it. The route that starts a sign-in needs no account and must be
//!   rate-limited, or the guard can be filled until answers are `Busy`.
//! - **Keys.** On [`idtoken::IdTokenError::UnknownKeyId`], fetch the key set
//!   again once and verify again.

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
