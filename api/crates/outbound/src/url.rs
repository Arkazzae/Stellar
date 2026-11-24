//! A URL this client will request, and the resolving of a redirect's
//! `Location` against it.
//!
//! Only `http` and `https`, an ASCII host (a caller converts an
//! internationalised name first), no user name or password in the address
//! and no fragment: the fragment never goes on the wire.

use std::fmt;

use http::Uri;

use crate::error::{Error, Result};

/// The two schemes that are requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    fn name(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

/// An absolute `http` or `https` URL.
#[derive(Clone, PartialEq, Eq)]
pub struct Url {
    scheme: Scheme,
    /// Lower case; an IPv6 address keeps its brackets.
    host: String,
    port: u16,
    /// Starts with `/`; the query is included.
    path_and_query: String,
}

impl Url {
    /// Reads an absolute `http` or `https` URL.
    pub fn parse(text: &str) -> Result<Url> {
        let text = text.trim();
        let text = text.split_once('#').map_or(text, |(before, _)| before);
        let uri: Uri = text
            .parse()
            .map_err(|_| Error::InvalidUrl("it does not parse"))?;
        let scheme = match uri.scheme_str() {
            Some(scheme) if scheme.eq_ignore_ascii_case("http") => Scheme::Http,
            Some(scheme) if scheme.eq_ignore_ascii_case("https") => Scheme::Https,
            Some(_) => return Err(Error::InvalidUrl("only http and https are requested")),
            None => return Err(Error::InvalidUrl("it is not absolute")),
        };
        let authority = uri.authority().ok_or(Error::InvalidUrl("it has no host"))?;
        if authority.as_str().contains('@') {
            return Err(Error::InvalidUrl("it carries a user name or password"));
        }
        let host = authority.host();
        if host.is_empty() {
            return Err(Error::InvalidUrl("it has no host"));
        }
        // What follows the host in the authority is `:port` or nothing. The
        // parser above tolerates a port that does not fit, so read it here.
        let port = match authority.as_str().get(host.len()..) {
            Some("") => scheme.default_port(),
            Some(rest) => rest
                .strip_prefix(':')
                .and_then(|digits| digits.parse::<u16>().ok())
                .filter(|port| *port != 0)
                .ok_or(Error::InvalidUrl("the port is not valid"))?,
            None => return Err(Error::InvalidUrl("it has no host")),
        };
        let path = uri.path();
        let path_and_query = match uri.query() {
            Some(query) => format!("{path}?{query}"),
            None => path.to_owned(),
        };
        Ok(Url {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
            path_and_query: if path_and_query.starts_with('/') {
                path_and_query
            } else {
                format!("/{path_and_query}")
            },
        })
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn is_https(&self) -> bool {
        self.scheme == Scheme::Https
    }

    /// The host as written in a URL: an IPv6 address has its brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The host without brackets, as a resolver or a certificate wants it.
    pub fn bare_host(&self) -> &str {
        self.host
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(&self.host)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The path and the query, which go on the request line.
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    /// The value of the `Host` header.
    pub fn host_header(&self) -> String {
        if self.port == self.scheme.default_port() {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Whether two URLs have the same scheme, host and port.
    pub fn same_origin(&self, other: &Url) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }

    /// Resolves a `Location` against this URL the way RFC 3986 says, for
    /// every form a server writes: absolute, scheme-relative, absolute path,
    /// relative path, query only and empty.
    ///
    /// A location in another scheme than `http` or `https` is refused.
    pub fn join(&self, location: &str) -> Result<Url> {
        let location = encode_lenient(location.trim());
        let location = location
            .split_once('#')
            .map_or(location.as_str(), |(before, _)| before);

        if let Some(scheme) = scheme_of(location) {
            if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
                return Err(Error::RedirectRefused("it leaves http and https"));
            }
            return Url::parse(location).map_err(|_| Error::RedirectRefused("it does not parse"));
        }

        let origin = format!("{}://{}", self.scheme.name(), self.host_header());
        let target = if location.starts_with("//") {
            format!("{}:{location}", self.scheme.name())
        } else if location.starts_with('/') {
            let (path, query) = split_query(location);
            format!("{origin}{}{query}", remove_dot_segments(path))
        } else if location.is_empty() {
            format!("{origin}{}", self.path_and_query)
        } else if location.starts_with('?') {
            let (path, _) = split_query(&self.path_and_query);
            format!("{origin}{path}{location}")
        } else {
            let (base_path, _) = split_query(&self.path_and_query);
            let directory = &base_path[..=base_path.rfind('/').unwrap_or(0)];
            let (path, query) = split_query(location);
            format!(
                "{origin}{}{query}",
                remove_dot_segments(&format!("{directory}{path}"))
            )
        };
        Url::parse(&target).map_err(|_| Error::RedirectRefused("it does not parse"))
    }
}

/// Shows the origin and nothing after it: a path can hold a capability and
/// a query holds keys.
impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}/…", self.scheme.name(), self.host_header())
    }
}

/// The scheme of a reference, when it has one.
fn scheme_of(reference: &str) -> Option<&str> {
    let end = reference.find(':')?;
    let scheme = &reference[..end];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

fn split_query(reference: &str) -> (&str, &str) {
    match reference.find('?') {
        Some(at) => reference.split_at(at),
        None => (reference, ""),
    }
}

/// Servers do write raw UTF-8 and spaces into `Location`. Percent-encode
/// what a URL cannot hold so that the redirect still works.
fn encode_lenient(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte > 0x7e || byte <= b' ' || matches!(byte, b'"' | b'<' | b'>' | b'\\' | b'^' | b'`') {
            out.push_str(&format!("%{byte:02X}"));
        } else {
            out.push(byte as char);
        }
    }
    out
}

/// RFC 3986 section 5.2.4 on a path that starts with `/`.
fn remove_dot_segments(path: &str) -> String {
    let mut output: Vec<&str> = Vec::new();
    let mut segments = path.split('/').peekable();
    // The first segment of an absolute path is the empty one before `/`.
    segments.next();
    while let Some(segment) = segments.next() {
        let last = segments.peek().is_none();
        match segment {
            "." => {
                if last {
                    output.push("");
                }
            }
            ".." => {
                output.pop();
                if last {
                    output.push("");
                }
            }
            other => output.push(other),
        }
    }
    format!("/{}", output.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn parses_the_parts_of_a_url() {
        let url = url("HTTPS://Example.COM:8443/a/b?x=1#frag");
        assert!(url.is_https());
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), 8443);
        assert_eq!(url.path_and_query(), "/a/b?x=1");
        assert_eq!(url.host_header(), "example.com:8443");
        assert_eq!(self::url("http://example.com").path_and_query(), "/");
        assert_eq!(
            self::url("http://example.com?q=1").path_and_query(),
            "/?q=1"
        );
        assert_eq!(self::url("http://example.com").host_header(), "example.com");
        assert_eq!(
            self::url("https://example.com:443/").host_header(),
            "example.com"
        );
    }

    #[test]
    fn keeps_the_brackets_of_an_ipv6_host_only_where_a_url_needs_them() {
        let url = url("http://[::1]:5200/v1/events");
        assert_eq!(url.host(), "[::1]");
        assert_eq!(url.bare_host(), "::1");
        assert_eq!(url.host_header(), "[::1]:5200");
    }

    #[test]
    fn refuses_what_is_not_an_http_url() {
        for text in [
            "",
            "/path",
            "example.com/path",
            "ftp://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://",
            "http:///path",
            "http://user:pass@example.com/",
            "http://user@example.com/",
            "http://exa mple.com/",
            "http://example.com:99999/",
            "http://example.com:0/",
            "http://example.com:/",
        ] {
            assert!(Url::parse(text).is_err(), "{text:?} must be refused");
        }
    }

    #[test]
    fn debug_shows_no_path_and_no_query() {
        let shown = format!("{:?}", url("https://example.com/secret/token?key=hunter2"));
        assert!(!shown.contains("hunter2"));
        assert!(!shown.contains("secret"));
        assert!(shown.contains("example.com"));
    }

    #[test]
    fn same_origin_compares_scheme_host_and_port() {
        let base = url("https://example.com/a");
        assert!(base.same_origin(&url("https://EXAMPLE.com:443/b")));
        assert!(!base.same_origin(&url("http://example.com/a")));
        assert!(!base.same_origin(&url("https://example.com:8443/a")));
        assert!(!base.same_origin(&url("https://www.example.com/a")));
    }

    #[test]
    fn resolves_every_form_of_location() {
        let base = url("https://example.com/a/b/c?old=1");
        let cases = [
            ("https://other.org/x?y=2", "https://other.org/x?y=2"),
            ("//cdn.example.org/x", "https://cdn.example.org/x"),
            ("/root?q=1", "https://example.com/root?q=1"),
            ("/a/./b/../d", "https://example.com/a/d"),
            ("d", "https://example.com/a/b/d"),
            ("d/e?z=1", "https://example.com/a/b/d/e?z=1"),
            ("../d", "https://example.com/a/d"),
            ("../../../../d", "https://example.com/d"),
            (".", "https://example.com/a/b/"),
            ("..", "https://example.com/a/"),
            ("?new=2", "https://example.com/a/b/c?new=2"),
            ("", "https://example.com/a/b/c?old=1"),
            ("/with#fragment", "https://example.com/with"),
            (
                "/caf\u{e9} au lait",
                "https://example.com/caf%C3%A9%20au%20lait",
            ),
        ];
        for (location, expected) in cases {
            assert_eq!(
                base.join(location).unwrap(),
                url(expected),
                "location {location:?}"
            );
        }
    }

    #[test]
    fn a_relative_location_keeps_a_non_default_port() {
        let base = url("http://127.0.0.1:8080/start");
        assert_eq!(
            base.join("/next").unwrap(),
            url("http://127.0.0.1:8080/next")
        );
    }

    #[test]
    fn refuses_a_location_in_another_scheme() {
        let base = url("https://example.com/");
        for location in [
            "ftp://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "gopher://example.com/",
            "localhost:8080/path",
        ] {
            assert!(
                matches!(base.join(location), Err(Error::RedirectRefused(_))),
                "{location:?}"
            );
        }
    }

    #[test]
    fn refuses_a_location_with_credentials() {
        let base = url("https://example.com/");
        assert!(base.join("https://user:pass@evil.example/").is_err());
    }
}
