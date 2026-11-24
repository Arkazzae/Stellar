//! HTTPS against local servers with test certificates, and one real request.

mod support;

use std::time::Duration;

use stellarr_outbound::{AddressPolicy, Client, Error, Request, tls};
use support::{Server, client, fixture, handler};

fn local(url: &str) -> Request {
    Request::get(url)
        .unwrap()
        .policy(AddressPolicy::Anywhere)
        .timeout(Duration::from_secs(10))
}

async fn hello(name: &str) -> Server {
    Server::tls(
        name,
        handler(|mut conn| async move {
            let alpn = conn.alpn.clone().unwrap_or_default();
            conn.ok("text/plain", &alpn).await;
        }),
    )
    .await
}

#[tokio::test]
async fn reads_an_answer_over_tls_by_name_and_offers_only_http_1_1() {
    let server = hello("server").await;

    // `localhost` may resolve to ::1 first; the server listens on 127.0.0.1.
    let answer = client()
        .send_collected(local(&server.url("/")))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 200);
    assert_eq!(answer.body.as_ref(), b"http/1.1");
}

#[tokio::test]
async fn reads_an_answer_over_tls_by_ip_address() {
    let server = hello("server").await;

    let answer = client()
        .send_collected(local(&format!("https://127.0.0.1:{}/", server.addr.port())))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 200);
}

#[tokio::test]
async fn refuses_a_certificate_from_an_authority_it_does_not_trust() {
    let server = hello("untrusted").await;

    let error = client().send(local(&server.url("/"))).await.err().unwrap();

    assert!(matches!(error, Error::Tls { .. }), "{error}");
    assert_eq!(server.hits(), 0);
}

#[tokio::test]
async fn refuses_a_certificate_made_for_another_name() {
    let server = hello("wrong-name").await;

    let error = client().send(local(&server.url("/"))).await.err().unwrap();

    assert!(matches!(error, Error::Tls { .. }), "{error}");
    assert_eq!(server.hits(), 0);
}

#[tokio::test]
async fn refuses_an_expired_certificate() {
    let server = hello("expired").await;

    let error = client().send(local(&server.url("/"))).await.err().unwrap();

    assert!(matches!(error, Error::Tls { .. }), "{error}");
}

#[tokio::test]
async fn a_client_with_the_default_roots_does_not_trust_the_test_authority() {
    // Whatever the machine's bundle holds, it does not hold this root.
    let server = hello("server").await;

    let error = Client::new()
        .send(local(&server.url("/")))
        .await
        .err()
        .unwrap();

    assert!(
        matches!(error, Error::Tls { .. } | Error::Roots(_)),
        "{error}"
    );
}

#[tokio::test]
async fn a_plain_server_behind_an_https_url_fails_instead_of_hanging() {
    // It answers in plain text the moment a connection arrives, as a server
    // that is not expecting TLS does when it finds the hello unreadable.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    });

    let error = client()
        .send(local(&format!("https://127.0.0.1:{port}/")))
        .await
        .err()
        .unwrap();
    task.abort();

    assert!(matches!(error, Error::Tls { .. }), "{error}");
}

#[tokio::test]
async fn the_tls_error_for_a_bad_certificate_names_the_host_and_no_path() {
    let server = hello("untrusted").await;

    let error = client()
        .send(local(&format!(
            "{}?token=hunter2",
            server.url("/capability")
        )))
        .await
        .err()
        .unwrap();

    let shown = format!("{error}");
    assert!(
        !shown.contains("hunter2") && !shown.contains("capability"),
        "{shown}"
    );
}

#[test]
fn the_fixture_root_loads() {
    let roots = tls::roots_from_pem(&fixture("ca.crt.txt")).unwrap();
    assert_eq!(roots.len(), 1);
}

/// One real HTTPS request to a public site, to prove that the system's
/// certificate bundle is found and trusted on this machine. It needs the
/// network, so it only runs when asked: `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "needs the network"]
async fn reads_a_public_site_with_the_system_roots() {
    let client = Client::new();
    client.load_roots().await.unwrap();

    let answer = client
        .send_collected(
            Request::get("https://example.com/")
                .unwrap()
                .policy(AddressPolicy::PublicOnly)
                .max_redirects(3)
                .max_response_bytes(1024 * 1024)
                .timeout(Duration::from_secs(20)),
        )
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 200);
    assert!(String::from_utf8_lossy(&answer.body).contains("Example Domain"));
}
