//! The HTTP side: negotiation, who may connect, and what a bad upgrade is
//! answered with, against a real server on a real socket.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::response::IntoResponse;
use http::StatusCode;
use serde_json::Value;
use stellarr_hub::{Config, Hub, router};
use stellarr_websocket::frame::{OP_TEXT, encode_frame};
use support::{RS, TestApp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A server whose only account is the cookie `session=ann` (or `bob`).
async fn server() -> (u16, Hub) {
    let hub = Hub::new(&Config::default());
    let app = Arc::new(TestApp::default());
    let router = router(
        "/hubs/app",
        app,
        hub.clone(),
        Config::default(),
        |parts| async move {
            let cookie = parts
                .headers
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            match cookie.strip_prefix("session=") {
                Some(name) if ["ann", "bob"].contains(&name) => {
                    Ok(format!("{name}-{}", parts.uri.query().unwrap_or("")))
                }
                _ => Err((StatusCode::UNAUTHORIZED, "").into_response()),
            }
        },
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (port, hub)
}

struct Reply {
    status: u16,
    head: String,
    body: String,
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(
            tokio::time::timeout(Duration::from_secs(5), stream.read_u8())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    String::from_utf8(head).unwrap()
}

async fn http(port: u16, method: &str, path: &str, headers: &[&str]) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n"
    );
    for header in headers {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut all = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut all))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8(all).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    // Chunked bodies have their sizes on separate lines; the tests only look
    // for text inside the body.
    Reply {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    }
}

const ANN: &str = "Cookie: session=ann";

async fn negotiate(port: u16, query: &str) -> Value {
    let reply = http(port, "POST", &format!("/hubs/app/negotiate{query}"), &[ANN]).await;
    assert_eq!(reply.status, 200);
    serde_json::from_str(reply.body.trim()).unwrap_or_else(|_| {
        // A chunked body: drop the chunk-size lines.
        let json: String = reply.body.lines().filter(|l| l.starts_with('{')).collect();
        serde_json::from_str(&json).unwrap()
    })
}

async fn upgrade(port: u16, path: &str, extra: &[&str]) -> (u16, TcpStream) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{ANN}\r\n"
    );
    for header in extra {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let head = read_head(&mut stream).await;
    (head.split(' ').nth(1).unwrap().parse().unwrap(), stream)
}

async fn read_text_frame(stream: &mut TcpStream) -> String {
    loop {
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).await.unwrap();
        let len = usize::from(head[1] & 0x7f);
        assert!(len < 126);
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await.unwrap();
        if head[0] & 0x0f == OP_TEXT {
            return String::from_utf8(payload).unwrap();
        }
    }
}

#[tokio::test]
async fn negotiation_needs_a_person_and_a_post() {
    let (port, _) = server().await;
    assert_eq!(
        http(port, "POST", "/hubs/app/negotiate?negotiateVersion=1", &[])
            .await
            .status,
        401
    );
    assert_eq!(
        http(
            port,
            "POST",
            "/hubs/app/negotiate?negotiateVersion=1",
            &["Cookie: session=eve"]
        )
        .await
        .status,
        401
    );
    let get = http(
        port,
        "GET",
        "/hubs/app/negotiate?negotiateVersion=1",
        &[ANN],
    )
    .await;
    assert_eq!(get.status, 405);
    assert!(get.head.to_ascii_lowercase().contains("allow: post"));
    assert_eq!(
        http(port, "PUT", "/hubs/app/negotiate", &[ANN])
            .await
            .status,
        405
    );
}

#[tokio::test]
async fn negotiation_answers_with_the_connection_and_only_the_websocket_transport() {
    let (port, _) = server().await;
    let v1 = negotiate(port, "?negotiateVersion=1").await;
    assert_eq!(v1["negotiateVersion"], 1);
    assert_eq!(v1["connectionId"].as_str().unwrap().len(), 22);
    assert_eq!(v1["connectionToken"].as_str().unwrap().len(), 22);
    assert_ne!(v1["connectionId"], v1["connectionToken"]);
    assert_eq!(v1["availableTransports"][0]["transport"], "WebSockets");
    assert_eq!(v1["availableTransports"].as_array().unwrap().len(), 1);
    let v0 = negotiate(port, "").await;
    assert_eq!(v0["negotiateVersion"], 0);
    assert!(v0.get("connectionToken").is_none());
    let bad = negotiate(port, "?negotiateVersion=abc").await;
    assert_eq!(
        bad["error"],
        "The client requested a non-integer protocol version."
    );
}

#[tokio::test]
async fn a_negotiated_connection_connects_once_and_then_only_when_it_is_gone() {
    let (port, hub) = server().await;
    let token = negotiate(port, "?negotiateVersion=1").await["connectionToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/hubs/app?id={token}&serverId=abc");
    let (status, mut first) = upgrade(port, &path, &[]).await;
    assert_eq!(status, 101);
    first
        .write_all(&encode_frame(
            true,
            OP_TEXT,
            format!("{{\"protocol\":\"json\",\"version\":1}}{RS}").as_bytes(),
            Some([1, 2, 3, 4]),
        ))
        .await
        .unwrap();
    assert_eq!(read_text_frame(&mut first).await, format!("{{}}{RS}"));
    // The identity function saw the query string.
    assert_eq!(
        read_text_frame(&mut first).await,
        format!(
            "{{\"type\":1,\"target\":\"Hello\",\"arguments\":[\"ann-id={token}&serverId=abc\"]}}{RS}"
        )
    );
    assert_eq!(hub.connection_count(), 1);

    let (status, _) = upgrade(port, &path, &[]).await;
    assert_eq!(status, 409, "a second socket on the same token");

    drop(first);
    for _ in 0..200 {
        if hub.connection_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(hub.connection_count(), 0);
    let (status, _) = upgrade(port, &path, &[]).await;
    assert_eq!(status, 404, "a finished connection is not revived");
}

#[tokio::test]
async fn a_bad_id_a_missing_id_and_a_refused_person() {
    let (port, _) = server().await;
    assert_eq!(upgrade(port, "/hubs/app?id=nope", &[]).await.0, 404);
    let unauthenticated = http(port, "GET", "/hubs/app?id=nope", &[]).await;
    assert_eq!(unauthenticated.status, 401);
    let plain = http(port, "GET", "/hubs/app", &[ANN]).await;
    assert_eq!(plain.status, 400);
    assert!(plain.body.contains("Connection ID required"));
    let unknown = http(port, "GET", "/hubs/app?id=nope", &[ANN]).await;
    assert_eq!(unknown.status, 404);
    assert!(unknown.body.contains("No Connection with that ID"));
    assert_eq!(http(port, "DELETE", "/hubs/app", &[ANN]).await.status, 400);
    assert_eq!(http(port, "GET", "/hubs/other", &[ANN]).await.status, 404);
}

#[tokio::test]
async fn a_websocket_without_negotiation_is_let_in() {
    let (port, hub) = server().await;
    let (status, mut socket) = upgrade(port, "/hubs/app", &[]).await;
    assert_eq!(status, 101);
    socket
        .write_all(&encode_frame(
            true,
            OP_TEXT,
            format!("{{\"protocol\":\"json\",\"version\":1}}{RS}").as_bytes(),
            Some([1, 2, 3, 4]),
        ))
        .await
        .unwrap();
    assert_eq!(read_text_frame(&mut socket).await, format!("{{}}{RS}"));
    assert_eq!(hub.connection_count(), 1);
}

#[tokio::test]
async fn a_broken_handshake_is_refused_and_does_not_use_up_the_connection() {
    let (port, _) = server().await;
    let token = negotiate(port, "?negotiateVersion=1").await["connectionToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/hubs/app?id={token}");
    // Wrong version: 426.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 8\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{ANN}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let head = read_head(&mut stream).await;
    assert!(head.starts_with("HTTP/1.1 426"), "{head}");
    // A good attempt afterwards still finds its connection.
    assert_eq!(upgrade(port, &path, &[]).await.0, 101);
}

#[tokio::test]
async fn a_negotiated_connection_that_nobody_claims_expires() {
    let hub = Hub::new(&Config::default());
    let config = Config {
        pending_connection_ttl: Duration::from_millis(100),
        ..Config::default()
    };
    let router = router(
        "/hubs/app",
        Arc::new(TestApp::default()),
        hub,
        config,
        |_| async { Ok("ann".to_owned()) },
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let token = negotiate(port, "?negotiateVersion=1").await["connectionToken"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        upgrade(port, &format!("/hubs/app?id={token}"), &[]).await.0,
        404
    );
}

#[tokio::test]
async fn too_many_waiting_connections_are_refused() {
    let hub = Hub::new(&Config::default());
    let config = Config {
        max_pending_connections: 2,
        ..Config::default()
    };
    let router = router(
        "/hubs/app",
        Arc::new(TestApp::default()),
        hub,
        config,
        |_| async { Ok("ann".to_owned()) },
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for _ in 0..2 {
        assert_eq!(
            http(port, "POST", "/hubs/app/negotiate?negotiateVersion=1", &[])
                .await
                .status,
            200
        );
    }
    assert_eq!(
        http(port, "POST", "/hubs/app/negotiate?negotiateVersion=1", &[])
            .await
            .status,
        503
    );
}
