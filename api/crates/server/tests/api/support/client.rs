//! A small HTTP client on hyper, with a cookie jar.
//!
//! Each request opens a connection of its own and closes it, which keeps the
//! tests free of anything a pool would hide.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;

/// What the web client sends to say what it is.
pub const BROWSER: [(&str, &str); 2] = [
    ("X-Stellarr-Client", "web"),
    ("X-Stellarr-Client-Version", "2.0.0"),
];

type RequestBody = BoxBody<Bytes, Infallible>;

pub struct Client {
    address: SocketAddr,
    headers: Vec<(String, String)>,
    jar: Mutex<Vec<(String, String)>>,
}

/// An answer, read whole.
#[derive(Debug)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("not JSON ({error}): {}", self.text()))
    }

    /// The value of a header, or none when it is not there.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// A request that is not the usual one: another method, headers or body.
pub struct Request {
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: RequestBody,
}

impl Request {
    pub fn new(method: Method, path: &str) -> Request {
        Request {
            method,
            path: path.to_string(),
            headers: Vec::new(),
            body: empty(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn json(mut self, value: &serde_json::Value) -> Request {
        self.headers
            .push(("Content-Type".to_string(), "application/json".to_string()));
        self.body = Full::new(Bytes::from(value.to_string()))
            .map_err(|never| match never {})
            .boxed();
        self
    }

    /// A body that does not say what it is.
    pub fn untyped_body(mut self, body: &str) -> Request {
        self.body = Full::new(Bytes::from(body.to_string()))
            .map_err(|never| match never {})
            .boxed();
        self
    }

    pub fn body(mut self, content_type: &str, body: &str) -> Request {
        self.headers
            .push(("Content-Type".to_string(), content_type.to_string()));
        self.body = Full::new(Bytes::from(body.to_string()))
            .map_err(|never| match never {})
            .boxed();
        self
    }
}

fn empty() -> RequestBody {
    Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

impl Client {
    /// A client that says it is the browser build.
    pub fn browser(address: SocketAddr) -> Client {
        let mut client = Client::anonymous(address);
        for (name, value) in BROWSER {
            client = client.with_header(name, value);
        }
        client
    }

    /// A client that sends nothing to say what it is.
    pub fn anonymous(address: SocketAddr) -> Client {
        Client {
            address,
            headers: Vec::new(),
            jar: Mutex::new(Vec::new()),
        }
    }

    /// Sends this header with every request from now on, in place of one of
    /// the same name.
    pub fn with_header(mut self, name: &str, value: &str) -> Client {
        self.headers
            .retain(|(have, _)| !have.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn without_header(mut self, name: &str) -> Client {
        self.headers
            .retain(|(have, _)| !have.eq_ignore_ascii_case(name));
        self
    }

    /// The value of a cookie the server has set and not taken away.
    pub fn cookie(&self, name: &str) -> Option<String> {
        let jar = self.jar.lock().unwrap();
        jar.iter()
            .find(|(have, _)| have == name)
            .map(|(_, value)| value.clone())
    }

    pub async fn get(&self, path: &str) -> Response {
        self.send(Request::new(Method::GET, path)).await
    }

    pub async fn post_json(&self, path: &str, body: &serde_json::Value) -> Response {
        self.send(Request::new(Method::POST, path).json(body)).await
    }

    pub async fn send(&self, request: Request) -> Response {
        self.try_send(request)
            .await
            .expect("the request got an answer")
    }

    /// Like [`Client::send`], but says so when the connection broke instead.
    pub async fn try_send(&self, request: Request) -> Result<Response, String> {
        tokio::time::timeout(Duration::from_secs(60), self.exchange(request))
            .await
            .map_err(|_| "no answer within a minute".to_string())?
    }

    async fn exchange(&self, request: Request) -> Result<Response, String> {
        let stream = TcpStream::connect(self.address)
            .await
            .map_err(|e| e.to_string())?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let mut builder = http::Request::builder()
            .method(request.method)
            .uri(&request.path)
            .header("Host", self.address.to_string());
        for (name, value) in self.headers.iter().chain(&request.headers) {
            builder = builder.header(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        let cookies = self
            .jar
            .lock()
            .unwrap()
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        if !cookies.is_empty() {
            builder = builder.header("Cookie", cookies);
        }
        let response = sender
            .send_request(builder.body(request.body).unwrap())
            .await
            .map_err(|e| e.to_string())?;
        let (parts, body) = response.into_parts();
        let body = body.collect().await.map_err(|e| e.to_string())?.to_bytes();
        self.remember_cookies(&parts.headers);
        Ok(Response {
            status: parts.status,
            headers: parts.headers,
            body,
        })
    }

    /// Sends a `POST` whose body is `sent` zero bytes, written by hand over a
    /// socket, with its length `declared` or sent in chunks, and reads
    /// whatever the server answers. A server that answers before it has read
    /// the body and closes the connection is the point of the tests that use
    /// it, and a client library would call that an error. Sending less than
    /// was declared lets the server refuse on the declaration alone, with
    /// nothing unread when it closes, so that its answer is not lost to a reset.
    pub async fn post_zeros(&self, path: &str, declared: Option<u64>, sent: u64) -> Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut head = format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
            self.address
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&match declared {
            Some(length) => format!("Content-Length: {length}\r\n\r\n"),
            None => "Transfer-Encoding: chunked\r\n\r\n".to_string(),
        });
        let stream = TcpStream::connect(self.address)
            .await
            .expect("a connection");
        let (mut reader, mut writer) = stream.into_split();
        let sending = tokio::spawn(async move {
            // The server may stop listening at any point; that is not a failure here.
            if writer.write_all(head.as_bytes()).await.is_err() {
                return writer;
            }
            let block = vec![0u8; 64 * 1024];
            let mut left = sent;
            while left > 0 {
                // The last byte goes alone, so that the server has read all
                // there is when it finds the body too long, and closes
                // without a reset that would take its answer with it.
                let size = if left == 1 {
                    1
                } else {
                    (left - 1).min(block.len() as u64) as usize
                };
                let written = if declared.is_some() {
                    writer.write_all(&block[..size]).await
                } else {
                    let framed =
                        [format!("{size:x}\r\n").as_bytes(), &block[..size], b"\r\n"].concat();
                    writer.write_all(&framed).await
                };
                if written.is_err() {
                    return writer;
                }
                left -= size as u64;
            }
            if declared.is_none() {
                let _ = writer.write_all(b"0\r\n\r\n").await;
            }
            // Kept open: a server takes a closed write side for a client that
            // has gone, and drops its answer.
            writer
        });
        let mut raw = Vec::new();
        // A reset after the answer is part of a refusal that closes early.
        let _ = tokio::time::timeout(Duration::from_secs(60), reader.read_to_end(&mut raw)).await;
        let _kept_open = sending;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or_else(|| {
                panic!("no status line in {path} (declared {declared:?}, sent {sent}): {head:?}")
            });
        let mut headers = HeaderMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.append(
                    HeaderName::from_bytes(name.trim().to_ascii_lowercase().as_bytes()).unwrap(),
                    HeaderValue::from_str(value.trim()).unwrap(),
                );
            }
        }
        Response {
            status: StatusCode::from_u16(status).unwrap(),
            headers,
            body: Bytes::from(body.to_string()),
        }
    }

    /// Sends a `GET` and closes the connection without waiting for the answer.
    pub async fn get_and_hang_up(&self, path: &str) {
        use tokio::io::AsyncWriteExt;

        let mut head = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.address
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let mut stream = TcpStream::connect(self.address)
            .await
            .expect("a connection");
        stream
            .write_all(head.as_bytes())
            .await
            .expect("the request is sent");
        drop(stream);
    }

    fn remember_cookies(&self, headers: &HeaderMap) {
        let mut jar = self.jar.lock().unwrap();
        for value in headers.get_all("set-cookie") {
            let Ok(text) = value.to_str() else { continue };
            let mut parts = text.split(';');
            let Some((name, value)) = parts.next().and_then(|pair| pair.split_once('=')) else {
                continue;
            };
            let gone = value.is_empty()
                || parts.any(|attribute| attribute.trim().eq_ignore_ascii_case("max-age=0"));
            jar.retain(|(have, _)| have != name.trim());
            if !gone {
                jar.push((name.trim().to_string(), value.trim().to_string()));
            }
        }
    }
}
