//! `application/x-www-form-urlencoded`, for the query of an authorization
//! request, the answer the provider posts back and the token request body.

/// Longest form or query this crate reads. A sealed state is under 2 KiB and a
/// code well under 8 KiB, so anything near this size is not an answer.
pub const MAX_FORM_LEN: usize = 32 * 1024;
const MAX_PAIRS: usize = 64;

/// Percent-encodes everything but the unreserved characters.
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn encode(pairs: &[(&str, &str)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect();
    parts.join("&")
}

/// Percent-decodes a path or query component, `+` left alone. Invalid
/// escapes stay as they are and invalid UTF-8 becomes U+FFFD.
pub fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = hex_pair(bytes.get(i + 1..i + 3))
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(pair: Option<&[u8]>) -> Option<u8> {
    let pair = pair?;
    let digit = |c: u8| char::from(c).to_digit(16);
    Some((digit(pair[0])? * 16 + digit(pair[1])?) as u8)
}

/// Splits a form or query into decoded pairs. `None` when it is too large,
/// has too many pairs, or holds an escape that is not valid.
pub fn parse(input: &str) -> Option<Vec<(String, String)>> {
    if input.len() > MAX_FORM_LEN {
        return None;
    }
    let mut pairs = Vec::new();
    for part in input.split('&').filter(|part| !part.is_empty()) {
        if pairs.len() == MAX_PAIRS {
            return None;
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        pairs.push((decode(key)?, decode(value)?));
    }
    Some(pairs)
}

fn decode(part: &str) -> Option<String> {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                out.push(hex_pair(bytes.get(i + 1..i + 3))?);
                i += 2;
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_parses_back() {
        let text = encode(&[("scope", "openid profile email"), ("a b", "é&=+")]);
        assert_eq!(text, "scope=openid%20profile%20email&a%20b=%C3%A9%26%3D%2B");
        let pairs = parse(&text).unwrap();
        assert_eq!(pairs[0], ("scope".into(), "openid profile email".into()));
        assert_eq!(pairs[1], ("a b".into(), "é&=+".into()));
    }

    #[test]
    fn plus_is_a_space_in_a_form() {
        assert_eq!(parse("a=b+c").unwrap()[0].1, "b c");
    }

    #[test]
    fn refuses_malformed_and_oversized_input() {
        assert_eq!(parse("a=%zz"), None);
        assert_eq!(parse("a=%4"), None);
        assert_eq!(parse("a=%ff"), None, "not UTF-8");
        assert_eq!(parse(&"a=1&".repeat(65)), None);
        assert_eq!(parse(&"a".repeat(MAX_FORM_LEN + 1)), None);
    }

    #[test]
    fn unescape_leaves_bad_escapes_and_plus() {
        assert_eq!(unescape("/a%2Fb+c%zz%2"), "/a/b+c%zz%2");
    }
}
