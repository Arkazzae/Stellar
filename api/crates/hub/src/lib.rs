//! The application hub serves the `stellarr.v1` JSON protocol over WebSocket.
//! The application implements [`Application`] to admit a connection to
//! groups and run its calls. [`Hub`] delivers events through bounded queues.

mod app;
mod config;
mod connection;
mod hub;
mod ids;
mod mount;
mod native;

pub use app::{Application, Call, HubError};
pub use config::Config;
pub use connection::serve;
pub use hub::{Hub, Outbound};
pub use mount::router;
