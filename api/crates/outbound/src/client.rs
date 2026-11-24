//! The client: one request, its redirects, and its answer.
//!
//! # One connection per request
//!
//! Every request opens its own connection and closes it with the answer
//! (`Connection: close`). A pool was weighed and left out:
//!
//! - The busiest user is the long poll to the media server on loopback,
//!   once every 25 seconds. A loopback connect costs microseconds; a pooled
//!   connection would sit idle for 25 seconds, long enough for the other end
//!   to have closed it, and the first write then fails on a connection that
//!   looked healthy. Retrying that is safe for a GET and not for a POST.
//! - The address check belongs to the connection. With none shared, a
//!   connection judged for one request, with one set of credentials, can
//!   never carry another's.
//! - With one request per connection the size of the answer is exactly what
//!   came over that socket, and abandoning a body is closing the socket.
//! - The cost is a TLS handshake per request to the few hosts that are
//!   called often (the mail provider, the chat import). rustls resumes
//!   sessions from a cache that lives as long as the configuration, so the
//!   repeat handshakes are short. If a profile shows otherwise, a pool keyed
//!   by scheme, host, port and policy fits behind this same interface.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http::header::{
    ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_ENCODING, CONTENT_LANGUAGE, CONTENT_LENGTH,
    CONTENT_TYPE, COOKIE, EXPECT, HOST, LOCATION, PROXY_AUTHORIZATION, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE,
};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::time::Instant;

use crate::address::AddressPolicy;
use crate::body::{Body, DriverGuard, Outgoing, ResponseBody};
use crate::connect::{self, Resolve, SystemResolver};
use crate::encoding::{self, Decoder};
use crate::error::{Error, Result};
use crate::tls;
use crate::url::Url;

/// The smallest read buffer hyper accepts; it also bounds the size of the
/// head of an answer.
const MIN_HEAD_BUFFER: usize = 8 * 1024;

/// The largest head of an answer read, whatever the ceiling: 100 headers of
/// the sizes servers send fit in a fraction of this.
const MAX_HEAD_BUFFER: usize = 256 * 1024;

/// Headers that are the client's to set. One of these from a caller could
/// change how the message is framed or where it goes.
const FORBIDDEN: &[HeaderName] = &[
    HOST,
    TRANSFER_ENCODING,
    CONNECTION,
    UPGRADE,
    EXPECT,
    TE,
    TRAILER,
    ACCEPT_ENCODING,
];

/// One request: what to send, where, and the limits to hold the other end
/// to. [`Request::new`] starts from the strictest settings that make
/// sense: public addresses only, no redirects, ten seconds to connect,
/// thirty in all, a one mebibyte ceiling and compressed answers decoded.
pub struct Request {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Body,
    connect_timeout: Duration,
    timeout: Duration,
    max_response_bytes: u64,
    max_redirects: u32,
    policy: AddressPolicy,
    decompress: bool,
}

impl Request {
    pub fn new(method: Method, url: &str) -> Result<Request> {
        Ok(Request {
            method,
            url: Url::parse(url)?,
            headers: HeaderMap::new(),
            body: Body::Empty,
            connect_timeout: Duration::from_secs(10),
            timeout: Duration::from_secs(30),
            max_response_bytes: 1024 * 1024,
            max_redirects: 0,
            policy: AddressPolicy::PublicOnly,
            decompress: true,
        })
    }

    pub fn get(url: &str) -> Result<Request> {
        Request::new(Method::GET, url)
    }

    pub fn post(url: &str) -> Result<Request> {
        Request::new(Method::POST, url)
    }

    /// Adds a header. A value that carries a key or a token is marked
    /// before it is given: build it with [`HeaderValue::from_str`] (or
    /// `from_bytes`) and call `set_sensitive(true)` on it. A redirect to
    /// another origin then does not forward it, and `Debug` output never
    /// shows it. `Authorization` and `Proxy-Authorization` are not forwarded
    /// by any redirect, marked or not.
    ///
    /// The headers the client sets itself are refused when the request is
    /// sent, with [`Error::ForbiddenHeader`]. The one exception is a single
    /// `Content-Length` equal to the length of the body of this request.
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Request {
        self.headers.append(name, value);
        self
    }

    pub fn body(mut self, body: Body) -> Request {
        self.body = body;
        self
    }

    /// How long resolving, connecting and the TLS handshake may take, for
    /// each connection, so for each hop of a redirect chain.
    pub fn connect_timeout(mut self, timeout: Duration) -> Request {
        self.connect_timeout = timeout;
        self
    }

    /// How long the whole request may take: every hop, the body that is
    /// sent and the body that is read.
    pub fn timeout(mut self, timeout: Duration) -> Request {
        self.timeout = timeout;
        self
    }

    /// The ceiling on the size of the answer, which is the most memory it may
    /// take: the head and the body as the caller receives it, after any
    /// decoding, may not pass it, and the body as it came over the wire is
    /// held to the same number. An answer that is not compressed is counted
    /// once. One that is compressed is stopped the moment what it decodes to
    /// passes the ceiling, however small it was on the wire.
    pub fn max_response_bytes(mut self, bytes: u64) -> Request {
        self.max_response_bytes = bytes;
        self
    }

    /// How many redirects are followed. Zero returns a redirect as it is.
    pub fn max_redirects(mut self, redirects: u32) -> Request {
        self.max_redirects = redirects;
        self
    }

    pub fn policy(mut self, policy: AddressPolicy) -> Request {
        self.policy = policy;
        self
    }

    /// Whether the request offers `gzip` and `deflate` and decodes what
    /// comes back. On by default: pages, JSON and other text are what this
    /// client mostly reads, and hosts that compress send much less of it.
    /// Turn it off for a body whose bytes matter as they are on the wire: a
    /// download that is stored or checked against a size or a digest. The
    /// request then says `identity`, the body is handed over exactly as it
    /// arrives, and the answer's `Content-Encoding` and `Content-Length` are
    /// left as the server sent them, so a caller can see a host that
    /// compressed anyway.
    ///
    /// Decoded, an answer loses those two headers (they describe the wire),
    /// and a coding that is not `gzip`, `deflate` or `identity` makes reading
    /// the body fail.
    pub fn decompress(mut self, decompress: bool) -> Request {
        self.decompress = decompress;
        self
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Refuses the headers that are the client's to set. A `Content-Length`
    /// is the one exception, when there is exactly one and it states the
    /// length of the body this request carries: a caller that signs it can
    /// hand it back. It is then taken out, since the client frames the body
    /// itself and says the same number, so it is never sent twice.
    fn check_headers(&mut self) -> Result<()> {
        if let Some(name) = self.headers.keys().find(|name| FORBIDDEN.contains(name)) {
            return Err(Error::ForbiddenHeader(name.to_string()));
        }
        let mut lengths = self.headers.get_all(CONTENT_LENGTH).iter();
        if let Some(value) = lengths.next() {
            let states = value
                .to_str()
                .ok()
                .map(str::trim)
                .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|text| text.parse::<u64>().ok());
            if lengths.next().is_some() || states != Some(self.body.len()) {
                return Err(Error::ForbiddenHeader(CONTENT_LENGTH.to_string()));
            }
            self.headers.remove(CONTENT_LENGTH);
        }
        Ok(())
    }
}

/// Shows the method and the origin. Not the path, not the query and not a
/// header value.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// An answer whose body is yet to be read.
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    /// Where the answer came from: the last URL of the redirect chain. A
    /// relative address in the body resolves against this one.
    pub url: Url,
    pub body: ResponseBody,
}

impl Response {
    /// Reads the whole body under the ceiling.
    pub async fn collect(self) -> Result<Collected> {
        Ok(Collected {
            status: self.status,
            headers: self.headers,
            url: self.url,
            body: self.body.collect().await?,
        })
    }
}

/// An answer with its body read.
pub struct Collected {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub url: Url,
    pub body: Bytes,
}

/// Shows the status, the names of the headers and the size of the body, and
/// no value: an answer can hold tokens and personal data.
impl std::fmt::Debug for Collected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collected")
            .field("status", &self.status)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("url", &self.url)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Where the trusted roots come from.
#[derive(Clone)]
enum Trust {
    /// The caller's own configuration.
    Fixed(Arc<rustls::ClientConfig>),
    /// The system's bundle, read when the first `https` request needs it.
    System(Arc<Mutex<Option<Arc<rustls::ClientConfig>>>>),
}

struct Inner {
    trust: Trust,
    resolver: Arc<dyn Resolve>,
}

/// The HTTP client. Cheap to clone; keep one for the process.
///
/// No cookies are kept, no proxy is read from the environment, and
/// nothing is logged that came from a URL's path or query or from a header.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Default for Client {
    fn default() -> Client {
        Client::new()
    }
}

impl Client {
    /// A client that trusts the system's certificate bundle. The file is
    /// read when the first `https` request needs it, so a process that only
    /// calls loopback does not need one.
    pub fn new() -> Client {
        Client::from_trust(Trust::System(Arc::new(Mutex::new(None))))
    }

    /// A client with a configuration of the caller's own. It should offer
    /// `http/1.1` only, which [`crate::tls::client_config`] does.
    pub fn with_tls_config(config: Arc<rustls::ClientConfig>) -> Client {
        Client::from_trust(Trust::Fixed(config))
    }

    /// Replaces the resolver, for a test that must control what a name
    /// resolves to.
    pub fn with_resolver(self, resolver: Arc<dyn Resolve>) -> Client {
        Client {
            inner: Arc::new(Inner {
                trust: self.inner.trust.clone(),
                resolver,
            }),
        }
    }

    fn from_trust(trust: Trust) -> Client {
        Client {
            inner: Arc::new(Inner {
                trust,
                resolver: Arc::new(SystemResolver),
            }),
        }
    }

    /// Reads the system bundle now, so that a missing one is known at start
    /// and not at the first request. Does nothing for a client with its own
    /// configuration.
    pub async fn load_roots(&self) -> Result<()> {
        self.tls_config().await.map(|_| ())
    }

    async fn tls_config(&self) -> Result<Arc<rustls::ClientConfig>> {
        match &self.inner.trust {
            Trust::Fixed(config) => Ok(config.clone()),
            Trust::System(loaded) => {
                if let Some(config) = loaded.lock().unwrap().clone() {
                    return Ok(config);
                }
                // A file of a few hundred kilobytes; off the executor anyway.
                let roots = tokio::task::spawn_blocking(tls::system_roots)
                    .await
                    .map_err(|_| Error::Roots("reading the bundle was interrupted".into()))??;
                let config = tls::client_config(roots);
                *loaded.lock().unwrap() = Some(config.clone());
                Ok(config)
            }
        }
    }

    /// Sends a request and returns its answer once the head has arrived.
    /// The body is read from the [`Response`]. The request's total timeout
    /// keeps running while it is.
    pub async fn send(&self, mut request: Request) -> Result<Response> {
        request.check_headers()?;
        // A timeout too large to add is no timeout at all.
        let now = Instant::now();
        let deadline = now
            .checked_add(request.timeout)
            .unwrap_or_else(|| now + Duration::from_secs(365 * 24 * 3600));
        match tokio::time::timeout_at(deadline, self.follow(request, deadline)).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Sends a request and reads the whole answer under the ceiling and the
    /// timeout.
    pub async fn send_collected(&self, request: Request) -> Result<Collected> {
        self.send(request).await?.collect().await
    }

    /// The redirect loop.
    async fn follow(&self, request: Request, deadline: Instant) -> Result<Response> {
        let Request {
            mut method,
            mut url,
            mut headers,
            mut body,
            connect_timeout,
            max_response_bytes,
            max_redirects,
            policy,
            decompress,
            ..
        } = request;
        let mut hops = 0u32;
        loop {
            // A stream goes out once; a body held in memory as often as needed.
            let mut spent = false;
            let sending = match body.replay() {
                Some(copy) => copy,
                None => {
                    spent = true;
                    std::mem::replace(&mut body, Body::Empty)
                }
            };
            let response = self
                .exchange(
                    &method,
                    &url,
                    &headers,
                    sending,
                    policy,
                    connect_timeout,
                    max_response_bytes,
                    decompress,
                    deadline,
                )
                .await?;
            tracing::debug!(
                host = url.bare_host(),
                hop = hops,
                status = response.status.as_u16(),
                "answer received"
            );
            let Some(location) = redirect_target(&response, max_redirects) else {
                return Ok(Response { url, ..response });
            };
            if hops >= max_redirects {
                return Err(Error::TooManyRedirects);
            }
            let next = url.join(&location)?;
            if url.is_https() && !next.is_https() {
                return Err(Error::RedirectRefused("it leaves https for http"));
            }
            let status = response.status;
            drop(response);

            // What the status says about the method, as browsers and the
            // HTTP specification have it: a POST that is answered 300, 301,
            // 302 or 303 is asked again as a GET, and any other method but
            // GET and HEAD too after a 303. 307 and 308 repeat the request.
            let becomes_get = match status.as_u16() {
                300..=302 => method == Method::POST,
                303 => method != Method::GET && method != Method::HEAD,
                _ => false,
            };
            if becomes_get {
                method = Method::GET;
                body = Body::Empty;
                for name in [CONTENT_TYPE, CONTENT_ENCODING, CONTENT_LANGUAGE] {
                    headers.remove(name);
                }
            } else {
                if spent {
                    return Err(Error::BodyNotReplayable);
                }
            }
            strip_credentials(&mut headers, !url.same_origin(&next));
            url = next;
            hops += 1;
        }
    }

    /// One request on one new connection.
    #[allow(clippy::too_many_arguments)]
    async fn exchange(
        &self,
        method: &Method,
        url: &Url,
        headers: &HeaderMap,
        body: Body,
        policy: AddressPolicy,
        connect_timeout: Duration,
        ceiling: u64,
        decompress: bool,
        deadline: Instant,
    ) -> Result<Response> {
        let connector = if url.is_https() {
            Some(connect::connector(self.tls_config().await?))
        } else {
            None
        };
        let left = deadline.saturating_duration_since(Instant::now());
        let stream = connect::open(
            url,
            policy,
            self.inner.resolver.as_ref(),
            connector.as_ref(),
            connect_timeout.min(left),
        )
        .await
        .map_err(|error| match error {
            // The request's own time ran out, not the connect limit.
            Error::ConnectTimeout { .. } if left <= connect_timeout => Error::Timeout,
            other => other,
        })?;

        let head_room = usize::try_from(ceiling)
            .unwrap_or(usize::MAX)
            .clamp(MIN_HEAD_BUFFER, MAX_HEAD_BUFFER);
        let (mut sender, connection) = http1::Builder::new()
            .max_buf_size(head_room)
            .handshake::<_, Outgoing>(TokioIo::new(stream))
            .await
            .map_err(|error| Error::Protocol(crate::body::describe(&error)))?;
        let driver = DriverGuard(tokio::spawn(async move {
            // An error here shows up as the error of the request itself.
            let _ = connection.await;
        }));

        // A POST or PUT with nothing in it still says so: some servers
        // answer 411 to one that does not.
        let says_zero =
            body.is_empty() && matches!(*method, Method::POST | Method::PUT | Method::PATCH);
        let mut request = http::Request::builder()
            .method(method.clone())
            .uri(url.path_and_query())
            .body(Outgoing::new(body))
            .map_err(|_| Error::InvalidUrl("it cannot be sent"))?;
        let headers_out = request.headers_mut();
        for (name, value) in headers {
            headers_out.append(name.clone(), value.clone());
        }
        headers_out.insert(
            HOST,
            HeaderValue::from_str(&url.host_header())
                .map_err(|_| Error::InvalidUrl("the host cannot be a header"))?,
        );
        if says_zero {
            headers_out.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
        }
        headers_out.insert(CONNECTION, HeaderValue::from_static("close"));
        headers_out.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static(if decompress {
                encoding::ACCEPT_DECODED
            } else {
                encoding::ACCEPT_RAW
            }),
        );

        let response = sender.send_request(request).await.map_err(|error| {
            if error.to_string().contains("too large") {
                Error::TooLarge
            } else if error.is_user() {
                Error::RequestBody(crate::body::describe(&error))
            } else {
                Error::Protocol(crate::body::describe(&error))
            }
        })?;
        let (parts, incoming): (_, Incoming) = response.into_parts();

        let head_bytes = head_size(&parts.status, &parts.headers);
        if head_bytes > ceiling {
            return Err(Error::TooLarge);
        }
        // A body that says up front it does not fit is not read at all.
        if let Some(length) = incoming_length(&incoming)
            && head_bytes.saturating_add(length) > ceiling
        {
            return Err(Error::TooLarge);
        }
        let mut headers = parts.headers;
        let decoder = Decoder::for_response(
            method,
            parts.status,
            &mut headers,
            decompress,
            ceiling - head_bytes,
        );
        let body = ResponseBody::new(incoming, decoder, head_bytes, ceiling, deadline, driver);
        Ok(Response {
            status: parts.status,
            headers,
            url: url.clone(),
            body,
        })
    }
}

fn incoming_length(incoming: &Incoming) -> Option<u64> {
    use hyper::body::Body as _;
    incoming.size_hint().exact()
}

/// The `Location` to follow, when the answer is a redirect that is to be
/// followed. A redirect without one is an answer like any other.
fn redirect_target(response: &Response, max_redirects: u32) -> Option<String> {
    if max_redirects == 0 || !matches!(response.status.as_u16(), 300 | 301 | 302 | 303 | 307 | 308)
    {
        return None;
    }
    response
        .headers
        .get(LOCATION)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .map(str::to_owned)
}

/// What a redirect does not carry along. `Authorization` and
/// `Proxy-Authorization` never go with a redirect, to the same origin or to
/// another: a caller that needs them again asks again, knowing where it
/// asks. To another origin neither do cookies, nor any header the caller
/// marked as sensitive, which is how a key in a custom header is kept home.
fn strip_credentials(headers: &mut HeaderMap, other_origin: bool) {
    for name in [AUTHORIZATION, PROXY_AUTHORIZATION] {
        headers.remove(name);
    }
    if !other_origin {
        return;
    }
    headers.remove(COOKIE);
    let sensitive: Vec<HeaderName> = headers
        .iter()
        .filter(|(_, value)| value.is_sensitive())
        .map(|(name, _)| name.clone())
        .collect();
    for name in sensitive {
        headers.remove(name);
    }
}

/// The size of a head on the wire, near enough: the status line, each
/// header with its separators, and the empty line.
fn head_size(status: &StatusCode, headers: &HeaderMap) -> u64 {
    let status_line = 9 + 3 + 1 + status.canonical_reason().map_or(0, str::len) as u64 + 2;
    let fields: u64 = headers
        .iter()
        .map(|(name, value)| (name.as_str().len() + 2 + value.len() + 2) as u64)
        .sum();
    status_line + fields + 2
}
