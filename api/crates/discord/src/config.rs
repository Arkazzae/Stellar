//! How the reader talks to Discord: where, with which client, and how long it
//! waits.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use stellarr_outbound::{AddressPolicy, Client};

use crate::time::{Clock, SystemClock};

/// Version 10 of the API, always on this host: no address in a request comes
/// from outside.
pub const API_BASE: &str = "https://discord.com/api/v10/";

/// How long the reader waits for Discord before it gives up and says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPolicy {
    /// The longest single wait. Discord can ask for twenty minutes on a
    /// shared resource; an import job would rather hear that and decide than
    /// sit in one request for it.
    pub max_wait: Duration,
    /// How many 429s in a row one request may get before it fails.
    pub max_rate_limit_retries: u32,
    /// The wait after a 429 that says nothing about how long, as an edge's
    /// HTML page may.
    pub unstated_retry_after: Duration,
    /// How many times a 5xx is tried again, after one pause, then two.
    pub max_server_error_retries: u32,
    pub server_error_pause: Duration,
}

impl Default for RequestPolicy {
    fn default() -> RequestPolicy {
        RequestPolicy {
            max_wait: Duration::from_secs(60),
            max_rate_limit_retries: 5,
            unstated_retry_after: Duration::from_secs(5),
            max_server_error_retries: 2,
            server_error_pause: Duration::from_secs(1),
        }
    }
}

/// Everything a [`crate::DiscordClient`] is made from. [`ClientConfig::default`]
/// is the production setup: Discord's own host, public addresses only, no
/// redirect followed, thirty seconds for a whole answer and thirty-two
/// mebibytes at most of it, and nothing logged that came from a request.
///
/// The base address and the address policy are here for tests that put a
/// server of their own in Discord's place. They are the operator's to set in
/// code; nothing a person or Discord sends reaches them.
#[derive(Clone)]
pub struct ClientConfig {
    pub client: Client,
    /// The API's address with a trailing slash.
    pub base: String,
    pub address_policy: AddressPolicy,
    pub clock: Arc<dyn Clock>,
    pub policy: RequestPolicy,
    /// How long one request may take, its answer included.
    pub timeout: Duration,
    /// The most an answer may weigh. The largest, a page of a hundred
    /// messages, is a few megabytes.
    pub max_response_bytes: u64,
}

impl Default for ClientConfig {
    fn default() -> ClientConfig {
        ClientConfig {
            client: Client::new(),
            base: API_BASE.to_owned(),
            address_policy: AddressPolicy::PublicOnly,
            clock: Arc::new(SystemClock),
            policy: RequestPolicy::default(),
            timeout: Duration::from_secs(30),
            max_response_bytes: 32 * 1024 * 1024,
        }
    }
}

impl fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientConfig")
            .field("base", &self.base)
            .field("address_policy", &self.address_policy)
            .field("policy", &self.policy)
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish_non_exhaustive()
    }
}
