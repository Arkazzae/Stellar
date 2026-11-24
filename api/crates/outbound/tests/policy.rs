//! The address policy end to end: what is refused is refused before any
//! connection is made, and what is allowed is connected to at the address
//! that was judged.

mod support;

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use stellarr_outbound::{AddressPolicy, Client, Error, Request, Resolve, TableResolver};
use support::{Server, handler};

fn request(url: &str, policy: AddressPolicy) -> Request {
    Request::get(url)
        .unwrap()
        .policy(policy)
        .max_redirects(3)
        .timeout(Duration::from_secs(10))
}

fn table(entries: &[(&str, &[&str])]) -> Arc<TableResolver> {
    Arc::new(TableResolver(
        entries
            .iter()
            .map(|(name, addresses)| {
                (
                    name.to_string(),
                    addresses.iter().map(|a| a.parse().unwrap()).collect(),
                )
            })
            .collect(),
    ))
}

async fn page() -> Server {
    Server::plain(handler(|mut conn| async move {
        conn.ok("text/html", b"<title>Local &amp; private</title>")
            .await;
    }))
    .await
}

#[tokio::test]
async fn public_only_refuses_every_form_of_the_machine_itself() {
    let server = page().await;
    let port = server.addr.port();
    // The last forms are the ones a resolver turns into 127.0.0.1: a short
    // dotted form, hexadecimal, a single number.
    for host in [
        "127.0.0.1",
        "localhost",
        "[::1]",
        "[::ffff:127.0.0.1]",
        "127.1",
        "0x7f.0.0.1",
        "2130706433",
        "0.0.0.0",
    ] {
        let url = format!("http://{host}:{port}/");

        let error = Client::new()
            .send(request(&url, AddressPolicy::PublicOnly))
            .await
            .err()
            .unwrap();

        assert!(
            error.is_address_refused()
                || matches!(error, Error::Resolve { .. } | Error::InvalidUrl(_)),
            "{host}: {error}"
        );
    }
    assert_eq!(server.hits(), 0, "something connected");
}

#[tokio::test]
async fn nothing_listens_at_the_ports_the_old_tests_named_and_the_refusal_comes_first() {
    // The refusal comes before any connection, so nothing needs to listen.
    for url in [
        "http://127.0.0.1:9/",
        "http://localhost:9/",
        "http://[::1]:9/",
    ] {
        let error = Client::new()
            .send(request(url, AddressPolicy::PublicOnly))
            .await
            .err()
            .unwrap();
        assert!(error.is_address_refused(), "{url}: {error}");
    }
}

#[tokio::test]
async fn public_only_refuses_the_private_ranges_by_literal() {
    for host in [
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "224.0.0.1",
        "[fe80::1]",
        "[fd00::1]",
        "[::ffff:10.0.0.1]",
        "[64:ff9b::a00:1]",
        "[2002:a00:1::1]",
    ] {
        let error = Client::new()
            .send(request(
                &format!("http://{host}:80/"),
                AddressPolicy::PublicOnly,
            ))
            .await
            .err()
            .unwrap();
        assert!(error.is_address_refused(), "{host}: {error}");
    }
}

#[tokio::test]
async fn a_public_name_that_resolves_to_a_private_address_is_refused() {
    let server = page().await;
    let port = server.addr.port();
    let client = Client::new().with_resolver(table(&[
        ("looks-public.test", &["10.1.2.3"]),
        ("rebinder.test", &["192.168.0.10", "127.0.0.1"]),
        ("v6.test", &["fd12::1", "::1"]),
        ("mapped.test", &["::ffff:127.0.0.1"]),
        ("nat64.test", &["64:ff9b::7f00:1"]),
        ("empty.test", &[]),
    ]));

    for name in ["looks-public", "rebinder", "v6", "mapped", "nat64", "empty"] {
        let error = client
            .send(request(
                &format!("http://{name}.test:{port}/"),
                AddressPolicy::PublicOnly,
            ))
            .await
            .err()
            .unwrap();
        assert!(error.is_address_refused(), "{name}: {error}");
    }
    assert_eq!(server.hits(), 0, "something connected");
}

#[tokio::test]
async fn a_name_with_private_and_allowed_addresses_is_reached_only_at_the_allowed_one() {
    let server = page().await;
    let port = server.addr.port();
    // 10.0.0.1 is judged and dropped; the socket goes to 127.0.0.1.
    let client = Client::new().with_resolver(table(&[("mixed.test", &["10.0.0.1", "127.0.0.1"])]));

    let answer = client
        .send_collected(request(
            &format!("http://mixed.test:{port}/"),
            AddressPolicy::LoopbackOnly,
        ))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 200);
    assert_eq!(server.hits(), 1);
}

/// Answers the first question with one address and every later one with
/// another, as a host does that changes its DNS answer between the check
/// and the connect.
struct Rebinding {
    calls: AtomicUsize,
    first: IpAddr,
    later: IpAddr,
}

impl Resolve for Rebinding {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![if call == 0 { self.first } else { self.later }])
        })
    }
}

#[tokio::test]
async fn the_connection_goes_to_the_address_that_was_judged_and_the_name_is_asked_once() {
    let server = page().await;
    let port = server.addr.port();
    let resolver = Arc::new(Rebinding {
        calls: AtomicUsize::new(0),
        first: "127.0.0.1".parse().unwrap(),
        later: "10.9.9.9".parse().unwrap(),
    });
    let client = Client::new().with_resolver(resolver.clone());

    let answer = client
        .send_collected(request(
            &format!("http://rebind.test:{port}/"),
            AddressPolicy::LoopbackOnly,
        ))
        .await
        .unwrap();

    assert_eq!(answer.status.as_u16(), 200);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_second_dns_answer_cannot_swap_the_address_of_a_redirect_hop_either() {
    // Hop one is judged at 127.0.0.1 and connected there. Hop two asks again
    // (every hop is its own connection) and gets the private address, which
    // is refused.
    let server = Server::plain(handler(|mut conn| async move {
        conn.redirect(302, "/next").await;
    }))
    .await;
    let port = server.addr.port();
    let resolver = Arc::new(Rebinding {
        calls: AtomicUsize::new(0),
        first: "127.0.0.1".parse().unwrap(),
        later: "10.9.9.9".parse().unwrap(),
    });
    let client = Client::new().with_resolver(resolver.clone());

    let error = client
        .send(request(
            &format!("http://rebind.test:{port}/"),
            AddressPolicy::LoopbackOnly,
        ))
        .await
        .err()
        .unwrap();

    assert!(error.is_address_refused(), "{error}");
    assert_eq!(server.hits(), 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_redirect_to_an_address_the_policy_refuses_is_refused_and_never_connected() {
    let inner = page().await;
    let port = inner.addr.port();
    for target in [
        format!("http://10.0.0.5:{port}/"),
        "http://169.254.169.254/latest/meta-data/".to_string(),
        format!("http://[fe80::1]:{port}/"),
        format!("http://internal.test:{port}/"),
        format!("http://[::ffff:10.0.0.5]:{port}/"),
        format!("http://8.8.8.8:{port}/"),
    ] {
        let first = {
            let target = target.clone();
            Server::plain(handler(move |mut conn| {
                let target = target.clone();
                async move { conn.redirect(302, &target).await }
            }))
            .await
        };
        let client = Client::new().with_resolver(table(&[("internal.test", &["10.0.0.5"])]));

        let error = client
            .send(request(&first.url("/"), AddressPolicy::LoopbackOnly))
            .await
            .err()
            .unwrap();

        assert!(error.is_address_refused(), "{target}: {error}");
        assert_eq!(first.hits(), 1);
    }
    assert_eq!(inner.hits(), 0);
}

#[tokio::test]
async fn anywhere_reaches_a_private_host_only_when_the_operator_allows_it() {
    // The previewer's rule: refused by default, read when allowed.
    let server = page().await;

    let refused = Client::new()
        .send(request(&server.url("/"), AddressPolicy::PublicOnly))
        .await
        .err()
        .unwrap();
    let allowed = Client::new()
        .send_collected(request(&server.url("/"), AddressPolicy::Anywhere))
        .await
        .unwrap();

    assert!(refused.is_address_refused());
    assert_eq!(allowed.body.as_ref(), b"<title>Local &amp; private</title>");
}

#[tokio::test]
async fn loopback_only_refuses_what_is_not_loopback() {
    for url in [
        "http://10.0.0.1:5200/",
        "http://8.8.8.8/",
        "http://[fe80::1]/",
    ] {
        let error = Client::new()
            .send(request(url, AddressPolicy::LoopbackOnly))
            .await
            .err()
            .unwrap();
        assert!(error.is_address_refused(), "{url}: {error}");
    }
}

#[tokio::test]
async fn a_name_that_does_not_exist_is_a_resolve_error_naming_only_the_host() {
    let client = Client::new().with_resolver(table(&[]));

    let error = client
        .send(request(
            "http://nobody.test/secret?key=abc",
            AddressPolicy::PublicOnly,
        ))
        .await
        .err()
        .unwrap();

    assert!(matches!(error, Error::Resolve { .. }), "{error}");
    let shown = error.to_string();
    assert!(shown.contains("nobody.test") && !shown.contains("abc") && !shown.contains("secret"));
}
