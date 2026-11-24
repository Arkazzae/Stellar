//! The address an operator names for the instance, reduced to what a client
//! needs: scheme, host and a port that is not the scheme's own.

/// `https://Chat.Example.test:443/` is `https://chat.example.test`. None when
/// the text is not an absolute `http` or `https` address, names a user, or
/// has a query or a fragment: a half-understood address would send every
/// client to the wrong place, so the start is refused instead.
pub fn canonical(text: &str) -> Option<String> {
    let (scheme, rest) = text.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    if rest.contains(['?', '#', '\\']) {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') {
        return None;
    }
    let (host, port) = split_port(authority)?;
    let host = canonical_host(host)?;
    Some(match port {
        Some(port) if port != default_port => format!("{scheme}://{host}:{port}"),
        _ => format!("{scheme}://{host}"),
    })
}

/// A name in lower case, an IPv6 address in brackets, or an IPv4 address in
/// whatever way an address may be written (`127.1`, `0x7f.0.0.1`), which is
/// said as four numbers.
fn canonical_host(host: &str) -> Option<String> {
    if let Some(inside) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inside
            .parse::<std::net::Ipv6Addr>()
            .ok()
            .map(|_| format!("[{}]", inside.to_ascii_lowercase()));
    }
    if host.is_empty() {
        return None;
    }
    if let Some(address) = ipv4(host) {
        return Some(address);
    }
    // One empty label at the end is the root, which a name may name.
    let labels = host.strip_suffix('.').unwrap_or(host);
    let usable = !labels.is_empty()
        && labels.split('.').all(|label| {
            !label.is_empty()
                && label.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || c == '-'
                        || c == '_'
                        || (!c.is_ascii() && stellarr_unicode::is_letter_or_digit(c))
                })
        });
    usable.then(|| host.chars().flat_map(char::to_lowercase).collect())
}

/// An address written as one to four numbers, each in decimal, octal (a
/// leading 0) or hexadecimal (0x), the last standing for all the bytes that
/// are left.
fn ipv4(host: &str) -> Option<String> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() > 4 {
        return None;
    }
    let mut numbers = Vec::new();
    for part in &parts {
        let lower = part.to_ascii_lowercase();
        let (digits, radix) = match lower.strip_prefix("0x") {
            Some(hex) => (hex.to_string(), 16),
            None if lower.len() > 1 && lower.starts_with('0') => (lower[1..].to_string(), 8),
            None => (lower, 10),
        };
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        numbers.push(u64::from_str_radix(&digits, radix).ok()?);
    }
    let (last, leading) = numbers.split_last()?;
    if leading.iter().any(|n| *n > 255) || *last >= 1u64 << (8 * (5 - parts.len())) {
        return None;
    }
    let mut address = *last;
    for (index, number) in leading.iter().enumerate() {
        address |= number << (8 * (3 - index));
    }
    Some(format!(
        "{}.{}.{}.{}",
        address >> 24,
        address >> 16 & 255,
        address >> 8 & 255,
        address & 255
    ))
}

/// The host and the port after it, which may be left empty (`host:`).
fn split_port(authority: &str) -> Option<(&str, Option<u16>)> {
    let colon = match authority.rfind(']') {
        Some(end) => authority[end..].find(':').map(|at| at + end),
        None => authority.rfind(':'),
    };
    let Some(colon) = colon else {
        return Some((authority, None));
    };
    let (host, digits) = (&authority[..colon], &authority[colon + 1..]);
    if digits.is_empty() {
        return Some((host, None));
    }
    if digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((host, Some(digits.parse().ok()?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_is_reduced_to_scheme_host_and_a_port_of_its_own() {
        for (given, expected) in [
            ("https://chat.example.test/", "https://chat.example.test"),
            ("https://Chat.Example.TEST", "https://chat.example.test"),
            (
                "https://chat.example.test:443/x/y",
                "https://chat.example.test",
            ),
            ("http://chat.example.test:80", "http://chat.example.test"),
            (
                "http://chat.example.test:8080/",
                "http://chat.example.test:8080",
            ),
            ("HTTP://localhost:5000", "http://localhost:5000"),
            ("https://chat.example.test:/", "https://chat.example.test"),
            ("http://[::1]:5000/", "http://[::1]:5000"),
            ("http://127.0.0.1", "http://127.0.0.1"),
            ("http://[2001:DB8::1]:8080/a", "http://[2001:db8::1]:8080"),
            ("http://127.1:81", "http://127.0.0.1:81"),
            ("http://0x7f.0.0.1", "http://127.0.0.1"),
            ("http://0177.0.0.1", "http://127.0.0.1"),
            ("http://2130706433", "http://127.0.0.1"),
            ("https://chat.example.test./x", "https://chat.example.test."),
            ("https://chat_x.example.test", "https://chat_x.example.test"),
            ("https://CHÄT.example.test", "https://chät.example.test"),
            (
                "https://chat.example.test:0443",
                "https://chat.example.test",
            ),
            ("https://chat.example.test:0", "https://chat.example.test:0"),
            ("https://chat.example.test/a b", "https://chat.example.test"),
        ] {
            assert_eq!(canonical(given).as_deref(), Some(expected), "{given}");
        }
    }

    // Every answer above that is not obvious was checked against the first
    // server's own parser, on the same inputs, with a throwaway program.
    #[test]
    fn what_is_not_an_http_origin_without_extras_is_refused() {
        for given in [
            "chat.example.test",
            "ftp://chat.example.test",
            "https://operator:secret@chat.example.test",
            "https://chat.example.test/?next=1",
            "https://chat.example.test/#fragment",
            "https://",
            "https:///path",
            "https://chat.example.test:99999",
            "https://chat.example.test:80x",
            "https://chat example.test",
            "https://chat..example.test",
            "https://.chat.example.test",
            "https://chat.example.test\\path",
            "https://chat.example.test:+80",
            "https://[::1",
            "https://[g::1]",
            "https://[]",
            "http:/chat.example.test",
            "https://chat.example.test:80:80",
            "/relative",
            "",
        ] {
            assert_eq!(canonical(given), None, "{given}");
        }
    }
}
