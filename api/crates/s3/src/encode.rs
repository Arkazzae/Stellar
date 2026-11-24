//! Percent-encoding the way Signature Version 4 defines it.
//!
//! The standard library has none and the general-purpose encoders disagree
//! with S3 about which bytes stay as they are, so the rule lives here once. The
//! same encoded string is used for the request target and for the canonical
//! request, which is what keeps them from drifting apart.

/// Appends `input` with every byte encoded except the unreserved set
/// (`A-Z a-z 0-9 - . _ ~`); `/` stays when `keep_slash` is set.
pub fn encode_into(input: &str, keep_slash: bool, out: &mut String) {
    for &byte in input.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && byte == b'/')
        {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 15));
        }
    }
}

/// An object key as a path: slashes stay, everything else is encoded.
pub fn encode_path(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 8);
    encode_into(key, true, &mut out);
    out
}

/// A query name or value: the slash is encoded too.
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    encode_into(text, false, &mut out);
    out
}

/// The query of a request in two spellings of one string: the canonical form
/// that is signed (every pair as `name=value`) and the form that is sent, which
/// leaves the `=` off a name without a value, the way S3 clients write
/// subresources like `?uploads`. Pairs are encoded, then sorted.
pub fn query_strings(pairs: &[(&str, &str)]) -> (String, String) {
    let mut encoded: Vec<(String, String)> = pairs
        .iter()
        .map(|(n, v)| (encode_component(n), encode_component(v)))
        .collect();
    encoded.sort();
    let canonical = encoded
        .iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let sent = encoded
        .iter()
        .map(|(n, v)| {
            if v.is_empty() {
                n.clone()
            } else {
                format!("{n}={v}")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    (canonical, sent)
}

fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789ABCDEF"[usize::from(nibble)])
}

/// Undoes percent-encoding. With `plus_is_space` a `+` becomes a space, which is
/// how a list answer written with `encoding-type=url` spells one. A stray `%`,
/// a truncated escape or bytes that are not UTF-8 give `None`.
pub fn decode(text: &str, plus_is_space: bool) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' => {
                let high = hex_value(*bytes.get(at + 1)?)?;
                let low = hex_value(*bytes.get(at + 2)?)?;
                out.push(high << 4 | low);
                at += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                at += 1;
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreserved_bytes_stay() {
        assert_eq!(encode_component("AZaz09-._~"), "AZaz09-._~");
    }

    #[test]
    fn everything_else_is_upper_case_hex() {
        assert_eq!(encode_component("a b+c%d/é"), "a%20b%2Bc%25d%2F%C3%A9");
        assert_eq!(encode_path("a b/c"), "a%20b/c");
        assert_eq!(
            encode_component("$*'()!,;:@&=\""),
            "%24%2A%27%28%29%21%2C%3B%3A%40%26%3D%22"
        );
    }

    #[test]
    fn slashes_are_never_collapsed() {
        assert_eq!(encode_path("a//b/"), "a//b/");
        assert_eq!(encode_path("/lead"), "/lead");
    }

    #[test]
    fn decoding_reads_back_what_encoding_wrote() {
        for key in [
            "plain",
            "a b/c+d",
            "100%",
            "é/ü ß",
            "a//b",
            "..",
            "tab\there",
            "+++",
            "%2F",
        ] {
            assert_eq!(decode(&encode_path(key), false).as_deref(), Some(key));
        }
    }

    #[test]
    fn plus_means_a_space_only_when_asked() {
        assert_eq!(decode("a+b%2Bc", true).as_deref(), Some("a b+c"));
        assert_eq!(decode("a+b", false).as_deref(), Some("a+b"));
    }

    #[test]
    fn the_query_is_sorted_after_encoding_and_one_string_serves_both_uses() {
        let (canonical, sent) = query_strings(&[
            ("uploads", ""),
            ("prefix", "a b/c"),
            ("Prefix", "x"),
            ("token", "a+b/c=="),
        ]);
        assert_eq!(
            canonical,
            "Prefix=x&prefix=a%20b%2Fc&token=a%2Bb%2Fc%3D%3D&uploads="
        );
        assert_eq!(
            sent,
            "Prefix=x&prefix=a%20b%2Fc&token=a%2Bb%2Fc%3D%3D&uploads"
        );
        assert_eq!(query_strings(&[]), (String::new(), String::new()));
    }

    #[test]
    fn broken_escapes_are_refused() {
        for bad in ["%", "%2", "%zz", "%G1", "a%", "%ff", "%c3%28"] {
            assert_eq!(decode(bad, true), None, "{bad}");
        }
    }
}
