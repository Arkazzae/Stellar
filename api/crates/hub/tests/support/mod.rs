//! A test application and a client that plays the browser's part over an
//! in-memory connection.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stellarr_hub::{Application, Call, Config, Hub, HubError, Outbound, serve};
use stellarr_websocket::frame::{OP_CLOSE, OP_PING, OP_TEXT, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub const RS: char = '\u{1e}';

#[derive(Default)]
pub struct TestApp {
    pub log: Mutex<Vec<String>>,
    pub running: AtomicUsize,
    pub max_running: AtomicUsize,
    pub fail_connect: bool,
    pub connect_delay: Duration,
}

impl TestApp {
    pub fn log(&self, line: impl Into<String>) {
        self.log.lock().unwrap().push(line.into());
    }

    pub fn lines(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Application for TestApp {
    type Identity = String;

    async fn connected(&self, call: &Call<String>) -> Result<(), HubError> {
        tokio::time::sleep(self.connect_delay).await;
        self.log(format!("connected {}", call.identity()));
        if self.fail_connect {
            return Err(HubError::visible("not today"));
        }
        call.hub().add_to_group(call.connection_id(), "all");
        call.hub()
            .add_to_group(call.connection_id(), &format!("user:{}", call.identity()));
        call.send(&Outbound::single("Hello", &call.identity()).unwrap());
        Ok(())
    }

    async fn disconnected(&self, call: &Call<String>, error: Option<&str>) {
        self.log(format!(
            "disconnected {} {}",
            call.identity(),
            error.unwrap_or("-")
        ));
    }

    fn has_method(&self, target: &str) -> bool {
        const KNOWN: [&str; 11] = [
            "echo", "void", "fail", "internal", "panic", "slow", "say", "others", "abortme",
            "whoami", "big",
        ];
        KNOWN.iter().any(|known| known.eq_ignore_ascii_case(target))
    }

    async fn invoke(
        &self,
        call: &Call<String>,
        target: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, HubError> {
        match target.to_ascii_lowercase().as_str() {
            "echo" => match arguments.as_slice() {
                [value] => Ok(value.clone()),
                _ => Err(HubError::InvalidArguments),
            },
            "void" => Ok(Value::Null),
            "fail" => Err(HubError::visible("Not allowed here.")),
            "internal" => Err(HubError::internal("the database is down")),
            "panic" => panic!("a bug"),
            "slow" => {
                let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_running.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                self.running.fetch_sub(1, Ordering::SeqCst);
                Ok(arguments.first().cloned().unwrap_or(Value::Null))
            }
            "say" => {
                let [Value::String(group), Value::String(text)] = arguments.as_slice() else {
                    return Err(HubError::InvalidArguments);
                };
                call.hub()
                    .send_to_group(group, &Outbound::single("Said", text).unwrap());
                Ok(Value::Null)
            }
            "others" => {
                call.hub().send_to_group_except(
                    "all",
                    &[call.connection_id()],
                    &Outbound::bare("Others"),
                );
                Ok(Value::Null)
            }
            "abortme" => {
                call.abort();
                Ok(Value::Null)
            }
            "whoami" => Ok(json!(call.identity())),
            "big" => Ok(json!("x".repeat(200_000))),
            _ => Err(HubError::UnknownMethod),
        }
    }
}

pub struct Client {
    pub io: DuplexStream,
    pub pending: VecDeque<String>,
    text: String,
}

/// What the server wrote: a frame's opcode and payload.
pub type Frame = (u8, Vec<u8>);

impl Client {
    pub fn wrap(io: DuplexStream) -> Self {
        Self {
            io,
            pending: VecDeque::new(),
            text: String::new(),
        }
    }

    pub async fn send_frame(&mut self, opcode: u8, payload: &[u8]) {
        self.io
            .write_all(&encode_frame(true, opcode, payload, Some([9, 8, 7, 6])))
            .await
            .unwrap();
    }

    pub async fn send(&mut self, text: &str) {
        self.send_frame(OP_TEXT, text.as_bytes()).await;
    }

    /// Records, each ended by the separator, in one message.
    pub async fn send_records(&mut self, records: &[Value]) {
        let text: String = records.iter().map(|r| format!("{r}{RS}")).collect();
        self.send(&text).await;
    }

    pub async fn handshake(&mut self) {
        self.send(&format!("{{\"protocol\":\"json\",\"version\":1}}{RS}"))
            .await;
        assert_eq!(self.record().await, "{}");
    }

    pub async fn invoke(&mut self, id: &str, target: &str, arguments: Value) {
        self.send_records(&[
            json!({"type": 1, "invocationId": id, "target": target, "arguments": arguments}),
        ])
        .await;
    }

    pub async fn frame(&mut self) -> Option<Frame> {
        let mut head = [0u8; 2];
        match timeout(Duration::from_secs(5), self.io.read_exact(&mut head)).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return None,
            Err(_) => panic!("the server wrote nothing for five seconds"),
        }
        assert_eq!(head[1] & 0x80, 0, "the server never masks");
        let len = match head[1] & 0x7f {
            126 => {
                let mut b = [0u8; 2];
                self.io.read_exact(&mut b).await.unwrap();
                usize::from(u16::from_be_bytes(b))
            }
            127 => {
                let mut b = [0u8; 8];
                self.io.read_exact(&mut b).await.unwrap();
                u64::from_be_bytes(b) as usize
            }
            n => usize::from(n),
        };
        let mut payload = vec![0u8; len];
        self.io.read_exact(&mut payload).await.unwrap();
        Some((head[0] & 0x0f, payload))
    }

    /// The next record the server sent, as text. Transport pings are
    /// answered the way a browser would, and skipped.
    pub async fn record(&mut self) -> String {
        loop {
            if let Some(record) = self.pending.pop_front() {
                return record;
            }
            let (opcode, payload) = self
                .frame()
                .await
                .expect("the connection ended while a record was expected");
            match opcode {
                OP_TEXT => {
                    self.text.push_str(std::str::from_utf8(&payload).unwrap());
                    while let Some(end) = self.text.find(RS) {
                        let record: String = self.text.drain(..=end).collect();
                        self.pending
                            .push_back(record[..record.len() - 1].to_owned());
                    }
                }
                OP_PING => self.send_frame(10, &payload).await,
                OP_CLOSE => panic!("closed by the server while a record was expected"),
                other => panic!("unexpected opcode {other}"),
            }
        }
    }

    pub async fn json(&mut self) -> Value {
        serde_json::from_str(&self.record().await).unwrap()
    }

    /// Reads until the server's close frame, collecting the records before it.
    pub async fn until_close(&mut self) -> (Vec<Value>, Option<u16>) {
        let mut records = Vec::new();
        loop {
            while let Some(record) = self.pending.pop_front() {
                records.push(serde_json::from_str(&record).unwrap());
            }
            match self.frame().await {
                None => return (records, None),
                Some((OP_TEXT, payload)) => {
                    self.text.push_str(std::str::from_utf8(&payload).unwrap());
                    while let Some(end) = self.text.find(RS) {
                        let record: String = self.text.drain(..=end).collect();
                        self.pending
                            .push_back(record[..record.len() - 1].to_owned());
                    }
                }
                Some((OP_CLOSE, payload)) => {
                    while let Some(record) = self.pending.pop_front() {
                        records.push(serde_json::from_str(&record).unwrap());
                    }
                    // Answer the close as a client does, so the server can finish.
                    self.send_frame(OP_CLOSE, &payload).await;
                    return (records, Some(u16::from_be_bytes([payload[0], payload[1]])));
                }
                Some((OP_PING, payload)) => self.send_frame(10, &payload).await,
                Some(_) => {}
            }
        }
    }
}

pub struct Running {
    pub client: Client,
    pub hub: Hub,
    pub app: Arc<TestApp>,
    pub task: JoinHandle<()>,
}

pub fn start(app: TestApp, config: Config, user: &str) -> Running {
    let hub = Hub::new(&config);
    start_on(Arc::new(app), hub, config, user, stellarr_hub_id(user))
}

fn stellarr_hub_id(user: &str) -> String {
    format!("conn-{user}")
}

pub fn start_on(app: Arc<TestApp>, hub: Hub, config: Config, user: &str, id: String) -> Running {
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let (sender, receiver) = stellarr_websocket::split(
        server_io,
        stellarr_websocket::Config {
            max_message_size: usize::MAX,
            ..Default::default()
        },
    );
    let task = tokio::spawn(serve(
        app.clone(),
        hub.clone(),
        Arc::new(config),
        Arc::new(user.to_owned()),
        id.into(),
        sender,
        receiver,
    ));
    Running {
        client: Client {
            io: client_io,
            pending: VecDeque::new(),
            text: String::new(),
        },
        hub,
        app,
        task,
    }
}

/// Waits until a condition holds, for conditions that depend on tasks
/// finishing.
pub async fn eventually(mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never came true");
}
