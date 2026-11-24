//! The operator's command line, `stellarr`.
//!
//! It works on the data folder of an instance, running or stopped: the
//! database, the keys, the staged restore. It never builds a server, starts a
//! worker or opens a port; what it needs of the server's own rules (the
//! settings and their validation, the sealed secrets, the recovery of an
//! account) it takes from the server's library, so that a command run in a
//! container cannot accept a value the server would refuse.
//!
//! [`run`] is the whole tool with its input and output given, which is how the
//! tests drive it; `main.rs` gives it the terminal.

pub mod app;
pub mod backup;
pub mod catalog;
pub mod config;
pub mod database;
pub mod errors;
pub mod help;
pub mod invocation;
pub mod json;
pub mod runtime;
pub mod secrets;
pub mod security;
pub mod sessions;
pub mod status;
pub mod support;
pub mod terminal;
pub mod users;

pub use app::run;
pub use runtime::{Environment, Io};
