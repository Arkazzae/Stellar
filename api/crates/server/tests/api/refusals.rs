//! A refusal sent before the body of the request was read must reach the
//! client, whatever the client is still writing.
//!
//! A server that answers and closes with the rest of the body unread makes the
//! operating system send a reset, which can take the answer with it: the
//! client sees a broken connection where there was a 413 or a 401. The server
//! therefore ends every connection by sending its end first and reading and
//! discarding what is still coming for a short, bounded time (`src/http/linger.rs`).
//!
//! The clients here are the harness's own (a hyper client, which is also what
//! the desktop app is) and a raw one that writes as fast as it can, and every
//! count is of requests that got the answer the route gives.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::http::StatusCode;
use http::Method;
use stellarr_api::http::{Endpoints, Json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::support::{Request, Server};

const MIB: usize = 1024 * 1024;
const CLIENTS: usize = 16;

fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/test/json", json)
        .json_body()
        .post("/api/test/upload", upload)
        .authorized()
}

async fn json(Json(_): Json<serde_json::Value>) -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Needs a session, and the clients have none: refused before its body.
async fn upload() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// How a request ended, as the client saw it.
#[derive(Default)]
struct Outcomes {
    answered: AtomicU64,
    nothing: AtomicU64,
    wrong: AtomicU64,
}

impl Outcomes {
    fn note(&self, status: Option<u16>, expected: u16) {
        match status {
            Some(status) if status == expected => &self.answered,
            Some(_) => &self.wrong,
            None => &self.nothing,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    fn lost(&self) -> (u64, u64) {
        (
            self.nothing.load(Ordering::Relaxed),
            self.wrong.load(Ordering::Relaxed),
        )
    }
}

/// How the body of a raw request is framed.
#[derive(Clone, Copy)]
enum Framing {
    Declared(usize),
    Chunked,
}

/// Writes a request by hand, `write` bytes of body as fast as the connection
/// takes them, while another task reads; a write that fails ends the writing
/// and not the reading, as a client that reads while it sends does.
async fn raw(
    address: std::net::SocketAddr,
    path: &str,
    framing: Framing,
    write: usize,
    slowly: bool,
) -> Option<u16> {
    let stream = TcpStream::connect(address).await.ok()?;
    let (mut reader, mut writer) = stream.into_split();
    let framing_line = match framing {
        Framing::Declared(length) => format!("Content-Length: {length}\r\n"),
        Framing::Chunked => "Transfer-Encoding: chunked\r\n".to_string(),
    };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n{framing_line}\r\n"
    );
    let reading = tokio::spawn(async move {
        let mut answer = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(30), reader.read_to_end(&mut answer)).await;
        answer
    });
    if writer.write_all(head.as_bytes()).await.is_ok() {
        let block = vec![b'0'; if slowly { 8 * 1024 } else { 64 * 1024 }];
        let mut written = 0;
        while written < write {
            let size = block.len().min(write - written);
            let sent = match framing {
                Framing::Declared(_) => writer.write_all(&block[..size]).await,
                Framing::Chunked => {
                    writer
                        .write_all(&[format!("{size:x}\r\n").as_bytes(), &block[..size], b"\r\n"].concat())
                        .await
                }
            };
            if sent.is_err() {
                break;
            }
            written += size;
            if slowly {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }
    let answer = reading.await.ok()?;
    let text = String::from_utf8_lossy(&answer);
    text.strip_prefix("HTTP/1.1 ")?.get(..3)?.parse().ok()
}

async fn hammer(requests_per_client: usize, slowly: bool) {
    let server = Server::in_process(routes(), &[]).await;
    let address = server.address();
    // (what is sent, where, what the route answers)
    let hyper_json = Arc::new(Outcomes::default());
    let hyper_upload = Arc::new(Outcomes::default());
    let raw_json = Arc::new(Outcomes::default());
    let raw_json_chunked = Arc::new(Outcomes::default());
    let raw_upload = Arc::new(Outcomes::default());
    let raw_upload_chunked = Arc::new(Outcomes::default());
    let mut clients = Vec::new();
    for _ in 0..CLIENTS {
        let (hj, hu, rj, rjc, ru, ruc) = (
            Arc::clone(&hyper_json),
            Arc::clone(&hyper_upload),
            Arc::clone(&raw_json),
            Arc::clone(&raw_json_chunked),
            Arc::clone(&raw_upload),
            Arc::clone(&raw_upload_chunked),
        );
        let client = server.client();
        clients.push(tokio::spawn(async move {
            for _ in 0..requests_per_client {
                let body = vec![b'0'; MIB + 1];
                let sent = client
                    .try_send(Request::new(Method::POST, "/api/test/json").bytes("application/json", body))
                    .await;
                hj.note(sent.ok().map(|answer| answer.status.as_u16()), 413);
                let body = vec![b'0'; 3 * MIB];
                let sent = client
                    .try_send(Request::new(Method::POST, "/api/test/upload").bytes("application/json", body))
                    .await;
                hu.note(sent.ok().map(|answer| answer.status.as_u16()), 401);

                let write = if slowly { MIB + MIB / 4 } else { 3 * MIB };
                rj.note(raw(address, "/api/test/json", Framing::Declared(MIB + 1), write, slowly).await, 413);
                rjc.note(raw(address, "/api/test/json", Framing::Chunked, write, slowly).await, 400);
                ru.note(raw(address, "/api/test/upload", Framing::Declared(128 * MIB + 1), write, slowly).await, 401);
                ruc.note(raw(address, "/api/test/upload", Framing::Chunked, write, slowly).await, 401);
            }
        }));
    }
    for client in clients {
        client.await.unwrap();
    }
    let total = CLIENTS * requests_per_client;
    let mut report = Vec::new();
    for (name, outcomes) in [
        ("hyper client, JSON 1 MiB + 1 declared", &hyper_json),
        ("hyper client, 3 MiB to a route that needs a session", &hyper_upload),
        ("raw client, JSON 1 MiB + 1 declared", &raw_json),
        ("raw client, JSON chunked", &raw_json_chunked),
        ("raw client, upload 128 MiB + 1 declared", &raw_upload),
        ("raw client, upload chunked", &raw_upload_chunked),
    ] {
        let (nothing, wrong) = outcomes.lost();
        report.push(format!("{name}: of {total}, {nothing} got no answer, {wrong} another one"));
        println!("{}", report.last().unwrap());
    }
    let lost: u64 = [&hyper_json, &hyper_upload, &raw_json, &raw_json_chunked, &raw_upload, &raw_upload_chunked]
        .iter()
        .map(|outcomes| outcomes.lost().0 + outcomes.lost().1)
        .sum();
    assert_eq!(lost, 0, "{report:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refusal_before_the_body_reaches_clients_that_keep_writing() {
    hammer(25, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refusal_before_the_body_reaches_clients_that_write_slowly() {
    hammer(5, true).await;
}

/// 16 clients, `STELLARR_STRESS_REQUESTS` requests of each kind each (250 by
/// default, 24 000 requests in all):
///
/// ```sh
/// cargo test -p stellarr-api --test api refusals::many -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "sends tens of thousands of large requests: see the doc comment"]
async fn many_refusals_before_the_body_reach_their_clients() {
    let each = std::env::var("STELLARR_STRESS_REQUESTS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(250);
    hammer(each, false).await;
}

/// What a hyper client (as the desktop app is) gets of a refusal from the first
/// server and from this one, for bodies that are refused unread:
///
/// ```sh
/// oracle start refuse 5271 Production
/// ORACLE_ADDRESS=127.0.0.1:5271 cargo test -p stellarr-api --test api \
///     refusals::as_the_first_server -- --ignored --nocapture
/// oracle stop refuse
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the first server: see the doc comment"]
async fn refusals_reach_a_hyper_client_as_the_first_server_gives_them() {
    let oracle: std::net::SocketAddr = std::env::var("ORACLE_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:5271".into())
        .parse()
        .expect("an address");
    let server = Server::in_process(routes(), &[]).await;
    let mut lost_here = 0;
    for (name, address, path) in [
        ("first server", oracle, "/api/uploads"),
        ("this server", server.address(), "/api/test/upload"),
    ] {
        for size in [3 * MIB, 20 * MIB, 140 * MIB] {
            let outcomes = Arc::new(Outcomes::default());
            let mut clients = Vec::new();
            let each = if size > 100 * MIB { 1 } else { 5 };
            for _ in 0..CLIENTS {
                let outcomes = Arc::clone(&outcomes);
                let client = crate::support::Client::browser(address);
                clients.push(tokio::spawn(async move {
                    for _ in 0..each {
                        let sent = client
                            .try_send(Request::new(Method::POST, path).bytes("multipart/form-data; boundary=x", vec![b'0'; size]))
                            .await;
                        // Any answer at all is what is counted.
                        outcomes.note(sent.ok().map(|_| 0), 0);
                    }
                }));
            }
            for client in clients {
                client.await.unwrap();
            }
            let (nothing, _) = outcomes.lost();
            println!("{name}, {} MiB: of {}, {nothing} got no answer", size / MIB, CLIENTS * each);
            if name == "this server" && size <= 100 * MIB {
                lost_here += nothing;
            }
        }
    }
    assert_eq!(lost_here, 0);
}
