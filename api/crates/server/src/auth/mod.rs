//! Who a request is from, and how people sign in: passwords, the setup code,
//! the policy that asks for a second step, sessions and the cookie that
//! carries them, and the flow every proof of identity ends in.
//!
//! The second step has its parts here too: `totp`, `email_codes`,
//! `recovery_codes`, `trusted_devices` and `passkeys` are what a proof is made
//! of; `mfa_methods` keeps the rows of the methods an account has and `mfa`
//! checks a proof against a challenge; `fresh` asks for one passed a moment
//! ago; `ways_in` keeps the last way in; `recovery` is an operator's recovery
//! of an account that lost its steps.

pub mod ceremony;
pub mod cookies;
#[cfg(test)]
mod differential;
pub mod email_codes;
pub mod email_send;
pub mod events;
#[cfg(test)]
mod fixtures;
pub mod flow;
pub mod fresh;
pub mod mfa;
pub mod mfa_methods;
pub mod passkeys;
pub mod password;
pub mod policy;
pub mod recovery;
pub mod recovery_codes;
pub mod security_cookies;
pub mod session;
pub mod setup_code;
#[cfg(test)]
mod shared_database;
pub mod tokens;
pub mod totp;
pub mod trusted_devices;
pub mod ways_in;

pub use session::Caller;
