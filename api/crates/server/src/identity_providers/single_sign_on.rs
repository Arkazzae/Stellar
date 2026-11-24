//! What the sign-in through a provider keeps between requests, and every
//! call it makes to a provider.
//!
//! A provider is a host the operator named, so the calls go to any address,
//! including providers on local networks. The discovery and key requests
//! follow a limited number of redirects;
//! the token request and the profile request follow none, since a redirected
//! token request would carry the code to a host nobody named. Every call has
//! the client's timeouts (ten seconds to connect, thirty in all) and a
//! limit on the response size.
//!
//! What a provider publishes is kept for twelve hours and read again earlier
//! only when a token names a key that is not in the set, and then at most
//! once in five minutes. Nothing
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
    /// None when the provider's key set could not be read; sign-in does not
    /// need it until an answer comes back.
    pub keys: Option<Arc<KeySet>>,
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

    /// A token named a key the copy does not have. The provider may have
    /// rotated its keys, so what it publishes is read again, for the sign-ins
    /// after this one, which has already failed.
    /// At most once in five minutes, so that tokens cannot make this server
    /// read it over and over.
    pub async fn ask_for_refresh(&self, provider: &Provider) {
        let slot = self.slot(provider);
        let mut slot = slot.lock().await;
        if slot.forced.is_some_and(|at| at.elapsed() < FORCE_EVERY) {
            return;
        }
        slot.forced = Some(Instant::now());
        match self.fetch(provider).await {
            Ok(fresh) => slot.current = Some(fresh),
            Err(failed) => tracing::warn!("\"{}\" cannot be read again: {}", provider.id, failed.0),
        }
    }

    /// The keys of a provider whose metadata was read without them, read now.
    pub async fn keys_of(
        &self,
        provider: &Provider,
        metadata: &Metadata,
    ) -> Result<Arc<KeySet>, Failed> {
        if let Some(keys) = &metadata.keys {
            return Ok(keys.clone());
        }
        let slot = self.slot(provider);
        let mut slot = slot.lock().await;
        let keys = Arc::new(self.keys(&metadata.discovery).await?);
        if let Some(current) = slot.current.as_mut() {
            current.keys = Some(keys.clone());
        }
        Ok(keys)
    }

    async fn keys(&self, discovery: &Discovery) -> Result<KeySet, Failed> {
        let body = self.document(&discovery.jwks_uri).await?;
        jwks::parse(&body).map_err(Failed::of)
    }

    async fn fetch(&self, provider: &Provider) -> Result<Metadata, Failed> {
        let url = discovery::discovery_url(provider.authority());
        let body = self.document(&url).await?;
        let found = discovery::parse(&body).map_err(Failed::of)?;
        // Failure to fetch keys stops sign-in. An empty key set permits sign-in
        // to start, but its response cannot be verified and triggers a refresh.
        let keys = match self.document(&found.jwks_uri).await {
            Err(failed) => return Err(failed),
            Ok(body) => match jwks::parse(&body) {
                Ok(keys) => Some(Arc::new(keys)),
                Err(jwks::JwksError::NoUsableKeys) => None,
                Err(error) => return Err(Failed::of(error)),
            },
        };
        Ok(Metadata {
            discovery: Arc::new(found),
            keys,
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
