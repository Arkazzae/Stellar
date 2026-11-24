//! A Discord of the tests' own: a server on the loopback that answers as it
//! is told to, a clock the test holds, and a client pointed at both through
//! the resolver table, so that nothing leaves the machine.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stellarr_discord::{BotReader, BotToken, ClientConfig, Clock, DiscordClient, RequestPolicy};
use stellarr_outbound::{AddressPolicy, Client, TableResolver};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

pub mod guild;

/// A recognisable token, which no output of the crate may hold.
pub const TOKEN: &str = "sample.bot-token.not-a-real-one-0123456789";

/// What the server read of a request.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The path without its query.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    /// One query parameter, with its percent-escapes undone.
    pub fn query(&self, name: &str) -> Option<String> {
        let query = self.target.split_once('?')?.1;
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| unescape(value))
        })
    }
}

pub fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[at + 1..at + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the server does with a request.
pub enum Answer {
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Says nothing, and keeps the connection open.
    Stall,
    /// Promises a long body, sends the start of it and then nothing.
    StallHalfway(Vec<u8>),
    /// Hangs up.
    Close,
}

impl Answer {
    pub fn json(body: impl Into<String>) -> Answer {
        Answer::status(200, body).with_header("Content-Type", "application/json")
    }

    pub fn status(status: u16, body: impl Into<String>) -> Answer {
        Answer::Respond {
            status,
            headers: Vec::new(),
            body: body.into().into_bytes(),
        }
    }

    /// Discord's error body.
    pub fn error(status: u16, code: i32, message: &str) -> Answer {
        Answer::status(
            status,
            serde_json::json!({ "message": message, "code": code }).to_string(),
        )
        .with_header("Content-Type", "application/json")
    }

    /// A 429 as Discord sends it.
    pub fn too_many(retry_after: f64, global: bool) -> Answer {
        Answer::status(
            429,
            serde_json::json!({
                "message": "You are being rate limited.",
                "retry_after": retry_after,
                "global": global,
            })
            .to_string(),
        )
        .with_header("Content-Type", "application/json")
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Answer {
        if let Answer::Respond { headers, .. } = &mut self {
            headers.push((name.to_owned(), value.to_owned()));
        }
        self
    }
}

type Handler = Arc<dyn Fn(&Recorded, usize) -> Answer + Send + Sync>;

pub struct Server {
    pub addr: SocketAddr,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    /// A server that answers every request with what `handler` says, given
    /// the request and how many came before it.
    pub async fn start(
        handler: impl Fn(&Recorded, usize) -> Answer + Send + Sync + 'static,
    ) -> Server {
        let handler: Handler = Arc::new(handler);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::<Recorded>::new()));
        let log = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let handler = handler.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let Some(request) = read_head(&mut stream).await else {
                        return;
                    };
                    let index = {
                        let mut all = log.lock().unwrap();
                        all.push(request.clone());
                        all.len() - 1
                    };
                    match handler(&request, index) {
                        Answer::Respond {
                            status,
                            headers,
                            body,
                        } => {
                            let mut head = format!("HTTP/1.1 {status} Status\r\n");
                            for (name, value) in headers {
                                let _ = write!(head, "{name}: {value}\r\n");
                            }
                            let _ = write!(
                                head,
                                "Content-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(head.as_bytes()).await;
                            let _ = stream.write_all(&body).await;
                            let _ = stream.flush().await;
                        }
                        Answer::Stall => hold_open(&mut stream).await,
                        Answer::StallHalfway(start) => {
                            let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n";
                            let _ = stream.write_all(head.as_bytes()).await;
                            let _ = stream.write_all(&start).await;
                            let _ = stream.flush().await;
                            hold_open(&mut stream).await;
                        }
                        Answer::Close => {}
                    }
                });
            }
        });
        Server {
            addr,
            requests,
            task,
        }
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// The paths with their queries, in order.
    pub fn targets(&self) -> Vec<String> {
        self.recorded().into_iter().map(|r| r.target).collect()
    }
}

async fn hold_open(stream: &mut tokio::net::TcpStream) {
    let mut buffer = [0u8; 256];
    while matches!(stream.read(&mut buffer).await, Ok(n) if n > 0) {}
}

async fn read_head(stream: &mut tokio::net::TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let text = String::from_utf8_lossy(&buffer[..end]).into_owned();
            let mut lines = text.split("\r\n");
            let mut first = lines.next()?.split(' ');
            let method = first.next()?.to_owned();
            let target = first.next()?.to_owned();
            let headers = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .collect();
            return Some(Recorded {
                method,
                target,
                headers,
            });
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
    }
}

/// A clock the test holds: a wait is written down and either passes at once,
/// moving the time on by it, or, held, never ends.
pub struct FakeClock {
    now: Mutex<Duration>,
    waits: Mutex<Vec<Duration>>,
    pub hold: AtomicBool,
}

impl FakeClock {
    /// 2026-11-23T12:00:00Z.
    pub fn new() -> Arc<FakeClock> {
        Arc::new(FakeClock {
            now: Mutex::new(Duration::from_secs(1_795_435_200)),
            waits: Mutex::new(Vec::new()),
            hold: AtomicBool::new(false),
        })
    }

    pub fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap().clone()
    }

    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }

    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.waits.lock().unwrap().push(duration);
        if self.hold.load(Ordering::SeqCst) {
            Box::pin(std::future::pending())
        } else {
            *self.now.lock().unwrap() += duration;
            Box::pin(std::future::ready(()))
        }
    }
}

/// Everything a test needs: the server, the clock and a client pointed at
/// them.
pub struct Rig {
    pub server: Server,
    pub clock: Arc<FakeClock>,
    pub client: DiscordClient,
}

impl Rig {
    pub async fn new(handler: impl Fn(&Recorded, usize) -> Answer + Send + Sync + 'static) -> Rig {
        Rig::configured(handler, |_| {}).await
    }

    pub async fn configured(
        handler: impl Fn(&Recorded, usize) -> Answer + Send + Sync + 'static,
        change: impl FnOnce(&mut ClientConfig),
    ) -> Rig {
        let server = Server::start(handler).await;
        let clock = FakeClock::new();
        let mut config = config_for(server.addr.port(), clock.clone());
        change(&mut config);
        Rig {
            server,
            clock,
            client: DiscordClient::with_config(config),
        }
    }

    pub fn bot(&self) -> BotReader {
        self.client.for_bot(BotToken::new(TOKEN).unwrap())
    }
}

/// A configuration that sends everything to `port` on this machine, by way of
/// a name that resolves there, as the policy for public addresses would
/// refuse the real thing.
pub fn config_for(port: u16, clock: Arc<FakeClock>) -> ClientConfig {
    let resolver = TableResolver(vec![(
        "discord.test".to_owned(),
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    )]);
    ClientConfig {
        client: Client::new().with_resolver(Arc::new(resolver)),
        base: format!("http://discord.test:{port}/api/v10/"),
        address_policy: AddressPolicy::Anywhere,
        clock,
        policy: RequestPolicy::default(),
        timeout: Duration::from_secs(10),
        max_response_bytes: 32 * 1024 * 1024,
    }
}

/// What a `tracing` subscriber caught, as text.
#[derive(Clone, Default)]
pub struct Catch(pub Arc<Mutex<String>>);

impl Catch {
    pub fn text(&self) -> String {
        self.0.lock().unwrap().clone()
    }
}

impl Visit for Catch {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0.lock().unwrap(), "{}={value:?} ", field.name());
    }
}

impl Subscriber for Catch {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        attributes.record(&mut self.clone());
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, values: &Record<'_>) {
        values.record(&mut self.clone());
    }
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let _ = write!(self.0.lock().unwrap(), "{} ", event.metadata().target());
        event.record(&mut self.clone());
        self.0.lock().unwrap().push('\n');
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}
