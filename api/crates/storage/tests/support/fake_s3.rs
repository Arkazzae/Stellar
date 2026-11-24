//! A bucket server that speaks the part of the protocol the application uses,
//! for tests. It checks every signature it is sent and every body's hash, and
//! keeps a log of the requests exactly as they arrived, so a test can show what
//! the client put on the wire.
//!
//! It is written from the protocol's documentation, not from a real store's
//! behaviour: what a real store does is the business of the MinIO run.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use stellarr_s3::internals::{
    Request as SignRequest, Scope, decode, encode_component, query_strings, sign,
};
use stellarr_s3::{Credentials, sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub const ACCESS_KEY: &str = "fake-access";
pub const SECRET_KEY: &str = "fake-secret-key-0123456789";
pub const MIN_PART: usize = 5 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Logged {
    pub method: String,
    /// The request target as it arrived: path and query, still encoded.
    pub target: String,
    /// Header names in lower case, values as sent.
    pub headers: Vec<(String, String)>,
    pub body_length: usize,
    pub status: u16,
}

impl Logged {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    pub fn query(&self) -> &str {
        self.target.split_once('?').map_or("", |(_, q)| q)
    }
}

#[derive(Default)]
pub struct Settings {
    /// Keys per listing page; 0 means 1000.
    pub page_size: usize,
    /// Answer a range with the whole object and a 200.
    pub ignore_range: bool,
    /// Answer CompleteMultipartUpload with a 200 that carries an error.
    pub complete_with_error: bool,
    /// Put blanks before the answer to CompleteMultipartUpload, as a store
    /// that keeps the connection alive during a long assembly does.
    pub complete_leading_whitespace: bool,
    /// Fail the next requests of this method with this status: (method, status, count).
    pub faults: VecDeque<(String, u16, usize)>,
}

#[derive(Default)]
struct Object {
    bytes: Vec<u8>,
    content_type: String,
}

#[derive(Default)]
struct Upload {
    bucket: String,
    key: String,
    parts: BTreeMap<u32, (Vec<u8>, String)>,
}

#[derive(Default)]
pub struct State {
    buckets: BTreeMap<String, BTreeMap<String, Object>>,
    uploads: HashMap<String, Upload>,
    pub log: Vec<Logged>,
    pub settings: Settings,
    pub aborted: Vec<String>,
    /// For each completed multipart upload, the sizes of its parts.
    pub completed_part_sizes: Vec<Vec<usize>>,
    next_id: u64,
}

pub struct FakeS3 {
    pub endpoint: String,
    pub state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeS3 {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let state = shared.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, state).await;
                });
            }
        });
        Self {
            endpoint,
            state,
            task,
        }
    }

    pub fn requests(&self) -> Vec<Logged> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
    }

    pub fn open_uploads(&self) -> usize {
        self.state.lock().unwrap().uploads.len()
    }

    pub fn aborted(&self) -> usize {
        self.state.lock().unwrap().aborted.len()
    }

    pub fn fail_next(&self, method: &str, status: u16, count: usize) {
        self.state
            .lock()
            .unwrap()
            .settings
            .faults
            .push_back((method.to_owned(), status, count));
    }

    /// Puts an object in a bucket behind the store's back, with any key.
    pub fn plant(&self, bucket: &str, key: &str, bytes: &[u8]) {
        self.state
            .lock()
            .unwrap()
            .buckets
            .entry(bucket.to_owned())
            .or_default()
            .insert(
                key.to_owned(),
                Object {
                    bytes: bytes.to_vec(),
                    content_type: String::new(),
                },
            );
    }

    pub fn stored(&self, bucket: &str, key: &str) -> Option<(Vec<u8>, String)> {
        self.state
            .lock()
            .unwrap()
            .buckets
            .get(bucket)?
            .get(key)
            .map(|o| (o.bytes.clone(), o.content_type.clone()))
    }
}

struct Incoming {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct Outgoing {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// For HEAD: the length to announce instead of the body's.
    announce: Option<usize>,
}

impl Outgoing {
    fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            announce: None,
        }
    }

    fn xml(status: u16, body: String) -> Self {
        let mut out = Self::new(status);
        out.headers
            .push(("content-type".into(), "application/xml".into()));
        out.body = body.into_bytes();
        out
    }

    fn error(status: u16, code: &str, message: &str) -> Self {
        Self::xml(
            status,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{message}</Message><RequestId>REQ1</RequestId></Error>"
            ),
        )
    }
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream).await? else {
        return Ok(());
    };
    let method = request.method.clone();
    let (response, logged) = {
        let mut guard = state.lock().unwrap();
        let mut response = handle(&mut guard, &request);
        if method == "HEAD" {
            response.announce = Some(response.body.len());
            response.body.clear();
        }
        let logged = Logged {
            method: request.method.clone(),
            target: request.target.clone(),
            headers: request.headers.clone(),
            body_length: request.body.len(),
            status: response.status,
        };
        guard.log.push(logged.clone());
        (response, logged)
    };
    let _ = logged;
    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        416 => "Range Not Satisfiable",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let length = response.announce.unwrap_or(response.body.len());
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", response.status);
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "content-length: {length}\r\nconnection: close\r\n\r\n"
    ));
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Incoming>> {
    let mut buffer = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..n]);
        if buffer.len() > 256 * 1024 {
            return Ok(None);
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(Some(Incoming {
        method,
        target,
        headers,
        body,
    }))
}

fn header<'a>(request: &'a Incoming, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// Checks the signature the way a store does, from what arrived.
fn verify(request: &Incoming) -> Result<(), Outgoing> {
    let denied = |why: &str| Err(Outgoing::error(403, "SignatureDoesNotMatch", why));
    let Some(authorization) = header(request, "authorization") else {
        return Err(Outgoing::error(403, "AccessDenied", "no authorization"));
    };
    let Some(rest) = authorization.strip_prefix("AWS4-HMAC-SHA256 ") else {
        return denied("not SigV4");
    };
    let field = |name: &str| {
        rest.split(", ")
            .find_map(|part| part.strip_prefix(name).and_then(|v| v.strip_prefix('=')))
    };
    let (Some(credential), Some(signed), Some(signature)) = (
        field("Credential"),
        field("SignedHeaders"),
        field("Signature"),
    ) else {
        return denied("malformed authorization");
    };
    let mut scope = credential.split('/');
    let (access, date, region, service) = (
        scope.next().unwrap_or_default(),
        scope.next().unwrap_or_default(),
        scope.next().unwrap_or_default(),
        scope.next().unwrap_or_default(),
    );
    if access != ACCESS_KEY {
        return Err(Outgoing::error(403, "InvalidAccessKeyId", "unknown key"));
    }
    let Some(amz_date) = header(request, "x-amz-date") else {
        return denied("no date");
    };
    if !amz_date.starts_with(date) {
        return denied("scope date differs from x-amz-date");
    }
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target.as_str(), ""));
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (n, v) = p.split_once('=').unwrap_or((p, ""));
            (
                decode(n, false).unwrap_or_default(),
                decode(v, false).unwrap_or_default(),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let (canonical_query, _) = query_strings(&borrowed);
    let mut signed_values: Vec<(String, String)> = Vec::new();
    for name in signed.split(';') {
        let Some(value) = header(request, name) else {
            return denied("a signed header is missing");
        };
        signed_values.push((name.to_owned(), value.to_owned()));
    }
    let borrowed: Vec<(&str, &str)> = signed_values
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let payload = header(request, "x-amz-content-sha256").unwrap_or_default();
    let expected = sign(
        &Credentials {
            access_key: ACCESS_KEY.into(),
            secret_key: SECRET_KEY.into(),
        },
        &Scope {
            amz_date,
            region,
            service,
        },
        &SignRequest {
            method: &request.method,
            path,
            query: &canonical_query,
            headers: &borrowed,
            payload_hash: payload,
        },
    );
    if expected.signature != signature {
        return denied("the signature does not match the request as it arrived");
    }
    if payload.len() == 64 && payload != stellarr_s3::internals::hex(&sha256(&request.body)) {
        return Err(Outgoing::error(
            400,
            "XAmzContentSHA256Mismatch",
            "the body does not hash to the signed value",
        ));
    }
    Ok(())
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(n, false).as_deref() == Some(name)).then(|| decode(v, false).unwrap_or_default())
    })
}

fn has_query(query: &str, name: &str) -> bool {
    query_value(query, name).is_some()
}

fn handle(state: &mut State, request: &Incoming) -> Outgoing {
    if let Some(at) = state
        .settings
        .faults
        .iter()
        .position(|(m, _, _)| *m == request.method)
    {
        let (_, status, count) = &mut state.settings.faults[at];
        let status = *status;
        *count -= 1;
        if *count == 0 {
            state.settings.faults.remove(at);
        }
        return Outgoing::error(status, "InternalError", "injected");
    }
    if let Err(response) = verify(request) {
        return response;
    }
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target.as_str(), ""));
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let (bucket_raw, key_raw) = match trimmed.split_once('/') {
        Some((b, k)) => (b, Some(k)),
        None => (trimmed, None),
    };
    let bucket = decode(bucket_raw, false).unwrap_or_default();
    let key = key_raw.map(|k| decode(k, false).unwrap_or_default());
    match (request.method.as_str(), bucket.is_empty(), key.as_deref()) {
        ("GET", true, _) => {
            let names: String = state
                .buckets
                .keys()
                .map(|b| format!("<Bucket><Name>{b}</Name></Bucket>"))
                .collect();
            Outgoing::xml(
                200,
                format!(
                    "<ListAllMyBucketsResult><Owner><ID>x</ID></Owner><Buckets>{names}</Buckets></ListAllMyBucketsResult>"
                ),
            )
        }
        ("PUT", false, None) => {
            if state.buckets.contains_key(&bucket) {
                return Outgoing::error(409, "BucketAlreadyOwnedByYou", "yours");
            }
            state.buckets.insert(bucket, BTreeMap::new());
            Outgoing::new(200)
        }
        ("GET", false, None | Some("")) => list(state, &bucket, query),
        (_, false, Some(key)) if !key.is_empty() => object(state, request, &bucket, key, query),
        _ => Outgoing::error(400, "InvalidRequest", "unsupported"),
    }
}

fn list(state: &State, bucket: &str, query: &str) -> Outgoing {
    let Some(objects) = state.buckets.get(bucket) else {
        return Outgoing::error(404, "NoSuchBucket", "no bucket");
    };
    if query_value(query, "list-type").as_deref() != Some("2") {
        return Outgoing::error(400, "InvalidArgument", "list-type 2 only");
    }
    let page = match query_value(query, "max-keys").and_then(|v| v.parse().ok()) {
        Some(n) if n > 0 => n,
        _ => 1000,
    };
    let page = if state.settings.page_size > 0 {
        page.min(state.settings.page_size)
    } else {
        page
    };
    let after = query_value(query, "continuation-token");
    let url = query_value(query, "encoding-type").as_deref() == Some("url");
    let encode = |key: &str| {
        if url {
            encode_component(key)
                .replace("%2F", "/")
                .replace("%20", "+")
        } else {
            key.replace('&', "&amp;").replace('<', "&lt;")
        }
    };
    let mut contents = String::new();
    let mut taken = 0;
    let mut last = None;
    let mut truncated = false;
    for (key, object) in objects {
        if after.as_deref().is_some_and(|a| key.as_str() <= a) {
            continue;
        }
        if taken == page {
            truncated = true;
            break;
        }
        contents.push_str(&format!(
            "<Contents><Key>{}</Key><Size>{}</Size></Contents>",
            encode(key),
            object.bytes.len()
        ));
        taken += 1;
        last = Some(key.clone());
    }
    let token = if truncated {
        format!(
            "<NextContinuationToken>{}</NextContinuationToken>",
            encode_component(last.as_deref().unwrap_or_default())
        )
    } else {
        String::new()
    };
    Outgoing::xml(
        200,
        format!(
            "<ListBucketResult><Name>{bucket}</Name>{}<KeyCount>{taken}</KeyCount><IsTruncated>{truncated}</IsTruncated>{token}{contents}</ListBucketResult>",
            if url {
                "<EncodingType>url</EncodingType>"
            } else {
                ""
            }
        ),
    )
}

fn etag_of(bytes: &[u8]) -> String {
    format!("\"{}\"", &stellarr_s3::internals::hex(&sha256(bytes))[..32])
}

fn object(state: &mut State, request: &Incoming, bucket: &str, key: &str, query: &str) -> Outgoing {
    if !state.buckets.contains_key(bucket) {
        return Outgoing::error(404, "NoSuchBucket", "no bucket");
    }
    match request.method.as_str() {
        "PUT" if has_query(query, "partNumber") => {
            let id = query_value(query, "uploadId").unwrap_or_default();
            let number: u32 = query_value(query, "partNumber")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            if !(1..=10_000).contains(&number) {
                return Outgoing::error(400, "InvalidArgument", "part number");
            }
            let Some(upload) = state.uploads.get_mut(&id) else {
                return Outgoing::error(404, "NoSuchUpload", "no upload");
            };
            let etag = etag_of(&request.body);
            upload
                .parts
                .insert(number, (request.body.clone(), etag.clone()));
            let mut out = Outgoing::new(200);
            out.headers.push(("etag".into(), etag));
            out
        }
        "PUT" => {
            let etag = etag_of(&request.body);
            let content_type = header(request, "content-type")
                .unwrap_or_default()
                .to_owned();
            state.buckets.get_mut(bucket).unwrap().insert(
                key.to_owned(),
                Object {
                    bytes: request.body.clone(),
                    content_type,
                },
            );
            let mut out = Outgoing::new(200);
            out.headers.push(("etag".into(), etag));
            out
        }
        "POST" if has_query(query, "uploads") => {
            state.next_id += 1;
            let id = format!("upload-{}", state.next_id);
            state.uploads.insert(
                id.clone(),
                Upload {
                    bucket: bucket.to_owned(),
                    key: key.to_owned(),
                    parts: BTreeMap::new(),
                },
            );
            Outgoing::xml(
                200,
                format!(
                    "<InitiateMultipartUploadResult><Bucket>{bucket}</Bucket><Key>x</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>"
                ),
            )
        }
        "POST" => complete(state, request, bucket, key, query),
        "DELETE" if has_query(query, "uploadId") => {
            let id = query_value(query, "uploadId").unwrap_or_default();
            if state.uploads.remove(&id).is_none() {
                return Outgoing::error(404, "NoSuchUpload", "no upload");
            }
            state.aborted.push(id);
            Outgoing::new(204)
        }
        "DELETE" => {
            state.buckets.get_mut(bucket).unwrap().remove(key);
            Outgoing::new(204)
        }
        "GET" | "HEAD" => {
            let Some(found) = state.buckets[bucket].get(key) else {
                return Outgoing::error(404, "NoSuchKey", "no key");
            };
            let total = found.bytes.len();
            let mut out = Outgoing::new(200);
            out.headers.push(("etag".into(), etag_of(&found.bytes)));
            out.headers
                .push(("content-type".into(), found.content_type.clone()));
            let range = header(request, "range").filter(|_| request.method == "GET");
            if let Some(range) = range
                && !state.settings.ignore_range
            {
                let Some((from, to)) = range.strip_prefix("bytes=").and_then(|r| r.split_once('-'))
                else {
                    return Outgoing::error(400, "InvalidArgument", "range");
                };
                let from: usize = from.parse().unwrap_or(usize::MAX);
                if total == 0 || from >= total {
                    return Outgoing::error(416, "InvalidRange", "the range is not satisfiable");
                }
                let to = to.parse::<usize>().map_or(total - 1, |t| t.min(total - 1));
                out.status = 206;
                out.headers
                    .push(("content-range".into(), format!("bytes {from}-{to}/{total}")));
                out.body = found.bytes[from..=to].to_vec();
            } else {
                out.body = found.bytes.clone();
            }
            out
        }
        _ => Outgoing::error(400, "InvalidRequest", "unsupported"),
    }
}

fn complete(
    state: &mut State,
    request: &Incoming,
    bucket: &str,
    key: &str,
    query: &str,
) -> Outgoing {
    let id = query_value(query, "uploadId").unwrap_or_default();
    if state.settings.complete_with_error {
        state.uploads.remove(&id);
        let mut out = Outgoing::error(200, "InternalError", "assembly failed");
        if state.settings.complete_leading_whitespace {
            out.body = [b"  \n".as_slice(), &out.body].concat();
        }
        return out;
    }
    let Some(upload) = state.uploads.get(&id) else {
        return Outgoing::error(404, "NoSuchUpload", "no upload");
    };
    if upload.bucket != bucket || upload.key != key {
        return Outgoing::error(404, "NoSuchUpload", "wrong target");
    }
    let body = String::from_utf8_lossy(&request.body).into_owned();
    let mut listed: Vec<(u32, String)> = Vec::new();
    for piece in body.split("<Part>").skip(1) {
        let number = piece
            .split("<PartNumber>")
            .nth(1)
            .and_then(|s| s.split("</PartNumber>").next())
            .and_then(|s| s.parse().ok());
        let etag = piece
            .split("<ETag>")
            .nth(1)
            .and_then(|s| s.split("</ETag>").next())
            .map(|s| s.replace("&quot;", "\""));
        match (number, etag) {
            (Some(n), Some(e)) => listed.push((n, e)),
            _ => return Outgoing::error(400, "MalformedXML", "part"),
        }
    }
    if listed.is_empty() || listed.windows(2).any(|w| w[0].0 >= w[1].0) {
        return Outgoing::error(400, "InvalidPartOrder", "ascending");
    }
    let mut assembled = Vec::new();
    let mut sizes = Vec::new();
    for (at, (number, etag)) in listed.iter().enumerate() {
        let Some((bytes, stored)) = upload.parts.get(number) else {
            return Outgoing::error(400, "InvalidPart", "unknown part");
        };
        if stored != etag {
            return Outgoing::error(400, "InvalidPart", "etag");
        }
        if at + 1 < listed.len() && bytes.len() < MIN_PART {
            return Outgoing::error(400, "EntityTooSmall", "part too small");
        }
        sizes.push(bytes.len());
        assembled.extend_from_slice(bytes);
    }
    let etag = etag_of(&assembled);
    state.uploads.remove(&id);
    state.completed_part_sizes.push(sizes);
    state.buckets.get_mut(bucket).unwrap().insert(
        key.to_owned(),
        Object {
            bytes: assembled,
            content_type: String::new(),
        },
    );
    let mut out = Outgoing::xml(
        200,
        format!(
            "<CompleteMultipartUploadResult><Location>x</Location><Bucket>{bucket}</Bucket><Key>k</Key><ETag>{}</ETag></CompleteMultipartUploadResult>",
            etag.replace('"', "&quot;")
        ),
    );
    if state.settings.complete_leading_whitespace {
        out.body = [b"\n \n".as_slice(), &out.body].concat();
    }
    out
}
