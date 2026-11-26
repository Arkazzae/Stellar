//! A cookie-authenticated client that speaks the native hub protocol over TCP.

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Value, json};
use stellarr_websocket::frame::{OP_CLOSE, OP_PING, OP_PONG, OP_TEXT, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// How long a test waits for the server before it calls it silent.
const PATIENCE: Duration = Duration::from_secs(10);

/// The head of the answer to an upgrade request.
pub struct Upgrade {
    pub status: u16,
    pub head: String,
}

pub struct HubClient {
    stream: TcpStream,
    records: VecDeque<Value>,
    next_invocation: u32,
    closed: Option<u16>,
}

/// The upgrade request to `/hubs/app` with a query string and a cookie, and
/// the answer's head. The stream is the connection, upgraded when the status
/// is 101.
pub async fn upgrade(
    address: std::net::SocketAddr,
    query: &str,
    cookie: Option<&str>,
) -> (Upgrade, TcpStream) {
    let host = address.to_string();
    let origin = format!("http://{host}");
    upgrade_with(
        address,
        query,
        cookie,
        &host,
        &[
            ("Origin", &origin),
            ("Sec-WebSocket-Protocol", "stellarr.v1"),
        ],
    )
    .await
}

pub async fn upgrade_with(
    address: std::net::SocketAddr,
    query: &str,
    cookie: Option<&str>,
    host: &str,
    headers: &[(&str, &str)],
) -> (Upgrade, TcpStream) {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("the server listens");
    let mut request = format!(
        "GET /hubs/app{query} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let byte = timeout(PATIENCE, stream.read_u8())
            .await
            .expect("the server answers")
            .expect("the server answers");
        head.push(byte);
    }
    let head = String::from_utf8(head).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (Upgrade { status, head }, stream)
}

impl HubClient {
    pub async fn connect(address: std::net::SocketAddr, cookie: &str) -> HubClient {
        HubClient::connect_to(address, "", cookie).await
    }

    pub async fn connect_to(address: std::net::SocketAddr, query: &str, cookie: &str) -> HubClient {
        Self::from_upgrade(upgrade(address, query, Some(cookie)).await).await
    }

    pub async fn from_upgrade((answer, stream): (Upgrade, TcpStream)) -> HubClient {
        assert_eq!(answer.status, 101, "{}", answer.head);
        assert!(
            answer
                .head
                .to_ascii_lowercase()
                .contains("\r\nsec-websocket-protocol: stellarr.v1\r\n"),
            "{}",
            answer.head
        );
        let mut client = HubClient {
            stream,
            records: VecDeque::new(),
            next_invocation: 0,
            closed: None,
        };
        assert_eq!(client.record().await, json!({"type": "ready"}));
        client
    }

    async fn send_frame(&mut self, opcode: u8, payload: &[u8]) {
        let frame = encode_frame(true, opcode, payload, Some([7, 3, 9, 1]));
        self.stream.write_all(&frame).await.unwrap();
    }

    pub async fn send_text(&mut self, text: &str) {
        self.send_frame(OP_TEXT, text.as_bytes()).await;
    }

    /// One frame of the server's: its opcode and payload. `None` once the
    /// server has closed the socket.
    async fn frame(&mut self, wait: Duration) -> Option<Option<(u8, Vec<u8>)>> {
        let mut head = [0u8; 2];
        match timeout(wait, self.stream.read_exact(&mut head)).await {
            Err(_) => return None,
            Ok(Err(_)) => return Some(None),
            Ok(Ok(_)) => {}
        }
        let len = match head[1] & 0x7f {
            126 => {
                let mut bytes = [0u8; 2];
                self.stream.read_exact(&mut bytes).await.ok()?;
                usize::from(u16::from_be_bytes(bytes))
            }
            127 => {
                let mut bytes = [0u8; 8];
                self.stream.read_exact(&mut bytes).await.ok()?;
                u64::from_be_bytes(bytes) as usize
            }
            short => usize::from(short),
        };
        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload).await.ok()?;
        Some(Some((head[0] & 0x0f, payload)))
    }

    async fn receive_close(&mut self, payload: &[u8]) {
        self.closed = payload
            .get(..2)
            .map(|code| u16::from_be_bytes([code[0], code[1]]));
        self.send_frame(OP_CLOSE, payload).await;
    }

    /// The next text message other than a heartbeat.
    pub async fn raw_record(&mut self) -> String {
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            match self
                .frame(deadline.saturating_duration_since(tokio::time::Instant::now()))
                .await
            {
                Some(Some((OP_TEXT, payload))) if payload == br#"{"type":"ping"}"# => {}
                Some(Some((OP_TEXT, payload))) => return String::from_utf8(payload).unwrap(),
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                other => panic!("expected a native text message, got {other:?}"),
            }
        }
    }

    pub async fn record(&mut self) -> Value {
        match self.records.pop_front() {
            Some(record) => record,
            None => serde_json::from_str(&self.raw_record().await).unwrap(),
        }
    }

    pub async fn event(&mut self, name: &str) -> Value {
        let record = self.record().await;
        assert_eq!(record["type"], "event", "{record}");
        assert_eq!(record["name"], name, "{record}");
        record["data"].clone()
    }

    /// Collects events until the socket is quiet for the requested window.
    pub async fn quiet_for(&mut self, wait: Duration) -> Vec<Value> {
        let mut seen: Vec<Value> = self.records.drain(..).collect();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return seen;
            }
            match self.frame(left).await {
                None | Some(None) => return seen,
                Some(Some((OP_TEXT, payload))) => {
                    let record: Value = serde_json::from_slice(&payload).unwrap();
                    if record["type"] != "ping" {
                        seen.push(record);
                    }
                }
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                Some(Some((OP_CLOSE, payload))) => {
                    self.receive_close(&payload).await;
                    return seen;
                }
                Some(Some(_)) => {}
            }
        }
    }

    /// Calls a method and keeps any events that arrive before its answer.
    pub async fn invoke(&mut self, method: &str, args: Value) -> Value {
        self.next_invocation += 1;
        let id = self.next_invocation;
        self.send_text(
            &json!({"type": "call", "id": id, "method": method, "args": args}).to_string(),
        )
        .await;
        loop {
            let answer: Value = serde_json::from_str(&self.raw_record().await).unwrap();
            if answer["id"] == id {
                assert!(
                    matches!(answer["type"].as_str(), Some("result" | "error")),
                    "{answer}"
                );
                return answer;
            }
            self.records.push_back(answer);
        }
    }

    /// Waits for and acknowledges the WebSocket close.
    pub async fn finish_close(&mut self) -> Option<u16> {
        if self.closed.is_some() {
            return self.closed;
        }
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            match self
                .frame(deadline.saturating_duration_since(tokio::time::Instant::now()))
                .await
            {
                None => panic!("the server did not finish closing"),
                Some(None) => return self.closed,
                Some(Some((OP_CLOSE, payload))) => {
                    self.receive_close(&payload).await;
                    return self.closed;
                }
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                Some(Some(_)) => {}
            }
        }
    }

    pub async fn hang_up(mut self) {
        self.send_frame(OP_CLOSE, &1000u16.to_be_bytes()).await;
        let _ = timeout(Duration::from_secs(5), self.finish_close()).await;
    }
}
