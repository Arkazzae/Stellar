//! What the sign-in through a provider keeps between requests, and every
//! call it makes to a provider.
//!
//! A provider is a host the operator named, so the calls go to any address,
//! a provider on the local network included, as the first server's did. The
//! documents (discovery and keys) follow a few redirects, as its client did;
//! the token request and the profile request follow none, since a redirected
//! token request would carry the code to a host nobody named. Every call has
//! the client's own timeouts (ten seconds to connect, thirty in all, where the
//! first server waited a minute) and a ceiling on the answer.
//!
//! What a provider publishes is kept for twelve hours and read again earlier
//! only when a token names a key that is not in the set, and then at most
//! once in five minutes, which is what the first server's library did. Nothing
//! is fetched at start: a provider that is down must not stop the server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use stellarr_oidc::discovery::{self, Discovery};
use stellarr_oidc::flow::ReplayGuard;
use stellarr_oidc::jwks::{self, KeySet};
use stellarr_oidc::options::Provider;
use stellarr_oidc::sealed::SealingKey;
use stellarr_oidc::token::{TokenRequest, UserinfoRequest};
use stellarr_outbound::{
    AddressPolicy, Body, Client, HeaderValue, Request, header,
};

use crate::seal::Sealer;

/// How long what a provider published is trusted without asking again.
const REFRESH_AFTER: Duration = Duration::from_secs(12 * 60 * 60);
/// How soon a token that names an unknown key may make the keys be read again.
const FORCE_EVERY: Duration = Duration::from_secs(5 * 60);
/// Redirects a document may be moved through.
const DOCUMENT_REDIRECTS: u32 = 5;
/// The most an answer may be: the crate's own limit on a document, with room
/// for the head.
const CEILING: u64 = stellarr_oidc::MAX_DOCUMENT as u64 + 16 * 1024;

/// Why a call to a provider did not give what it was for. Only ever logged.
#[derive(Debug)]
pub struct Failed(pub String);

impl Failed {
    fn of(error: impl std::fmt::Display) -> Failed {
        Failed(error.to_string())
    }
}

/// What a provider publishes about itself.
#[derive(Clone)]
pub struct Metadata {
    pub discovery: Arc<Discovery>,
    pub keys: Arc<KeySet>,
    fetched: Instant,
}

#[derive(Default)]
struct Slot {
    current: Option<Metadata>,
    forced: Option<Instant>,
}

pub struct SingleSignOn {
    key: SealingKey,
    guard: ReplayGuard,
    client: Client,
    slots: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Slot>>>>,
}

impl SingleSignOn {
    pub fn new(sealer: &Sealer) -> SingleSignOn {
        SingleSignOn {
            key: SealingKey::from_bytes(&sealer.derive_key("oidc"))
                .expect("the derived key has the length the library takes"),
            guard: ReplayGuard::new(),
            client: Client::new(),
            slots: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn key(&self) -> &SealingKey {
        &self.key
    }

    pub(super) fn guard(&self) -> &ReplayGuard {
        &self.guard
    }

    fn slot(&self, provider: &Provider) -> Arc<tokio::sync::Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.entry(provider.id.clone()).or_default().clone()
    }

    /// What the provider published, from the copy kept or read now.
    pub async fn metadata(&self, provider: &Provider) -> Result<Metadata, Failed> {
        let slot = self.slot(provider);
        let mut slot = slot.lock().await;
        if let Some(current) = &slot.current
            && current.fetched.elapsed() < REFRESH_AFTER
        {
            return Ok(current.clone());
        }
        let fresh = self.fetch(provider).await?;
        slot.current = Some(fresh.clone());
        Ok(fresh)
    }

    /// The same, read again because a token named a key `seen` does not have:
    /// once in five minutes, and not at all when somebody else just read it.
    /// `None` when there is nothing newer to try.
    pub async fn metadata_again(
        &self,
        provider: &Provider,
        seen: &Metadata,
    ) -> Result<Option<Metadata>, Failed> {
        let slot = self.slot(provider);
        let mut slot = slot.lock().await;
        if let Some(current) = &slot.current
            && current.fetched > seen.fetched
        {
            return Ok(Some(current.clone()));
        }
        if slot.forced.is_some_and(|at| at.elapsed() < FORCE_EVERY) {
            return Ok(None);
        }
        slot.forced = Some(Instant::now());
        let fresh = self.fetch(provider).await?;
        slot.current = Some(fresh.clone());
        Ok(Some(fresh))
    }

    async fn fetch(&self, provider: &Provider) -> Result<Metadata, Failed> {
        let url = discovery::discovery_url(provider.authority());
        let body = self.document(&url).await?;
        let found = discovery::parse(&body).map_err(Failed::of)?;
        let keys_body = self.document(&found.jwks_uri).await?;
        let keys = jwks::parse(&keys_body).map_err(Failed::of)?;
        Ok(Metadata {
            discovery: Arc::new(found),
            keys: Arc::new(keys),
            fetched: Instant::now(),
        })
    }

    async fn document(&self, url: &str) -> Result<Vec<u8>, Failed> {
        let request = Request::get(url)
            .map_err(Failed::of)?
            .header(header::ACCEPT, HeaderValue::from_static("application/json"))
            .policy(AddressPolicy::Anywhere)
            .max_redirects(DOCUMENT_REDIRECTS)
            .max_response_bytes(CEILING);
        let answer = self.client.send_collected(request).await.map_err(Failed::of)?;
        if !answer.status.is_success() {
            return Err(Failed(format!("a document answered {}", answer.status)));
        }
        Ok(answer.body.to_vec())
    }

    /// Redeems the code at the token endpoint: the status and the body.
    pub async fn redeem(&self, request: &TokenRequest) -> Result<(u16, Vec<u8>), Failed> {
        let asked = Request::post(&request.url)
            .map_err(Failed::of)?
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static(TokenRequest::CONTENT_TYPE),
            )
            .header(header::ACCEPT, HeaderValue::from_static("application/json"))
            .body(Body::bytes(request.body.clone().into_bytes()))
            .policy(AddressPolicy::Anywhere)
            .max_redirects(0)
            .max_response_bytes(CEILING);
        let answer = self.client.send_collected(asked).await.map_err(Failed::of)?;
        Ok((answer.status.as_u16(), answer.body.to_vec()))
    }

    /// Asks for the person's profile: the status, the content type and the body.
    pub async fn profile(
        &self,
        request: &UserinfoRequest,
    ) -> Result<(u16, Option<String>, Vec<u8>), Failed> {
        let mut authorization =
            HeaderValue::from_str(&request.authorization).map_err(Failed::of)?;
        authorization.set_sensitive(true);
        let asked = Request::get(&request.url)
            .map_err(Failed::of)?
            .header(header::AUTHORIZATION, authorization)
            .header(header::ACCEPT, HeaderValue::from_static("application/json"))
            .policy(AddressPolicy::Anywhere)
            .max_redirects(0)
            .max_response_bytes(CEILING);
        let answer = self.client.send_collected(asked).await.map_err(Failed::of)?;
        let content_type = answer
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        Ok((answer.status.as_u16(), content_type, answer.body.to_vec()))
    }
}

/// Now, in Unix seconds, as the crate takes it.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}
