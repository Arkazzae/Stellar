//! The opening handshake against a real hyper server, over an in-memory
//! connection.

use bytes::Bytes;
use http_body_util::Full;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use stellarr_websocket::frame::{OP_TEXT, encode_frame};
use stellarr_websocket::{Config, Event, accept, accept_key, take_upgrade};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

/// A server that upgrades every request it accepts and echoes text back.
fn serve() -> DuplexStream {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let service = service_fn(|request| async move {
        let (mut parts, _) = request.into_parts();
        Ok::<_, std::convert::Infallible>(match accept(&parts) {
            Ok(response) => {
                let on_upgrade = take_upgrade(&mut parts).unwrap();
                tokio::spawn(async move {
                    let (sender, mut receiver) =
                        stellarr_websocket::upgraded(on_upgrade, Config::default())
                            .await
                            .unwrap();
                    while let Ok(Event::Data { bytes, end, .. }) = receiver.next().await {
                        if end {
                            sender.send_text(&bytes).await.unwrap();
                        }
                    }
                });
                response.map(|()| Full::new(Bytes::new()))
            }
            Err(error) => error
                .response()
                .map(|()| Full::new(Bytes::from_static(error.message().as_bytes()))),
        })
    });
    tokio::spawn(async move {
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server), service)
            .with_upgrades()
            .await;
    });
    client
}

/// Sends a request and returns the head of the response, lower-cased header
/// names included, and the stream.
async fn request(lines: &[&str]) -> (String, DuplexStream) {
    let mut client = serve();
    let text = format!("{}\r\n\r\n", lines.join("\r\n"));
    client.write_all(text.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(client.read_u8().await.unwrap());
    }
    (String::from_utf8(head).unwrap(), client)
}

const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

fn good<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut lines = vec![
        "GET /chat HTTP/1.1",
        "Host: example",
        "Upgrade: websocket",
        "Connection: Upgrade",
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
        "Sec-WebSocket-Version: 13",
    ];
    lines.extend_from_slice(extra);
    lines
}

#[test]
fn the_accept_value_is_the_one_in_the_standard() {
    // RFC 6455, section 1.3. The value is also what openssl computes and
    // what a reference server answered for this key.
    assert_eq!(accept_key(KEY), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[tokio::test]
async fn a_good_handshake_switches_protocols_and_then_speaks_frames() {
    let (head, mut client) = request(&good(&[])).await;
    assert!(
        head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
        "{head}"
    );
    let lower = head.to_ascii_lowercase();
    assert!(lower.contains("upgrade: websocket\r\n"));
    assert!(lower.contains("connection: upgrade\r\n"));
    let accept = head
        .lines()
        .find_map(|line| line.strip_prefix("sec-websocket-accept: "))
        .unwrap();
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    client
        .write_all(&encode_frame(true, OP_TEXT, b"ping?", Some([1, 2, 3, 4])))
        .await
        .unwrap();
    let mut echo = [0u8; 7];
    client.read_exact(&mut echo).await.unwrap();
    assert_eq!(echo, [0x81, 5, b'p', b'i', b'n', b'g', b'?']);
}

#[tokio::test]
async fn header_names_and_the_token_lists_are_matched_the_way_http_does() {
    let (head, _) = request(&[
        "GET / HTTP/1.1",
        "host: example",
        "UPGRADE: WebSocket",
        "connection: keep-alive, Upgrade",
        "sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==",
        "SEC-WEBSOCKET-VERSION: 13",
        "Sec-WebSocket-Protocol: chat, superchat",
        "Sec-WebSocket-Extensions: permessage-deflate",
    ])
    .await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let lower = head.to_ascii_lowercase();
    assert!(
        !lower.contains("sec-websocket-protocol"),
        "no subprotocol is chosen"
    );
    assert!(
        !lower.contains("sec-websocket-extensions"),
        "no extension is accepted"
    );
}

#[tokio::test]
async fn another_version_is_answered_with_426_and_the_version_we_speak() {
    let mut lines = good(&[]);
    lines[5] = "Sec-WebSocket-Version: 8";
    let (head, _) = request(&lines).await;
    assert!(head.starts_with("HTTP/1.1 426"), "{head}");
    assert!(
        head.to_ascii_lowercase()
            .contains("sec-websocket-version: 13\r\n")
    );
}

#[tokio::test]
async fn keys_that_are_not_sixteen_bytes_of_base64_are_refused() {
    for key in [
        "Sec-WebSocket-Key: abc",
        "Sec-WebSocket-Key: ",
        "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAA",
        "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        let mut lines = good(&[]);
        lines[4] = key;
        let (head, _) = request(&lines).await;
        assert!(head.starts_with("HTTP/1.1 400"), "{key}: {head}");
    }
}

#[tokio::test]
async fn a_missing_or_repeated_key_is_refused() {
    let mut lines = good(&[]);
    lines.remove(4);
    assert!(request(&lines).await.0.starts_with("HTTP/1.1 400"));
    let (head, _) = request(&good(&["Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ=="])).await;
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
}

#[tokio::test]
async fn a_request_that_does_not_ask_for_an_upgrade_is_refused() {
    let mut lines = good(&[]);
    lines.remove(3);
    assert!(request(&lines).await.0.starts_with("HTTP/1.1 400"));
    let mut lines = good(&[]);
    lines.remove(2);
    assert!(request(&lines).await.0.starts_with("HTTP/1.1 400"));
    let mut lines = good(&[]);
    lines[3] = "Connection: keep-alive";
    assert!(request(&lines).await.0.starts_with("HTTP/1.1 400"));
}

#[tokio::test]
async fn only_get_upgrades() {
    let mut lines = good(&[]);
    lines[0] = "POST /chat HTTP/1.1";
    lines.push("Content-Length: 0");
    let (head, _) = request(&lines).await;
    assert!(head.starts_with("HTTP/1.1 405"), "{head}");
}
