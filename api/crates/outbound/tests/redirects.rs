//! Following redirects: which method and body survive, which headers are
//! forwarded, and the hops that are refused.

mod support;

use std::sync::Arc;
use std::time::Duration;

use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE};
use http::{HeaderName, HeaderValue, Method};
use stellarr_outbound::{AddressPolicy, Body, Client, Error, Request, TableResolver};
use support::{Server, handler};

fn local(url: &str) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(10))
}

/// A server that tells where it is and what it was sent.
async fn echo() -> Server {
    Server::plain(handler(|mut conn| async move {
        let body = conn.read_body().await;
        let line = format!(
            "{} {} body={}",
            conn.request.method,
            conn.request.target,
            body.len()
        );
        conn.ok("text/plain", line.as_bytes()).await;
    }))
    .await
}

/// A server that sends everything to `to`.
async fn sends_to(status: u16, to: String) -> Server {
    Server::plain(handler(move |mut conn| {
        let to = to.clone();
        async move {
            let _ = conn.read_body().await;
            conn.redirect(status, &to).await;
        }
    }))
    .await
}

#[tokio::test]
async fn follows_a_chain_up_to_the_limit_and_reports_where_it_ended() {
    let end = echo().await;
    let b = sends_to(302, end.url("/final?x=1")).await;
    let a = sends_to(301, b.url("/b")).await;

    let answer = Client::new()
        .send_collected(local(&a.url("/a")).max_redirects(2))
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"GET /final?x=1 body=0");
    assert_eq!(answer.url.path_and_query(), "/final?x=1");
    assert_eq!(answer.url.port(), end.addr.port());
}

#[tokio::test]
async fn one_hop_too_many_is_an_error() {
    let end = echo().await;
    let b = sends_to(302, end.url("/")).await;
    let a = sends_to(301, b.url("/")).await;

    let error = Client::new()
        .send(local(&a.url("/")).max_redirects(1))
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::TooManyRedirects), "{error}");
    assert_eq!(end.hits(), 0);
}

#[tokio::test]
async fn a_redirect_loop_stops_at_the_limit() {
    let server = Server::plain(handler(|mut conn| async move {
        let target = if conn.request.target == "/ping" {
            "/pong"
        } else {
            "/ping"
        };
        conn.redirect(302, target).await;
    }))
    .await;

    let error = Client::new()
        .send(local(&server.url("/ping")).max_redirects(5))
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::TooManyRedirects), "{error}");
    assert_eq!(server.hits(), 6);
}

#[tokio::test]
async fn with_no_redirects_allowed_the_redirect_is_the_answer() {
    let end = echo().await;
    let a = sends_to(302, end.url("/")).await;

    let answer = Client::new()
        .send_collected(local(&a.url("/")))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 302);
    assert!(answer.headers.contains_key("location"));
    assert_eq!(end.hits(), 0);
}

#[tokio::test]
async fn a_redirect_without_a_location_is_the_answer() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.respond("302 Found", &[], b"nowhere").await;
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/")).max_redirects(3))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 302);
    assert_eq!(answer.body.as_ref(), b"nowhere");
}

#[tokio::test]
async fn statuses_that_are_not_redirects_are_not_followed_even_with_a_location() {
    let end = echo().await;
    for status in [300u16, 304, 305] {
        let a = sends_to(status, end.url("/")).await;
        let answer = Client::new()
            .send_collected(local(&a.url("/")).max_redirects(3))
            .await
            .unwrap();
        assert_eq!(answer.status.as_u16(), status);
    }
    assert_eq!(end.hits(), 0);
}

#[tokio::test]
async fn a_post_answered_301_302_or_303_is_asked_again_as_a_get_without_its_body() {
    for status in [301u16, 302, 303] {
        let end = echo().await;
        let a = sends_to(status, end.url("/next")).await;

        let answer = Client::new()
            .send_collected(
                Request::post(&a.url("/submit"))
                    .unwrap()
                    .policy(AddressPolicy::Anywhere)
                    .max_redirects(3)
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::bytes(&b"{\"a\":1}"[..])),
            )
            .await
            .unwrap();

        assert_eq!(answer.body.as_ref(), b"GET /next body=0", "status {status}");
        let seen = &end.recorded()[0];
        assert_eq!(seen.header("content-type"), None, "status {status}");
        assert_eq!(seen.header("content-length"), None, "status {status}");
    }
}

#[tokio::test]
async fn a_put_answered_303_becomes_a_get_but_after_302_it_stays_a_put() {
    let end = echo().await;
    let see_other = sends_to(303, end.url("/")).await;
    let found = sends_to(302, end.url("/")).await;
    let put = |url: String| {
        Request::new(Method::PUT, &url)
            .unwrap()
            .policy(AddressPolicy::Anywhere)
            .max_redirects(1)
            .body(Body::bytes(&b"abc"[..]))
    };

    let after_303 = Client::new()
        .send_collected(put(see_other.url("/")))
        .await
        .unwrap();
    let after_302 = Client::new()
        .send_collected(put(found.url("/")))
        .await
        .unwrap();

    assert_eq!(after_303.body.as_ref(), b"GET / body=0");
    assert_eq!(after_302.body.as_ref(), b"PUT / body=3");
}

#[tokio::test]
async fn a_307_and_a_308_repeat_the_request_with_its_body() {
    for status in [307u16, 308] {
        let end = echo().await;
        let a = sends_to(status, end.url("/again")).await;

        let answer = Client::new()
            .send_collected(
                Request::post(&a.url("/"))
                    .unwrap()
                    .policy(AddressPolicy::Anywhere)
                    .max_redirects(1)
                    .body(Body::bytes(&b"payload"[..])),
            )
            .await
            .unwrap();

        assert_eq!(
            answer.body.as_ref(),
            b"POST /again body=7",
            "status {status}"
        );
    }
}

#[tokio::test]
async fn a_307_cannot_repeat_a_stream_and_says_so() {
    let end = echo().await;
    let a = sends_to(307, end.url("/again")).await;

    let error = Client::new()
        .send(
            Request::new(Method::PUT, &a.url("/"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .max_redirects(1)
                .body(Body::stream(&b"streamed"[..], 8)),
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::BodyNotReplayable), "{error}");
    assert_eq!(end.hits(), 0);
}

#[tokio::test]
async fn a_302_drops_a_stream_instead_of_needing_it() {
    let end = echo().await;
    let a = sends_to(302, end.url("/get")).await;

    let answer = Client::new()
        .send_collected(
            Request::post(&a.url("/"))
                .unwrap()
                .policy(AddressPolicy::Anywhere)
                .max_redirects(1)
                .body(Body::stream(&b"streamed"[..], 8)),
        )
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"GET /get body=0");
}

#[tokio::test]
async fn credentials_go_along_on_the_same_origin() {
    let server = Server::plain(handler(|mut conn| async move {
        if conn.request.target == "/start" {
            conn.redirect(302, "/next").await;
        } else {
            conn.ok("text/plain", b"done").await;
        }
    }))
    .await;
    let mut secret = HeaderValue::from_static("hunter2");
    secret.set_sensitive(true);
    Client::new()
        .send_collected(
            local(&server.url("/start"))
                .max_redirects(1)
                .header(AUTHORIZATION, HeaderValue::from_static("Bearer t"))
                .header(COOKIE, HeaderValue::from_static("a=b"))
                .header(HeaderName::from_static("x-api-key"), secret),
        )
        .await
        .unwrap();

    let second = &server.recorded()[1];
    assert_eq!(second.target, "/next");
    assert_eq!(second.header("authorization"), Some("Bearer t"));
    assert_eq!(second.header("cookie"), Some("a=b"));
    assert_eq!(second.header("x-api-key"), Some("hunter2"));
}

#[tokio::test]
async fn credentials_are_not_forwarded_to_another_origin() {
    // Same host, another port is another origin.
    let end = echo().await;
    let a = sends_to(302, end.url("/landing")).await;
    let mut secret = HeaderValue::from_static("hunter2");
    secret.set_sensitive(true);

    Client::new()
        .send_collected(
            local(&a.url("/"))
                .max_redirects(1)
                .header(AUTHORIZATION, HeaderValue::from_static("Bearer t"))
                .header(COOKIE, HeaderValue::from_static("a=b"))
                .header(
                    http::header::PROXY_AUTHORIZATION,
                    HeaderValue::from_static("Basic x"),
                )
                .header(HeaderName::from_static("x-api-key"), secret)
                .header(ACCEPT, HeaderValue::from_static("application/json"))
                .header(
                    HeaderName::from_static("idempotency-key"),
                    HeaderValue::from_static("k1"),
                ),
        )
        .await
        .unwrap();

    let first = &a.recorded()[0];
    assert_eq!(first.header("authorization"), Some("Bearer t"));
    assert_eq!(first.header("x-api-key"), Some("hunter2"));
    let second = &end.recorded()[0];
    assert_eq!(second.header("authorization"), None);
    assert_eq!(second.header("cookie"), None);
    assert_eq!(second.header("proxy-authorization"), None);
    assert_eq!(second.header("x-api-key"), None);
    // What is not a credential is kept.
    assert_eq!(second.header("accept"), Some("application/json"));
    assert_eq!(second.header("idempotency-key"), Some("k1"));
}

#[tokio::test]
async fn credentials_are_not_forwarded_to_another_host_either() {
    let end = echo().await;
    let a = sends_to(302, format!("http://other.test:{}/", end.addr.port())).await;
    let resolver = TableResolver(vec![(
        "other.test".into(),
        vec!["127.0.0.1".parse().unwrap()],
    )]);

    let answer = Client::new()
        .with_resolver(Arc::new(resolver))
        .send_collected(
            local(&a.url("/"))
                .max_redirects(1)
                .header(AUTHORIZATION, HeaderValue::from_static("Bearer t")),
        )
        .await
        .unwrap();

    assert_eq!(answer.url.host(), "other.test");
    let second = &end.recorded()[0];
    assert_eq!(second.header("authorization"), None);
    assert_eq!(
        second.header("host"),
        Some(format!("other.test:{}", end.addr.port()).as_str())
    );
}

#[tokio::test]
async fn credentials_removed_at_one_hop_stay_removed_when_it_comes_back() {
    // origin A -> origin B -> origin A again: the header went at the first
    // hop and must not reappear.
    let a_target = Arc::new(std::sync::Mutex::new(String::new()));
    let b = {
        let a_target = a_target.clone();
        Server::plain(handler(move |mut conn| {
            let to = a_target.lock().unwrap().clone();
            async move { conn.redirect(302, &to).await }
        }))
        .await
    };
    let a = {
        let b_url = b.url("/");
        Server::plain(handler(move |mut conn| {
            let b_url = b_url.clone();
            async move {
                if conn.request.target == "/start" {
                    conn.redirect(302, &b_url).await;
                } else {
                    conn.ok("text/plain", b"back").await;
                }
            }
        }))
        .await
    };
    *a_target.lock().unwrap() = a.url("/home");

    Client::new()
        .send_collected(
            local(&a.url("/start"))
                .max_redirects(3)
                .header(AUTHORIZATION, HeaderValue::from_static("Bearer t")),
        )
        .await
        .unwrap();

    let seen = a.recorded();
    assert_eq!(seen[0].header("authorization"), Some("Bearer t"));
    assert_eq!(seen[1].target, "/home");
    assert_eq!(seen[1].header("authorization"), None);
}

#[tokio::test]
async fn refuses_a_redirect_to_another_scheme() {
    for location in [
        "ftp://127.0.0.1/file",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "gopher://127.0.0.1/",
        "data:text/plain,hi",
    ] {
        let a = sends_to(302, location.into()).await;

        let error = Client::new()
            .send(local(&a.url("/")).max_redirects(3))
            .await
            .err()
            .unwrap();

        assert!(
            matches!(error, Error::RedirectRefused(_)),
            "{location}: {error}"
        );
    }
}

#[tokio::test]
async fn refuses_a_redirect_from_https_to_http() {
    let plain = echo().await;
    let secure = {
        let to = plain.url("/");
        Server::tls(
            "server",
            handler(move |mut conn| {
                let to = to.clone();
                async move { conn.redirect(302, &to).await }
            }),
        )
        .await
    };

    let error = support::client()
        .send(local(&secure.url("/")).max_redirects(3))
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::RedirectRefused(_)), "{error}");
    assert_eq!(plain.hits(), 0);
}

#[tokio::test]
async fn follows_a_redirect_from_http_to_https_and_drops_credentials_there() {
    let secure = Server::tls(
        "server",
        handler(|mut conn| async move { conn.ok("text/plain", b"secure").await }),
    )
    .await;
    let plain = sends_to(301, secure.url("/")).await;

    let answer = support::client()
        .send_collected(
            local(&plain.url("/"))
                .max_redirects(3)
                .header(AUTHORIZATION, HeaderValue::from_static("Bearer t")),
        )
        .await
        .unwrap();

    assert_eq!(answer.body.as_ref(), b"secure");
    assert_eq!(secure.recorded()[0].header("authorization"), None);
}

#[tokio::test]
async fn resolves_every_kind_of_relative_location() {
    let server = Server::plain(handler(|mut conn| async move {
        match conn.request.target.as_str() {
            "/dir/start" => conn.redirect(302, "sibling").await,
            "/dir/sibling" => conn.redirect(302, "../up?x=1").await,
            "/up?x=1" => conn.redirect(302, "//127.0.0.1:1/never").await,
            _ => conn.ok("text/plain", b"?").await,
        }
    }))
    .await;

    let error = Client::new()
        .send(local(&server.url("/dir/start")).max_redirects(5))
        .await
        .err()
        .unwrap();

    let targets: Vec<_> = server.recorded().into_iter().map(|r| r.target).collect();
    assert_eq!(targets, ["/dir/start", "/dir/sibling", "/up?x=1"]);
    // The last hop is scheme-relative to a port nobody listens on.
    assert!(matches!(error, Error::Connect { .. }), "{error}");
}

#[tokio::test]
async fn a_location_with_a_space_or_raw_utf8_still_works() {
    let server = Server::plain(handler(|mut conn| async move {
        if conn.request.target == "/start" {
            conn.write(
                "HTTP/1.1 302 Found\r\nLocation: /caf\u{e9} au lait\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_bytes(),
            )
            .await;
        } else {
            conn.ok("text/plain", b"ok").await;
        }
    }))
    .await;

    let answer = Client::new()
        .send_collected(local(&server.url("/start")).max_redirects(1))
        .await
        .unwrap();

    assert_eq!(answer.url.path_and_query(), "/caf%C3%A9%20au%20lait");
}

#[tokio::test]
async fn the_total_timeout_covers_the_whole_chain() {
    // Each hop takes 300 ms; three of them do not fit in 700 ms.
    let server = Server::plain(handler(|mut conn| async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let next = match conn.request.target.as_str() {
            "/1" => "/2",
            "/2" => "/3",
            _ => "/done",
        };
        conn.redirect(302, next).await;
    }))
    .await;

    let error = Client::new()
        .send(
            local(&server.url("/1"))
                .max_redirects(10)
                .timeout(Duration::from_millis(700)),
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::Timeout), "{error}");
}
