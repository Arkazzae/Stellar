//! Who a request is from, and how people sign in: passwords, the setup code,
//! the policy that asks for a second step, sessions and the cookie that
//! carries them, and the flow every proof of identity ends in.

pub mod cookies;
pub mod events;
pub mod flow;
pub mod password;
pub mod policy;
pub mod session;
pub mod setup_code;
pub mod tokens;

pub use session::Caller;
