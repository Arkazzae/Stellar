//! What a value has to look like: ports, addresses, release tags and git
//! references, and the order of releases.

pub fn is_port(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 5
        && value.bytes().all(|b| b.is_ascii_digit())
        && !value.starts_with('0')
        && value.parse::<u32>().is_ok_and(|n| n <= 65535)
}

pub fn port_number(value: &str) -> Option<u32> {
    if is_port(value) {
        value.parse().ok()
    } else {
        None
    }
}

pub fn is_ipv4(value: &str) -> bool {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
    {
        return false;
    }
    let octets: Vec<&str> = value.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|octet| {
            (1..=3).contains(&octet.len()) && octet.parse::<u32>().is_ok_and(|n| n <= 255)
        })
}

/// Letters, digits, dots and hyphens, at most 253 characters, no label longer
/// than 63 or starting or ending with a hyphen. Digits and dots alone would be
/// an IP address, which `is_ipv4` judges.
pub fn is_hostname(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 253
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return false;
    }
    value
        .split('.')
        .all(|label| label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-'))
}

pub fn is_address(value: &str) -> bool {
    is_ipv4(value) || is_hostname(value)
}

/// Whether people reach the address only on this machine.
pub fn is_local_address(value: &str) -> bool {
    value == "localhost" || value.starts_with("127.")
}

fn is_tag_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-'
}

/// A docker tag: the image versions are tags, and so are latest and 1.4.
pub fn is_tag(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|&b| is_tag_byte(b))
}

/// A release such as 1.4.0 or 1.5.0-rc.1.
pub fn is_exact_version(value: &str) -> bool {
    exact_version(value).is_some()
}

pub fn exact_version(value: &str) -> Option<semver::Version> {
    if !is_tag(value) {
        return None;
    }
    semver::Version::parse(value).ok()
}

/// A stable release series, such as 1.4.
pub fn release_series(value: &str) -> Option<(u64, u64)> {
    let version = semver::Version::parse(&format!("{value}.0")).ok()?;
    (version.pre.is_empty() && version.build.is_empty()).then_some((version.major, version.minor))
}

/// A git tag or branch name, short of everything git allows.
pub fn is_ref(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 200
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|&b| is_tag_byte(b) || b == b'/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.ends_with('/')
}

/// The bare host in what was typed or passed: a pasted scheme, path or port
/// goes, and letters become lower case.
pub fn normalize_address(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let mut value = lower.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b');
    if let Some((_, rest)) = value.split_once("://") {
        value = rest;
    }
    if let Some(end) = value.find(['/', '?', '#']) {
        value = &value[..end];
    }
    if let Some((_, host)) = value.rsplit_once('@') {
        value = host;
    }
    if let Some((host, _)) = value.rsplit_once(':') {
        value = host;
    }
    value.strip_suffix('.').unwrap_or(value).to_string()
}

/// Whether release `a` is older than release `b`, both exact versions. A
/// pre-release comes before the release of the same number.
pub fn version_older(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a.cmp_precedence(&b).is_lt(),
        _ => false,
    }
}

/// The HTTP bind: a port, or an IPv4 or bracketed IPv6 address and a port.
pub fn is_http_binding(binding: &str) -> bool {
    match binding.rsplit_once(':') {
        None => is_port(binding),
        Some((host, port)) => {
            is_port(port)
                && (is_ipv4(host)
                    || (host.len() > 2
                        && host.starts_with('[')
                        && host.ends_with(']')
                        && host[1..host.len() - 1]
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')))
        }
    }
}

/// The port of a binding: what follows its last colon.
pub fn binding_port(binding: &str) -> &str {
    binding.rsplit_once(':').map_or(binding, |(_, port)| port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports() {
        for good in ["1", "443", "65535"] {
            assert!(is_port(good), "{good}");
        }
        for bad in ["", "0", "08443", "65536", "70000", "abc", "123456", "8 443"] {
            assert!(!is_port(bad), "{bad}");
        }
    }

    #[test]
    fn addresses() {
        assert!(is_ipv4("203.0.113.7"));
        assert!(is_ipv4("010.0.0.1"));
        for bad in [
            "1.2.3.999",
            "1.2.3",
            "1.2.3.4.5",
            ".1.2.3",
            "1..2.3",
            "1.2.3.4.",
            "a.b.c.d",
            "1.2.3.1234",
        ] {
            assert!(!is_ipv4(bad), "{bad}");
        }
        assert!(is_hostname("chat.example.com"));
        assert!(is_hostname("localhost"));
        for bad in [
            "bad_host",
            "-x.example.com",
            "x-.example.com",
            "1.2.3.4",
            "Chat.example.com",
            "",
        ] {
            assert!(!is_hostname(bad), "{bad}");
        }
        assert!(is_hostname(&format!("{}.com", "a".repeat(63))));
        assert!(!is_hostname(&format!("{}.com", "a".repeat(64))));
    }

    #[test]
    fn normalizes_pasted_addresses() {
        assert_eq!(
            normalize_address("https://Chat.Example.com:8443/setup"),
            "chat.example.com"
        );
        assert_eq!(
            normalize_address("  chat.example.com.  "),
            "chat.example.com"
        );
        assert_eq!(normalize_address("user@host.example:22"), "host.example");
        assert_eq!(normalize_address("203.0.113.7"), "203.0.113.7");
        assert_eq!(normalize_address("not a host!"), "not a host!");
    }

    #[test]
    fn tags_and_refs() {
        assert!(is_tag("latest") && is_tag("1.4") && is_tag("1.4.0-rc.1"));
        assert!(!is_tag("bad!tag") && !is_tag("-x") && !is_tag(""));
        assert!(is_exact_version("1.4.0") && is_exact_version("1.5.0-rc.1"));
        assert!(
            !is_exact_version("1.4") && !is_exact_version("latest") && !is_exact_version("1.4.0-")
        );
        assert!(is_ref("main") && is_ref("feature/x") && is_ref("v1.4.0"));
        assert!(!is_ref("../main") && !is_ref("main/") && !is_ref("a//b") && !is_ref("/main"));
    }

    #[test]
    fn release_order() {
        assert!(version_older("1.4.0", "1.5.0"));
        assert!(version_older("1.9.0", "1.10.0"));
        assert!(!version_older("1.5.0", "1.4.0"));
        assert!(!version_older("1.4.0", "1.4.0"));
        assert!(version_older("1.5.0-rc.1", "1.5.0"));
        assert!(!version_older("1.5.0", "1.5.0-rc.1"));
        assert!(version_older("1.5.0-rc.1", "1.5.0-rc.2"));
        assert!(version_older("1.5.0-rc.2", "1.5.0-rc.10"));
        assert!(!version_older("1.5.0-rc.10", "1.5.0-rc.2"));
    }

    #[test]
    fn http_bindings() {
        assert!(
            is_http_binding("8888")
                && is_http_binding("127.0.0.1:8888")
                && is_http_binding("[::1]:8888")
        );
        assert!(
            !is_http_binding("localhost:8888")
                && !is_http_binding("127.0.0.1:0")
                && !is_http_binding("")
        );
        assert_eq!(binding_port("127.0.0.1:8888"), "8888");
        assert_eq!(binding_port("8888"), "8888");
    }
}
