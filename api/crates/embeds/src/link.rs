//! An absolute web address, as the previews need to read it.
//!
//! A message is free text and a page names its picture however it likes, so
//! this reads what people really write: any case in the scheme and host,
//! backslashes for slashes, a name in any script, relative references. It is
//! not a general address parser. What it accepts is `http` and `https`, and
//! what it hands on to the client is the ASCII form that the client can
//! send.

use std::fmt;

/// The longest address that is read as a link. Anything longer is text: no
/// page worth a card has an address that long, and the bound keeps the work
/// on one message small.
pub const MAX_LINK_CHARS: usize = 65_519;

/// The longest host name, in characters of its ASCII form.
const MAX_HOST_CHARS: usize = 255;

/// The two schemes that are fetched.
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

/// An absolute `http` or `https` address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    scheme: Scheme,
    user_info: Option<String>,
    /// Lower case, in the script it was written in; an IPv6 address keeps
    /// its brackets.
    host: String,
    /// `None` when absent or the default of the scheme.
    port: Option<u16>,
    /// Starts with `/`, dot segments removed.
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl fmt::Display for Link {
    /// The address in its normal form: lower-case scheme and host, the
    /// default port left out, an empty path written as `/`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://", self.scheme.name())?;
        if let Some(user) = &self.user_info {
            write!(f, "{user}@")?;
        }
        f.write_str(&self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        f.write_str(&self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

impl Link {
    /// Reads an absolute `http` or `https` address. `None` for anything else:
    /// another scheme, no host, a port out of range, characters that cannot
    /// be in a host.
    pub fn parse(text: &str) -> Option<Link> {
        if text.chars().count() > MAX_LINK_CHARS {
            return None;
        }
        let text = text.trim();
        let (scheme, rest) = split_scheme(text)?;
        let rest = rest.strip_prefix("//").or_else(|| {
            // Backslashes stand for slashes in a web address.
            rest.strip_prefix("\\\\")
                .or_else(|| rest.strip_prefix("/\\"))
                .or_else(|| rest.strip_prefix("\\/"))
        })?;

        let authority_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(authority_end);
        let (user_info, host_port) = match authority.rfind('@') {
            Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
            None => (None, authority),
        };
        let (host, port) = split_host_port(host_port)?;
        let host = clean_host(host)?;
        let port = match port {
            None | Some("") => None,
            Some(digits) => {
                if digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let port: u32 = digits.parse().ok()?;
                if port > 65_535 {
                    return None;
                }
                Some(port as u16).filter(|port| *port != scheme.default_port())
            }
        };

        let (before_fragment, fragment) = match tail.split_once('#') {
            Some((before, fragment)) => (before, Some(fragment)),
            None => (tail, None),
        };
        let (raw_path, query) = match before_fragment.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (before_fragment, None),
        };
        let raw_path = raw_path.replace('\\', "/");
        let path = remove_dot_segments(&escape_controls(&raw_path));
        Some(Link {
            scheme,
            user_info: user_info.map(escape_controls),
            host,
            port,
            path,
            query: query.map(escape_controls),
            fragment: fragment.map(escape_controls),
        })
    }

    /// Resolves what a page wrote against the page's own address. `None`
    /// when the result is not an `http` or `https` address, so a `data:` or
    /// `javascript:` reference is dropped here.
    pub fn join(&self, reference: &str) -> Option<Link> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Some(self.without_fragment());
        }
        if let Some(scheme) = scheme_of(reference) {
            let rest = &reference[scheme.len() + 1..];
            let is_web =
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
            // `http:foo` is not an address this reads.
            return if is_web && (rest.starts_with("//") || rest.starts_with('\\')) {
                Link::parse(reference)
            } else {
                None
            };
        }
        let reference = slashes_in_path(reference);
        let origin = self.origin();
        let target = if reference.starts_with("//") {
            format!("{}:{reference}", self.scheme.name())
        } else if reference.starts_with('/') {
            format!("{origin}{reference}")
        } else if reference.starts_with('?') {
            format!("{origin}{}{reference}", self.path)
        } else if reference.starts_with('#') {
            let query = self
                .query
                .as_ref()
                .map_or(String::new(), |q| format!("?{q}"));
            format!("{origin}{}{query}{reference}", self.path)
        } else {
            let directory = &self.path[..=self.path.rfind('/').unwrap_or(0)];
            format!("{origin}{directory}{reference}")
        };
        Link::parse(&target)
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn is_https(&self) -> bool {
        self.scheme == Scheme::Https
    }

    /// The host as a reader knows it: lower case, in its own script.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The path as written, starting with `/`.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// The host without a leading `www.`: how a card names the site.
    pub fn domain(&self) -> &str {
        self.host.strip_prefix("www.").unwrap_or(&self.host)
    }

    /// The address to hand to the HTTP client: the host in ASCII, the
    /// characters an address cannot hold written as `%XX`, and no fragment.
    /// `None` when the host cannot be written in ASCII.
    pub fn fetch_address(&self) -> Option<String> {
        let host = ascii_host(&self.host)?;
        let mut out = format!("{}://", self.scheme.name());
        if let Some(user) = &self.user_info {
            out.push_str(&encode_lenient(user));
            out.push('@');
        }
        out.push_str(&host);
        if let Some(port) = self.port {
            out.push_str(&format!(":{port}"));
        }
        out.push_str(&encode_lenient(&self.path));
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(&encode_lenient(query));
        }
        Some(out)
    }

    fn origin(&self) -> String {
        let mut out = format!("{}://", self.scheme.name());
        if let Some(user) = &self.user_info {
            out.push_str(user);
            out.push('@');
        }
        out.push_str(&self.host);
        if let Some(port) = self.port {
            out.push_str(&format!(":{port}"));
        }
        out
    }

    fn without_fragment(&self) -> Link {
        Link {
            fragment: None,
            ..self.clone()
        }
    }
}

/// `scheme:` of an address, when it starts with one.
fn scheme_of(reference: &str) -> Option<&str> {
    let end = reference.find([':', '/', '?', '#'])?;
    if reference.as_bytes()[end] != b':' {
        return None;
    }
    let scheme = &reference[..end];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

fn split_scheme(text: &str) -> Option<(Scheme, &str)> {
    let scheme = scheme_of(text)?;
    let rest = &text[scheme.len() + 1..];
    if scheme.eq_ignore_ascii_case("http") {
        Some((Scheme::Http, rest))
    } else if scheme.eq_ignore_ascii_case("https") {
        Some((Scheme::Https, rest))
    } else {
        None
    }
}

/// Backslashes before the query stand for slashes.
fn slashes_in_path(reference: &str) -> String {
    let end = reference.find(['?', '#']).unwrap_or(reference.len());
    format!(
        "{}{}",
        reference[..end].replace('\\', "/"),
        &reference[end..]
    )
}

fn split_host_port(host_port: &str) -> Option<(&str, Option<&str>)> {
    if let Some(rest) = host_port.strip_prefix('[') {
        let close = rest.find(']')?;
        let host = &host_port[..close + 2];
        return match &rest[close + 1..] {
            "" => Some((host, None)),
            after => Some((host, Some(after.strip_prefix(':')?))),
        };
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((host_port, None)),
    }
}

/// The host in lower case, or `None` when it has characters that no host
/// has. Characters of every script are fine; what is not is anything that
/// would be punctuation, a control or a space.
fn clean_host(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        let valid = !inner.is_empty()
            && inner.contains(':')
            && inner
                .chars()
                .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.'));
        return valid.then(|| host.to_ascii_lowercase());
    }
    let valid = host.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '-' | '.' | '_')
            || (!c.is_ascii() && c.is_alphanumeric())
    });
    if !valid {
        return None;
    }
    let lower = host.to_lowercase();
    // The ASCII form must be writable, and not longer than a name can be.
    let ascii = ascii_host(&lower)?;
    (ascii.len() <= MAX_HOST_CHARS).then_some(lower)
}

/// The host with every non-ASCII label in punycode.
fn ascii_host(host: &str) -> Option<String> {
    if host.is_ascii() {
        return Some(host.to_owned());
    }
    let mut labels = Vec::new();
    for label in host.split('.') {
        if label.is_ascii() {
            labels.push(label.to_owned());
        } else {
            labels.push(format!("xn--{}", punycode_encode(label)?));
        }
    }
    Some(labels.join("."))
}

/// RFC 3492 section 6.3, for one label.
fn punycode_encode(label: &str) -> Option<String> {
    const BASE: u32 = 36;
    const T_MIN: u32 = 1;
    const T_MAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;

    fn digit(d: u32) -> char {
        if d < 26 {
            (b'a' + d as u8) as char
        } else {
            (b'0' + (d - 26) as u8) as char
        }
    }
    fn adapt(mut delta: u32, points: u32, first: bool) -> u32 {
        delta = if first { delta / DAMP } else { delta / 2 };
        delta += delta / points;
        let mut k = 0;
        while delta > ((BASE - T_MIN) * T_MAX) / 2 {
            delta /= BASE - T_MIN;
            k += BASE;
        }
        k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
    }

    let input: Vec<u32> = label.chars().map(u32::from).collect();
    let mut out: String = label.chars().filter(char::is_ascii).collect();
    let basic = out.len() as u32;
    let mut handled = basic;
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    while (handled as usize) < input.len() {
        let next = input.iter().copied().filter(|c| *c >= n).min()?;
        delta = delta.checked_add((next - n).checked_mul(handled + 1)?)?;
        n = next;
        for &c in &input {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        T_MIN
                    } else if k >= bias + T_MAX {
                        T_MAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Some(out)
}

/// Writes controls as `%XX`: an address with one in it is odd, not unsafe,
/// and a stray control must not reach a log or a card.
fn escape_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c.is_ascii() {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Percent-encodes what the client cannot send as it is: non-ASCII bytes,
/// controls, spaces and the few ASCII characters no address holds.
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

/// RFC 3986 section 5.2.4 on a path that starts with `/`, where `%2e` is a
/// dot as well, so that an encoded `..` cannot slip past a check on the
/// path.
fn remove_dot_segments(path: &str) -> String {
    let mut output: Vec<&str> = Vec::new();
    let mut segments = path.split('/').peekable();
    segments.next();
    while let Some(segment) = segments.next() {
        let last = segments.peek().is_none();
        match dot_kind(segment) {
            Dot::Single => {
                if last {
                    output.push("");
                }
            }
            Dot::Double => {
                output.pop();
                if last {
                    output.push("");
                }
            }
            Dot::None => output.push(segment),
        }
    }
    format!("/{}", output.join("/"))
}

enum Dot {
    None,
    Single,
    Double,
}

fn dot_kind(segment: &str) -> Dot {
    if segment.len() > 6 {
        return Dot::None;
    }
    let lower = segment.to_ascii_lowercase().replace("%2e", ".");
    match lower.as_str() {
        "." => Dot::Single,
        ".." => Dot::Double,
        _ => Dot::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Link {
        Link::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn writes_the_normal_form() {
        assert_eq!(
            parse("HTTPS://Example.ORG").to_string(),
            "https://example.org/"
        );
        assert_eq!(
            parse("https://example.org:443/a").to_string(),
            "https://example.org/a"
        );
        assert_eq!(
            parse("http://example.org:8080/a/./b/../c?q=1#f").to_string(),
            "http://example.org:8080/a/c?q=1#f"
        );
        assert_eq!(
            parse("http://example.org\\a\\b").to_string(),
            "http://example.org/a/b"
        );
    }

    #[test]
    fn refuses_what_is_not_a_web_address() {
        for text in [
            "",
            "javascript:alert(1)",
            "ftp://example.org",
            "file:///etc/passwd",
            "http://",
            "https:///path",
            "http://exa mple.org",
            "http://example.org:99999/",
            "http://example.org:80a/",
            "http://exa!mple.org/",
            "http://exa%6dple.org/",
            "http://[::1/",
            "http://[zz]/",
            "http://a\u{1}b/",
            "http:example.org",
            "example.org/path",
        ] {
            assert!(Link::parse(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn keeps_what_a_reader_sees_of_the_host() {
        assert_eq!(parse("https://www.Example.org/x").domain(), "example.org");
        assert_eq!(parse("https://[::1]:8080/").host(), "[::1]");
        let polish = parse("https://Łódź.example/ścieżka");
        assert_eq!(polish.host(), "łódź.example");
        assert_eq!(polish.to_string(), "https://łódź.example/ścieżka");
    }

    #[test]
    fn user_info_does_not_pass_for_the_host() {
        let link = parse("https://i.ytimg.com@evil.example/vi/x");
        assert_eq!(link.host(), "evil.example");
    }

    // Checked against `"bücher".encode("punycode")` and `"łódź".encode("idna")`.
    #[test]
    fn writes_the_host_in_punycode_for_the_client() {
        assert_eq!(
            parse("https://bücher.example/a b?x=ż#frag")
                .fetch_address()
                .unwrap(),
            "https://xn--bcher-kva.example/a%20b?x=%C5%BC"
        );
        assert_eq!(
            parse("https://łódź.pl/").fetch_address().unwrap(),
            "https://xn--d-uga0v4h.pl/"
        );
        assert_eq!(punycode_encode("münchen").as_deref(), Some("mnchen-3ya"));
        assert_eq!(punycode_encode("日本語").as_deref(), Some("wgv71a119e"));
    }

    #[test]
    fn joins_every_form_of_reference() {
        let base = parse("https://example.org/dir/page?x=1");
        let cases = [
            ("/media/cover.png", "https://example.org/media/cover.png"),
            ("cover.png", "https://example.org/dir/cover.png"),
            ("../cover.png", "https://example.org/cover.png"),
            ("//cdn.example/a.png", "https://cdn.example/a.png"),
            ("?y=2", "https://example.org/dir/page?y=2"),
            ("#top", "https://example.org/dir/page?x=1#top"),
            ("", "https://example.org/dir/page?x=1"),
            ("http://other.example/a.png", "http://other.example/a.png"),
            ("  /padded.png  ", "https://example.org/padded.png"),
            ("\\media\\a.png", "https://example.org/media/a.png"),
        ];
        for (reference, expected) in cases {
            assert_eq!(
                base.join(reference).unwrap().to_string(),
                expected,
                "{reference}"
            );
        }
    }

    #[test]
    fn drops_references_that_leave_the_web() {
        let base = parse("https://example.org/");
        for reference in [
            "javascript:alert(1)",
            "data:image/svg+xml,<svg onload='alert(1)'/>",
            "mailto:a@example.org",
            "localhost:8080/a.png",
            "C:\\windows\\a.png",
            "http:cover.png",
        ] {
            assert!(base.join(reference).is_none(), "{reference}");
        }
    }

    #[test]
    fn an_encoded_dot_segment_is_a_dot_segment() {
        assert_eq!(parse("https://example.org/a/%2e%2E/b").path(), "/b");
    }

    #[test]
    fn a_control_in_a_path_is_written_out_and_one_in_a_host_refuses() {
        assert_eq!(parse("https://example.org/a\u{7}b").path(), "/a%07b");
        assert!(Link::parse("https://exa\u{7}mple.org/").is_none());
    }

    #[test]
    fn a_very_long_address_is_not_a_link() {
        let long = format!("https://example.org/{}", "a".repeat(MAX_LINK_CHARS));
        assert!(Link::parse(&long).is_none());
    }
}
