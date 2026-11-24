//! Native WebSocket upgrades and messages against a mounted hub on real sockets.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::response::IntoResponse;
use http::StatusCode;
use serde_json::{Value, json};
use stellarr_hub::{Config, Hub, Outbound, router};
use stellarr_websocket::frame::{OP_BINARY, OP_CLOSE, OP_CONTINUATION, OP_TEXT, encode_frame};
use support::{TestApp, eventually};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A server whose only account is the cookie `session=ann` (or `bob`).
async fn server() -> (u16, Hub) {
    server_with(Config::default(), TestApp::default()).await
}

async fn server_with(config: Config, app: TestApp) -> (u16, Hub) {
    let hub = Hub::new(&config);
    let router = router(
        "/hubs/app",
        Arc::new(app),
        hub.clone(),
        config,
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

async fn http(port: u16, method: &str, path: &str, headers: &[&str]) -> u16 {
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
    text.split_whitespace().nth(1).unwrap().parse().unwrap()
}

const ANN: &str = "Cookie: session=ann";

async fn upgrade(port: u16, path: &str, extra: &[&str]) -> (u16, TcpStream) {
    let (head, stream) = upgrade_response(port, path, Some(ANN), extra).await;
    (head.split(' ').nth(1).unwrap().parse().unwrap(), stream)
}

async fn upgrade_response(
    port: u16,
    path: &str,
    cookie: Option<&str>,
    extra: &[&str],
) -> (String, TcpStream) {
    upgrade_response_with_version(port, path, cookie, extra, 13).await
}

async fn upgrade_response_with_version(
    port: u16,
    path: &str,
    cookie: Option<&str>,
    extra: &[&str],
    version: u8,
) -> (String, TcpStream) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: {version}\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    for header in cookie.into_iter().chain(extra.iter().copied()) {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let head = read_head(&mut stream).await;
    (head, stream)
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
async fn there_is_no_negotiation_endpoint() {
    let (port, _) = server().await;
    for method in ["GET", "POST"] {
        assert_eq!(http(port, method, "/hubs/app/negotiate", &[ANN]).await, 404);
    }
}

#[tokio::test]
async fn an_upgrade_requires_a_session_and_an_upgrade_request() {
    let (port, _) = server().await;
    for cookie in [None, Some("Cookie: session=eve")] {
        let (head, _) = upgrade_response(port, "/hubs/app", cookie, &[NATIVE]).await;
        assert!(head.starts_with("HTTP/1.1 401"));
    }
    assert_eq!(http(port, "GET", "/hubs/app", &[ANN, NATIVE]).await, 400);
    assert_eq!(http(port, "GET", "/hubs/other", &[ANN, NATIVE]).await, 404);
}

#[tokio::test]
async fn an_unsupported_websocket_version_does_not_open_a_connection() {
    let (port, hub) = server().await;
    let (head, _) = upgrade_response_with_version(port, "/hubs/app", Some(ANN), &[NATIVE], 8).await;
    assert!(head.starts_with("HTTP/1.1 426"));
    assert!(
        head.to_ascii_lowercase()
            .contains("\r\nsec-websocket-version: 13\r\n")
    );
    assert_eq!(hub.connection_count(), 0);
    let _socket = native_socket(port, "/hubs/app").await;
    assert_eq!(hub.connection_count(), 1);
}

#[tokio::test]
async fn dropping_a_socket_cleans_it_up_and_a_new_connection_gets_a_new_id() {
    let (port, hub) = server().await;
    let mut first = native_socket(port, "/hubs/app?serverId=abc").await;
    write_frame(
        &mut first,
        true,
        OP_TEXT,
        br#"{"type":"call","id":1,"method":"whoami","args":[]}"#,
    )
    .await;
    assert_eq!(
        native_body(&mut first).await,
        json!({"type":"result","id":1,"value":"ann-serverId=abc"})
    );
    let first_id = hub.connection_ids().pop().unwrap();
    drop(first);
    eventually(|| hub.connection_count() == 0).await;
    assert!(hub.group_members("all").is_empty());

    let _second = native_socket(port, "/hubs/app?serverId=abc").await;
    let second_id = hub.connection_ids().pop().unwrap();
    assert_ne!(first_id, second_id);
}

const NATIVE: &str = "Sec-WebSocket-Protocol: stellarr.v1";

async fn write_frame(socket: &mut TcpStream, fin: bool, opcode: u8, body: &[u8]) {
    socket
        .write_all(&encode_frame(fin, opcode, body, Some([1, 2, 3, 4])))
        .await
        .unwrap();
}

async fn native_body(socket: &mut TcpStream) -> Value {
    let text = tokio::time::timeout(Duration::from_secs(5), read_text_frame(socket))
        .await
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

async fn close_code(socket: &mut TcpStream) -> u16 {
    let mut head = [0; 2];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut head))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head[0] & 0x0f, OP_CLOSE);
    let mut body = vec![0; usize::from(head[1])];
    socket.read_exact(&mut body).await.unwrap();
    u16::from_be_bytes(body[..2].try_into().unwrap())
}

async fn native_socket(port: u16, path: &str) -> TcpStream {
    let (status, mut socket) = upgrade(port, path, &[NATIVE]).await;
    assert_eq!(status, 101);
    assert_eq!(native_body(&mut socket).await, json!({"type":"ready"}));
    assert_eq!(native_body(&mut socket).await["name"], "Hello");
    socket
}

#[tokio::test]
async fn selects_the_native_subprotocol_and_refuses_missing_or_unknown_offers() {
    let (port, _) = server().await;
    for protocol in ["stellarr.v2", "Stellarr.v1", ""] {
        let header = format!("Sec-WebSocket-Protocol: {protocol}");
        assert_eq!(upgrade(port, "/hubs/app", &[&header]).await.0, 400);
    }
    let (head, mut socket) = upgrade_response(
        port,
        "/hubs/app",
        Some(ANN),
        &["Sec-WebSocket-Protocol: another.v1, stellarr.v1"],
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 101"));
    assert!(
        head.to_ascii_lowercase()
            .contains("\r\nsec-websocket-protocol: stellarr.v1\r\n")
    );
    assert_eq!(native_body(&mut socket).await, json!({"type":"ready"}));
    assert_eq!(
        native_body(&mut socket).await,
        json!({"type":"event","name":"Hello","data":"ann-"})
    );
    let (head, _) = upgrade_response(port, "/hubs/app", Some(ANN), &[]).await;
    assert!(head.starts_with("HTTP/1.1 400"));
}

#[tokio::test]
async fn native_calls_and_sends_share_dispatch_order_and_hide_internal_errors() {
    let (port, _) = server().await;
    let mut socket = native_socket(port, "/hubs/app").await;
    let call =
        r#"{"type":"call","id":9007199254740991,"method":"echo","args":["naïve"]}"#.as_bytes();
    let split = call.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
    write_frame(&mut socket, false, OP_TEXT, &call[..split]).await;
    write_frame(&mut socket, true, OP_CONTINUATION, &call[split..]).await;
    for payload in [
        r#"{"type":"send","method":"say","args":["all","hello"]}"#,
        r#"{"type":"call","id":2,"method":"void","args":[]}"#,
        r#"{"type":"call","id":3,"method":"internal","args":[]}"#,
        r#"{"type":"call","id":4,"method":"echo","args":[]}"#,
        r#"{"type":"call","id":5,"method":"absent","args":[]}"#,
        r#"{"type":"call","id":6,"method":"fail","args":[]}"#,
    ] {
        write_frame(&mut socket, true, OP_TEXT, payload.as_bytes()).await;
    }
    for expected in [
        json!({"type":"result","id":9007199254740991_u64,"value":"naïve"}),
        json!({"type":"event","name":"Said","data":"hello"}),
        json!({"type":"result","id":2,"value":null}),
        json!({"type":"error","id":3,"code":"internal"}),
        json!({"type":"error","id":4,"code":"invalid_arguments"}),
        json!({"type":"error","id":5,"code":"unknown_method"}),
        json!({"type":"error","id":6,"code":"visible","message":"Not allowed here."}),
    ] {
        assert_eq!(native_body(&mut socket).await, expected);
    }
}

#[tokio::test]
async fn overlapping_groups_deliver_one_event_to_each_tab() {
    let (port, hub) = server().await;
    let mut first = native_socket(port, "/hubs/app?tab=1").await;
    let mut second = native_socket(port, "/hubs/app?tab=2").await;
    let ids = hub.group_members("all");
    assert_eq!(ids.len(), 2);
    for id in ids {
        hub.add_to_group(&id, "overlap");
    }
    assert_eq!(
        hub.send_to_groups(["all", "overlap"], &Outbound::bare("Update")),
        2
    );
    assert_eq!(hub.send_to_group("all", &Outbound::bare("Done")), 2);
    for socket in [&mut first, &mut second] {
        assert_eq!(
            native_body(socket).await,
            json!({"type":"event","name":"Update","data":null})
        );
        assert_eq!(
            native_body(socket).await,
            json!({"type":"event","name":"Done","data":null})
        );
    }
}

#[tokio::test]
async fn malformed_native_messages_end_the_connection_as_protocol_errors() {
    let (port, _) = server().await;
    for (opcode, message) in [
        (
            OP_TEXT,
            r#"{"type":"call","id":0,"method":"echo","args":[]}"#,
        ),
        (
            OP_TEXT,
            r#"{"type":"call","id":9007199254740992,"method":"echo","args":[]}"#,
        ),
        (
            OP_TEXT,
            r#"{"type":"call","id":0.5,"method":"echo","args":[]}"#,
        ),
        (OP_TEXT, r#"{"type":"send","method":"echo","args":{}}"#),
        (OP_TEXT, r#"{"type":"send","method":"echo","args":[]}{}"#),
        (OP_TEXT, r#"["send","echo",[]]"#),
        (OP_BINARY, r#"{"type":"send","method":"echo","args":[]}"#),
    ] {
        let mut socket = native_socket(port, "/hubs/app").await;
        write_frame(&mut socket, true, opcode, message.as_bytes()).await;
        assert_eq!(close_code(&mut socket).await, 1002);
    }
}

#[tokio::test]
async fn native_message_limit_counts_all_fragments() {
    let (port, _) = server_with(
        Config {
            max_message_size: 96,
            ..Config::default()
        },
        TestApp::default(),
    )
    .await;
    let mut socket = native_socket(port, "/hubs/app").await;
    write_frame(&mut socket, false, OP_TEXT, &[b' '; 64]).await;
    write_frame(&mut socket, true, OP_CONTINUATION, &[b' '; 33]).await;
    assert_eq!(close_code(&mut socket).await, 1009);
}

#[tokio::test]
async fn native_session_loss_and_restart_have_distinct_close_codes() {
    let (port, hub) = server().await;
    let mut socket = native_socket(port, "/hubs/app").await;
    let id = hub.connection_ids().pop().unwrap();
    assert!(hub.end_session(&id));
    assert_eq!(close_code(&mut socket).await, 4401);

    let mut socket = native_socket(port, "/hubs/app").await;
    hub.close_all();
    assert_eq!(close_code(&mut socket).await, 1012);
}

#[tokio::test]
async fn a_failed_native_connect_hook_closes_before_ready() {
    let (port, _) = server_with(
        Config::default(),
        TestApp {
            fail_connect: true,
            ..TestApp::default()
        },
    )
    .await;
    let (status, mut socket) = upgrade(port, "/hubs/app", &[NATIVE]).await;
    assert_eq!(status, 101);
    assert_eq!(close_code(&mut socket).await, 1011);
}

#[tokio::test]
async fn native_sockets_send_a_browser_visible_heartbeat_while_idle() {
    let (port, _) = server_with(
        Config {
            keep_alive_interval: Duration::from_millis(10),
            ..Config::default()
        },
        TestApp::default(),
    )
    .await;
    let mut socket = native_socket(port, "/hubs/app").await;
    assert_eq!(native_body(&mut socket).await, json!({"type":"ping"}));
}
