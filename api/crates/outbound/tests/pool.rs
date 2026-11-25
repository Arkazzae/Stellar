//! Connection reuse with real TCP and TLS sockets.

mod support;

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use stellarr_outbound::{AddressPolicy, Body, Client, Error, Request, Resolve, TableResolver};
use support::{Server, handler};

fn get(url: &str) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(5))
}

#[tokio::test]
async fn clones_reuse_tcp_and_tls_after_chunked_and_empty_responses() {
    for tls in [false, true] {
        let serve = handler(|mut conn| async move {
            if conn.request.target != "/first" {
                conn.ok("text/plain", b"opened again").await;
                return;
            }
            conn.write(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                  5\r\nfirst\r\n0\r\nX-Trailer: done\r\n\r\n",
            )
            .await;
            conn.next_request().await.unwrap();
            assert_eq!(conn.request.method, "POST");
            assert_eq!(conn.request.target, "/upload");
            assert_eq!(conn.read_body().await, b"data");
            conn.write(b"HTTP/1.1 204 No Content\r\n\r\n").await;
            conn.next_request().await.unwrap();
            assert_eq!(conn.request.target, "/last");
            conn.ok("text/plain", b"same socket").await;
        });
        let (server, client) = if tls {
            (Server::tls("server", serve).await, support::client())
        } else {
            (Server::plain(serve).await, Client::new())
        };

        let first = client
            .send_collected(get(&server.url("/first")))
            .await
            .unwrap();
        assert_eq!(first.body, "first");
        let upload = Request::post(&server.url("/upload"))
            .unwrap()
            .policy(AddressPolicy::Anywhere)
            .body(Body::stream(&b"data"[..], 4))
            .timeout(Duration::from_secs(5));
        let empty = client.clone().send(upload).await.unwrap();
        assert_eq!(empty.status.as_u16(), 204);
        let last = client
            .send_collected(get(&server.url("/last")))
            .await
            .unwrap();
        assert_eq!(last.body, "same socket");
        assert_eq!(server.hits(), 1);
    }
}

#[tokio::test]
async fn closes_idle_sockets_without_another_request() {
    let (closed, observed) = tokio::sync::oneshot::channel();
    let closed = std::sync::Mutex::new(Some(closed));
    let server = Server::plain(handler(move |mut conn| {
        let closed = closed.lock().unwrap().take().unwrap();
        async move {
            conn.write(b"HTTP/1.1 204 No Content\r\n\r\n").await;
            conn.stall().await;
            let _ = closed.send(());
        }
    }))
    .await;
    let client = Client::new();
    client.send_collected(get(&server.url("/"))).await.unwrap();
    assert_eq!(server.closed.load(Ordering::SeqCst), 0);

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), observed)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn an_idle_socket_cannot_cross_origins_or_address_policies() {
    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfresh")
            .await;
        if conn.next_request().await.is_some() {
            conn.ok("text/plain", b"reused").await;
        }
    }))
    .await;
    let client = Client::new().with_resolver(Arc::new(TableResolver(vec![
        ("first.test".into(), vec![server.addr.ip()]),
        ("other.test".into(), vec![server.addr.ip()]),
    ])));
    let port = server.addr.port();
    let first = format!("http://first.test:{port}/");
    let other = format!("http://other.test:{port}/");
    for request in [
        get(&first),
        get(&first).policy(AddressPolicy::LoopbackOnly),
        get(&other),
    ] {
        assert_eq!(client.send_collected(request).await.unwrap().body, "fresh");
    }
    assert_eq!(server.hits(), 3);
}

#[tokio::test]
async fn an_idle_socket_must_match_the_current_dns_answer() {
    struct MovingHost(AtomicUsize);
    impl Resolve for MovingHost {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>> {
            Box::pin(async {
                let last = if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    1
                } else {
                    2
                };
                Ok(vec![IpAddr::from([127, 0, 0, last])])
            })
        }
    }

    let server = Server::plain(handler(|mut conn| async move {
        conn.write(b"HTTP/1.1 204 No Content\r\n\r\n").await;
        if conn.next_request().await.is_some() {
            conn.ok("text/plain", b"wrong address").await;
        }
    }))
    .await;
    let client = Client::new().with_resolver(Arc::new(MovingHost(AtomicUsize::new(0))));
    let url = format!("http://moving.test:{}/", server.addr.port());
    client.send_collected(get(&url)).await.unwrap();
    let error = client.send_collected(get(&url)).await.unwrap_err();
    assert!(matches!(error, Error::Connect { .. }), "{error}");
}

#[tokio::test]
async fn a_sent_post_is_never_repeated_after_a_pooled_socket_breaks() {
    let (post, received) = tokio::sync::oneshot::channel();
    let post = std::sync::Mutex::new(Some(post));
    let server = Server::plain(handler(move |mut conn| {
        let post = post.lock().unwrap().take();
        async move {
            if conn.request.target != "/first" {
                conn.ok("text/plain", b"post was repeated").await;
                return;
            }
            conn.write(b"HTTP/1.1 204 No Content\r\n\r\n").await;
            conn.next_request().await.unwrap();
            assert_eq!(conn.request.method, "POST");
            post.unwrap().send(conn.read_body().await).unwrap();
        }
    }))
    .await;
    let client = Client::new();
    client
        .send_collected(get(&server.url("/first")))
        .await
        .unwrap();
    let request = Request::post(&server.url("/post"))
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .body(Body::bytes("once"))
        .timeout(Duration::from_secs(5));
    let error = client.send_collected(request).await.unwrap_err();
    assert!(matches!(error, Error::Protocol(_)), "{error}");
    assert_eq!(received.await.unwrap(), b"once");
    assert_eq!(server.hits(), 1);
}

#[tokio::test]
async fn a_peer_that_closes_its_idle_socket_gets_a_new_connection() {
    let server = Server::plain(handler(|mut conn| async move {
        if conn.request.target == "/first" {
            // The peer closes without announcing it in the response.
            conn.write(b"HTTP/1.1 204 No Content\r\n\r\n").await;
        } else {
            conn.ok("text/plain", b"fresh").await;
        }
    }))
    .await;
    let client = Client::new();
    client
        .send_collected(get(&server.url("/first")))
        .await
        .unwrap();
    let next = client
        .send_collected(get(&server.url("/next")))
        .await
        .unwrap();
    assert_eq!(next.body, "fresh");
    assert_eq!(server.hits(), 2);
}

#[tokio::test]
async fn an_early_empty_response_closes_the_unfinished_upload() {
    let (closed, observed) = tokio::sync::oneshot::channel();
    let closed = std::sync::Mutex::new(Some(closed));
    let server = Server::plain(handler(move |mut conn| {
        let closed = closed.lock().unwrap().take().unwrap();
        async move {
            conn.write(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                .await;
            conn.stall().await;
            let _ = closed.send(());
        }
    }))
    .await;
    let client = Client::new();
    let (reader, _unwritten) = tokio::io::duplex(1);
    let request = Request::post(&server.url("/upload"))
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .body(Body::stream(reader, 1))
        .timeout(Duration::from_secs(5));
    let response = client.send(request).await.unwrap();
    assert_eq!(response.status.as_u16(), 413);
    tokio::time::timeout(Duration::from_secs(5), observed)
        .await
        .unwrap()
        .unwrap();
}
