//! A small local server for the tests: whatever bytes the test wants, well
//! formed or not, on a loopback port, with a count of the connections it saw.
#![allow(dead_code)]

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use stellarr_embeds::{EmbedSettings, Fetcher};
use stellarr_outbound::{Client, TableResolver};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// What the server read of a request.
#[derive(Clone, Debug)]
pub struct Recorded {
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
}

pub struct Conn {
    stream: TcpStream,
    pub request: Recorded,
}

impl Conn {
    pub async fn write(&mut self, bytes: &[u8]) {
        let _ = self.stream.write_all(bytes).await;
        let _ = self.stream.flush().await;
    }

    /// A complete answer with a length and `Connection: close`.
    pub async fn respond(&mut self, status: &str, headers: &[(&str, &str)], body: &[u8]) {
        let mut head = format!("HTTP/1.1 {status}\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        self.write(head.as_bytes()).await;
        self.write(body).await;
    }

    pub async fn ok(&mut self, content_type: &str, body: &[u8]) {
        self.respond("200 OK", &[("Content-Type", content_type)], body)
            .await;
    }

    pub async fn redirect(&mut self, status: u16, location: &str) {
        self.respond(&format!("{status} Moved"), &[("Location", location)], b"")
            .await;
    }

    /// A body that ends when the connection does, so its size is not announced.
    pub async fn streamed(&mut self, content_type: &str, body: &[u8]) {
        let head =
            format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n");
        self.write(head.as_bytes()).await;
        self.write(body).await;
    }

    /// Says nothing more and waits for the client to go away.
    pub async fn stall(&mut self) {
        let mut buffer = [0u8; 1024];
        loop {
            match self.stream.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
}

pub type Handler = Arc<dyn Fn(Conn) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub fn handler<F, Fut>(f: F) -> Handler
where
    F: Fn(Conn) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    Arc::new(move |conn| Box::pin(f(conn)))
}

pub struct Server {
    pub addr: SocketAddr,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(handler: Handler) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let task = {
            let (requests, hits) = (requests.clone(), hits.clone());
            tokio::spawn(async move {
                let mut connections = tokio::task::JoinSet::new();
                while let Ok((tcp, _)) = listener.accept().await {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let (handler, requests) = (handler.clone(), requests.clone());
                    connections.spawn(async move {
                        let mut stream = tcp;
                        let Some(request) = read_head(&mut stream).await else {
                            return;
                        };
                        requests.lock().unwrap().push(request.clone());
                        handler(Conn { stream, request }).await;
                    });
                }
            })
        };
        Server {
            addr,
            requests,
            hits,
            task,
        }
    }

    /// A server that answers every request with the same page.
    pub async fn page(content_type: &'static str, body: Vec<u8>) -> Server {
        Server::start(handler(move |mut conn| {
            let body = body.clone();
            async move { conn.ok(content_type, &body).await }
        }))
        .await
    }

    /// The address of `path` under `name`, which the fetcher's resolver
    /// leads here.
    pub fn url(&self, name: &str, path: &str) -> String {
        format!("http://{name}:{}{path}", self.addr.port())
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

async fn read_head(stream: &mut TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
            let mut lines = head.lines();
            let target = lines.next()?.split(' ').nth(1)?.to_owned();
            let headers = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                .collect();
            return Some(Recorded { target, headers });
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
    }
}

/// Names that lead to this machine, as a made-up public name would lead to a
/// test server.
pub fn loopback_names(names: &[&str]) -> Client {
    let table = names
        .iter()
        .map(|n| {
            (
                (*n).to_owned(),
                vec!["127.0.0.1".parse::<IpAddr>().unwrap()],
            )
        })
        .collect();
    Client::new().with_resolver(Arc::new(TableResolver(table)))
}

/// A fetcher that may connect to this machine, under the given settings.
pub fn fetcher(names: &[&str], settings: EmbedSettings) -> Fetcher {
    Fetcher::with_client(
        loopback_names(names),
        EmbedSettings {
            allow_private_networks: true,
            ..settings
        },
    )
}
