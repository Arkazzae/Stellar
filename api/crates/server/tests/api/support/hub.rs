//! A client of the hub that plays the browser's part over a raw socket:
//! the WebSocket upgrade with the session cookie, the hub protocol's
//! handshake, calls with answers, and the records the server sends.

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Value, json};
use stellarr_websocket::frame::{OP_CLOSE, OP_PING, OP_PONG, OP_TEXT, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// The byte that ends a record of the hub protocol.
const RS: char = '\u{1e}';

/// How long a test waits for the server before it calls it silent.
const PATIENCE: Duration = Duration::from_secs(10);

/// The head of the answer to an upgrade request.
pub struct Upgrade {
    pub status: u16,
    pub head: String,
}

pub struct HubClient {
    stream: TcpStream,
    text: String,
    records: VecDeque<Value>,
    next_invocation: u32,
    pub closed: Option<u16>,
}

/// The upgrade request to `/hubs/app` with a query string and a cookie, and
/// the answer's head. The stream is the connection, upgraded when the status
/// is 101.
pub async fn upgrade(
    address: std::net::SocketAddr,
    query: &str,
    cookie: Option<&str>,
) -> (Upgrade, TcpStream) {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("the server listens");
    let mut request = format!(
        "GET /hubs/app{query} HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
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
    /// A connection that has been admitted and has done the protocol's
    /// handshake, signed in with `cookie` (`name=value`).
    pub async fn connect(address: std::net::SocketAddr, cookie: &str) -> HubClient {
        HubClient::connect_to(address, "", cookie).await
    }

    /// The same, with a query string (`?serverId=...`).
    pub async fn connect_to(address: std::net::SocketAddr, query: &str, cookie: &str) -> HubClient {
        let (answer, stream) = upgrade(address, query, Some(cookie)).await;
        assert_eq!(answer.status, 101, "{}", answer.head);
        let mut client = HubClient {
            stream,
            text: String::new(),
            records: VecDeque::new(),
            next_invocation: 0,
            closed: None,
        };
        client
            .send_text(&format!("{{\"protocol\":\"json\",\"version\":1}}{RS}"))
            .await;
        assert_eq!(client.next_text().await, "{}");
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

    /// The next record as text: transport pings are answered as a browser
    /// would, and the hub's own pings are skipped.
    async fn next_text(&mut self) -> String {
        loop {
            if let Some(end) = self.text.find(RS) {
                let record: String = self.text.drain(..=end).collect();
                return record[..record.len() - 1].to_string();
            }
            match self.frame(PATIENCE).await {
                None => panic!("the server said nothing for {PATIENCE:?}"),
                Some(None) => panic!("the connection ended while a record was expected"),
                Some(Some((OP_TEXT, payload))) => {
                    self.text.push_str(std::str::from_utf8(&payload).unwrap())
                }
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                Some(Some((OP_CLOSE, payload))) => {
                    self.closed = payload
                        .get(..2)
                        .map(|code| u16::from_be_bytes([code[0], code[1]]));
                    panic!("the server closed the connection while a record was expected")
                }
                Some(Some(_)) => {}
            }
        }
    }

    /// The next record exactly as it went over the wire.
    pub async fn raw_record(&mut self) -> String {
        loop {
            let text = self.next_text().await;
            if !text.starts_with("{\"type\":6") {
                return text;
            }
        }
    }

    /// The next record the server sent that is not a ping.
    pub async fn record(&mut self) -> Value {
        loop {
            if let Some(record) = self.records.pop_front() {
                return record;
            }
            let record: Value = serde_json::from_str(&self.next_text().await).unwrap();
            if record["type"] != 6 {
                return record;
            }
        }
    }

    /// The arguments of the next event, which must be named `name`.
    pub async fn event(&mut self, name: &str) -> Value {
        let record = self.record().await;
        assert_eq!(record["type"], 1, "{record}");
        assert_eq!(record["target"], name, "{record}");
        record["arguments"][0].clone()
    }

    /// Everything the server sends within `wait`, which is expected to be
    /// nothing a test did not ask for.
    pub async fn quiet_for(&mut self, wait: Duration) -> Vec<Value> {
        let mut seen: Vec<Value> = self.records.drain(..).collect();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return seen;
            }
            match self.frame(left).await {
                None => return seen,
                Some(None) => return seen,
                Some(Some((OP_TEXT, payload))) => {
                    self.text.push_str(std::str::from_utf8(&payload).unwrap());
                    while let Some(end) = self.text.find(RS) {
                        let record: String = self.text.drain(..=end).collect();
                        let record: Value =
                            serde_json::from_str(&record[..record.len() - 1]).unwrap();
                        if record["type"] != 6 {
                            seen.push(record);
                        }
                    }
                }
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                Some(Some((OP_CLOSE, payload))) => {
                    self.closed = payload
                        .get(..2)
                        .map(|code| u16::from_be_bytes([code[0], code[1]]));
                    return seen;
                }
                Some(Some(_)) => {}
            }
        }
    }

    /// Calls a method and waits for its completion, keeping the events that
    /// come first for later.
    pub async fn invoke(&mut self, target: &str, arguments: Value) -> Value {
        self.next_invocation += 1;
        let id = self.next_invocation.to_string();
        let record =
            json!({"type": 1, "invocationId": id, "target": target, "arguments": arguments});
        self.send_text(&format!("{record}{RS}")).await;
        let mut kept = Vec::new();
        let completion = loop {
            let record: Value = serde_json::from_str(&self.next_text().await).unwrap();
            if record["type"] == 3 && record["invocationId"] == id.as_str() {
                break record;
            }
            if record["type"] != 6 {
                kept.push(record);
            }
        };
        for record in kept.into_iter().rev() {
            self.records.push_front(record);
        }
        completion
    }

    /// Reads until the server closes, and gives the records that came before
    /// the hub's close message and that message.
    pub async fn until_close(&mut self) -> (Vec<Value>, Option<Value>) {
        let mut records: Vec<Value> = self.records.drain(..).collect();
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match self.frame(left).await {
                None => panic!("the server did not close the connection"),
                Some(None) => return (records, None),
                Some(Some((OP_TEXT, payload))) => {
                    self.text.push_str(std::str::from_utf8(&payload).unwrap());
                    while let Some(end) = self.text.find(RS) {
                        let record: String = self.text.drain(..=end).collect();
                        let record: Value =
                            serde_json::from_str(&record[..record.len() - 1]).unwrap();
                        if record["type"] == 7 {
                            return (records, Some(record));
                        }
                        if record["type"] != 6 {
                            records.push(record);
                        }
                    }
                }
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                Some(Some((OP_CLOSE, payload))) => {
                    self.closed = payload
                        .get(..2)
                        .map(|code| u16::from_be_bytes([code[0], code[1]]));
                    return (records, None);
                }
                Some(Some(_)) => {}
            }
        }
    }

    /// Waits for the WebSocket close that follows the hub's own close message
    /// and answers it, as a browser does.
    pub async fn finish_close(&mut self) -> Option<u16> {
        loop {
            match self.frame(PATIENCE).await {
                None => panic!("the server did not finish closing"),
                Some(None) => return self.closed,
                Some(Some((OP_CLOSE, payload))) => {
                    self.closed = payload
                        .get(..2)
                        .map(|code| u16::from_be_bytes([code[0], code[1]]));
                    self.send_frame(OP_CLOSE, &payload).await;
                    return self.closed;
                }
                Some(Some((OP_PING, payload))) => self.send_frame(OP_PONG, &payload).await,
                Some(Some(_)) => {}
            }
        }
    }

    /// Hangs up as a browser that was closed does.
    pub async fn hang_up(mut self) {
        self.send_frame(OP_CLOSE, &1000u16.to_be_bytes()).await;
        let _ = timeout(Duration::from_secs(5), self.finish_close()).await;
    }
}
