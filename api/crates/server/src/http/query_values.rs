//! The values of a query string, read as the first server bound them.
//!
//! A route that took `string? name` or `int? limit` from its query saw:
//!
//! - names without regard to case (`?AFTER=1` is `after`);
//! - a name given more than once as its non-empty values joined with commas, so
//!   `?limit=5&limit=` is `5` and `?limit=5&limit=6` is `5,6`, which is not a
//!   number; a name given only with empty values is the empty text;
//! - a name that is not there at all as nothing, which is not the same as the
//!   empty text: a number cannot be empty, a text can;
//! - `+` and `%20` as a space, `%XX` as a byte where bytes make a UTF-8
//!   character, and an escape that does not stays as it was written;
//! - a number as the runtime's integer parser read it: white space around it, a
//!   sign in front, ASCII digits, a trailing null only after the white space,
//!   and nothing past the range of its type.
//!
//! A value that is no number is a 400 with no body, before anything else about
//! the request is looked at. [`QueryValues`] is the extractor; take it where a
//! route would take `Query<T>` and read each name with [`QueryValues::text`],
//! [`QueryValues::int`] or [`QueryValues::long`].

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use super::error::ApiError;

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

    /// The value of a name: none when it was not given at all.
    pub fn text(&self, name: &str) -> Option<String> {
        let mut seen = false;
        let mut joined = String::new();
        for (key, value) in &self.0 {
            if key.eq_ignore_ascii_case(name) {
                seen = true;
                if !value.is_empty() {
                    if !joined.is_empty() {
                        joined.push(',');
                    }
                    joined.push_str(value);
                }
            }
        }
        seen.then_some(joined)
    }

    /// A number of 32 bits; a value that is none is a bad request.
    pub fn int(&self, name: &str) -> Result<Option<i32>, ApiError> {
        match self.text(name) {
            Some(text) => parse_int(&text).map(Some).ok_or_else(ApiError::bad_request),
            None => Ok(None),
        }
    }

    /// A number of 64 bits; a value that is none is a bad request.
    pub fn long(&self, name: &str) -> Result<Option<i64>, ApiError> {
        match self.text(name) {
            Some(text) => parse_long(&text)
                .map(Some)
                .ok_or_else(ApiError::bad_request),
            None => Ok(None),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for QueryValues {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<QueryValues, ApiError> {
        Ok(QueryValues::parse(parts.uri.query()))
    }
}

/// A number of 32 bits as the first server's integer parser read it.
pub fn parse_int(text: &str) -> Option<i32> {
    parse_long(text).and_then(|number| i32::try_from(number).ok())
}

/// A number of 64 bits as the first server's integer parser read it.
pub fn parse_long(text: &str) -> Option<i64> {
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
    let mut value: i128 = 0;
    for digit in digits.bytes() {
        value = value * 10 + i128::from(digit - b'0');
        if value > i128::from(i64::MAX) + 1 {
            return None;
        }
    }
    i64::try_from(if negative { -value } else { value }).ok()
}

fn form_decode(text: &str) -> String {
    unescape(&text.replace('+', " "))
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
/// the range the first of them may be in; none for a byte that starts nothing.
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
/// character are decoded; an escape that does not (a lone continuation byte, a
/// truncated or overlong sequence, a surrogate, a value past U+10FFFF, or
/// something that is not two hexadecimal digits) stays as it was written.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let Some(first) = escaped(bytes, at) else {
            out.push(bytes[at]);
            at += 1;
            continue;
        };
        if first < 0x80 {
            out.push(first);
            at += 3;
            continue;
        }
        let whole = sequence_after(first).and_then(|(more, low, high)| {
            let mut sequence = vec![first];
            for index in 0..more {
                let next = escaped(bytes, at + 3 * (index + 1))?;
                let (low, high) = if index == 0 {
                    (low, high)
                } else {
                    (0x80, 0xBF)
                };
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
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_read_as_a_form_is() {
        assert_eq!(form_decode("a+b%2Fc"), "a b/c");
        assert_eq!(form_decode("%00ff"), "\0ff");
        assert_eq!(form_decode("%FFmpeg"), "%FFmpeg");
        assert_eq!(form_decode("%C3%28"), "%C3(");
        assert_eq!(form_decode("%F0%9F%91%8D"), "\u{1F44D}");
        assert_eq!(form_decode("%zz%4"), "%zz%4");
    }

    #[test]
    fn names_have_no_case_and_repeated_values_are_joined() {
        let values = QueryValues::parse(Some("Query=a&query=b&limit=&cursor"));
        assert_eq!(values.text("query").as_deref(), Some("a,b"));
        assert_eq!(values.text("limit").as_deref(), Some(""));
        assert_eq!(values.text("cursor").as_deref(), Some(""));
        assert_eq!(values.text("kind"), None);
    }

    #[test]
    fn an_empty_value_beside_another_is_left_out() {
        for (query, expected) in [
            ("limit=5&limit=", Some("5")),
            ("limit=&limit=5", Some("5")),
            ("limit=&limit=", Some("")),
            ("limit=5&limit=6", Some("5,6")),
            ("LIMIT=5", Some("5")),
        ] {
            assert_eq!(
                QueryValues::parse(Some(query)).text("limit").as_deref(),
                expected,
                "{query}"
            );
        }
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

    #[test]
    fn a_long_number_has_the_range_of_64_bits() {
        assert_eq!(parse_long("9223372036854775807"), Some(i64::MAX));
        assert_eq!(parse_long("9223372036854775808"), None);
        assert_eq!(parse_long("-9223372036854775808"), Some(i64::MIN));
        assert_eq!(parse_long(" +12 "), Some(12));
    }

    #[test]
    fn a_name_that_is_not_a_number_is_a_bad_request() {
        let values = QueryValues::parse(Some("limit=abc&after=5,6&ok=%205"));
        assert!(values.int("limit").is_err());
        assert!(values.long("after").is_err());
        assert_eq!(values.int("ok").unwrap(), Some(5));
        assert_eq!(values.int("absent").unwrap(), None);
    }
}
