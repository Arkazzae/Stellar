//! Reading ids, query values and the reaction's path segment the way the
//! first server did, so that the same address means the same thing.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::de::{self, Deserializer};
use serde::Deserialize;

use crate::http::ApiError;
use crate::permissions::membership::parse_guid;
use crate::values::Id;

/// An id in a path, in any spelling the first server's route constraint read:
/// with or without hyphens, in braces, in capitals. Anything else is a path
/// that does not exist.
#[derive(Debug, Clone, Copy)]
pub struct Guid(pub Id);

impl<'de> Deserialize<'de> for Guid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Guid, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_guid(&text)
            .map(Guid)
            .ok_or_else(|| de::Error::custom("not an id"))
    }
}

/// The values of a query string, by name without regard to case, the way the
/// first server bound them: a name given twice has its values joined with a
/// comma, and a name given with no value has an empty one.
pub struct QueryValues(Vec<(String, String)>);

impl QueryValues {
    pub fn parse(query: Option<&str>) -> QueryValues {
        let mut pairs = Vec::new();
        for pair in query.unwrap_or("").split('&') {
            if pair.is_empty() {
                continue;
            }
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            pairs.push((form_decode(name), form_decode(value)));
        }
        QueryValues(pairs)
    }

    /// The value of a name; none when it was not given at all.
    pub fn get(&self, name: &str) -> Option<String> {
        let mut found: Option<String> = None;
        for (key, value) in &self.0 {
            if key.eq_ignore_ascii_case(name) {
                match &mut found {
                    Some(joined) => {
                        joined.push(',');
                        joined.push_str(value);
                    }
                    None => found = Some(value.clone()),
                }
            }
        }
        found
    }
}

impl<S: Send + Sync> FromRequestParts<S> for QueryValues {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<QueryValues, ApiError> {
        Ok(QueryValues::parse(parts.uri.query()))
    }
}

/// A number from a query value as the first server read an `int`: white space
/// around it, a sign in front, ASCII digits, and nothing past the range of a
/// 32-bit integer. An empty value is not a number.
pub fn parse_int(text: &str) -> Option<i32> {
    // The characters a number may be padded with: the controls from tab to
    // carriage return and the space, and a trailing null.
    let leading = |c: char| matches!(c, '\t'..='\r' | ' ');
    // Trailing nulls come after any trailing white space, never before it.
    let body = text
        .trim_start_matches(leading)
        .trim_end_matches('\0')
        .trim_end_matches(leading);
    let (negative, digits) = match body.as_bytes().first()? {
        b'-' => (true, &body[1..]),
        b'+' => (false, &body[1..]),
        _ => (false, body),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut value: i64 = 0;
    for digit in digits.bytes() {
        value = value * 10 + i64::from(digit - b'0');
        if value > i64::from(i32::MAX) + 1 {
            return None;
        }
    }
    let value = if negative { -value } else { value };
    i32::try_from(value).ok()
}

/// A query component: `+` is a space and `%XX` a byte, as for a form.
fn form_decode(text: &str) -> String {
    unescape(&text.replace('+', " "), Slash::Decoded, Nul::Kept).unwrap_or_default()
}

/// What a reaction's address names: the message, and the emoji, which is
/// whatever the person chose. The first server's way of reading a path is
/// kept: `%2F` stays as it was written, a byte sequence that is not UTF-8
/// stays as written too, and `%00` is a bad request.
///
/// It is read from the path as it came, not from the router's parameters:
/// those are decoded in one way for everything and refuse what is not UTF-8.
pub struct ReactionTarget {
    pub message: Id,
    pub emoji: String,
}

impl<S: Send + Sync> FromRequestParts<S> for ReactionTarget {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<ReactionTarget, ApiError> {
        // `/api/messages/{id}/reactions/{emoji}`
        let mut segments = parts.uri.path().split('/').skip(3);
        let (Some(message), Some(_), Some(emoji)) =
            (segments.next(), segments.next(), segments.next())
        else {
            return Err(ApiError::not_found());
        };
        let emoji = unescape(emoji, Slash::Kept, Nul::Refused).ok_or_else(ApiError::bad_request)?;
        let message = unescape(message, Slash::Kept, Nul::Refused)
            .and_then(|text| parse_guid(&text))
            .ok_or_else(ApiError::not_found)?;
        Ok(ReactionTarget { message, emoji })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slash {
    /// `%2F` is a slash like any other byte.
    Decoded,
    /// `%2F` stays as written: a slash in a path segment would end it.
    Kept,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Nul {
    Kept,
    Refused,
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// The byte `%XX` stands for at `at`, if there is one.
fn escaped(bytes: &[u8], at: usize) -> Option<u8> {
    if bytes.get(at) != Some(&b'%') {
        return None;
    }
    Some(hex(*bytes.get(at + 1)?)? << 4 | hex(*bytes.get(at + 2)?)?)
}

/// How many continuation bytes a sequence that starts with `lead` needs, and
/// the range its first one may be in. None for a byte that starts nothing.
fn sequence_after(lead: u8) -> Option<(usize, u8, u8)> {
    Some(match lead {
        0xC2..=0xDF => (1, 0x80, 0xBF),
        0xE0 => (2, 0xA0, 0xBF),
        0xED => (2, 0x80, 0x9F),
        0xE1..=0xEC | 0xEE..=0xEF => (2, 0x80, 0xBF),
        0xF0 => (3, 0x90, 0xBF),
        0xF4 => (3, 0x80, 0x8F),
        0xF1..=0xF3 => (3, 0x80, 0xBF),
        _ => return None,
    })
}

/// Decodes the `%XX` escapes of a text. Bytes that make a whole UTF-8
/// character are decoded; an escape that does not (a lone continuation, a
/// truncated or overlong sequence, a surrogate, a value past U+10FFFF, or
/// something that is not two hexadecimal digits) stays as it was written.
fn unescape(text: &str, slash: Slash, nul: Nul) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let Some(first) = escaped(bytes, at) else {
            out.push(bytes[at]);
            at += 1;
            continue;
        };
        if first == 0 && nul == Nul::Refused {
            return None;
        }
        if first == b'/' && slash == Slash::Kept {
            out.extend_from_slice(&bytes[at..at + 3]);
            at += 3;
            continue;
        }
        if first < 0x80 {
            out.push(first);
            at += 3;
            continue;
        }
        let whole = sequence_after(first).and_then(|(more, low, high)| {
            let mut sequence = vec![first];
            for index in 0..more {
                let next = escaped(bytes, at + 3 * (index + 1))?;
                let (low, high) = if index == 0 { (low, high) } else { (0x80, 0xBF) };
                if !(low..=high).contains(&next) {
                    return None;
                }
                sequence.push(next);
            }
            Some(sequence)
        });
        match whole {
            Some(sequence) => {
                at += 3 * sequence.len();
                out.extend(sequence);
            }
            None => {
                out.extend_from_slice(&bytes[at..at + 3]);
                at += 3;
            }
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> Option<String> {
        unescape(text, Slash::Kept, Nul::Refused)
    }

    #[test]
    fn a_path_is_read_as_the_first_server_read_it() {
        // Each pair is what the first server answered for that segment.
        let cases = [
            ("%F0%9F%91%8D", "\u{1F44D}"),
            ("%2F", "%2F"),
            ("%2f", "%2f"),
            ("%25", "%"),
            ("%FF", "%FF"),
            ("%C3%28", "%C3("),
            ("%C0%AF", "%C0%AF"),
            ("%ED%A0%80", "%ED%A0%80"),
            ("%EF%BF%BF", "\u{FFFF}"),
            ("%F4%90%80%80", "%F4%90%80%80"),
            ("%E2%82", "%E2%82"),
            ("%u00e9", "%u00e9"),
            ("%zz", "%zz"),
            ("a%", "a%"),
            ("%4", "%4"),
            ("%41", "A"),
            ("%5C", "\\"),
            ("%c3%a9", "\u{e9}"),
            ("+", "+"),
        ];
        for (raw, expected) in cases {
            assert_eq!(path(raw).as_deref(), Some(expected), "{raw}");
        }
        assert_eq!(path("%00"), None);
    }

    #[test]
    fn a_query_is_read_as_a_form_is() {
        assert_eq!(form_decode("a+b%2Fc"), "a b/c");
        assert_eq!(form_decode("%00ff"), "\0ff");
        assert_eq!(form_decode("%FFmpeg"), "%FFmpeg");
    }

    #[test]
    fn names_have_no_case_and_repeated_values_are_joined() {
        let values = QueryValues::parse(Some("Query=a&query=b&limit=&cursor"));
        assert_eq!(values.get("query").as_deref(), Some("a,b"));
        assert_eq!(values.get("limit").as_deref(), Some(""));
        assert_eq!(values.get("cursor").as_deref(), Some(""));
        assert_eq!(values.get("kind"), None);
    }

    #[test]
    fn a_number_is_read_as_the_first_server_read_an_int() {
        for (text, expected) in [
            ("5", Some(5)),
            (" 5", Some(5)),
            ("+5", Some(5)),
            ("-5", Some(-5)),
            ("5 ", Some(5)),
            ("5\0", Some(5)),
            ("5 \0", Some(5)),
            ("5\0 ", None),
            ("", None),
            ("abc", None),
            ("5,6", None),
            ("5.0", None),
            ("2147483647", Some(i32::MAX)),
            ("2147483648", None),
            ("-2147483648", Some(i32::MIN)),
            ("+", None),
            ("\u{0663}", None),
        ] {
            assert_eq!(parse_int(text), expected, "{text:?}");
        }
    }
}
