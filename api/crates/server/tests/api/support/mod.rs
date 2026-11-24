//! What the integration tests stand on: a server of their own for each test,
//! and a client to talk to it.
//!
//! A test starts the real `stellarr-api` binary on a free port, with a
//! temporary data folder, and waits until `/health/ready` answers. When the
//! test's `Server` goes out of scope the process is killed and the folder
//! removed. Tests run in parallel, each with a server of its own, so they
//! share nothing.
//!
//! A test of the pipeline itself, which needs a route that fails or is rate
//! limited, starts the server in the test's own process with
//! [`Server::in_process`] and the routes it wants: the same library, the same
//! pipeline, the same client.

#![allow(dead_code)]

mod client;
mod hub;
mod log;
mod people;
mod raw;
mod server;

pub use client::{Client, Request, Response};
pub use hub::{HubClient, upgrade};
pub use log::LogCapture;
#[allow(unused_imports)]
pub use people::{PASSWORD, Person};
pub use raw::{Answer, raw_get, raw_request, raw_send};
pub use server::{Server, TempDir};
