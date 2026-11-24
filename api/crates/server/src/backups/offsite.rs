//! The offsite copy of a completed archive: a private bucket on Cloudflare R2
//! (or any S3-compatible store) reached with S3 credentials, or the relay
//! Worker that sits in front of one and takes a token instead.
//!
//! Sending is made to be repeated. An archive is first looked up under its
//! key; one that is there with the same length and checksum counts as sent,
//! so a lost answer never needs an overwrite, which a locked bucket would
//! refuse. After an upload the object is asked about again and must match.
//!
//! Nothing that identifies a credential is ever put in an error or a log line:
//! a failure is described by what kind of failure it was, not by the store's
//! own words, because a store's error document may name the access key.

use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::digest;
use stellarr_outbound::{
    AddressPolicy, Body, Client as Http, HeaderName, HeaderValue, Method, Request,
};
use stellarr_s3 as s3;
use stellarr_storage::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::BackupFailed;
use super::destination::Destination as DestinationTrait;
use super::policy;
use crate::secrets;
use crate::settings::{R2_MODE_S3, R2_MODE_WORKER};
use crate::state::AppState;
use crate::values::Id;

const MIB: u64 = 1024 * 1024;
const CONTENT_TYPE: &str = "application/zip";
/// How a failed call is worded when nothing more specific is known.
const SETTINGS_MISSING: &str = "Backup destination credentials or settings are missing.";
const METADATA_MISMATCH: &str = "Remote backup metadata does not match the local archive.";

/// How the upload is cut up and retried. The defaults are what runs; tests
/// make the parts small and the pauses short.
#[derive(Clone, Debug)]
pub struct Tuning {
    /// A file of at least this many bytes goes up in parts.
    pub multipart_threshold: u64,
    /// The size of every part but the last. One part is held in memory.
    pub part_size: u64,
    /// How often a call that is safe to repeat is tried, the first try included.
    pub attempts: u32,
    /// The pause before the second try; it doubles after each.
    pub backoff: Duration,
    /// How long a call that moves little may take.
    pub request_timeout: Duration,
    /// How long a call that moves a part or the whole file may take.
    pub transfer_timeout: Duration,
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning {
            multipart_threshold: 16 * MIB,
            part_size: 16 * MIB,
            attempts: 3,
            backoff: Duration::from_millis(200),
            request_timeout: Duration::from_secs(30),
            transfer_timeout: Duration::from_secs(2 * 3600),
        }
    }
}

/// A bucket and the credentials for it.
#[derive(Clone)]
pub struct BucketTarget {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
}

impl fmt::Debug for BucketTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BucketTarget")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key", &"<hidden>")
            .field("secret_key", &"<hidden>")
            .finish()
    }
}

/// The relay Worker and its token.
#[derive(Clone)]
pub struct RelayTarget {
    pub endpoint: String,
    pub token: String,
}

impl fmt::Debug for RelayTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayTarget")
            .field("endpoint", &self.endpoint)
            .field("token", &"<hidden>")
            .finish()
    }
}

/// The key an archive is kept under in the destination.
pub fn object_key(prefix: &str, server: Id, archive: &Path) -> Result<String, BackupFailed> {
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| BackupFailed::new("The archive has no usable file name."))?;
    Ok(format!("{prefix}/{server}/{name}"))
}

/// The destination the operator saved in the panel.
pub struct Cloudflare {
    state: AppState,
    http: Http,
    tuning: Tuning,
}

impl Cloudflare {
    pub fn new(state: AppState) -> Cloudflare {
        Cloudflare {
            state,
            http: Http::new(),
            tuning: Tuning::default(),
        }
    }

    /// What is in force now: the destination and the credentials, read when
    /// asked, so a change made in the panel counts from the next copy.
    async fn resolve(
        &self,
    ) -> Result<(policy::Destination, secrets::ResolvedSecrets), BackupFailed> {
        let sealer = self.state.sealer().clone();
        let defaults = self.state.settings().cloudflare_r2.clone();
        let environment = self.state.settings().secrets.clone();
        self.state
            .database()
            .read(move |connection| -> Result<_, crate::database::DbError> {
                let (destination, _) = policy::current_destination(connection, &sealer, &defaults)?;
                let resolved = secrets::read(connection, &environment, &sealer)?;
                Ok((destination, resolved))
            })
            .await
            .map_err(|error| {
                BackupFailed::new(format!("The backup destination could not be read: {error}"))
            })
    }
}

impl DestinationTrait for Cloudflare {
    fn enabled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match self.resolve().await {
                Ok((destination, _)) => destination.enabled,
                Err(error) => {
                    tracing::warn!(target: "Stellarr.Api.Backups", "{error}");
                    false
                }
            }
        })
    }

    fn upload<'a>(
        &'a self,
        archive: &'a Path,
        server: Id,
    ) -> BoxFuture<'a, Result<(), BackupFailed>> {
        Box::pin(async move {
            let (destination, resolved) = self.resolve().await?;
            if !destination.enabled {
                return Ok(());
            }
            let filled = |definition: &secrets::SecretDefinition| {
                !resolved.get(definition).expose().trim().is_empty()
            };
            let usable = destination.is_valid()
                && match destination.mode.as_str() {
                    R2_MODE_S3 => {
                        filled(&secrets::CLOUDFLARE_R2_ACCESS_KEY)
                            && filled(&secrets::CLOUDFLARE_R2_SECRET_KEY)
                    }
                    R2_MODE_WORKER => filled(&secrets::CLOUDFLARE_R2_WORKER_TOKEN),
                    _ => false,
                };
            if !usable {
                return Err(BackupFailed::new(SETTINGS_MISSING));
            }
            let key = object_key(&destination.prefix, server, archive)?;
            if destination.mode == R2_MODE_WORKER {
                let target = RelayTarget {
                    endpoint: destination.endpoint.clone(),
                    token: resolved
                        .get(&secrets::CLOUDFLARE_R2_WORKER_TOKEN)
                        .expose()
                        .to_string(),
                };
                upload_to_relay(&self.http, &target, &key, archive, &self.tuning).await
            } else {
                let target = BucketTarget {
                    endpoint: destination.endpoint.clone(),
                    region: destination.region.clone(),
                    bucket: destination.bucket.clone(),
                    access_key: resolved
                        .get(&secrets::CLOUDFLARE_R2_ACCESS_KEY)
                        .expose()
                        .to_string(),
                    secret_key: resolved
                        .get(&secrets::CLOUDFLARE_R2_SECRET_KEY)
                        .expose()
                        .to_string(),
                };
                upload_to_bucket(&self.http, &target, &key, archive, &self.tuning).await
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Both modes: the checksum and the length of the file.

/// The SHA-256 of the file as lower-case hex, and its length.
async fn measure(path: &Path) -> Result<(String, u64), BackupFailed> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut context = digest::Context::new(&digest::SHA256);
    let mut buffer = vec![0u8; 256 * 1024];
    let mut length = 0u64;
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
        length += read as u64;
    }
    let hash: String = context
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((hash, length))
}

// ---------------------------------------------------------------------------
// The bucket.

/// What a failed call to the bucket was, without the store's own words.
fn describe(kind: s3::ErrorKind, status: u16) -> String {
    use s3::ErrorKind as K;
    match kind {
        K::BadCredentials | K::AccessDenied | K::ClockSkew => {
            "The bucket refused the credentials.".to_string()
        }
        K::BucketNotFound => "The bucket does not exist.".to_string(),
        K::Throttled | K::Unavailable => {
            format!("The bucket is not available (HTTP {status}).")
        }
        _ => format!("The bucket refused the upload (HTTP {status})."),
    }
}

fn fault(error: &s3::Error) -> BackupFailed {
    BackupFailed::new(describe(error.kind, error.status))
}

fn network_fault(error: &stellarr_outbound::Error) -> BackupFailed {
    use stellarr_outbound::Error as E;
    BackupFailed::new(match error {
        E::Resolve { .. } | E::Connect { .. } | E::ConnectTimeout { .. } | E::Timeout => {
            "The bucket could not be reached."
        }
        E::Tls { .. } | E::Roots(_) => "The TLS connection to the bucket failed.",
        E::AddressRefused { .. } | E::InvalidUrl(_) => "The bucket's address is not usable.",
        _ => "The transfer to the bucket failed.",
    })
}

fn retryable(error: &stellarr_outbound::Error) -> bool {
    use stellarr_outbound::Error as E;
    !matches!(
        error,
        E::Tls { .. }
            | E::Roots(_)
            | E::AddressRefused { .. }
            | E::InvalidUrl(_)
            | E::ForbiddenHeader(_)
            | E::BodyNotReplayable
    )
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: bytes::Bytes,
}

struct Bucket<'a> {
    http: &'a Http,
    client: std::sync::Arc<s3::Client>,
    tuning: &'a Tuning,
}

impl Bucket<'_> {
    fn now() -> Result<s3::Timestamp, BackupFailed> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        s3::Timestamp::from_unix(seconds)
            .ok_or_else(|| BackupFailed::new("The clock is out of range."))
    }

    /// Sends one signed call and reads its answer, sending it again when the
    /// store or the network failed in a way that may pass. `build` runs for
    /// every try, so each is signed with the moment it is sent.
    async fn call(
        &self,
        timeout: Duration,
        attempts: u32,
        build: impl Fn(s3::Timestamp) -> Result<(s3::Prepared, Body), BackupFailed>,
    ) -> Result<Answer, BackupFailed> {
        let mut attempt = 1;
        loop {
            let (prepared, body) = build(Self::now()?)?;
            let request = to_request(&prepared, body, timeout)?;
            let again = attempt < attempts;
            match self.http.send_collected(request).await {
                Ok(collected) => {
                    let status = collected.status.as_u16();
                    if !(again && (status >= 500 || status == 429)) {
                        return Ok(Answer {
                            status,
                            headers: collected
                                .headers
                                .iter()
                                .filter_map(|(name, value)| {
                                    Some((
                                        name.as_str().to_owned(),
                                        value.to_str().ok()?.to_owned(),
                                    ))
                                })
                                .collect(),
                            body: collected.body,
                        });
                    }
                }
                Err(error) => {
                    if !(again && retryable(&error)) {
                        return Err(network_fault(&error));
                    }
                }
            }
            tokio::time::sleep(self.tuning.backoff * 2u32.saturating_pow(attempt - 1)).await;
            attempt += 1;
        }
    }

    async fn small(
        &self,
        build: impl Fn(s3::Timestamp) -> Result<s3::Prepared, s3::RequestError>,
    ) -> Result<Answer, BackupFailed> {
        self.call(self.tuning.request_timeout, self.tuning.attempts, |now| {
            let prepared = build(now).map_err(request_fault)?;
            let body = match &prepared.body {
                s3::RequestBody::Bytes(bytes) => Body::bytes(bytes.clone()),
                _ => Body::empty(),
            };
            Ok((prepared, body))
        })
        .await
    }

    /// What the store says of the object, or none when there is none.
    async fn head(&self, key: &str) -> Result<Option<s3::ObjectHead>, BackupFailed> {
        let answer = self.small(|now| self.client.head_object(key, now)).await?;
        match s3::read_head_object(answer.status, &answer.headers) {
            Ok(head) => Ok(Some(head)),
            Err(error) if error.kind == s3::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(fault(&error)),
        }
    }

    async fn put_part_bytes(
        &self,
        key: &str,
        id: &str,
        number: u32,
        bytes: bytes::Bytes,
    ) -> Result<String, BackupFailed> {
        let length = bytes.len() as u64;
        let payload = s3::Payload::hash_of(&bytes);
        let answer = self
            .call(self.tuning.transfer_timeout, self.tuning.attempts, |now| {
                let prepared = self
                    .client
                    .upload_part(key, id, number, length, payload, now)
                    .map_err(request_fault)?;
                Ok((prepared, Body::bytes(bytes.clone())))
            })
            .await?;
        s3::read_upload_part(answer.status, &answer.headers, &answer.body).map_err(|e| fault(&e))
    }

    async fn abort(&self, key: &str, id: &str) {
        let outcome = self
            .small(|now| self.client.abort_multipart_upload(key, id, now))
            .await;
        if outcome.is_err() {
            tracing::warn!(
                target: "Stellarr.Api.Backups",
                "A multipart upload to the bucket could not be aborted; its parts stay there"
            );
        }
    }
}

fn request_fault(_: s3::RequestError) -> BackupFailed {
    BackupFailed::new("The upload to the bucket could not be prepared.")
}

/// A prepared call as a request of the HTTP client: the signed headers go as
/// they are, but for the length, which the body states.
fn to_request(
    prepared: &s3::Prepared,
    body: Body,
    timeout: Duration,
) -> Result<Request, BackupFailed> {
    let unusable = || BackupFailed::new("The upload to the bucket could not be prepared.");
    let method: Method = prepared.method.as_str().parse().map_err(|_| unusable())?;
    let mut request = Request::new(method, &prepared.url)
        .map_err(|_| unusable())?
        .policy(AddressPolicy::Anywhere)
        .max_redirects(0)
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .max_response_bytes(MIB);
    for (name, value) in &prepared.headers {
        if name == "content-length" {
            continue;
        }
        request = request.header(
            name.parse().map_err(|_| unusable())?,
            value.parse().map_err(|_| unusable())?,
        );
    }
    Ok(request.body(body))
}

fn matches(head: &s3::ObjectHead, length: u64, hash: &str) -> Result<(), BackupFailed> {
    if head.length == length && head.metadata_value("sha256") == Some(hash) {
        Ok(())
    } else {
        Err(BackupFailed::new(METADATA_MISMATCH))
    }
}

/// Copies the archive to the bucket under `key`.
pub async fn upload_to_bucket(
    http: &Http,
    target: &BucketTarget,
    key: &str,
    archive: &Path,
    tuning: &Tuning,
) -> Result<(), BackupFailed> {
    let config = s3::Config::new(
        &target.endpoint,
        &target.region,
        &target.bucket,
        s3::Credentials {
            access_key: target.access_key.clone(),
            secret_key: target.secret_key.clone(),
        },
    )
    .map_err(|_| {
        BackupFailed::new("The bucket's address, region or credentials are not usable.")
    })?;
    let bucket = Bucket {
        http,
        client: std::sync::Arc::new(s3::Client::new(config)),
        tuning,
    };
    let (hash, length) = measure(archive).await?;

    // A lost answer to an earlier upload must not need an overwrite, which a
    // bucket under a lock refuses; what is there already and is the same
    // archive is done.
    if let Some(existing) = bucket.head(key).await? {
        return matches(&existing, length, &hash);
    }

    let metadata = [("sha256", hash.as_str())];
    if length < tuning.multipart_threshold {
        let mut file = tokio::fs::File::open(archive).await?;
        let bytes = read_exactly(&mut file, length).await?;
        let payload = s3::Payload::hash_of(&bytes);
        let bytes = bytes::Bytes::from(bytes);
        let answer = bucket
            .call(tuning.transfer_timeout, tuning.attempts, |now| {
                let prepared = bucket
                    .client
                    .put_object(key, length, Some(CONTENT_TYPE), &metadata, payload, now)
                    .map_err(request_fault)?;
                Ok((prepared, Body::bytes(bytes.clone())))
            })
            .await?;
        s3::read_put_object(answer.status, &answer.headers, &answer.body).map_err(|e| fault(&e))?;
    } else {
        upload_in_parts(&bucket, key, archive, length, &metadata).await?;
    }

    match bucket.head(key).await? {
        Some(uploaded) => matches(&uploaded, length, &hash),
        None => Err(BackupFailed::new(METADATA_MISMATCH)),
    }
}

async fn read_exactly(file: &mut tokio::fs::File, length: u64) -> Result<Vec<u8>, BackupFailed> {
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    file.take(length).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 != length {
        return Err(BackupFailed::new(
            "The archive changed while it was being sent.",
        ));
    }
    Ok(bytes)
}

/// A multipart upload that is aborted if it is dropped before it finished,
/// so a stop of the server does not leave parts the store keeps charging for.
struct OpenUpload {
    http: Http,
    client: std::sync::Arc<s3::Client>,
    tuning: Tuning,
    key: String,
    id: String,
    armed: bool,
}

impl Drop for OpenUpload {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (http, client, tuning) = (self.http.clone(), self.client.clone(), self.tuning.clone());
        let (key, id) = (self.key.clone(), self.id.clone());
        runtime.spawn(async move {
            let bucket = Bucket {
                http: &http,
                client,
                tuning: &tuning,
            };
            bucket.abort(&key, &id).await;
        });
    }
}

async fn upload_in_parts(
    bucket: &Bucket<'_>,
    key: &str,
    archive: &Path,
    length: u64,
    metadata: &[(&str, &str)],
) -> Result<(), BackupFailed> {
    let tuning = bucket.tuning;
    let part_size = tuning.part_size.max(length.div_ceil(s3::MAX_PARTS)).max(1);
    let answer = bucket
        .small(|now| {
            bucket
                .client
                .create_multipart_upload(key, Some(CONTENT_TYPE), metadata, now)
        })
        .await?;
    let id = s3::read_create_multipart_upload(answer.status, &answer.headers, &answer.body)
        .map_err(|e| fault(&e))?;
    let mut open = OpenUpload {
        http: bucket.http.clone(),
        client: bucket.client.clone(),
        tuning: tuning.clone(),
        key: key.to_owned(),
        id: id.clone(),
        armed: true,
    };
    let outcome = send_parts(bucket, key, &id, archive, length, part_size).await;
    open.armed = false;
    if outcome.is_err() {
        bucket.abort(key, &id).await;
    }
    outcome
}

async fn send_parts(
    bucket: &Bucket<'_>,
    key: &str,
    id: &str,
    archive: &Path,
    length: u64,
    part_size: u64,
) -> Result<(), BackupFailed> {
    let mut file = tokio::fs::File::open(archive).await?;
    let mut parts = Vec::new();
    let mut number = 1u32;
    let mut offset = 0u64;
    while offset < length {
        let size = part_size.min(length - offset);
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let bytes = read_exactly(&mut file, size).await?;
        let etag = bucket
            .put_part_bytes(key, id, number, bytes::Bytes::from(bytes))
            .await?;
        parts.push(s3::Part { number, etag });
        number += 1;
        offset += size;
    }
    // Completing is not repeated: if the first try worked and its answer was
    // lost, a second would say the upload is gone.
    let answer = bucket
        .call(bucket.tuning.transfer_timeout, 1, |now| {
            let prepared = bucket
                .client
                .complete_multipart_upload(key, id, &parts, now)
                .map_err(request_fault)?;
            let body = match &prepared.body {
                s3::RequestBody::Bytes(bytes) => Body::bytes(bytes.clone()),
                _ => Body::empty(),
            };
            Ok((prepared, body))
        })
        .await?;
    s3::read_complete_multipart_upload(answer.status, &answer.headers, &answer.body)
        .map(|_| ())
        .map_err(|e| fault(&e))
}

// ---------------------------------------------------------------------------
// The relay.

fn relay_url(endpoint: &str) -> String {
    format!("{}/object", endpoint.trim_end_matches('/'))
}

fn relay_request(
    method: Method,
    target: &RelayTarget,
    key: &str,
    hash: Option<&str>,
    timeout: Duration,
) -> Result<Request, BackupFailed> {
    let unusable = || BackupFailed::new("The backup relay's address or token is not usable.");
    let mut token =
        HeaderValue::from_str(&format!("Bearer {}", target.token)).map_err(|_| unusable())?;
    token.set_sensitive(true);
    let mut request = Request::new(method, &relay_url(&target.endpoint))
        .map_err(|_| unusable())?
        .policy(AddressPolicy::Anywhere)
        .max_redirects(0)
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .max_response_bytes(MIB)
        .header(stellarr_outbound::header::AUTHORIZATION, token)
        .header(
            HeaderName::from_static("x-stellarr-key"),
            HeaderValue::from_str(key).map_err(|_| unusable())?,
        );
    if let Some(hash) = hash {
        request = request.header(
            HeaderName::from_static("x-stellarr-sha256"),
            HeaderValue::from_str(hash).map_err(|_| unusable())?,
        );
    }
    Ok(request)
}

fn relay_fault(error: &stellarr_outbound::Error) -> BackupFailed {
    use stellarr_outbound::Error as E;
    BackupFailed::new(match error {
        E::Resolve { .. } | E::Connect { .. } | E::ConnectTimeout { .. } | E::Timeout => {
            "The backup relay could not be reached."
        }
        E::Tls { .. } | E::Roots(_) => "The TLS connection to the backup relay failed.",
        E::AddressRefused { .. } | E::InvalidUrl(_) => "The backup relay's address is not usable.",
        _ => "The transfer to the backup relay failed.",
    })
}

/// Copies the archive through the relay, which keeps it under `key`.
pub async fn upload_to_relay(
    http: &Http,
    target: &RelayTarget,
    key: &str,
    archive: &Path,
    tuning: &Tuning,
) -> Result<(), BackupFailed> {
    let (hash, length) = measure(archive).await?;
    let file = tokio::fs::File::open(archive).await?;
    let upload = relay_request(
        Method::PUT,
        target,
        key,
        Some(&hash),
        tuning.transfer_timeout,
    )?
    .header(
        stellarr_outbound::header::CONTENT_TYPE,
        HeaderValue::from_static(CONTENT_TYPE),
    )
    .body(Body::stream(file, length));
    let response = http
        .send_collected(upload)
        .await
        .map_err(|e| relay_fault(&e))?;
    match response.status.as_u16() {
        409 => {
            return Err(BackupFailed::new(
                "The remote backup already exists with different metadata.",
            ));
        }
        200 | 201 => {}
        other => {
            return Err(BackupFailed::new(format!(
                "The backup relay refused the upload (HTTP {other})."
            )));
        }
    }

    let head = relay_request(Method::HEAD, target, key, None, tuning.request_timeout)?;
    let answer = http
        .send_collected(head)
        .await
        .map_err(|e| relay_fault(&e))?;
    if !answer.status.is_success() {
        return Err(BackupFailed::new(format!(
            "The backup relay did not confirm the upload (HTTP {}).",
            answer.status.as_u16()
        )));
    }
    let header = |name: &str| {
        answer
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    if header("x-stellarr-size") != Some(length.to_string().as_str())
        || header("x-stellarr-sha256") != Some(hash.as_str())
    {
        return Err(BackupFailed::new(METADATA_MISMATCH));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    use super::*;
    use crate::testing::TempDir;

    const ACCESS: &str = "AKIAFAKEACCESSKEY0001";
    const SECRET: &str = "fake-secret-key-0123456789-abcdef";
    const TOKEN: &str = "fake-relay-token-9876543210";

    #[derive(Clone, Debug)]
    struct Req {
        method: String,
        target: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Req {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
        fn query(&self, name: &str) -> Option<&str> {
            let query = self.target.split_once('?')?.1;
            query.split('&').find_map(|pair| {
                let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
                (n == name).then_some(v)
            })
        }
        fn path(&self) -> &str {
            self.target.split('?').next().unwrap_or("")
        }
    }

    struct Resp {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    fn resp(status: u16) -> Resp {
        Resp {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    type Handler = Arc<dyn Fn(&Req) -> Resp + Send + Sync>;

    /// A server that speaks just enough HTTP/1.1, one request to a connection.
    async fn serve(handler: Handler) -> (u16, Arc<Mutex<Vec<Req>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (handler, log) = (handler.clone(), log.clone());
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 8192];
                    let head_end = loop {
                        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at;
                        }
                        match tokio::io::AsyncReadExt::read(&mut socket, &mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or("").split(' ');
                    let (method, target) = (
                        first.next().unwrap_or("").to_string(),
                        first.next().unwrap_or("").to_string(),
                    );
                    let headers: Vec<(String, String)> = lines
                        .filter_map(|line| line.split_once(':'))
                        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
                        .collect();
                    let length: usize = headers
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.parse().ok())
                        .unwrap_or(0);
                    let mut body = buffer[head_end + 4..].to_vec();
                    while body.len() < length {
                        match tokio::io::AsyncReadExt::read(&mut socket, &mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => body.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let request = Req {
                        method: method.clone(),
                        target,
                        headers,
                        body,
                    };
                    log.lock().unwrap().push(request.clone());
                    let answer = handler(&request);
                    let mut out = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", answer.status);
                    let mut has_length = false;
                    for (n, v) in &answer.headers {
                        has_length |= n.eq_ignore_ascii_case("content-length");
                        out.push_str(&format!("{n}: {v}\r\n"));
                    }
                    if !has_length {
                        out.push_str(&format!("Content-Length: {}\r\n", answer.body.len()));
                    }
                    out.push_str("\r\n");
                    let _ = socket.write_all(out.as_bytes()).await;
                    if method != "HEAD" {
                        let _ = socket.write_all(&answer.body).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });
        (port, seen)
    }

    type Upload = (Option<String>, Vec<(u32, Vec<u8>)>);

    #[derive(Default)]
    struct Store {
        objects: HashMap<String, (Vec<u8>, Option<String>)>,
        uploads: HashMap<String, Upload>,
        refuse: bool,
        fail_part: Option<u32>,
    }

    fn s3_fake(store: Arc<Mutex<Store>>) -> Handler {
        Arc::new(move |req: &Req| {
            let mut store = store.lock().unwrap();
            if store.refuse {
                let mut answer = resp(403);
                answer.body = format!(
                    "<Error><Code>InvalidAccessKeyId</Code><Message>The key {ACCESS} is not known</Message></Error>"
                )
                .into_bytes();
                return answer;
            }
            let key = req.path().trim_start_matches("/bucket/").to_string();
            let sha = req.header("x-amz-meta-sha256").map(str::to_string);
            match (
                req.method.as_str(),
                req.query("uploads"),
                req.query("uploadId"),
            ) {
                ("HEAD", _, _) => match store.objects.get(&key) {
                    None => resp(404),
                    Some((bytes, sha)) => {
                        let mut answer = resp(200);
                        answer
                            .headers
                            .push(("Content-Length".into(), bytes.len().to_string()));
                        if let Some(sha) = sha {
                            answer
                                .headers
                                .push(("x-amz-meta-sha256".into(), sha.clone()));
                        }
                        answer
                    }
                },
                ("PUT", _, Some(id)) => {
                    let number: u32 = req.query("partNumber").unwrap().parse().unwrap();
                    if store.fail_part == Some(number) {
                        return resp(500);
                    }
                    store
                        .uploads
                        .get_mut(id)
                        .unwrap()
                        .1
                        .push((number, req.body.clone()));
                    let mut answer = resp(200);
                    answer
                        .headers
                        .push(("ETag".into(), format!("\"p{number}\"")));
                    answer
                }
                ("PUT", _, None) => {
                    store.objects.insert(key, (req.body.clone(), sha));
                    resp(200)
                }
                ("POST", Some(_), _) => {
                    let id = format!("U{}", store.uploads.len() + 1);
                    store.uploads.insert(id.clone(), (sha, Vec::new()));
                    let mut answer = resp(200);
                    answer.body = format!(
                        "<InitiateMultipartUploadResult><Bucket>bucket</Bucket><Key>{key}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>"
                    )
                    .into_bytes();
                    answer
                }
                ("POST", None, Some(id)) => {
                    let (sha, mut parts) = store.uploads.remove(id).unwrap();
                    parts.sort();
                    let bytes = parts.into_iter().flat_map(|(_, bytes)| bytes).collect();
                    store.objects.insert(key, (bytes, sha));
                    let mut answer = resp(200);
                    answer.body = b"<CompleteMultipartUploadResult><ETag>\"done\"</ETag></CompleteMultipartUploadResult>".to_vec();
                    answer
                }
                ("DELETE", _, Some(id)) => {
                    store.uploads.remove(id);
                    resp(204)
                }
                _ => resp(400),
            }
        })
    }

    fn target(port: u16, secret: &str) -> BucketTarget {
        BucketTarget {
            endpoint: format!("http://127.0.0.1:{port}"),
            region: "auto".into(),
            bucket: "bucket".into(),
            access_key: ACCESS.into(),
            secret_key: secret.into(),
        }
    }

    fn quick() -> Tuning {
        Tuning {
            multipart_threshold: 10,
            part_size: 10,
            attempts: 2,
            backoff: Duration::from_millis(1),
            request_timeout: Duration::from_secs(5),
            transfer_timeout: Duration::from_secs(5),
        }
    }

    fn archive(folder: &TempDir, bytes: &[u8]) -> std::path::PathBuf {
        let path = folder.path().join("backup-test.zip");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn sha_of(bytes: &[u8]) -> String {
        s3::internals::hex(&s3::sha256(bytes))
    }

    #[tokio::test]
    async fn a_small_archive_is_looked_up_sent_in_one_request_and_checked() {
        let folder = TempDir::new();
        let path = archive(&folder, b"abcde");
        let store = Arc::new(Mutex::new(Store::default()));
        let (port, seen) = serve(s3_fake(store.clone())).await;

        upload_to_bucket(
            &Http::new(),
            &target(port, SECRET),
            "stellarr/s/backup-test.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap();

        let stored = store.lock().unwrap();
        let (bytes, sha) = &stored.objects["stellarr/s/backup-test.zip"];
        assert_eq!(bytes, b"abcde");
        assert_eq!(sha.as_deref(), Some(sha_of(b"abcde").as_str()));
        let calls: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.method.clone())
            .collect();
        assert_eq!(calls, ["HEAD", "PUT", "HEAD"]);
        let put = &seen.lock().unwrap()[1];
        assert_eq!(put.header("content-type"), Some("application/zip"));
        assert!(
            put.header("authorization")
                .unwrap()
                .contains("/auto/s3/aws4_request")
        );
    }

    #[tokio::test]
    async fn a_larger_archive_goes_up_in_equal_parts() {
        let folder = TempDir::new();
        let bytes: Vec<u8> = (0..25u8).collect();
        let path = archive(&folder, &bytes);
        let store = Arc::new(Mutex::new(Store::default()));
        let (port, seen) = serve(s3_fake(store.clone())).await;

        upload_to_bucket(
            &Http::new(),
            &target(port, SECRET),
            "k/a.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap();

        assert_eq!(store.lock().unwrap().objects["k/a.zip"].0, bytes);
        let sizes: Vec<usize> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "PUT")
            .map(|r| r.body.len())
            .collect();
        assert_eq!(sizes, [10, 10, 5]);
    }

    #[tokio::test]
    async fn what_is_there_already_counts_as_sent_only_if_it_is_the_same_archive() {
        let folder = TempDir::new();
        let path = archive(&folder, b"abcde");
        let store = Arc::new(Mutex::new(Store::default()));
        store.lock().unwrap().objects.insert(
            "k/a.zip".into(),
            (b"abcde".to_vec(), Some(sha_of(b"abcde"))),
        );
        let (port, seen) = serve(s3_fake(store.clone())).await;
        upload_to_bucket(
            &Http::new(),
            &target(port, SECRET),
            "k/a.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1, "one look and nothing sent");

        store.lock().unwrap().objects.insert(
            "k/a.zip".into(),
            (b"abcde".to_vec(), Some(sha_of(b"other"))),
        );
        let error = upload_to_bucket(
            &Http::new(),
            &target(port, SECRET),
            "k/a.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.message(), METADATA_MISMATCH);
        store.lock().unwrap().objects.insert(
            "k/a.zip".into(),
            (b"abcdefg".to_vec(), Some(sha_of(b"abcde"))),
        );
        assert!(
            upload_to_bucket(
                &Http::new(),
                &target(port, SECRET),
                "k/a.zip",
                &path,
                &quick()
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn a_bucket_that_cannot_be_reached_is_said_so() {
        let folder = TempDir::new();
        let path = archive(&folder, b"abcde");
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = upload_to_bucket(
            &Http::new(),
            &target(closed, SECRET),
            "k/a.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.message(), "The bucket could not be reached.");
    }

    #[tokio::test]
    async fn a_refusal_never_repeats_what_the_store_said() {
        let folder = TempDir::new();
        let path = archive(&folder, b"abcde");
        let store = Arc::new(Mutex::new(Store {
            refuse: true,
            ..Store::default()
        }));
        let (port, _) = serve(s3_fake(store)).await;
        let target = target(port, SECRET);
        let error = upload_to_bucket(&Http::new(), &target, "k/a.zip", &path, &quick())
            .await
            .unwrap_err();
        assert_eq!(error.message(), "The bucket refused the credentials.");
        for shown in [
            error.to_string(),
            format!("{error:?}"),
            format!("{target:?}"),
        ] {
            for secret in [ACCESS, SECRET, TOKEN] {
                assert!(!shown.contains(secret), "{shown}");
            }
        }
    }

    #[tokio::test]
    async fn a_multipart_upload_that_fails_halfway_is_aborted() {
        let folder = TempDir::new();
        let path = archive(&folder, &[7u8; 25]);
        let store = Arc::new(Mutex::new(Store {
            fail_part: Some(2),
            ..Store::default()
        }));
        let (port, seen) = serve(s3_fake(store.clone())).await;

        let error = upload_to_bucket(
            &Http::new(),
            &target(port, SECRET),
            "k/a.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap_err();

        assert!(error.message().contains("HTTP 500"), "{error}");
        let requests = seen.lock().unwrap();
        let abort = requests
            .iter()
            .find(|r| r.method == "DELETE")
            .expect("an abort");
        assert_eq!(abort.query("uploadId"), Some("U1"));
        let stored = store.lock().unwrap();
        assert!(stored.uploads.is_empty());
        assert!(stored.objects.is_empty());
    }

    fn relay_fake(put: u16, size: Option<String>, sha: Option<String>) -> Handler {
        Arc::new(move |req: &Req| match req.method.as_str() {
            "PUT" => resp(put),
            _ => {
                let mut answer = resp(200);
                if let Some(size) = &size {
                    answer
                        .headers
                        .push(("X-Stellarr-Size".into(), size.clone()));
                }
                if let Some(sha) = &sha {
                    answer
                        .headers
                        .push(("X-Stellarr-Sha256".into(), sha.clone()));
                }
                answer
            }
        })
    }

    fn relay(port: u16) -> RelayTarget {
        RelayTarget {
            endpoint: format!("http://127.0.0.1:{port}/"),
            token: TOKEN.into(),
        }
    }

    #[tokio::test]
    async fn the_relay_is_sent_the_archive_and_asked_to_confirm_it() {
        let folder = TempDir::new();
        let path = archive(&folder, b"abcde");
        let (port, seen) = serve(relay_fake(201, Some("5".into()), Some(sha_of(b"abcde")))).await;
        upload_to_relay(
            &Http::new(),
            &relay(port),
            "stellarr/s/a.zip",
            &path,
            &quick(),
        )
        .await
        .unwrap();
        let requests = seen.lock().unwrap();
        assert_eq!(requests[0].target, "/object");
        assert_eq!(requests[0].body, b"abcde");
        assert_eq!(
            requests[0].header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(
            requests[0].header("x-stellarr-key"),
            Some("stellarr/s/a.zip")
        );
        assert_eq!(
            requests[0].header("x-stellarr-sha256"),
            Some(sha_of(b"abcde").as_str())
        );
        assert_eq!(requests[1].method, "HEAD");
        assert!(requests[1].header("x-stellarr-sha256").is_none());
    }

    #[tokio::test]
    async fn the_relay_refusing_or_disagreeing_is_a_failure() {
        let folder = TempDir::new();
        let path = archive(&folder, b"abcde");
        let good = (Some("5".to_string()), Some(sha_of(b"abcde")));
        let cases = [
            (
                409,
                good.clone(),
                "The remote backup already exists with different metadata.",
            ),
            (
                500,
                good.clone(),
                "The backup relay refused the upload (HTTP 500).",
            ),
            (200, (Some("6".into()), good.1.clone()), METADATA_MISMATCH),
            (
                200,
                (good.0.clone(), Some("0".repeat(64))),
                METADATA_MISMATCH,
            ),
            (200, (None, None), METADATA_MISMATCH),
        ];
        for (status, (size, sha), expected) in cases {
            let (port, _) = serve(relay_fake(status, size, sha)).await;
            let error = upload_to_relay(&Http::new(), &relay(port), "k/a.zip", &path, &quick())
                .await
                .unwrap_err();
            assert_eq!(error.message(), expected);
            assert!(!format!("{error:?}{:?}", relay(port)).contains(TOKEN));
        }
    }

    #[tokio::test]
    async fn the_key_is_the_prefix_the_server_and_the_file_name() {
        let server = Id::parse("8F62A4D1-BDCB-4D86-BD1C-6E351900E629").unwrap();
        assert_eq!(
            object_key("stellarr", server, Path::new("/data/backups/backup-1.zip")).unwrap(),
            "stellarr/8f62a4d1-bdcb-4d86-bd1c-6e351900e629/backup-1.zip"
        );
    }

    /// Against a real server, by `tests/backups-bucket.sh`: it names the
    /// endpoint, the keys and the bucket in the environment.
    #[tokio::test]
    #[ignore = "needs a real S3 server; run by tests/backups-bucket.sh"]
    async fn a_real_bucket_takes_an_archive_once_and_refuses_wrong_keys() {
        let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        let real = BucketTarget {
            endpoint: env("STELLARR_TEST_S3_ENDPOINT"),
            region: "us-east-1".into(),
            bucket: "backups-test".into(),
            access_key: env("STELLARR_TEST_S3_ACCESS_KEY"),
            secret_key: env("STELLARR_TEST_S3_SECRET_KEY"),
        };
        let http = Http::new();
        let tuning = Tuning {
            multipart_threshold: 8 * MIB,
            part_size: 5 * MIB,
            ..Tuning::default()
        };
        // The bucket has to exist; making it is the store's own call.
        let config = s3::Config::new(
            &real.endpoint,
            &real.region,
            &real.bucket,
            s3::Credentials {
                access_key: real.access_key.clone(),
                secret_key: real.secret_key.clone(),
            },
        )
        .unwrap();
        let client = s3::Client::new(config);
        let now = Bucket::now().unwrap();
        let prepared = client.create_bucket(now).unwrap();
        let request = to_request(&prepared, Body::empty(), Duration::from_secs(30)).unwrap();
        http.send_collected(request).await.unwrap();

        let folder = TempDir::new();
        let sizes = [("small.zip", 70_000usize), ("large.zip", 12 * 1024 * 1024)];
        for (name, size) in sizes {
            let bytes: Vec<u8> = (0..size).map(|i| (i * 31 % 251) as u8).collect();
            let path = folder.path().join(name);
            std::fs::write(&path, &bytes).unwrap();
            let key = format!("stellarr/test/{name}");
            upload_to_bucket(&http, &real, &key, &path, &tuning)
                .await
                .unwrap();
            // The same archive again is already there.
            upload_to_bucket(&http, &real, &key, &path, &tuning)
                .await
                .unwrap();
            // Read it back: whole bytes and the checksum kept with it.
            let bucket = Bucket {
                http: &http,
                client: Arc::new(client_for(&real)),
                tuning: &tuning,
            };
            let head = bucket.head(&key).await.unwrap().unwrap();
            assert_eq!(head.length, size as u64);
            assert_eq!(head.metadata_value("sha256"), Some(sha_of(&bytes).as_str()));
            let prepared = bucket
                .client
                .get_object(&key, None, Bucket::now().unwrap())
                .unwrap();
            let request = to_request(&prepared, Body::empty(), Duration::from_secs(60))
                .unwrap()
                .max_response_bytes(64 * MIB);
            let got = http.send_collected(request).await.unwrap();
            assert_eq!(got.body.as_ref(), bytes.as_slice());
        }

        let path = folder.path().join("small.zip");
        let mut wrong = real.clone();
        wrong.secret_key = "not-the-secret".into();
        let error = upload_to_bucket(&http, &wrong, "stellarr/test/other.zip", &path, &tuning)
            .await
            .unwrap_err();
        assert_eq!(error.message(), "The bucket refused the credentials.");
        assert!(!error.message().contains(&real.secret_key));

        // The server is stopped by the script after this line is printed.
        println!("READY_TO_STOP");
        for _ in 0..60 {
            if std::path::Path::new(&env("STELLARR_TEST_S3_STOPPED_FLAG")).exists() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let error = upload_to_bucket(&http, &real, "stellarr/test/again.zip", &path, &tuning)
            .await
            .unwrap_err();
        assert_eq!(error.message(), "The bucket could not be reached.");
    }

    fn client_for(target: &BucketTarget) -> s3::Client {
        s3::Client::new(
            s3::Config::new(
                &target.endpoint,
                &target.region,
                &target.bucket,
                s3::Credentials {
                    access_key: target.access_key.clone(),
                    secret_key: target.secret_key.clone(),
                },
            )
            .unwrap(),
        )
    }
}
