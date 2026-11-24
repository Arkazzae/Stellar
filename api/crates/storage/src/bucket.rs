//! The bucket store: objects in one bucket of an S3-compatible store.
//!
//! Requests are built and signed by `stellarr-s3`, sent by `stellarr-outbound`
//! and their answers read by `stellarr-s3` again; this file is the glue and
//! the decisions that are the application's: when a write is one request and
//! when it is many, what a refusal means to a caller, what to retry.
//!
//! # Uploads
//!
//! An upload comes as a stream whose length is not known, so it is read one
//! part at a time into memory (8 MiB unless tuned). A stream that ends within
//! the first part goes up as one `PUT`; a longer one as a multipart upload
//! whose parts are all that size but the last. Every part is signed by its
//! SHA-256, which needs the bytes in hand, and that is also what lets a part
//! be sent again after a failure without asking the source for it twice. A
//! failed or cancelled upload aborts the parts it left on the store.
//!
//! Memory per upload is one part. Objects that go beyond ten thousand parts
//! (80 GiB at the default part size) are refused.
//!
//! # Existing keys
//!
//! Every write asks the store to create the object only if the key is free
//! (`If-None-Match: *`, on the single request and on completing an upload),
//! and a refusal because the key is taken is `AlreadyExists`. That is exclusive
//! creation, with no extra request, on a store that honours the condition. A
//! store that does not know it replaces what is under the key, and two
//! writers racing for one key can both succeed: which stores honour it is
//! something to find out of each, not to assume. A large upload that meets a
//! taken key finds out when it completes, after its parts are sent.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use stellarr_outbound::{AddressPolicy, Body, Client as Http, Request};
use stellarr_s3 as s3;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::body::ObjectBody;
use crate::error::{Error, Result, StoreFault};
use crate::key;
use crate::settings::StorageOptions;
use crate::{
    BoxFuture, ByteRange, ListPages, Listing, ObjectStorage, StoredObject, StoredObjectInfo,
};

/// The region requests are signed for. Stores that have no regions take any,
/// and want this one.
pub const REGION: &str = "us-east-1";

/// The largest part size that may be set. S3 takes up to 5 GiB, but a part is
/// held in memory whole, so the setting stops well short of that.
const MAX_PART_SIZE: u64 = 512 * 1024 * 1024;

/// How an answer's body is bounded when it is read whole: an error document,
/// a head, a part's answer.
const SMALL_ANSWER: u64 = 1024 * 1024;

/// Knobs of the bucket store with the defaults the application uses.
#[derive(Clone, Debug)]
pub struct BucketTuning {
    /// The size of the parts of a multipart upload, and so the size from which
    /// an upload is one, and what an upload holds in memory. From 5 MiB, the
    /// least S3 takes for every part but the last, to 512 MiB.
    pub part_size: u64,
    /// How long resolving, connecting and the TLS handshake may take.
    pub connect_timeout: Duration,
    /// How long a call that moves little may take, from the first byte sent
    /// to the last read.
    pub request_timeout: Duration,
    /// How long a call that moves an object or a part may take. There is no
    /// idle timeout, so this is the only thing that ends a connection that
    /// stalls mid-body.
    pub transfer_timeout: Duration,
    /// How often a call that is safe to repeat is tried, a first try included.
    pub attempts: u32,
}

impl Default for BucketTuning {
    fn default() -> Self {
        Self {
            part_size: 8 * 1024 * 1024,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            transfer_timeout: Duration::from_secs(3600),
            attempts: 3,
        }
    }
}

#[derive(Clone)]
pub struct BucketStore {
    inner: Arc<Inner>,
}

struct Inner {
    client: s3::Client,
    bucket: String,
    http: Http,
    tuning: BucketTuning,
}

/// A call's answer with its body read.
struct Answer {
    /// How many tries the call took; more than one means a request may have
    /// reached the store whose answer was lost.
    tries: u32,
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
}

impl BucketStore {
    pub fn new(options: &StorageOptions, http: Http) -> Result<Self> {
        Self::with_tuning(options, http, BucketTuning::default())
    }

    pub fn with_tuning(options: &StorageOptions, http: Http, tuning: BucketTuning) -> Result<Self> {
        if options.endpoint.trim().is_empty() || options.bucket.trim().is_empty() {
            return Err(Error::store(
                StoreFault::Rejected,
                "the S3 provider needs an endpoint and a bucket",
            ));
        }
        if !(s3::MIN_PART_SIZE..=MAX_PART_SIZE).contains(&tuning.part_size) {
            return Err(Error::store(
                StoreFault::Rejected,
                "the part size must be between 5 MiB and 512 MiB",
            ));
        }
        let config = s3::Config::new(
            &options.endpoint,
            REGION,
            &options.bucket,
            s3::Credentials {
                access_key: options.access_key.clone(),
                secret_key: options.secret_key.clone(),
            },
        )
        .map_err(|error| Error::store(StoreFault::Rejected, error.to_string()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                client: s3::Client::new(config),
                bucket: options.bucket.clone(),
                http,
                tuning: BucketTuning {
                    attempts: tuning.attempts.max(1),
                    ..tuning
                },
            }),
        })
    }

    fn now() -> Result<s3::Timestamp> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        s3::Timestamp::from_unix(seconds)
            .ok_or_else(|| Error::store(StoreFault::Rejected, "the clock is out of range"))
    }

    /// Turns a prepared call into a request of the HTTP client. The prepared
    /// headers go as they are, but for the length, which the body states.
    fn request(
        &self,
        prepared: &s3::Prepared,
        body: Body,
        timeout: Duration,
        ceiling: u64,
    ) -> Result<Request> {
        let invalid = |what: &str| Error::store(StoreFault::Rejected, what.to_owned());
        let mut request = Request::new(
            prepared
                .method
                .as_str()
                .parse()
                .map_err(|_| invalid("the method is not valid"))?,
            &prepared.url,
        )
        .map_err(|error| from_outbound(&error))?
        .policy(AddressPolicy::Anywhere)
        .max_redirects(0)
        .connect_timeout(self.inner.tuning.connect_timeout)
        .timeout(timeout)
        .max_response_bytes(ceiling);
        for (name, value) in &prepared.headers {
            if name == "content-length" {
                continue;
            }
            request = request.header(
                name.parse()
                    .map_err(|_| invalid("a header name is not valid"))?,
                value
                    .parse()
                    .map_err(|_| invalid("a header value is not valid"))?,
            );
        }
        Ok(request.body(body))
    }

    /// The body to send for a prepared call that carries no object.
    fn small_body(prepared: &s3::Prepared) -> Body {
        match &prepared.body {
            s3::RequestBody::Bytes(bytes) => Body::bytes(bytes.clone()),
            _ => Body::empty(),
        }
    }

    /// Sends a call and reads its whole answer, trying again when the store
    /// or the network failed in a way that may pass. `build` runs per try, so
    /// every try is signed with the time it is sent.
    async fn call(
        &self,
        timeout: Duration,
        ceiling: u64,
        attempts: u32,
        build: impl Fn(s3::Timestamp) -> std::result::Result<(s3::Prepared, Body), s3::RequestError>,
    ) -> Result<Answer> {
        let mut attempt = 1;
        loop {
            let (prepared, body) = build(Self::now()?)
                .map_err(|error| Error::store(StoreFault::Rejected, error.to_string()))?;
            let request = self.request(&prepared, body, timeout, ceiling)?;
            let outcome = self.inner.http.send_collected(request).await;
            let again = attempt < attempts;
            match outcome {
                Ok(collected) => {
                    let status = collected.status.as_u16();
                    if again && (status >= 500 || status == 429) {
                        tracing::debug!(status, attempt, "the store failed a call; trying again");
                    } else {
                        return Ok(Answer {
                            tries: attempt,
                            status,
                            headers: header_list(
                                collected
                                    .headers
                                    .iter()
                                    .map(|(name, value)| (name.as_str(), value.to_str().ok())),
                            ),
                            body: collected.body,
                        });
                    }
                }
                Err(error) => {
                    let retryable = from_outbound(&error).is_retryable();
                    if !(again && retryable) {
                        return Err(from_outbound(&error));
                    }
                    tracing::debug!(%error, attempt, "a call to the store failed; trying again");
                }
            }
            tokio::time::sleep(Duration::from_millis(100 << attempt.min(6))).await;
            attempt += 1;
        }
    }

    /// A call that moves little and may be repeated.
    async fn small(
        &self,
        build: impl Fn(s3::Timestamp) -> std::result::Result<s3::Prepared, s3::RequestError>,
    ) -> Result<Answer> {
        let tuning = &self.inner.tuning;
        self.call(
            tuning.request_timeout,
            SMALL_ANSWER,
            tuning.attempts,
            |now| {
                let prepared = build(now)?;
                let body = Self::small_body(&prepared);
                Ok((prepared, body))
            },
        )
        .await
    }

    async fn prepare(&self) -> Result<()> {
        let answer = self
            .small(|now| self.inner.client.list_buckets(now))
            .await?;
        let names = s3::read_list_buckets(answer.status, &answer.headers, &answer.body)
            .map_err(|error| from_s3(&error, ""))?;
        if names.iter().any(|name| *name == self.inner.bucket) {
            return Ok(());
        }
        let answer = self
            .small(|now| self.inner.client.create_bucket(now))
            .await?;
        match s3::read_create_bucket(answer.status, &answer.headers, &answer.body) {
            Ok(()) => Ok(()),
            // Another instance made it between the listing and now.
            Err(error) if error.kind == s3::ErrorKind::AlreadyOwned => Ok(()),
            Err(error) => Err(from_s3(&error, "")),
        }
    }

    async fn probe(&self) -> Result<()> {
        let answer = self
            .small(|now| {
                self.inner.client.list_objects(
                    &s3::ListRequest {
                        max_keys: Some(1),
                        ..Default::default()
                    },
                    now,
                )
            })
            .await?;
        s3::read_list_objects(answer.status, &answer.headers, &answer.body)
            .map(|_| ())
            .map_err(|error| from_s3(&error, ""))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        key::check(key)?;
        let answer = self
            .small(|now| self.inner.client.delete_object(key, now))
            .await?;
        s3::read_delete_object(answer.status, &answer.headers, &answer.body)
            .map_err(|error| from_s3(&error, key))
    }

    async fn get(&self, key: &str, range: Option<ByteRange>) -> Result<StoredObject> {
        key::check(key)?;
        if range.is_some_and(|r| r.to.is_some_and(|to| to < r.from)) {
            return Err(Error::RangeNotSatisfiable(key.to_owned()));
        }
        // No object has a byte past the largest signed 64-bit offset, and
        // stores read a range beyond it as no range at all.
        if range.is_some_and(|r| r.from > i64::MAX as u64) {
            return Err(Error::RangeNotSatisfiable(key.to_owned()));
        }
        let asked = range.map(|r| s3::ByteRange {
            from: r.from,
            to: r.to.map(|to| to.min(i64::MAX as u64)),
        });
        let tuning = &self.inner.tuning;
        let mut attempt = 1;
        let response = loop {
            let prepared = self
                .inner
                .client
                .get_object(key, asked, Self::now()?)
                .map_err(|error| Error::store(StoreFault::Rejected, error.to_string()))?;
            // The ceiling is the whole object's, which is not known yet; the
            // announced length is checked against what arrives instead.
            let request =
                self.request(&prepared, Body::empty(), tuning.transfer_timeout, u64::MAX)?;
            match self.inner.http.send(request).await {
                Ok(response) => {
                    let status = response.status.as_u16();
                    if attempt < tuning.attempts && (status >= 500 || status == 429) {
                        tracing::debug!(status, attempt, "the store failed a read; trying again");
                    } else {
                        break response;
                    }
                }
                Err(error) => {
                    let mapped = from_outbound(&error);
                    if attempt >= tuning.attempts || !mapped.is_retryable() {
                        return Err(mapped);
                    }
                    tracing::debug!(%error, attempt, "a read of the store failed; trying again");
                }
            }
            tokio::time::sleep(Duration::from_millis(100 << attempt.min(6))).await;
            attempt += 1;
        };

        let status = response.status.as_u16();
        let headers = header_list(
            response
                .headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.to_str().ok())),
        );
        let mut body = response.body;
        if !(200..300).contains(&status) {
            let (document, _) = body
                .read_up_to(s3::MAX_ERROR_BODY)
                .await
                .unwrap_or_default();
            return Err(from_s3(
                &s3::Error::from_response(status, &headers, &document),
                key,
            ));
        }
        let head =
            s3::read_get_object_head(status, &headers).map_err(|error| from_s3(&error, key))?;
        let served = match (asked, head.range) {
            (None, _) => None,
            (Some(_), Some(served)) => Some(served),
            // A store that answers 200 to a range sent the whole object. That
            // is right only for a range that asked for all of it.
            (Some(_), None) if head.total_length == 0 => {
                return Err(Error::RangeNotSatisfiable(key.to_owned()));
            }
            (Some(wanted), None)
                if wanted.from == 0 && wanted.to.is_none_or(|to| to >= head.total_length - 1) =>
            {
                Some(s3::ByteRange {
                    from: 0,
                    to: Some(head.total_length - 1),
                })
            }
            (Some(_), None) => {
                return Err(Error::store(
                    StoreFault::Malformed,
                    "the store ignored the range and sent the whole object",
                ));
            }
        };
        Ok(StoredObject {
            body: ObjectBody::from_http(body, head.length),
            length: head.length,
            total_length: head.total_length,
            range: served.map(|r| ByteRange {
                from: r.from,
                to: r.to,
            }),
        })
    }

    async fn put(&self, key: &str, data: &mut (dyn AsyncRead + Send + Unpin)) -> Result<()> {
        key::check(key)?;
        let part_size = self.inner.tuning.part_size;
        let part = read_part(data, Vec::new(), part_size).await?;
        let mut lookahead = None;
        if part.len() as u64 == part_size {
            lookahead = peek(data).await?;
        }
        let Some(first_of_next) = lookahead else {
            // Everything fits in one part: one request.
            return self.put_whole(key, Bytes::from(part)).await;
        };

        let id = self.create_upload(key).await?;
        let mut upload = Upload {
            store: self.clone(),
            key: key.to_owned(),
            id,
            armed: true,
        };
        let outcome = self
            .put_parts(key, &upload.id, data, part, first_of_next)
            .await;
        match outcome {
            Ok(()) => {
                upload.armed = false;
                Ok(())
            }
            Err(error) => {
                upload.armed = false;
                // The upload is given up on; what the store keeps of it is
                // storage someone pays for.
                self.abort(key, &upload.id).await;
                Err(error)
            }
        }
    }

    async fn put_whole(&self, key: &str, bytes: Bytes) -> Result<()> {
        let length = bytes.len() as u64;
        let payload = s3::Payload::hash_of(&bytes);
        let tuning = &self.inner.tuning;
        let answer = self
            .call(
                tuning.transfer_timeout,
                SMALL_ANSWER,
                tuning.attempts,
                |now| {
                    let prepared = self.inner.client.put_object_if_absent(
                        key,
                        length,
                        None,
                        &[],
                        payload,
                        now,
                    )?;
                    Ok((prepared, object_body(&bytes)))
                },
            )
            .await?;
        match s3::read_put_object(answer.status, &answer.headers, &answer.body) {
            Ok(_) => Ok(()),
            Err(error) if refused_as_taken(&error) => {
                // After a try whose answer was lost, "taken" may be this very
                // write. An object of the length that was sent is taken to be it.
                if answer.tries > 1 && self.stored_length(key).await == Some(length) {
                    return Ok(());
                }
                Err(Error::AlreadyExists(key.to_owned()))
            }
            Err(error) => Err(from_s3(&error, key)),
        }
    }

    /// The length of the object under `key`, if it can be told.
    async fn stored_length(&self, key: &str) -> Option<u64> {
        let answer = self
            .small(|now| self.inner.client.head_object(key, now))
            .await
            .ok()?;
        s3::read_head_object(answer.status, &answer.headers)
            .ok()
            .map(|head| head.length)
    }

    async fn create_upload(&self, key: &str) -> Result<String> {
        let answer = self
            .small(|now| {
                self.inner
                    .client
                    .create_multipart_upload(key, None, &[], now)
            })
            .await?;
        s3::read_create_multipart_upload(answer.status, &answer.headers, &answer.body)
            .map_err(|error| from_s3(&error, key))
    }

    /// Sends `part` (the first one, already read) and every part after it,
    /// then completes the upload. `next` is the byte that was read past the
    /// end of a full part to see whether there is more.
    async fn put_parts(
        &self,
        key: &str,
        id: &str,
        data: &mut (dyn AsyncRead + Send + Unpin),
        mut part: Vec<u8>,
        first_of_next: u8,
    ) -> Result<()> {
        let part_size = self.inner.tuning.part_size;
        let tuning = &self.inner.tuning;
        let mut parts: Vec<s3::Part> = Vec::new();
        let mut number = 1u32;
        let mut next = Some(first_of_next);
        loop {
            let etag = self.put_part(key, id, number, Bytes::from(part)).await?;
            parts.push(s3::Part { number, etag });
            // `next` starts the following part, which is therefore not empty.
            let Some(byte) = next else { break };
            if u64::from(number) >= s3::MAX_PARTS {
                return Err(Error::store(
                    StoreFault::Rejected,
                    "the object needs more than 10000 parts",
                ));
            }
            number += 1;
            part = read_part(data, vec![byte], part_size).await?;
            next = if part.len() as u64 == part_size {
                peek(data).await?
            } else {
                None
            };
        }

        // Completing is not repeated: if the first try succeeded and its
        // answer was lost, the second would say the upload is gone.
        let answer = self
            .call(tuning.transfer_timeout, SMALL_ANSWER, 1, |now| {
                let prepared = self
                    .inner
                    .client
                    .complete_multipart_upload_if_absent(key, id, &parts, now)?;
                let body = Self::small_body(&prepared);
                Ok((prepared, body))
            })
            .await?;
        s3::read_complete_multipart_upload(answer.status, &answer.headers, &answer.body)
            .map(|_| ())
            .map_err(|error| {
                if refused_as_taken(&error) {
                    Error::AlreadyExists(key.to_owned())
                } else {
                    from_s3(&error, key)
                }
            })
    }

    async fn put_part(&self, key: &str, id: &str, number: u32, bytes: Bytes) -> Result<String> {
        let length = bytes.len() as u64;
        let payload = s3::Payload::hash_of(&bytes);
        let tuning = &self.inner.tuning;
        let answer = self
            .call(
                tuning.transfer_timeout,
                SMALL_ANSWER,
                tuning.attempts,
                |now| {
                    let prepared = self
                        .inner
                        .client
                        .upload_part(key, id, number, length, payload, now)?;
                    Ok((prepared, object_body(&bytes)))
                },
            )
            .await?;
        s3::read_upload_part(answer.status, &answer.headers, &answer.body)
            .map_err(|error| from_s3(&error, key))
    }

    /// Throws away the parts of an upload that is given up on. A failure to do so
    /// is only worth a log line: the caller already has an error to report.
    async fn abort(&self, key: &str, id: &str) {
        let result = self
            .small(|now| self.inner.client.abort_multipart_upload(key, id, now))
            .await
            .and_then(|answer| {
                s3::read_abort_multipart_upload(answer.status, &answer.headers, &answer.body)
                    .map_err(|error| from_s3(&error, key))
            });
        if let Err(error) = result {
            tracing::warn!(%error, "could not abort a multipart upload; its parts stay on the store");
        }
    }
}

/// An upload that is aborted when it is dropped before it finished, so a
/// request that is cancelled does not leave parts behind.
struct Upload {
    store: BucketStore,
    key: String,
    id: String,
    armed: bool,
}

impl Drop for Upload {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (store, key, id) = (self.store.clone(), self.key.clone(), self.id.clone());
        runtime.spawn(async move { store.abort(&key, &id).await });
    }
}

/// Reads up to `size` bytes in total into `part`, which may already hold a
/// few. Fewer than `size` come back only at the end of the data.
async fn read_part(
    data: &mut (dyn AsyncRead + Send + Unpin),
    mut part: Vec<u8>,
    size: u64,
) -> Result<Vec<u8>> {
    let room = size - part.len() as u64;
    // One allocation of the part's size, and no doubling that would hold two
    // copies of a large part for a moment.
    part.reserve_exact(usize::try_from(room).unwrap_or(usize::MAX));
    (&mut *data).take(room).read_to_end(&mut part).await?;
    Ok(part)
}

/// The next byte of the data, or `None` at its end.
async fn peek(data: &mut (dyn AsyncRead + Send + Unpin)) -> Result<Option<u8>> {
    let mut byte = [0u8; 1];
    Ok(match data.read(&mut byte).await? {
        0 => None,
        _ => Some(byte[0]),
    })
}

fn object_body(bytes: &Bytes) -> Body {
    Body::bytes(bytes.clone())
}

fn header_list<'a>(
    headers: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> Vec<(String, String)> {
    headers
        .filter_map(|(name, value)| Some((name.to_owned(), value?.to_owned())))
        .collect()
}

impl ObjectStorage for BucketStore {
    fn prepare(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(BucketStore::prepare(self))
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        data: &'a mut (dyn AsyncRead + Send + Unpin),
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(BucketStore::put(self, key, data))
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<ByteRange>,
    ) -> BoxFuture<'a, Result<StoredObject>> {
        Box::pin(BucketStore::get(self, key, range))
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(BucketStore::delete(self, key))
    }

    fn list(&self) -> Listing {
        Listing::new(BucketPages {
            store: self.clone(),
            token: None,
            done: false,
        })
    }

    fn probe(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(BucketStore::probe(self))
    }
}

struct BucketPages {
    store: BucketStore,
    token: Option<String>,
    done: bool,
}

impl ListPages for BucketPages {
    fn next_page(&mut self) -> BoxFuture<'_, Result<Option<Vec<StoredObjectInfo>>>> {
        Box::pin(async move {
            if self.done {
                return Ok(None);
            }
            let token = self.token.clone();
            let answer = self
                .store
                .small(|now| {
                    self.store.inner.client.list_objects(
                        &s3::ListRequest {
                            continuation_token: token.as_deref(),
                            ..Default::default()
                        },
                        now,
                    )
                })
                .await?;
            let page = s3::read_list_objects(answer.status, &answer.headers, &answer.body)
                .map_err(|error| from_s3(&error, ""))?;
            if page.next_token.is_some() && page.next_token == self.token {
                return Err(Error::store(
                    StoreFault::Malformed,
                    "the store sent the same listing page twice",
                ));
            }
            self.done = page.next_token.is_none();
            self.token = page.next_token;
            let objects = page
                .objects
                .into_iter()
                .filter(|object| {
                    // Keys nobody here could have written, and that a restore
                    // into a folder must not follow.
                    let valid = key::is_valid(&object.key);
                    if !valid {
                        tracing::warn!("the bucket holds a key no store takes; it is not listed");
                    }
                    valid
                })
                .map(|object| StoredObjectInfo {
                    key: object.key,
                    size: object.size,
                })
                .collect();
            Ok(Some(objects))
        })
    }
}

/// What a failure of the HTTP client means to a caller.
pub(crate) fn from_outbound(error: &stellarr_outbound::Error) -> Error {
    use stellarr_outbound::Error as E;
    let kind = match error {
        E::TooLarge | E::Protocol(_) => StoreFault::Malformed,
        E::InvalidUrl(_)
        | E::ForbiddenHeader(_)
        | E::AddressRefused { .. }
        | E::Roots(_)
        | E::TooManyRedirects
        | E::RedirectRefused(_)
        | E::BodyNotReplayable => StoreFault::Rejected,
        _ => StoreFault::Unreachable,
    };
    // A refused TLS handshake is the operator's to fix, and no retry helps.
    let kind = if matches!(error, E::Tls { .. }) {
        StoreFault::Rejected
    } else {
        kind
    };
    Error::store(kind, error.to_string())
}

/// Whether a store refused a conditional create because the key is taken:
/// the precondition failed, or another write to it was running (the answer of
/// a store that sees two conditional writes at once).
fn refused_as_taken(error: &s3::Error) -> bool {
    matches!(
        error.kind,
        s3::ErrorKind::PreconditionFailed | s3::ErrorKind::Conflict
    )
}

/// What a refusal from the store means to a caller of the call made for `key`.
fn from_s3(error: &s3::Error, key: &str) -> Error {
    use s3::ErrorKind as K;
    let kind = match error.kind {
        K::NotFound => return Error::NotFound(key.to_owned()),
        K::RangeNotSatisfiable => return Error::RangeNotSatisfiable(key.to_owned()),
        K::BucketNotFound => StoreFault::NotReady,
        K::AccessDenied | K::BadCredentials | K::ClockSkew => StoreFault::Denied,
        K::Throttled => StoreFault::Throttled,
        K::Unavailable => StoreFault::Unreachable,
        K::Malformed => StoreFault::Malformed,
        K::UploadNotFound | K::AlreadyOwned | K::Conflict | K::PreconditionFailed | K::Rejected => {
            StoreFault::Rejected
        }
    };
    let mut message = format!("status {}", error.status);
    if let Some(code) = &error.code {
        message.push_str(&format!(" {code}"));
    }
    if let Some(text) = &error.message {
        message.push_str(&format!(": {text}"));
    }
    Error::store(kind, message)
}
