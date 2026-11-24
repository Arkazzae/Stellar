//! The server for the SignalR hub protocol that the web client speaks, as far
//! as this application uses it: negotiation, the JSON protocol over a
//! WebSocket, calls from the client with and without an answer, events sent
//! to connections and groups, and keep-alive.
//!
//! It is in three parts. [`record`] and [`message`] are the protocol itself,
//! with no I/O. [`Hub`] holds the connections and groups and does the
//! sending. [`router`] serves it over HTTP, and [`serve`] runs one
//! connection on any WebSocket, which is what the tests use.
//!
//! The application implements [`Application`]. Everything sent to a client
//! goes through [`Hub`] and never waits: each connection has a bounded queue,
//! and a client that cannot keep up is closed and starts over.
//!
//! What is left out because the client and the application never use it:
//! the other two transports (the answer to negotiation offers only
//! WebSockets), streaming in either direction, the binary protocol, and
//! sequenced acknowledgements for resuming a connection.

mod app;
mod config;
mod connection;
mod hub;
mod ids;
pub mod message;
mod mount;
pub mod record;

pub use app::{Application, Call, HubError};
pub use config::Config;
pub use connection::serve;
pub use hub::{CloseRequest, Hub, Outbound};
pub use mount::router;
