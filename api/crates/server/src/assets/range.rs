//! The `Range` header of a download.

use axum::http::{HeaderMap, header};
use stellarr_storage::ByteRange;

const BYTES_PREFIX: &str = "bytes=";

/// The first range of a `Range` header, or none for anything this does not
/// serve.
///
/// Only the `bytes` unit and only the first range are understood: no media
/// element asks for several, and a form that is not understood may be answered
/// with the whole file. That includes a suffix such as `bytes=-500`, which
/// would need the size of the object before it is read.
pub fn first_range(headers: &HeaderMap) -> Option<ByteRange> {
    // Several lines of the header are one list.
    let lines: Vec<&str> = headers
        .get_all(header::RANGE)
        .iter()
        .filter_map(|line| line.to_str().ok())
        .collect();
    parse(&lines.join(","))
}

pub fn parse(value: &str) -> Option<ByteRange> {
    let prefix = value.get(..BYTES_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(BYTES_PREFIX) {
        return None;
    }
    let rest = &value[BYTES_PREFIX.len()..];
    let first = rest.split(',').next().unwrap_or("");
    let (from, end) = first.split_once('-')?;
    let from = number(from)?;
    let end = end.trim_matches(is_space);
    if end.is_empty() {
        return Some(ByteRange { from, to: None });
    }
    let to = number(end)?;
    (to >= from).then_some(ByteRange { from, to: Some(to) })
}

fn is_space(c: char) -> bool {
    c.is_whitespace()
}

/// Digits only, as a signed 64-bit number.
fn number(text: &str) -> Option<u64> {
    let text = text.trim_matches(is_space);
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok().map(|n| n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(from: u64, to: Option<u64>) -> Option<ByteRange> {
        Some(ByteRange { from, to })
    }

    #[test]
    fn the_first_range_from_a_start_is_served() {
        assert_eq!(parse("bytes=0-99"), range(0, Some(99)));
        assert_eq!(parse("BYTES=5-"), range(5, None));
        assert_eq!(parse("bytes=2-3, 7-9"), range(2, Some(3)));
        assert_eq!(parse("bytes= 4 - 8 "), range(4, Some(8)));
        assert_eq!(parse("bytes=0-0"), range(0, Some(0)));
    }

    #[test]
    fn everything_else_is_the_whole_file() {
        for text in [
            "", "bytes", "bytes=", "bytes=-500", "bytes=5-2", "bytes=a-b", "items=1-2", "bytes=1", "bytes=+1-5",
            "bytes=9223372036854775808-", "bytes=1-9223372036854775808", "bytes=-", " bytes=1-2",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
