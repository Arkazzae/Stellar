//! Who may do what on a server, and who sees which channel.
//!
//! The permission rules are documented in `docs/architecture.md`
//! under "Server authorization" and "Private channels". Every decision comes
//! from the one pure function in [`evaluator`]; [`snapshot`] loads what it
//! reads; [`authorization`] and [`visibility`] are what a handler calls.
//!
//! Nothing is cached: every call asks the database it is lent, so a change of
//! roles or overrides counts from the next request on. Every function takes
//! the connection of the closure it runs in (`read` or `write`) and never
//! opens one.

pub mod access_changed;
pub mod audit;
pub mod authorization;
pub mod evaluator;
pub mod membership;
pub mod ownership;
pub mod privileges;
pub mod roles;
pub mod snapshot;
pub mod visibility;

mod error;
#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;

pub use error::DataError;
