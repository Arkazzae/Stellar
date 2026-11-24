//! Sending one push: the request `stellarr_webpush` describes, by the
//! outbound client, and what the answer means for the row that owns retries.
//!
//! The sender only ever opens a socket to a public address. That is the whole
//! defence against a person who subscribes with an endpoint that names the
//! operator's own network, so it is fixed here: [`Sender::new`] builds the one
//! the server runs, and no setting touches it. A test reaches a fake push
//! service on the loopback only by building a sender of its own with
//! [`Sender::wired`], which the server never does.
//!
//! The answer is read as far as its status and no further, and is never
//! decoded: what a push service says in a body is of no use, and a large or
//! compressed one must not cost memory or time. A redirect is not followed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rustls::RootCertStore;
use stellarr_outbound::header::{AUTHORIZATION, USER_AGENT};
use stellarr_outbound::{
    AddressPolicy, Body, Client, HeaderName, HeaderValue, Request, Resolve, tls,
};
use stellarr_webpush::{
    Dispatch, PushPayload, SendOptions, ValidatedSubscription, VapidKeys, build_request, classify,
};
use tracing::warn;

use super::TARGET;
use crate::settings::PushSettings;
use crate::values::{Id, Timestamp};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT_VALUE: &str = "Stellarr/2.0";
/// Only the head of the answer is read, so this bounds nothing in memory; it
/// is high so that a service that sends a large body with its status is
/// still heard as the status.
const ANSWER_CEILING: u64 = 1 << 40;

/// A subscription as it is stored.
#[derive(Debug, Clone)]
pub struct Stored {
    pub id: Id,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// Where a test's fake push service is. Never built by the server.
pub struct Wiring {
    /// The roots the fake's certificate is trusted by; the system's when none.
    pub roots: Option<RootCertStore>,
    /// How names are resolved; the system's when none.
    pub resolver: Option<Arc<dyn Resolve>>,
    /// Which addresses the sender may connect to.
    pub policy: AddressPolicy,
    /// The port to connect to instead of 443, since a subscription may name
    /// no other and a test cannot always listen on that one.
    pub port: Option<u16>,
}

/// Sends pushes with the instance's identity.
pub struct Sender {
    vapid: Option<VapidKeys>,
    http: Client,
    policy: AddressPolicy,
    port: Option<u16>,
    unavailable_logged: AtomicBool,
}

impl Sender {
    /// The sender of the running server: the operator's key pair, the system's
    /// roots, public addresses only.
    pub fn new(settings: &PushSettings) -> Sender {
        Sender {
            vapid: super::vapid::keys(settings),
            http: Client::new(),
            policy: AddressPolicy::PublicOnly,
            port: None,
            unavailable_logged: AtomicBool::new(false),
        }
    }

    /// A sender whose push service is somewhere else, for a test.
    pub fn wired(settings: &PushSettings, wiring: Wiring) -> Sender {
        let mut http = match wiring.roots {
            Some(roots) => Client::with_tls_config(tls::client_config(roots)),
            None => Client::new(),
        };
        if let Some(resolver) = wiring.resolver {
            http = http.with_resolver(resolver);
        }
        Sender {
            vapid: super::vapid::keys(settings),
            http,
            policy: wiring.policy,
            port: wiring.port,
            unavailable_logged: AtomicBool::new(false),
        }
    }

    /// Whether push is set up on this instance.
    pub fn is_available(&self) -> bool {
        self.vapid.is_some()
    }

    /// Sends one notification to one browser and says what the outbox should
    /// do with the row and the subscription.
    pub async fn send(&self, subscription: &Stored, payload: &PushPayload) -> Dispatch {
        let Some(vapid) = &self.vapid else {
            if !self.unavailable_logged.swap(true, Ordering::Relaxed) {
                warn!(target: TARGET, "A web push delivery for subscription {} was skipped because Web Push is not configured.", subscription.id);
            }
            return Dispatch::Retry;
        };
        // Checked again at delivery, not only when the row was written: an old
        // or altered row must not make this process call an internal address.
        let Some(validated) = ValidatedSubscription::validate(
            Some(&subscription.endpoint),
            Some(&subscription.p256dh),
            Some(&subscription.auth),
        ) else {
            return Dispatch::Gone;
        };
        let now = Timestamp::now().unix_seconds().max(0) as u64;
        let request = match build_request(
            vapid,
            &validated,
            &payload.to_json(),
            &SendOptions::default(),
            now,
        ) {
            Ok(request) => request,
            Err(error) => {
                // The subscription is still healthy when the notice is too
                // large; its keys being unusable is what drops it.
                return error.dispatch();
            }
        };
        let Some(post) = self.post(&request.url, &request.headers, request.body) else {
            return Dispatch::Gone;
        };
        match self.http.send(post).await {
            Ok(answer) => {
                let status = answer.status.as_u16();
                // The answer's body is not read: dropping it closes the
                // connection.
                let outcome = classify(status, None, now);
                let dispatch = outcome.dispatch();
                if dispatch != Dispatch::Delivered {
                    warn!(target: TARGET, "A web push delivery for subscription {} was refused with status {status}.", subscription.id);
                }
                dispatch
            }
            Err(_) => {
                // Nothing of the error is logged: it can name the endpoint.
                warn!(target: TARGET, "A web push delivery for subscription {} could not reach its push service.", subscription.id);
                Dispatch::Retry
            }
        }
    }

    fn post(&self, url: &str, headers: &[(String, String)], body: Vec<u8>) -> Option<Request> {
        let url = match self.port {
            Some(port) => with_port(url, port)?,
            None => url.to_string(),
        };
        let mut post = Request::post(&url).ok()?;
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
            let mut value = HeaderValue::from_str(value).ok()?;
            if name == AUTHORIZATION {
                value.set_sensitive(true);
            }
            post = post.header(name, value);
        }
        post = post.header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        Some(
            post.body(Body::bytes(body))
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(TIMEOUT)
                .max_response_bytes(ANSWER_CEILING)
                .max_redirects(0)
                .policy(self.policy)
                .decompress(false),
        )
    }
}

/// `https://host/..` as `https://host:port/..`.
fn with_port(url: &str, port: u16) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    Some(format!("https://{}:{port}{}", &rest[..end], &rest[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_goes_after_the_host_and_before_the_path() {
        assert_eq!(
            with_port("https://push.example.test/a/b?c", 5241).as_deref(),
            Some("https://push.example.test:5241/a/b?c")
        );
        assert_eq!(
            with_port("https://[2606:4700::1111]/x", 9).as_deref(),
            Some("https://[2606:4700::1111]:9/x")
        );
        assert_eq!(
            with_port("https://push.example.test?x", 9).as_deref(),
            Some("https://push.example.test:9?x")
        );
    }
}
