//! Who a request is from, and how people sign in: passwords, the setup code,
//! the policy that asks for a second step, sessions and the cookie that
//! carries them, and the flow every proof of identity ends in.

pub mod cookies;
#[cfg(test)]
mod differential;
pub mod email_codes;
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
pub mod recovery_codes;
pub mod security_cookies;
pub mod session;
pub mod setup_code;
pub mod tokens;
pub mod totp;
pub mod trusted_devices;

pub use session::Caller;
