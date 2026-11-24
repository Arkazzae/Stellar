//! What a request puts on the wire and what an answer gives back, over plain
//! HTTP.

mod support;

use std::time::Duration;

use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use http::{HeaderName, HeaderValue, Method};
use stellarr_outbound::{AddressPolicy, Body, Client, Error, Request};
use support::{Server, handler};

fn local(url: &str) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(10))
}

#[tokio::test]
async fn reads_status_headers_and_body() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.respond(
            "201 Created",
            &[("X-Thing", "one"), ("X-Thing", "two")],
            b"hello",
        )
        .await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/made")))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 201);
    assert_eq!(answer.body.as_ref(), b"hello");
    assert_eq!(answer.headers.get_all("x-thing").iter().count(), 2);
    assert_eq!(answer.url.path_and_query(), "/made");
}

#[tokio::test]
async fn sends_the_request_the_way_a_server_expects_it() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.ok("text/plain", b"").await;
    }))
    .await;

    Client::new()
        .send_collected(
            local(&server.url("/path/to?query=1&b=2"))
                .header(USER_AGENT, HeaderValue::from_static("Test/1"))
                .header(ACCEPT, HeaderValue::from_static("application/json")),
        )
        .await
        .unwrap();

    let seen = &server.recorded()[0];
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.target, "/path/to?query=1&b=2");
    assert_eq!(
        seen.header("host"),
        Some(format!("127.0.0.1:{}", server.addr.port()).as_str())
    );
    assert_eq!(seen.header("user-agent"), Some("Test/1"));
    assert_eq!(seen.header("accept"), Some("application/json"));
    assert_eq!(seen.header("accept-encoding"), Some("gzip, deflate"));
    assert_eq!(seen.header("connection"), Some("close"));
    assert_eq!(seen.header("cookie"), None);
    assert_eq!(seen.header("content-length"), None);
}

#[tokio::test]
async fn sends_a_body_from_memory_with_its_length() {
    let server = Server::plain(handler(|mut conn| async move {
        let body = conn.read_body().await;
        conn.ok("text/plain", &body).await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(
            Request::post(&server.url("/emails"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(Body::bytes(&b"{\"a\":1}"[..])),
        )
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"{\"a\":1}");
    let seen = &server.recorded()[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.header("content-length"), Some("7"));
    assert_eq!(seen.header("content-type"), Some("application/json"));
}

#[tokio::test]
async fn an_empty_post_says_so_with_a_zero_length() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.ok("text/plain", b"").await;
    }))
    .await;

    Client::new()
        .send_collected(
            Request::post(&server.url("/v1/sessions"))
                .unwrap()
                .policy(AddressPolicy::Anywhere),
        )
        .await
        .unwrap();

    assert_eq!(server.recorded()[0].header("content-length"), Some("0"));
}

#[tokio::test]
async fn sends_a_stream_of_known_length_in_pieces() {
    let server = Server::plain(handler(|mut conn| async move {
        let body = conn.read_body().await;
        conn.ok("text/plain", body.len().to_string().as_bytes())
            .await;
    }))
    .await;
    let size = 3 * 1024 * 1024 + 17;
    let (mut writer, reader) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let block = vec![7u8; 100_000];
        let mut left = size;
        while left > 0 {
            let n = left.min(block.len());
            writer.write_all(&block[..n]).await.unwrap();
            left -= n;
        }
    });

    let answer = Client::new()
        .send_collected(
            Request::new(Method::PUT, &server.url("/object"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .body(Body::stream(reader, size as u64)),
        )
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), size.to_string().as_bytes());
    assert_eq!(
        server.recorded()[0].header("content-length"),
        Some(size.to_string().as_str())
    );
}

#[tokio::test]
async fn a_stream_that_ends_before_its_length_fails_the_request() {
    let server = Server::plain(handler(|mut conn| async move {
        let _ = conn.read_body().await;
        conn.ok("text/plain", b"").await;
    }))
    .await;

    let error = Client::new()
        .send_collected(
            Request::post(&server.url("/"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .timeout(Duration::from_secs(5))
                .body(Body::stream(&b"short"[..], 100)),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(error, Error::RequestBody(_) | Error::Protocol(_)),
        "{error}"
    );
}

#[tokio::test]
async fn a_head_request_has_no_body_whatever_the_length_says() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 12345\r\nX-Stellarr-Size: 9\r\nConnection: close\r\n\r\n")
            .await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(
            Request::new(Method::HEAD, &server.url("/object"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .max_response_bytes(4096),
        )
        .await
        .unwrap();

    assert_eq!(answer.headers.get("x-stellarr-size").unwrap(), "9");
    assert!(answer.body.is_empty());
}

#[tokio::test]
async fn reads_a_chunked_body() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
              5\r\nhello\r\n1;ext=1\r\n \r\n5\r\nworld\r\n0\r\nTrailer-X: y\r\n\r\n",
        )
        .await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/")))
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"hello world");
}

#[tokio::test]
async fn reads_a_body_that_ends_when_the_connection_does() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nuntil the end")
            .await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/")))
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"until the end");
}

#[tokio::test]
async fn an_answer_in_a_charset_arrives_as_the_bytes_the_server_sent() {
    let page = "<html><head><title>Caf\u{e9} \u{dc}n\u{ef}code</title></head></html>";
    let latin1: Vec<u8> = page.chars().map(|c| c as u8).collect();
    let body = latin1.clone();
    let server = Server::plain(handler(move |mut conn| {
        let body = body.clone();
        async move { conn.ok("text/html; charset=iso-8859-1", &body).await }
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/")))
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), latin1.as_slice());
    assert_eq!(
        answer.headers.get(CONTENT_TYPE).unwrap(),
        "text/html; charset=iso-8859-1"
    );
}

#[tokio::test]
async fn a_compressed_answer_keeps_its_status_but_its_body_is_not_passed_off_as_plain() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.respond(
            "403 Forbidden",
            &[("Content-Encoding", "gzip")],
            b"\x1f\x8b\x08",
        )
        .await;
    }))
    .await;

    let response = Client::new().send(local(&server.url("/"))).await.unwrap();
    assert_eq!(response.status.as_u16(), 403);
    let error = response.body.collect().await.unwrap_err();

    assert!(matches!(error, Error::Body(_)), "{error}");
}

#[tokio::test]
async fn refuses_headers_that_are_the_clients_to_set() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.ok("text/plain", b"").await
    }))
    .await;
    for name in [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "upgrade",
        "expect",
        "te",
        "trailer",
        "accept-encoding",
    ] {
        let error = Client::new()
            .send(
                local(&server.url("/"))
                    .header(HeaderName::from_static(name), HeaderValue::from_static("x")),
            )
            .await
            .err()
            .unwrap();
        assert!(
            matches!(error, Error::ForbiddenHeader(_)),
            "{name}: {error}"
        );
    }
    assert_eq!(server.hits(), 0);
}

#[tokio::test]
async fn several_requests_at_once_do_not_wait_for_each_other() {
    let server = Server::plain(handler(|mut conn| async move {
        if conn.request.target == "/stall" {
            conn.stall().await;
        } else {
            conn.ok("text/plain", b"fine").await;
        }
    }))
    .await;
    let client = Client::new();
    let stalled = {
        let client = client.clone();
        let url = server.url("/stall");
        tokio::spawn(async move {
            client
                .send_collected(local(&url).timeout(Duration::from_secs(30)))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;

    for _ in 0..8 {
        let answer = client
            .send_collected(local(&server.url("/ok")))
            .await
            .unwrap();
        assert_eq!(answer.body.as_ref(), b"fine");
    }
    stalled.abort();
}

#[tokio::test]
async fn debug_output_and_errors_never_show_a_query_or_a_header_value() {
    let mut secret = HeaderValue::from_static("Bearer hunter2");
    secret.set_sensitive(true);
    let request = Request::get("http://127.0.0.1:1/p/capability-token?api_key=hunter2")
        .unwrap()
        .header(AUTHORIZATION, secret);
    let shown = format!("{request:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("capability-token"), "{shown}");

    // Port 1 on loopback: nothing listens.
    let request = Request::get("http://127.0.0.1:1/p/capability-token?api_key=hunter2")
        .unwrap()
        .policy(AddressPolicy::LoopbackOnly)
        .timeout(Duration::from_secs(5));
    let error = Client::new().send(request).await.err().unwrap();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("capability-token"), "{shown}");
}
