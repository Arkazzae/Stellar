//! Whether a form post comes from the page this server served.
//!
//! A browser says where a form came from in `Origin`, or, when it says no
//! such thing, in `Referer`. The post is accepted when what it names is this
//! server's own scheme and host, written as the proxy in front of the server
//! says them. A request with neither header is not from a browser (a script, a
//! test) and is accepted: the form's token, which only the page carries, is
//! what it still has to present.

use axum::http::{HeaderMap, header};

use crate::http::ClientInfo;

/// The scheme and host the request was made to, as the proxy says them, else
/// as the `Host` header does.
fn expected(headers: &HeaderMap, client: &ClientInfo) -> String {
    let host = client.host.clone().unwrap_or_else(|| {
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string()
    });
    format!("{}://{host}", client.scheme)
}

/// A header written on one line or several: its non-empty values joined with
/// commas.
fn joined(headers: &HeaderMap, name: header::HeaderName) -> String {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn is_same_origin(headers: &HeaderMap, client: &ClientInfo) -> bool {
    let expected = expected(headers, client);
    let origin = joined(headers, header::ORIGIN);
    if !origin.is_empty() {
        return origin.trim_end_matches('/').eq_ignore_ascii_case(&expected);
    }
    let referer = joined(headers, header::REFERER);
    if referer.is_empty() {
        return true;
    }
    match scheme_and_authority(&referer) {
        Some(found) => found.eq_ignore_ascii_case(&expected),
        None => false,
    }
}

/// `scheme://authority` of an absolute address, the way an address library
/// writes it: no user name, the host in lower case, and no port that is the
/// scheme's own. None when the text is no absolute address.
fn scheme_and_authority(text: &str) -> Option<String> {
    let text = text.trim();
    let (scheme, rest) = text.split_once("://")?;
    let mut letters = scheme.chars();
    if !letters.next()?.is_ascii_alphabetic()
        || !letters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or("");
    let host_and_port = authority.rsplit('@').next().unwrap_or("");
    let (host, port) = if host_and_port.starts_with('[') {
        let end = host_and_port.find(']')?;
        (&host_and_port[..=end], host_and_port[end + 1..].strip_prefix(':'))
    } else {
        match host_and_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_and_port, None),
        }
    };
    let default = match scheme.as_str() {
        "http" | "ws" => "80",
        "https" | "wss" => "443",
        "ftp" => "21",
        _ => "",
    };
    let port = port.filter(|port| !port.is_empty() && *port != default);
    let host = host.to_ascii_lowercase();
    Some(match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn client(scheme: &str, host: Option<&str>) -> ClientInfo {
        ClientInfo {
            address: "127.0.0.1".parse().unwrap(),
            scheme: scheme.to_string(),
            host: host.map(str::to_string),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    fn accepted(pairs: &[(&str, &str)]) -> bool {
        let mut all = vec![("host", "chat.test:8443")];
        all.extend_from_slice(pairs);
        is_same_origin(&headers(&all), &client("https", None))
    }

    #[test]
    fn a_post_that_names_no_origin_is_not_from_a_browser_and_is_let_through() {
        assert!(accepted(&[]));
    }

    #[test]
    fn origin_is_this_servers_scheme_and_host_in_any_case_and_with_slashes_after() {
        assert!(accepted(&[("origin", "https://chat.test:8443")]));
        assert!(accepted(&[("origin", "HTTPS://Chat.Test:8443//")]));
        for refused in [
            "http://chat.test:8443",
            "https://chat.test",
            "https://chat.test:8443/x",
            "https://evil.test",
            "https://chat.test:8443.evil.test",
            "null",
        ] {
            assert!(!accepted(&[("origin", refused)]), "{refused}");
        }
    }

    #[test]
    fn origin_wins_over_referer_and_two_origins_are_one_text() {
        assert!(accepted(&[("origin", "https://chat.test:8443"), ("referer", "https://evil.test/")]));
        assert!(!accepted(&[("origin", "https://evil.test"), ("referer", "https://chat.test:8443/")]));
        assert!(!accepted(&[("origin", "https://chat.test:8443"), ("origin", "https://evil.test")]));
        // An empty origin says nothing, and the referer is looked at.
        assert!(!accepted(&[("origin", ""), ("referer", "https://evil.test")]));
        assert!(accepted(&[("origin", ""), ("referer", "https://chat.test:8443/a")]));
    }

    #[test]
    fn referer_is_read_as_an_address_with_its_authority_only() {
        for good in [
            "https://chat.test:8443/a/b?c#d",
            "HTTPS://CHAT.TEST:8443/x",
            "https://user:pw@chat.test:8443/x",
            "  https://chat.test:8443/x ",
        ] {
            assert!(accepted(&[("referer", good)]), "{good}");
        }
        for bad in ["garbage", "/x", "https://chat.test/x", "http://chat.test:8443/", "https://", "://x"] {
            assert!(!accepted(&[("referer", bad)]), "{bad}");
        }
    }

    #[test]
    fn the_scheme_and_host_are_the_proxys_word_and_a_scheme_default_port_is_not_written() {
        let page = headers(&[("host", "inner:5000"), ("origin", "https://chat.test")]);
        assert!(is_same_origin(&page, &client("https", Some("chat.test"))));
        assert!(!is_same_origin(&page, &client("http", Some("chat.test"))));
        assert!(!is_same_origin(&page, &client("https", None)));
        let referer = headers(&[("referer", "https://chat.test:443/x")]);
        assert!(is_same_origin(&referer, &client("https", Some("chat.test"))));
        assert_eq!(
            scheme_and_authority("http://[::1]:80/").as_deref(),
            Some("http://[::1]")
        );
    }
}
