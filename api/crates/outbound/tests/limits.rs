//! Timeouts, the ceiling on the size of an answer, stalling and malformed
//! servers, and the cases of the link previewer's connection tests that
//! belong to the transport.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use http::{HeaderValue, Method};
use stellarr_outbound::{AddressPolicy, Client, Error, Request, TableResolver};
use support::{Server, handler};

fn local(url: &str) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(10))
}

async fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

// Timeouts ------------------------------------------------------------------

#[tokio::test]
async fn gives_up_on_a_host_that_never_answers() {
    let server = Server::plain(handler(|mut conn| async move { conn.stall().await })).await;
    let clock = Instant::now();

    let error = Client::new()
        .send(local(&server.url("/")).timeout(Duration::from_secs(1)))
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::Timeout), "{error}");
    assert!(error.is_timeout());
    assert!(
        clock.elapsed() < Duration::from_secs(8),
        "took {:?}",
        clock.elapsed()
    );
    // The connection was let go, not left open.
    assert!(eventually(|| server.closed.load(Ordering::SeqCst) == 1).await);
}

#[tokio::test]
async fn a_name_that_does_not_resolve_in_time_is_a_connect_timeout() {
    struct Never;
    impl stellarr_outbound::Resolve for Never {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> std::pin::Pin<
            Box<dyn Future<Output = std::io::Result<Vec<std::net::IpAddr>>> + Send + 'a>,
        > {
            Box::pin(std::future::pending())
        }
    }
    let client = Client::new().with_resolver(Arc::new(Never));
    let clock = Instant::now();

    let error = client
        .send(
            local("http://slow.test/")
                .connect_timeout(Duration::from_millis(300))
                .timeout(Duration::from_secs(10)),
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::ConnectTimeout { .. }), "{error}");
    assert!(clock.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_tls_handshake_that_stalls_counts_against_the_connect_timeout() {
    // The server accepts the connection and never says a word, so the TLS
    // client waits for a ServerHello that does not come.
    let server = Server::plain(handler(|mut conn| async move { conn.stall().await })).await;
    let clock = Instant::now();

    let error = support::client()
        .send(
            Request::get(&format!("https://127.0.0.1:{}/", server.addr.port()))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .connect_timeout(Duration::from_millis(400))
                .timeout(Duration::from_secs(10)),
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::ConnectTimeout { .. }), "{error}");
    assert!(clock.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn the_total_timeout_wins_over_a_longer_connect_timeout() {
    let server = Server::plain(handler(|mut conn| async move { conn.stall().await })).await;

    let error = support::client()
        .send(
            Request::get(&format!("https://127.0.0.1:{}/", server.addr.port()))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_millis(300)),
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::Timeout), "{error}");
}

#[tokio::test]
async fn a_body_that_stalls_after_the_head_times_out_and_the_next_request_finishes() {
    let stalled = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 100\r\n\r\n")
            .await;
        conn.stall().await;
    }))
    .await;
    let healthy = Server::plain(handler(|mut conn| async move {
        conn.ok("text/html", b"<title>Next card</title>").await;
    }))
    .await;
    let client = Client::new();
    let clock = Instant::now();

    let error = client
        .send_collected(local(&stalled.url("/")).timeout(Duration::from_secs(1)))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Timeout), "{error}");
    assert!(clock.elapsed() < Duration::from_secs(8));
    let next = client
        .send_collected(local(&healthy.url("/")))
        .await
        .unwrap();
    assert_eq!(next.body.as_ref(), b"<title>Next card</title>");
}

#[tokio::test]
async fn a_body_dripped_one_byte_at_a_time_still_meets_the_total_timeout() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
            .await;
        for _ in 0..1000 {
            conn.write(b"x").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }))
    .await;
    let clock = Instant::now();

    let error = Client::new()
        .send_collected(local(&server.url("/")).timeout(Duration::from_millis(800)))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Timeout), "{error}");
    assert!(clock.elapsed() < Duration::from_secs(4));
}

#[tokio::test]
async fn dropping_the_request_while_the_body_is_read_closes_the_connection() {
    let (sent, mut seen) = tokio::sync::mpsc::unbounded_channel::<()>();
    let server = Server::plain(handler(move |mut conn| {
        let sent = sent.clone();
        async move {
            conn.write(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 100\r\n\r\n",
            )
            .await;
            let _ = sent.send(());
            conn.stall().await;
        }
    }))
    .await;
    let client = Client::new();
    let pending = tokio::spawn({
        let url = server.url("/");
        async move {
            client
                .send_collected(local(&url).timeout(Duration::from_secs(60)))
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(8), seen.recv())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    pending.abort();

    assert!(pending.await.unwrap_err().is_cancelled());
    assert!(
        eventually(|| server.closed.load(Ordering::SeqCst) == 1).await,
        "the server never saw the connection close"
    );
}

#[tokio::test]
async fn dropping_a_streamed_body_closes_the_connection_too() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\nstart")
            .await;
        conn.stall().await;
    }))
    .await;

    let mut response = Client::new()
        .send(local(&server.url("/")).max_response_bytes(10_000_000))
        .await
        .unwrap();
    let first = response.body.chunk().await.unwrap().unwrap();
    assert_eq!(first.as_ref(), b"start");
    drop(response);

    assert!(eventually(|| server.closed.load(Ordering::SeqCst) == 1).await);
}

// The ceiling ---------------------------------------------------------------

#[tokio::test]
async fn refuses_a_body_that_announces_more_than_the_ceiling_without_reading_it() {
    let server = Server::plain(handler(|mut conn| async move {
        // Only the head is sent; the body would never come.
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5000\r\n\r\n")
            .await;
        conn.stall().await;
    }))
    .await;
    let clock = Instant::now();

    let error = Client::new()
        .send(local(&server.url("/")).max_response_bytes(1024))
        .await
        .err()
        .unwrap();

    assert!(error.is_too_large(), "{error}");
    assert!(clock.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn the_ceiling_counts_the_head_and_the_body_together() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
            .await;
        conn.write(&[b'a'; 1000]).await;
    }))
    .await;

    // The body alone is 1000 bytes. With the head it does not fit in 1020.
    let error = Client::new()
        .send(local(&server.url("/")).max_response_bytes(1020))
        .await
        .err()
        .unwrap();
    assert!(error.is_too_large(), "{error}");

    // It does with room for both.
    let answer = Client::new()
        .send_collected(local(&server.url("/")).max_response_bytes(1100))
        .await
        .unwrap();
    assert_eq!(answer.body.len(), 1000);
}

#[tokio::test]
async fn a_body_exactly_at_the_ceiling_is_read_and_one_byte_more_is_not() {
    let server = Server::plain(handler(|mut conn| async move {
        let size: usize = conn.request.target[1..].parse().unwrap();
        conn.write(format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\n\r\n").as_bytes())
            .await;
        conn.write(&vec![b'a'; size]).await;
    }))
    .await;
    let fetch = |size: usize, ceiling: u64| {
        let url = server.url(&format!("/{size}"));
        async move {
            Client::new()
                .send_collected(local(&url).max_response_bytes(ceiling))
                .await
        }
    };
    // Where the ceiling stops being too low for a body of 100 bytes: the
    // head is what lies between that and 100.
    let mut ceiling = 100;
    while fetch(100, ceiling).await.is_err() {
        ceiling += 1;
        assert!(ceiling < 200, "a body of 100 never fits");
    }
    assert!(
        ceiling - 100 < 80,
        "the head was counted as {} bytes",
        ceiling - 100
    );
    assert!(fetch(101, ceiling).await.unwrap_err().is_too_large());
    assert_eq!(fetch(101, ceiling + 1).await.unwrap().body.len(), 101);
    assert!(fetch(100, ceiling - 1).await.unwrap_err().is_too_large());
}

#[tokio::test]
async fn stops_reading_a_chunked_body_at_the_ceiling() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await;
        for _ in 0..10_000 {
            conn.write(b"400\r\n").await;
            conn.write(&[b'z'; 1024]).await;
            conn.write(b"\r\n").await;
        }
        conn.write(b"0\r\n\r\n").await;
    }))
    .await;

    let error = Client::new()
        .send_collected(local(&server.url("/")).max_response_bytes(64 * 1024))
        .await
        .unwrap_err();

    assert!(error.is_too_large(), "{error}");
}

#[tokio::test]
async fn stops_reading_a_body_that_has_no_length_at_the_ceiling() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .await;
        for _ in 0..10_000 {
            conn.write(&[b'z'; 1024]).await;
        }
    }))
    .await;

    let error = Client::new()
        .send_collected(local(&server.url("/")).max_response_bytes(8 * 1024))
        .await
        .unwrap_err();

    assert!(error.is_too_large(), "{error}");
}

#[tokio::test]
async fn a_body_that_lies_about_a_short_length_is_cut_by_the_framing_and_not_over_read() {
    // Content-Length says 5; ten bytes follow. Only five belong to the body.
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n0123456789")
            .await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/")))
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"01234");
}

#[tokio::test]
async fn refuses_a_head_over_the_ceiling() {
    let server = Server::plain(handler(|mut conn| async move {
        let big = "x".repeat(100_000);
        conn.respond("200 OK", &[("X-Big", &big)], b"").await;
    }))
    .await;

    let error = Client::new()
        .send(local(&server.url("/")).max_response_bytes(4096))
        .await
        .err()
        .unwrap();

    assert!(error.is_too_large(), "{error}");
}

#[tokio::test]
async fn a_head_that_never_ends_is_cut_off() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\n").await;
        for _ in 0..100_000 {
            conn.write(b"X-Filler: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n")
                .await;
        }
    }))
    .await;
    let clock = Instant::now();

    let error = Client::new()
        .send(local(&server.url("/")).max_response_bytes(1_000_000))
        .await
        .err()
        .unwrap();

    assert!(
        error.is_too_large() || matches!(error, Error::Protocol(_)),
        "{error}"
    );
    assert!(clock.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn read_up_to_gives_the_start_of_a_page_and_says_there_was_more() {
    // The previewer reads a page as far as its cap: what a card needs is in
    // the head, and the head comes first.
    let mut page = b"<meta property=\"og:title\" content=\"Streamed\">".to_vec();
    page.extend(vec![b' '; 2048]);
    page.extend_from_slice(b"<meta property=\"og:description\" content=\"Past the cap\">");
    let body = page.clone();
    let server = Server::plain(handler(move |mut conn| {
        let body = body.clone();
        async move {
            conn.write(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n")
                .await;
            conn.write(&body).await;
        }
    }))
    .await;

    let mut response = Client::new()
        .send(local(&server.url("/")).max_response_bytes(1_000_000))
        .await
        .unwrap();
    let (start, more) = response.body.read_up_to(1024).await.unwrap();

    assert!(more);
    assert_eq!(start.as_ref(), &page[..1024]);
    assert!(!String::from_utf8_lossy(&start).contains("Past the cap"));
}

#[tokio::test]
async fn read_up_to_says_nothing_more_when_the_body_fits_exactly() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.ok("text/html", &[b'a'; 100]).await;
    }))
    .await;

    let mut response = Client::new().send(local(&server.url("/"))).await.unwrap();
    let (all, more) = response.body.read_up_to(100).await.unwrap();

    assert_eq!(all.len(), 100);
    assert!(!more);
}

#[tokio::test]
async fn an_image_over_the_cap_is_dropped_whether_it_announced_its_size_or_not() {
    let image = vec![0x89u8; 5000];
    for announced in [true, false] {
        let body = image.clone();
        let server = Server::plain(handler(move |mut conn| {
            let body = body.clone();
            async move {
                if announced {
                    conn.ok("image/png", &body).await;
                } else {
                    conn.write(
                        b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                    conn.write(&body).await;
                }
            }
        }))
        .await;

        let error = Client::new()
            .send_collected(local(&server.url("/")).max_response_bytes(1024))
            .await
            .unwrap_err();

        assert!(error.is_too_large(), "announced={announced}: {error}");
    }
}

// Malformed answers ---------------------------------------------------------

#[tokio::test]
async fn malformed_answers_fail_cleanly() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("nothing at all", b"".to_vec()),
        ("garbage", b"\x00\x01\x02 not http at all\r\n\r\n".to_vec()),
        (
            "tls hello",
            b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc".to_vec(),
        ),
        ("no status line", b"Content-Length: 3\r\n\r\nabc".to_vec()),
        ("wrong version", b"HTTP/9.9 200 OK\r\n\r\n".to_vec()),
        ("status too small", b"HTTP/1.1 99 Low\r\n\r\n".to_vec()),
        ("status not a number", b"HTTP/1.1 abc OK\r\n\r\n".to_vec()),
        (
            "header without colon",
            b"HTTP/1.1 200 OK\r\nBroken header\r\n\r\n".to_vec(),
        ),
        (
            "bare newlines in a name",
            b"HTTP/1.1 200 OK\r\nA\x00B: c\r\n\r\n".to_vec(),
        ),
        (
            "two different lengths",
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\nabcd".to_vec(),
        ),
        (
            "length not a number",
            b"HTTP/1.1 200 OK\r\nContent-Length: ten\r\n\r\n".to_vec(),
        ),
        (
            "negative length",
            b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n".to_vec(),
        ),
        (
            "closes in the head",
            b"HTTP/1.1 200 OK\r\nContent-Ty".to_vec(),
        ),
        ("head without end", b"HTTP/1.1 200 OK\r\nX: y\r\n".to_vec()),
        (
            "length and chunked together",
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"
                .to_vec(),
        ),
        (
            "unknown transfer coding",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: weird\r\n\r\nabc".to_vec(),
        ),
    ];
    for (name, bytes) in cases {
        let server = Server::plain(handler(move |mut conn| {
            let bytes = bytes.clone();
            async move { conn.write(&bytes).await }
        }))
        .await;

        let outcome = Client::new()
            .send_collected(local(&server.url("/")).timeout(Duration::from_secs(5)))
            .await;

        // The two that the specification allows are answers: the chunked
        // framing wins over a length, and a coding that is not chunked is
        // read to the end of the connection.
        let allowed =
            name.starts_with("length and chunked") || name.starts_with("unknown transfer");
        match outcome {
            Ok(_) => assert!(allowed, "{name} was accepted"),
            Err(error) => {
                assert!(!allowed, "{name}: {error}");
                assert!(matches!(error, Error::Protocol(_)), "{name}: {error}");
            }
        }
    }
}

#[tokio::test]
async fn a_body_cut_short_is_an_error_not_a_short_body() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nonly a little")
            .await;
    }))
    .await;

    let error = Client::new()
        .send_collected(local(&server.url("/")))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Body(_)), "{error}");
}

#[tokio::test]
async fn a_chunked_body_with_a_bad_size_or_cut_short_is_an_error() {
    for bytes in [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nabc\r\n0\r\n\r\n"[..],
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nabc"[..],
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffffff\r\nabc"[..],
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabcXX0\r\n\r\n"[..],
    ] {
        let bytes = bytes.to_vec();
        let server = Server::plain(handler(move |mut conn| {
            let bytes = bytes.clone();
            async move { conn.write(&bytes).await }
        }))
        .await;

        let error = Client::new()
            .send_collected(local(&server.url("/")).timeout(Duration::from_secs(5)))
            .await
            .unwrap_err();

        assert!(
            matches!(error, Error::Body(_) | Error::Protocol(_)),
            "{error}"
        );
    }
}

#[tokio::test]
async fn a_server_that_answers_before_it_has_read_the_request_is_still_read() {
    // An answer that arrives while a large body is still being sent.
    let server = Server::plain(handler(|mut conn| async move {
        conn.respond("413 Payload Too Large", &[], b"no").await;
    }))
    .await;

    let outcome = Client::new()
        .send_collected(
            Request::new(Method::POST, &server.url("/"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .timeout(Duration::from_secs(5))
                .header(http::header::CONTENT_TYPE, HeaderValue::from_static("x/y"))
                .body(stellarr_outbound::Body::bytes(vec![0u8; 8 * 1024 * 1024])),
        )
        .await;

    // Either the status comes back or the broken write is reported; neither hangs.
    if let Ok(answer) = outcome {
        assert_eq!(answer.status.as_u16(), 413);
    }
}

#[tokio::test]
async fn a_refused_connection_names_the_host_and_not_the_path() {
    let error = Client::new()
        .send(
            Request::get("http://127.0.0.1:1/p?key=secret")
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .timeout(Duration::from_secs(5)),
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::Connect { .. }), "{error}");
    assert!(!error.to_string().contains("secret"));
}

#[tokio::test]
async fn a_second_address_is_tried_when_the_first_refuses() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.ok("text/plain", b"ok").await
    }))
    .await;
    // ::1 has nothing listening on this port, 127.0.0.1 has.
    let resolver = TableResolver(vec![(
        "twice.test".into(),
        vec!["::1".parse().unwrap(), "127.0.0.1".parse().unwrap()],
    )]);
    let client = Client::new().with_resolver(Arc::new(resolver));

    let answer = client
        .send_collected(local(&format!("http://twice.test:{}/", server.addr.port())))
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"ok");
}
