//! Local servers for the tests: plain and TLS, driven by a closure that
//! writes whatever bytes the test wants, well formed or not.
//!
//! The certificates in `tests/fixtures` are test keys made with openssl
//! (a root, a server certificate for `localhost`, `127.0.0.1` and `::1`
//! signed by it, one for another name, one signed by another root, one that
//! has expired). They carry a `.txt` suffix because the repository ignores
//! files that look like keys. They protect nothing.
#![allow(dead_code)]

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use stellarr_outbound::{Client, tls};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

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
}

pub struct Conn {
    pub stream: Box<dyn Io>,
    pub request: Recorded,
    /// Bytes of the body that arrived with the head.
    pub leftover: Vec<u8>,
    /// The protocol the TLS handshake settled on.
    pub alpn: Option<Vec<u8>>,
    closed: Arc<AtomicUsize>,
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
        let status = format!("{status} Moved");
        self.respond(&status, &[("Location", location)], b"").await;
    }

    /// The request body, by its `Content-Length`.
    pub async fn read_body(&mut self) -> Vec<u8> {
        let length: usize = self
            .request
            .header("content-length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let mut body = std::mem::take(&mut self.leftover);
        while body.len() < length {
            let mut buffer = [0u8; 8192];
            match self.stream.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => body.extend_from_slice(&buffer[..n]),
            }
        }
        body
    }

    /// Says nothing more and waits for the client to go away, then counts
    /// it. What a stalling server does.
    pub async fn stall(&mut self) {
        let mut buffer = [0u8; 1024];
        loop {
            match self.stream.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        self.closed.fetch_add(1, Ordering::SeqCst);
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
    pub tls: bool,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    /// Connections that a stalling handler saw the client close.
    pub closed: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn server_config(name: &str) -> Arc<rustls::ServerConfig> {
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(&fixture(&format!("{name}.crt.txt")))
            .collect::<Result<_, _>>()
            .unwrap();
    let key = PrivateKeyDer::from_pem_slice(&fixture(&format!("{name}.pk8.txt"))).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .unwrap();
    // Offered so that a test can see the client does not take h2.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(config)
}

impl Server {
    pub async fn plain(handler: Handler) -> Server {
        Server::start(None, handler).await
    }

    /// A TLS server with the certificate of fixture `name`.
    pub async fn tls(name: &str, handler: Handler) -> Server {
        Server::start(Some(server_config(name)), handler).await
    }

    async fn start(config: Option<Arc<rustls::ServerConfig>>, handler: Handler) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(AtomicUsize::new(0));
        let acceptor = config.clone().map(tokio_rustls::TlsAcceptor::from);
        let task = {
            let requests = requests.clone();
            let closed = closed.clone();
            tokio::spawn(async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    let Ok((tcp, _)) = listener.accept().await else {
                        break;
                    };
                    let handler = handler.clone();
                    let requests = requests.clone();
                    let closed = closed.clone();
                    let acceptor = acceptor.clone();
                    connections.spawn(async move {
                        let (mut stream, alpn): (Box<dyn Io>, Option<Vec<u8>>) = match acceptor {
                            None => (Box::new(tcp), None),
                            Some(acceptor) => match acceptor.accept(tcp).await {
                                Ok(tls) => {
                                    let alpn = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
                                    (Box::new(tls), alpn)
                                }
                                Err(_) => return,
                            },
                        };
                        let Some((request, leftover)) = read_head(&mut stream).await else {
                            return;
                        };
                        requests.lock().unwrap().push(request.clone());
                        handler(Conn {
                            stream,
                            request,
                            leftover,
                            alpn,
                            closed,
                        })
                        .await;
                    });
                }
            })
        };
        Server {
            addr,
            tls: config.is_some(),
            requests,
            closed,
            task,
        }
    }

    /// `http://127.0.0.1:port/path`, or `https://localhost:port/path`.
    pub fn url(&self, path: &str) -> String {
        if self.tls {
            format!("https://localhost:{}{path}", self.addr.port())
        } else {
            format!("http://127.0.0.1:{}{path}", self.addr.port())
        }
    }

    pub fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

async fn read_head(stream: &mut Box<dyn Io>) -> Option<(Recorded, Vec<u8>)> {
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
            let leftover = buffer[end + 4..].to_vec();
            let mut lines = text.split("\r\n");
            let mut first = lines.next()?.split(' ');
            let method = first.next()?.to_owned();
            let target = first.next()?.to_owned();
            let headers = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .collect();
            return Some((
                Recorded {
                    method,
                    target,
                    headers,
                },
                leftover,
            ));
        }
        if buffer.len() > 256 * 1024 {
            return None;
        }
    }
}

/// A client that trusts the fixtures' root and nothing else.
pub fn client() -> Client {
    let roots = tls::roots_from_pem(&fixture("ca.crt.txt")).unwrap();
    Client::with_tls_config(tls::client_config(roots))
}
