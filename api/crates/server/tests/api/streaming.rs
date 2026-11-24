//! A file answered by `http::file` arrives whole, however many clients ask at
//! once, on a connection of its own or a kept one.
//!
//! A download of a small file used to lose its only chunk about once in four
//! thousand requests when the task that read the file ended while the response
//! was being polled: the client got nothing, or the head and no body, and the
//! connection was closed (see `src/http/stream.rs`). The race needs several
//! threads, so these tests run on a multi-threaded runtime.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use stellarr_api::http::Endpoints;
use stellarr_api::http::file::{Download, answer};
use stellarr_api::state::AppState;
use stellarr_api::values::Timestamp;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::support::{Server, raw_get};

const CONTENT: &[u8] = b"a file of the size of a package's signature, small enough for one chunk";
const CLIENTS: usize = 16;

fn routes() -> Endpoints {
    Endpoints::new().get("/api/test/file", file)
}

async fn file(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let path = state.settings().data_root.join("payload.bin");
    let file = tokio::fs::File::open(path).await.expect("the file is there");
    answer(
        &headers,
        Download {
            file,
            length: CONTENT.len() as u64,
            content_type: "application/octet-stream",
            file_name: "payload.bin".to_string(),
            modified: Timestamp::from_unix_seconds(1_700_000_000).unwrap(),
        },
    )
    .await
}

/// What went wrong with one request, as the client saw it.
#[derive(Default)]
struct Failures {
    nothing: AtomicU64,
    short_body: AtomicU64,
    other: AtomicU64,
}

/// Reads one answer from a connection: its status and its body, as long as the
/// head says. `None` when the connection ended before the head was whole.
async fn read_answer(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> Option<(u16, Vec<u8>)> {
    let end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let status = head.split(' ').nth(1)?.parse().ok()?;
    let length = head
        .lines()
        .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
        .unwrap_or(0usize);
    buffer.drain(..end + 4);
    let mut body = std::mem::take(buffer);
    while body.len() < length {
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return Some((status, body)),
            Ok(read) => body.extend_from_slice(&chunk[..read]),
        }
    }
    *buffer = body.split_off(length);
    Some((status, body))
}

fn tally(failures: &Failures, answer: Option<(u16, Vec<u8>)>) {
    match answer {
        None => failures.nothing.fetch_add(1, Ordering::Relaxed),
        Some((200, body)) if body == CONTENT => 0,
        Some((200, _)) => failures.short_body.fetch_add(1, Ordering::Relaxed),
        Some(_) => failures.other.fetch_add(1, Ordering::Relaxed),
    };
}

async fn hammer(requests_per_client: usize) {
    let server = Server::in_process(routes(), &[]).await;
    std::fs::write(server.folder().join("payload.bin"), CONTENT).unwrap();
    let address = server.address();
    let failures = Arc::new(Failures::default());
    let mut clients = Vec::new();
    for client in 0..CLIENTS {
        let failures = Arc::clone(&failures);
        clients.push(tokio::spawn(async move {
            // Half of the clients open a connection for each request and half
            // keep one.
            let mut kept: Option<(TcpStream, Vec<u8>)> = None;
            for _ in 0..requests_per_client {
                if client % 2 == 0 {
                    let answered = raw_get(address, b"/api/test/file", &[]).await;
                    tally(
                        &failures,
                        (answered.status != 0).then_some((answered.status, answered.body)),
                    );
                    continue;
                }
                if kept.is_none() {
                    kept = Some((TcpStream::connect(address).await.unwrap(), Vec::new()));
                }
                let (stream, buffer) = kept.as_mut().unwrap();
                let sent = stream
                    .write_all(b"GET /api/test/file HTTP/1.1\r\nHost: x\r\n\r\n")
                    .await;
                let answered = if sent.is_ok() { read_answer(stream, buffer).await } else { None };
                if answered.as_ref().is_none_or(|(status, body)| *status != 200 || body != CONTENT) {
                    kept = None;
                }
                tally(&failures, answered);
            }
        }));
    }
    for client in clients {
        client.await.unwrap();
    }
    let (nothing, short, other) = (
        failures.nothing.load(Ordering::Relaxed),
        failures.short_body.load(Ordering::Relaxed),
        failures.other.load(Ordering::Relaxed),
    );
    assert_eq!(
        (nothing, short, other),
        (0, 0, 0),
        "of {} requests: {nothing} got no answer, {short} a body that was not whole, {other} another status",
        CLIENTS * requests_per_client
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_small_file_arrives_whole_from_sixteen_clients_at_once() {
    hammer(1000).await;
}

/// 16 clients, `STELLARR_STRESS_REQUESTS` requests each (12500, 200 000 in all,
/// when it is not set):
///
/// ```sh
/// cargo test -p stellarr-api --test api streaming::two_hundred_thousand -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "sends 200 000 requests: see the doc comment"]
async fn two_hundred_thousand_downloads_arrive_whole() {
    let each = std::env::var("STELLARR_STRESS_REQUESTS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(12_500);
    hammer(each).await;
}
