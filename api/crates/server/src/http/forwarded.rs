//! Who is really on the other end: the address, scheme and host a request
//! came with, taken from the `X-Forwarded-*` headers of the proxies that are
//! believed.
//!
//! The API listens on loopback behind the container's nginx, which appends
//! the address of whoever connected to it to `X-Forwarded-For`. With no
//! proxies listed that last entry is the address: the client's own, or the
//! proxy's when one sits in front.
//!
//! With proxies listed (`STELLARR_TRUSTED_PROXIES`), the header is read from
//! the right, one entry for each hop that is trusted. nginx's entry, believed
//! because nginx connects from loopback, names who connected to the
//! container; when that is a listed proxy, the entry before it names who
//! connected to that proxy, and so on, until an address that is not a listed
//! proxy: the client. Past nginx, loopback is an address like any other.
//! What a client writes into the header itself stays to the left of the entry
//! its proxy appended, so it is never reached, and a client that reaches the
//! container without going through a listed proxy is itself the first address
//! that ends the walk.
//!
//! The rate limits, the session records and the hub connections all take the
//! address the walk ends at. The scheme and host are nginx's word, whatever
//! is listed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, header::HeaderName};
use axum::middleware::Next;
use axum::response::Response;

use super::pipeline::Pipeline;
use crate::settings::{TrustedProxies, normalize};

const FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
const FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");

/// Where a request really came from. A request carries it as an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub address: IpAddr,
    /// What the proxy wrote in `X-Forwarded-Proto` (the last entry of it,
    /// trimmed, in the case it was written in, whatever it is), `http` when it
    /// wrote nothing. A scheme that is not `http` or `https` is passed on as it
    /// came, as the first server did; only [`ClientInfo::is_https`] looks at it.
    pub scheme: String,
    /// The host the client asked for, when a proxy said so.
    pub host: Option<String>,
}

impl ClientInfo {
    /// Whether the request came over TLS, which is what cookies and links
    /// depend on. The comparison ignores letter case.
    pub fn is_https(&self) -> bool {
        self.scheme.eq_ignore_ascii_case("https")
    }
}

pub(super) async fn middleware(
    State(pipeline): State<std::sync::Arc<Pipeline>>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |info| info.0.ip());
    let info = resolve(
        peer,
        request.headers(),
        &pipeline.state.settings().operations.trusted_proxies,
    );
    request.extensions_mut().insert(info);
    next.run(request).await
}

/// Reads the headers of one request that arrived from `peer`.
pub fn resolve(peer: IpAddr, headers: &HeaderMap, trusted: &TrustedProxies) -> ClientInfo {
    let scheme = last_value(headers, &FORWARDED_PROTO)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "http".to_string());
    let host = last_value(headers, &FORWARDED_HOST).filter(|value| !value.is_empty());

    let mut entries: Vec<String> = headers
        .get_all(&FORWARDED_FOR)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|entry| entry.trim().to_string())
        .collect();
    let mut address = normalize(peer);

    // The entry nginx appended: believed from loopback when proxies are
    // listed, from anyone when none is, since nobody then vouches for more
    // than the one hop.
    if trusted.is_empty() || is_loopback(address) {
        match entries.pop().as_deref().and_then(parse_entry) {
            Some(client) => address = client,
            None => {
                return ClientInfo {
                    address,
                    scheme,
                    host,
                };
            }
        }
    }
    // Then each hop through a listed proxy.
    while trusted.contains(address) {
        match entries.pop().as_deref().and_then(parse_entry) {
            Some(client) => address = client,
            None => break,
        }
    }
    ClientInfo {
        address,
        scheme,
        host,
    }
}

fn last_value(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .next_back()
        .map(str::to_string)
}

fn is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.octets()[0] == 127,
        IpAddr::V6(v6) => v6 == Ipv6Addr::LOCALHOST,
    }
}

/// An address as a proxy writes it: bare, with a port, or an IPv6 address in
/// brackets with or without one.
fn parse_entry(entry: &str) -> Option<IpAddr> {
    let address = if let Some(rest) = entry.strip_prefix('[') {
        rest.split_once(']')?.0
    } else if entry.matches(':').count() == 1 {
        entry.split_once(':')?.0
    } else {
        entry
    };
    address.parse::<IpAddr>().ok().map(normalize)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const NGINX: &str = "127.0.0.1";
    const PROXY: &str = "172.18.0.1";
    const CLIENT: &str = "198.51.100.7";
    const FORGED: &str = "203.0.113.66";

    fn read(proxies: &str, peer: &str, forwarded_for: &str) -> ClientInfo {
        read_with(proxies, peer, forwarded_for, None, None)
    }

    fn read_with(
        proxies: &str,
        peer: &str,
        forwarded_for: &str,
        proto: Option<&str>,
        host: Option<&str>,
    ) -> ClientInfo {
        let mut headers = HeaderMap::new();
        headers.insert(FORWARDED_FOR, HeaderValue::from_str(forwarded_for).unwrap());
        if let Some(proto) = proto {
            headers.insert(FORWARDED_PROTO, HeaderValue::from_str(proto).unwrap());
        }
        if let Some(host) = host {
            headers.insert(FORWARDED_HOST, HeaderValue::from_str(host).unwrap());
        }
        resolve(
            peer.parse().unwrap(),
            &headers,
            &TrustedProxies::parse(proxies).unwrap(),
        )
    }

    fn address(info: ClientInfo) -> IpAddr {
        info.address
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn with_nothing_listed_the_address_is_whoever_connected_to_the_container() {
        // A client that reaches the container directly.
        assert_eq!(address(read("", NGINX, CLIENT)), ip(CLIENT));
        // Whatever it writes into the header itself.
        assert_eq!(
            address(read("", NGINX, &format!("{FORGED}, {CLIENT}"))),
            ip(CLIENT)
        );
        // A client behind a proxy: the proxy is all the container sees.
        assert_eq!(
            address(read("", NGINX, &format!("{CLIENT}, {PROXY}"))),
            ip(PROXY)
        );
        assert_eq!(
            address(read("", NGINX, &format!("{FORGED}, {CLIENT}, {PROXY}"))),
            ip(PROXY)
        );
    }

    #[test]
    fn a_listed_proxy_names_the_client_and_nothing_the_client_wrote() {
        for forwarded in [
            format!("{CLIENT}, {PROXY}"),
            // What the client wrote stays to the left of what the proxy appended.
            format!("{FORGED}, {CLIENT}, {PROXY}"),
            format!("{FORGED}, 10.0.0.9, {CLIENT}, {PROXY}"),
            // Even a client that writes the proxy's own address in front of its own.
            format!("{FORGED}, {PROXY}, {CLIENT}, {PROXY}"),
        ] {
            assert_eq!(
                address(read(PROXY, NGINX, &forwarded)),
                ip(CLIENT),
                "{forwarded}"
            );
        }
    }

    #[test]
    fn a_sender_that_is_not_a_listed_proxy_changes_nothing() {
        // A client that reaches the container without the proxy is not believed,
        for forwarded in [
            format!("{FORGED}, {CLIENT}"),
            // even when it claims to have come through the proxy.
            format!("{FORGED}, {PROXY}, {CLIENT}"),
            // A proxy that is not listed is the address, as with nothing listed.
            format!("{FORGED}, 192.0.2.44"),
        ] {
            let last = forwarded.rsplit(", ").next().unwrap();
            assert_eq!(
                address(read(PROXY, NGINX, &forwarded)),
                ip(last),
                "{forwarded}"
            );
        }
    }

    #[test]
    fn only_the_containers_nginx_is_believed_without_being_listed() {
        assert_eq!(
            address(read(PROXY, "192.0.2.44", &format!("{FORGED}, {CLIENT}"))),
            ip("192.0.2.44")
        );
    }

    #[test]
    fn nginx_is_believed_on_either_loopback() {
        for peer in [NGINX, "::1", "::ffff:127.0.0.1"] {
            assert_eq!(
                address(read(PROXY, peer, &format!("{CLIENT}, {PROXY}"))),
                ip(CLIENT),
                "{peer}"
            );
        }
    }

    #[test]
    fn loopback_past_nginx_is_not_believed() {
        for loopback in ["127.0.0.1", "::1"] {
            // Something on the proxy's own machine connected to it over
            // loopback and wrote the first entry itself.
            let forwarded = format!("{FORGED}, {loopback}, {PROXY}");
            assert_eq!(
                address(read(PROXY, NGINX, &forwarded)),
                ip(loopback),
                "{loopback}"
            );
        }
    }

    #[test]
    fn loopback_past_nginx_is_believed_once_listed() {
        let forwarded = format!("{CLIENT}, 127.0.0.1, {PROXY}");
        assert_eq!(
            address(read(&format!("{PROXY}, 127.0.0.1"), NGINX, &forwarded)),
            ip(CLIENT)
        );
    }

    #[test]
    fn each_proxy_in_a_chain_has_to_be_listed() {
        let forwarded = format!("{FORGED}, {CLIENT}, 10.0.0.5, {PROXY}");
        // Two proxies in a row, both listed: the client.
        assert_eq!(
            address(read("172.18.0.1, 10.0.0.0/24", NGINX, &forwarded)),
            ip(CLIENT)
        );
        // Only the outer one listed: the inner proxy, which nobody vouches for.
        assert_eq!(
            address(read("172.18.0.1", NGINX, &forwarded)),
            ip("10.0.0.5")
        );
    }

    #[test]
    fn ipv4_and_ipv6_mix_freely() {
        for (proxies, forwarded, expected) in [
            (
                "2001:db8:1::/64",
                "2001:db8:ff::7, 2001:db8:1::10",
                "2001:db8:ff::7",
            ),
            (
                "2001:db8:1::/64",
                "2001:db8:1::99, 2001:db8:ff::7, 2001:db8:1::10",
                "2001:db8:ff::7",
            ),
            (
                "2001:db8:1::10",
                "[2001:db8:ff::7]:4711, 2001:db8:1::10",
                "2001:db8:ff::7",
            ),
            (
                "172.18.0.0/16",
                "2001:db8:ff::7, 172.18.0.5",
                "2001:db8:ff::7",
            ),
            ("fd00::5", "198.51.100.7:4711, fd00::5", CLIENT),
            (
                "2001:db8:1::/64",
                "2001:db8:ff::7, 172.18.0.5",
                "172.18.0.5",
            ),
        ] {
            assert_eq!(
                address(read(proxies, NGINX, forwarded)),
                ip(expected),
                "{proxies} / {forwarded}"
            );
        }
    }

    #[test]
    fn a_scheme_is_passed_on_as_it_was_written() {
        for (written, scheme, https) in [
            ("https", "https", true),
            ("HTTPS", "HTTPS", true),
            ("gopher", "gopher", false),
            ("http, gopher", "gopher", false),
            ("  https  ", "https", true),
            ("", "http", false),
        ] {
            let info = read_with("", NGINX, CLIENT, Some(written), None);
            assert_eq!(
                (info.scheme.as_str(), info.is_https()),
                (scheme, https),
                "{written:?}"
            );
        }
    }

    #[test]
    fn the_scheme_and_host_are_nginxs_whatever_is_listed() {
        for proxies in ["", PROXY] {
            let info = read_with(
                proxies,
                NGINX,
                &format!("{FORGED}, {CLIENT}, {PROXY}"),
                Some("https"),
                Some("forged.example, chat.example.test"),
            );

            assert_eq!(info.scheme, "https");
            assert_eq!(info.host.as_deref(), Some("chat.example.test"));
        }
    }

    #[test]
    fn without_forwarded_headers_the_peer_is_the_client_over_plain_http() {
        let info = resolve(ip(CLIENT), &HeaderMap::new(), &TrustedProxies::default());

        assert_eq!(
            info,
            ClientInfo {
                address: ip(CLIENT),
                scheme: "http".to_string(),
                host: None
            }
        );
    }

    #[test]
    fn an_entry_that_is_not_an_address_ends_the_walk_where_it_is() {
        assert_eq!(
            address(read(PROXY, NGINX, &format!("unknown, {PROXY}"))),
            ip(PROXY)
        );
        assert_eq!(address(read("", NGINX, "not-an-address")), ip(NGINX));
    }
}
