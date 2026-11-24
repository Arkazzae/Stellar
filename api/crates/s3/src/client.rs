//! Building the requests: addressing, headers, signing.

use std::fmt;

use crate::encode::{encode_component, encode_path, query_strings};
use crate::sign::{
    self, ChunkSigner, Credentials, EMPTY_SHA256, MIN_CHUNK, STREAMING_PAYLOAD, Scope,
    UNSIGNED_PAYLOAD, chunked_length, hex, sha256,
};
use crate::time::Timestamp;
use crate::xml;

const MAX_KEY: usize = 1024;
const MAX_CHUNK: usize = 16 * 1024 * 1024;
const MAX_UPLOAD_ID: usize = 1024;
const MAX_ETAG: usize = 256;
const MAX_METADATA_BYTES: usize = 2048;

/// The smallest part S3 takes, except for the last.
pub const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
/// At most this many parts make an object.
pub const MAX_PARTS: u64 = 10_000;
/// A file of at least this many bytes goes up in parts: below it one request
/// is cheaper than the extra round trips of a multipart upload.
pub const MULTIPART_THRESHOLD: u64 = 16 * 1024 * 1024;
const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;

/// What a store is called and signed as.
#[derive(Clone, Debug)]
pub struct Config {
    https: bool,
    host: String,
    base_path: String,
    region: String,
    bucket: String,
    credentials: Credentials,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    Endpoint(&'static str),
    Region,
    Bucket,
    Credentials,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Endpoint(why) => write!(f, "the endpoint is not usable: {why}"),
            Self::Region => f.write_str("the region is empty or has characters a region never has"),
            Self::Bucket => f.write_str("the bucket name is empty or too long"),
            Self::Credentials => {
                f.write_str("the access key or secret is empty or has control characters")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// `endpoint` is `http(s)://host[:port][/prefix]`, with no credentials,
    /// query or fragment. The region only goes into the signature, which is why
    /// stores that have no regions ask for `us-east-1`, or `auto` on R2.
    pub fn new(
        endpoint: &str,
        region: &str,
        bucket: &str,
        credentials: Credentials,
    ) -> Result<Self, ConfigError> {
        let (https, rest) = match endpoint.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
            _ => return Err(ConfigError::Endpoint("the scheme must be http or https")),
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, ""),
        };
        if path.contains(['?', '#', '%', '\\'])
            || rest.contains(['?', '#'])
            || authority.contains('@')
        {
            return Err(ConfigError::Endpoint(
                "no credentials, query, fragment or escapes are allowed",
            ));
        }
        let host = canonical_authority(authority, https)?;
        let base_path = path.trim_end_matches('/').to_owned();
        if base_path.contains("//") || base_path.split('/').any(|s| s == "." || s == "..") {
            return Err(ConfigError::Endpoint("the path prefix must be plain"));
        }
        if region.is_empty()
            || region.len() > 64
            || !region
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ConfigError::Region);
        }
        if bucket.is_empty() || bucket.len() > 255 {
            return Err(ConfigError::Bucket);
        }
        let plain = |text: &str| !text.is_empty() && !text.chars().any(char::is_control);
        if !plain(&credentials.access_key) || !plain(&credentials.secret_key) {
            return Err(ConfigError::Credentials);
        }
        Ok(Self {
            https,
            host,
            base_path,
            region: region.to_owned(),
            bucket: bucket.to_owned(),
            credentials,
        })
    }
}

/// Lower-cases the host and drops a port that is the scheme's default, so the
/// `Host` header the HTTP client derives is the one that is signed.
fn canonical_authority(authority: &str, https: bool) -> Result<String, ConfigError> {
    let bad = ConfigError::Endpoint("the host is not valid");
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (inside, after) = rest.split_once(']').ok_or(bad)?;
        if inside.is_empty()
            || !inside
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return Err(bad);
        }
        (
            format!("[{}]", inside.to_ascii_lowercase()),
            after.strip_prefix(':'),
        )
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.is_empty()
            || host.len() > 253
            || host.starts_with(['.', '-'])
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err(bad);
        }
        (host.to_ascii_lowercase(), port)
    };
    match port {
        None => Ok(host),
        Some(port) => {
            if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad);
            }
            let number: u32 = port.parse().map_err(|_| bad)?;
            if number == 0 || number > 65_535 {
                return Err(bad);
            }
            let default = if https { 443 } else { 80 };
            Ok(if number == default {
                host
            } else {
                format!("{host}:{number}")
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// An object key that S3 cannot hold: empty, or longer than 1024 bytes.
    BadKey(&'static str),
    /// A header value or name that cannot be sent as it is.
    BadHeader(&'static str),
    /// `Payload::Unsigned` only goes over TLS.
    UnsignedOverPlainHttp,
    /// A chunk size outside what S3 accepts.
    BadChunkSize,
    BadRange,
    BadUploadId,
    BadPart(&'static str),
    BadTime,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadKey(why) | Self::BadHeader(why) | Self::BadPart(why) => f.write_str(why),
            Self::UnsignedOverPlainHttp => {
                f.write_str("an unsigned payload is only allowed over https")
            }
            Self::BadChunkSize => f.write_str("the chunk size must be between 8 KiB and 16 MiB"),
            Self::BadRange => f.write_str("the range ends before it starts"),
            Self::BadUploadId => f.write_str("the upload id is empty or too long"),
            Self::BadTime => f.write_str("the time cannot be written as a date"),
        }
    }
}

impl std::error::Error for RequestError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Put,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Put => "PUT",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }
}

/// How the bytes of an object are covered by the signature. See the crate
/// documentation for when to use which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// The SHA-256 of the whole body, known before sending.
    Hashed([u8; 32]),
    /// Nothing signed over the body; https only.
    Unsigned,
    /// Signed chunks of this size, the last one shorter.
    Chunked { chunk_size: usize },
}

impl Payload {
    /// The hashed form for bytes in memory.
    pub fn hash_of(bytes: &[u8]) -> Self {
        Self::Hashed(sha256(bytes))
    }
}

/// A byte range as HTTP spells it: both ends inclusive, no end meaning "to the end".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub from: u64,
    pub to: Option<u64>,
}

/// What the HTTP client has to send as the body.
#[derive(Debug)]
pub enum RequestBody {
    /// No body.
    Empty,
    /// These bytes, as they are.
    Bytes(Vec<u8>),
    /// `length` bytes of the object, as they are.
    Object { length: u64 },
    /// The object framed by this signer. The object's bytes go in through
    /// [`ChunkSigner::chunk`] and the signer's output is the body; the
    /// `content-length` header already counts the framing.
    Chunked(ChunkSigner),
}

/// A request ready to be sent. Send the headers as they are, the `Host` header
/// as `host`, and no others that S3 reads (`x-amz-*`).
#[derive(Debug)]
pub struct Prepared {
    pub method: Method,
    pub url: String,
    pub host: String,
    /// Lower-case names. `Host` is not among them.
    pub headers: Vec<(String, String)>,
    pub body: RequestBody,
}

impl Prepared {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Which page of a bucket listing to ask for.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListRequest<'a> {
    pub max_keys: Option<u32>,
    pub continuation_token: Option<&'a str>,
    pub prefix: Option<&'a str>,
}

/// A part that was uploaded, as completing the upload names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub number: u32,
    pub etag: String,
}

/// How to send a file of a given size: in one request or in parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadPlan {
    Single,
    Multipart { part_size: u64, parts: u32 },
}

impl UploadPlan {
    /// `part_size` is the part size wanted. It is raised when the file would
    /// otherwise need more than 10 000 parts. Stores such as R2 want every part
    /// but the last to be the same size, which this keeps.
    pub fn new(length: u64, part_size: u64) -> Result<Self, RequestError> {
        if !(MIN_PART_SIZE..=MAX_PART_SIZE).contains(&part_size) {
            return Err(RequestError::BadPart(
                "the part size must be between 5 MiB and 5 GiB",
            ));
        }
        if length < MULTIPART_THRESHOLD {
            return Ok(Self::Single);
        }
        let part_size = part_size.max(length.div_ceil(MAX_PARTS));
        if part_size > MAX_PART_SIZE {
            return Err(RequestError::BadPart("the object is too large"));
        }
        Ok(Self::Multipart {
            part_size,
            parts: u32::try_from(length.div_ceil(part_size)).unwrap_or(u32::MAX),
        })
    }

    /// Where part `number` (from 1) of an object of `length` bytes starts and
    /// how long it is.
    pub fn part_span(&self, number: u32, length: u64) -> Option<(u64, u64)> {
        let Self::Multipart { part_size, parts } = *self else {
            return None;
        };
        if number == 0 || number > parts {
            return None;
        }
        let start = u64::from(number - 1) * part_size;
        Some((start, part_size.min(length - start)))
    }
}

pub struct Client {
    config: Config,
}

impl Client {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// `GET /`: every bucket the credentials can see.
    pub fn list_buckets(&self, now: Timestamp) -> Result<Prepared, RequestError> {
        self.build(
            Target::Service,
            Method::Get,
            &[],
            Vec::new(),
            Outgoing::Nothing,
            now,
        )
    }

    /// `PUT /bucket`, with no body: the default location of the store.
    pub fn create_bucket(&self, now: Timestamp) -> Result<Prepared, RequestError> {
        self.build(
            Target::Bucket,
            Method::Put,
            &[],
            Vec::new(),
            Outgoing::Bytes(Vec::new()),
            now,
        )
    }

    /// `GET /bucket?list-type=2`, one page. Keys come back URL-encoded so that
    /// no key can break the XML.
    pub fn list_objects(
        &self,
        request: &ListRequest<'_>,
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        let max_keys = request.max_keys.map(|n| n.to_string());
        let mut query = vec![("list-type", "2"), ("encoding-type", "url")];
        if let Some(max) = &max_keys {
            query.push(("max-keys", max));
        }
        if let Some(token) = request.continuation_token {
            query.push(("continuation-token", token));
        }
        if let Some(prefix) = request.prefix {
            query.push(("prefix", prefix));
        }
        self.build(
            Target::Bucket,
            Method::Get,
            &query,
            Vec::new(),
            Outgoing::Nothing,
            now,
        )
    }

    /// `GET /bucket/key`, whole or a range.
    pub fn get_object(
        &self,
        key: &str,
        range: Option<ByteRange>,
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        let mut headers = Vec::new();
        if let Some(range) = range {
            let value = match range.to {
                Some(to) if to < range.from => return Err(RequestError::BadRange),
                Some(to) => format!("bytes={}-{to}", range.from),
                None => format!("bytes={}-", range.from),
            };
            headers.push(("range".to_owned(), value));
        }
        self.build(
            Target::Object(key),
            Method::Get,
            &[],
            headers,
            Outgoing::Nothing,
            now,
        )
    }

    /// `HEAD /bucket/key`: size and metadata, no bytes.
    pub fn head_object(&self, key: &str, now: Timestamp) -> Result<Prepared, RequestError> {
        self.build(
            Target::Object(key),
            Method::Head,
            &[],
            Vec::new(),
            Outgoing::Nothing,
            now,
        )
    }

    /// `DELETE /bucket/key`.
    pub fn delete_object(&self, key: &str, now: Timestamp) -> Result<Prepared, RequestError> {
        self.build(
            Target::Object(key),
            Method::Delete,
            &[],
            Vec::new(),
            Outgoing::Nothing,
            now,
        )
    }

    /// `PUT /bucket/key` with `length` bytes of object. `metadata` becomes
    /// `x-amz-meta-*` headers.
    pub fn put_object(
        &self,
        key: &str,
        length: u64,
        content_type: Option<&str>,
        metadata: &[(&str, &str)],
        payload: Payload,
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        let mut headers = vec![(
            "content-type".to_owned(),
            content_type
                .unwrap_or("application/octet-stream")
                .to_owned(),
        )];
        check_header_value(&headers[0].1)?;
        headers.extend(metadata_headers(metadata)?);
        self.build(
            Target::Object(key),
            Method::Put,
            &[],
            headers,
            Outgoing::Object { length, payload },
            now,
        )
    }

    /// `POST /bucket/key?uploads`: starts a multipart upload. The type and
    /// metadata belong to the finished object.
    pub fn create_multipart_upload(
        &self,
        key: &str,
        content_type: Option<&str>,
        metadata: &[(&str, &str)],
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        let mut headers = vec![(
            "content-type".to_owned(),
            content_type
                .unwrap_or("application/octet-stream")
                .to_owned(),
        )];
        check_header_value(&headers[0].1)?;
        headers.extend(metadata_headers(metadata)?);
        self.build(
            Target::Object(key),
            Method::Post,
            &[("uploads", "")],
            headers,
            Outgoing::Bytes(Vec::new()),
            now,
        )
    }

    /// `PUT /bucket/key?partNumber=N&uploadId=...` with `length` bytes of the part.
    pub fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: u32,
        length: u64,
        payload: Payload,
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        check_upload_id(upload_id)?;
        if part_number == 0 || u64::from(part_number) > MAX_PARTS {
            return Err(RequestError::BadPart("part numbers run from 1 to 10000"));
        }
        let number = part_number.to_string();
        self.build(
            Target::Object(key),
            Method::Put,
            &[("partNumber", &number), ("uploadId", upload_id)],
            Vec::new(),
            Outgoing::Object { length, payload },
            now,
        )
    }

    /// `POST /bucket/key?uploadId=...` naming the parts in order. The answer can
    /// be a `200` that carries an error, so read it with
    /// [`read_complete_multipart_upload`](crate::read_complete_multipart_upload).
    pub fn complete_multipart_upload(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[Part],
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        check_upload_id(upload_id)?;
        if parts.is_empty() || parts.len() as u64 > MAX_PARTS {
            return Err(RequestError::BadPart(
                "an upload needs between 1 and 10000 parts",
            ));
        }
        let mut body = String::from(
            "<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
        );
        let mut last = 0;
        for part in parts {
            if part.number <= last || u64::from(part.number) > MAX_PARTS {
                return Err(RequestError::BadPart(
                    "parts must be numbered upwards from 1",
                ));
            }
            last = part.number;
            check_etag(&part.etag)?;
            body.push_str(&format!(
                "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
                part.number,
                xml::escape(&part.etag)
            ));
        }
        body.push_str("</CompleteMultipartUpload>");
        let headers = vec![("content-type".to_owned(), "application/xml".to_owned())];
        self.build(
            Target::Object(key),
            Method::Post,
            &[("uploadId", upload_id)],
            headers,
            Outgoing::Bytes(body.into_bytes()),
            now,
        )
    }

    /// `DELETE /bucket/key?uploadId=...`: throws away the parts of a failed upload.
    pub fn abort_multipart_upload(
        &self,
        key: &str,
        upload_id: &str,
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        check_upload_id(upload_id)?;
        self.build(
            Target::Object(key),
            Method::Delete,
            &[("uploadId", upload_id)],
            Vec::new(),
            Outgoing::Nothing,
            now,
        )
    }

    fn build(
        &self,
        target: Target<'_>,
        method: Method,
        query: &[(&str, &str)],
        mut headers: Vec<(String, String)>,
        outgoing: Outgoing,
        now: Timestamp,
    ) -> Result<Prepared, RequestError> {
        let config = &self.config;
        let mut path = config.base_path.clone();
        match target {
            Target::Service => path.push('/'),
            Target::Bucket => {
                path.push('/');
                path.push_str(&encode_component(&config.bucket));
            }
            Target::Object(key) => {
                if key.is_empty() {
                    return Err(RequestError::BadKey("an object key cannot be empty"));
                }
                if key.len() > MAX_KEY {
                    return Err(RequestError::BadKey("an object key is at most 1024 bytes"));
                }
                path.push('/');
                path.push_str(&encode_component(&config.bucket));
                path.push('/');
                path.push_str(&encode_path(key));
            }
        }
        let (canonical_query, sent_query) = query_strings(query);

        let amz_date = now.amz_date();
        let scope = Scope {
            amz_date: &amz_date,
            region: &config.region,
            service: "s3",
        };

        let (payload_hash, chunked, body) = match outgoing {
            Outgoing::Nothing => (EMPTY_SHA256.to_owned(), None, RequestBody::Empty),
            Outgoing::Bytes(bytes) => {
                let hash = hex(&sha256(&bytes));
                headers.push(("content-length".to_owned(), bytes.len().to_string()));
                (
                    hash,
                    None,
                    if bytes.is_empty() {
                        RequestBody::Empty
                    } else {
                        RequestBody::Bytes(bytes)
                    },
                )
            }
            Outgoing::Object { length, payload } => match payload {
                Payload::Hashed(hash) => {
                    headers.push(("content-length".to_owned(), length.to_string()));
                    (hex(&hash), None, RequestBody::Object { length })
                }
                Payload::Unsigned => {
                    if !config.https {
                        return Err(RequestError::UnsignedOverPlainHttp);
                    }
                    headers.push(("content-length".to_owned(), length.to_string()));
                    (
                        UNSIGNED_PAYLOAD.to_owned(),
                        None,
                        RequestBody::Object { length },
                    )
                }
                Payload::Chunked { chunk_size } => {
                    if !(MIN_CHUNK..=MAX_CHUNK).contains(&chunk_size) {
                        return Err(RequestError::BadChunkSize);
                    }
                    headers.push(("content-encoding".to_owned(), "aws-chunked".to_owned()));
                    headers.push((
                        "x-amz-decoded-content-length".to_owned(),
                        length.to_string(),
                    ));
                    headers.push((
                        "content-length".to_owned(),
                        chunked_length(length, chunk_size).to_string(),
                    ));
                    (
                        STREAMING_PAYLOAD.to_owned(),
                        Some((length, chunk_size)),
                        RequestBody::Empty,
                    )
                }
            },
        };
        // A bodyless PUT or POST still states its length, so the client cannot
        // choose a different framing than the one that was signed.
        if matches!(method, Method::Put | Method::Post)
            && !headers.iter().any(|(n, _)| n == "content-length")
        {
            headers.push(("content-length".to_owned(), "0".to_owned()));
        }
        headers.push(("x-amz-content-sha256".to_owned(), payload_hash.clone()));
        headers.push(("x-amz-date".to_owned(), amz_date.clone()));

        let mut signed_headers: Vec<(&str, &str)> = vec![("host", &config.host)];
        signed_headers.extend(headers.iter().map(|(n, v)| (n.as_str(), v.as_str())));
        let signed = sign::sign(
            &config.credentials,
            &scope,
            &sign::Request {
                method: method.as_str(),
                path: &path,
                query: &canonical_query,
                headers: &signed_headers,
                payload_hash: &payload_hash,
            },
        );
        headers.push(("authorization".to_owned(), signed.authorization.clone()));

        let body = match chunked {
            Some((length, chunk_size)) => {
                RequestBody::Chunked(ChunkSigner::new(&signed, &scope, length, chunk_size))
            }
            None => body,
        };
        let mut url = format!(
            "{}://{}{path}",
            if config.https { "https" } else { "http" },
            config.host
        );
        if !sent_query.is_empty() {
            url.push('?');
            url.push_str(&sent_query);
        }
        Ok(Prepared {
            method,
            url,
            host: config.host.clone(),
            headers,
            body,
        })
    }
}

enum Target<'a> {
    Service,
    Bucket,
    Object(&'a str),
}

enum Outgoing {
    Nothing,
    Bytes(Vec<u8>),
    Object { length: u64, payload: Payload },
}

fn check_header_value(value: &str) -> Result<(), RequestError> {
    let printable = value.bytes().all(|b| (0x20..0x7f).contains(&b));
    if value.is_empty()
        || value.len() > 1024
        || !printable
        || value.starts_with(' ')
        || value.ends_with(' ')
        || value.contains("  ")
    {
        return Err(RequestError::BadHeader(
            "a header value must be printable ASCII without stray blanks",
        ));
    }
    Ok(())
}

fn metadata_headers(metadata: &[(&str, &str)]) -> Result<Vec<(String, String)>, RequestError> {
    let mut total = 0;
    let mut out: Vec<(String, String)> = Vec::with_capacity(metadata.len());
    for (name, value) in metadata {
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(RequestError::BadHeader(
                "a metadata name is letters, digits and hyphens",
            ));
        }
        check_header_value(value)?;
        let header = format!("x-amz-meta-{}", name.to_ascii_lowercase());
        if out.iter().any(|(n, _)| *n == header) {
            return Err(RequestError::BadHeader("a metadata name is given twice"));
        }
        total += name.len() + value.len();
        out.push((header, (*value).to_owned()));
    }
    if total > MAX_METADATA_BYTES {
        return Err(RequestError::BadHeader("user metadata is at most 2 KiB"));
    }
    Ok(out)
}

fn check_upload_id(id: &str) -> Result<(), RequestError> {
    if id.is_empty() || id.len() > MAX_UPLOAD_ID {
        return Err(RequestError::BadUploadId);
    }
    Ok(())
}

fn check_etag(etag: &str) -> Result<(), RequestError> {
    if etag.is_empty() || etag.len() > MAX_ETAG || !etag.bytes().all(|b| (0x20..0x7f).contains(&b))
    {
        return Err(RequestError::BadPart(
            "an etag is printable ASCII of at most 256 bytes",
        ));
    }
    Ok(())
}
