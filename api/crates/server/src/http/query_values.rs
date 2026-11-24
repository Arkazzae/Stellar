//! The values of a query string, read exactly.
//!
//! A route that takes `string? name` or `int? limit` from its query reads it
//! here, with these rules:
//!
//! - a name matches as it is written, letter case included;
//! - a name given more than once is refused when the route reads it: no route
//!   takes a list, and joining the values would hide a mistake of the client;
//! - a name that is not there is nothing, which is not the same as the empty
//!   text: a number cannot be empty, a text can;
//! - `+` and `%20` are a space, `%XX` is a byte where bytes make a UTF-8
//!   character, and an escape that does not stays as it was written;
//! - a number is ASCII digits and nothing else (no sign, no spaces), and fits
//!   its type; a flag is `true` or `false`.
//!
//! What a route cannot read is a validation problem for that name, with the
//! same shape as any other refusal of a field. [`QueryValues`] is the
//! extractor; take it where a route would take `Query<T>` and read each name
//! with [`QueryValues::text`], [`QueryValues::int`], [`QueryValues::long`] or
//! [`QueryValues::flag`].

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use stellarr_messages::{MessageKey, Messages};

use super::error::ApiError;
use super::problem::Problem;
use crate::encoding::hex_digit;
use crate::state::AppState;

pub struct QueryValues {
    pairs: Vec<(String, String)>,
    messages: Messages,
}

impl QueryValues {
    pub fn parse(query: Option<&str>, messages: &Messages) -> QueryValues {
        let mut pairs = Vec::new();
        for pair in query.unwrap_or("").split('&') {
            if pair.is_empty() {
                continue;
            }
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            pairs.push((form_decode(name), form_decode(value)));
        }
        QueryValues {
            pairs,
            messages: *messages,
        }
    }

    /// The value of a name: none when it was not given at all, a refusal when
    /// it was given more than once.
    fn one(&self, name: &str) -> Result<Option<&str>, ApiError> {
        let mut given = self.pairs.iter().filter(|(key, _)| key == name);
        let first = given.next();
        if given.next().is_some() {
            return Err(self.invalid(name));
        }
        Ok(first.map(|(_, value)| value.as_str()))
    }

    fn invalid(&self, name: &str) -> ApiError {
        match self
            .messages
            .get(MessageKey::QueryParameterInvalid, &[("name", &name)])
        {
            Ok(text) => Problem::validation(vec![(name.to_string(), vec![text])]).into(),
            Err(error) => error.into(),
        }
    }

    /// The text of a name: none when it was not given at all.
    pub fn text(&self, name: &str) -> Result<Option<String>, ApiError> {
        Ok(self.one(name)?.map(str::to_string))
    }

    /// A number of 32 bits; a value that is none is a bad request.
    pub fn int(&self, name: &str) -> Result<Option<i32>, ApiError> {
        match self.one(name)? {
            Some(text) => digits(text)
                .and_then(|number| i32::try_from(number).ok())
                .map(Some)
                .ok_or_else(|| self.invalid(name)),
            None => Ok(None),
        }
    }

    /// A number of 64 bits; a value that is none is a bad request.
    pub fn long(&self, name: &str) -> Result<Option<i64>, ApiError> {
        match self.one(name)? {
            Some(text) => digits(text)
                .and_then(|number| i64::try_from(number).ok())
                .map(Some)
                .ok_or_else(|| self.invalid(name)),
            None => Ok(None),
        }
    }

    /// A flag, `true` or `false`.
    pub fn flag(&self, name: &str) -> Result<Option<bool>, ApiError> {
        match self.one(name)? {
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            Some(_) => Err(self.invalid(name)),
            None => Ok(None),
        }
    }
}

impl FromRequestParts<AppState> for QueryValues {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<QueryValues, ApiError> {
        Ok(QueryValues::parse(parts.uri.query(), state.messages()))
    }
}

/// A number written as ASCII digits only, none when it has any other
/// character or does not fit 64 bits.
fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn form_decode(text: &str) -> String {
    unescape(&text.replace('+', " "))
}

/// The byte `%XX` stands for at `at`, if there is one.
fn escaped(bytes: &[u8], at: usize) -> Option<u8> {
    if bytes.get(at) != Some(&b'%') {
        return None;
    }
    Some(hex_digit(*bytes.get(at + 1)?)? << 4 | hex_digit(*bytes.get(at + 2)?)?)
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

    fn read(query: &str) -> QueryValues {
        QueryValues::parse(
            Some(query),
            &Messages::new(stellarr_messages::Language::English),
        )
    }

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
    fn names_match_exactly() {
        let values = read("Query=a&limit=&cursor");
        assert_eq!(values.text("query").unwrap(), None);
        assert_eq!(values.text("Query").unwrap().as_deref(), Some("a"));
        assert_eq!(values.text("limit").unwrap().as_deref(), Some(""));
        assert_eq!(values.text("cursor").unwrap().as_deref(), Some(""));
        assert_eq!(values.text("kind").unwrap(), None);
    }

    #[test]
    fn a_repeated_name_is_refused_whatever_its_values() {
        for query in [
            "limit=5&limit=",
            "limit=&limit=5",
            "limit=&limit=",
            "limit=5&limit=6",
            "limit=5&limit=5",
        ] {
            let values = read(query);
            assert!(values.text("limit").is_err(), "{query}");
            assert!(values.int("limit").is_err(), "{query}");
        }
        // A name the route does not read does not matter.
        assert_eq!(
            read("limit=5&other=1&other=2").int("limit").unwrap(),
            Some(5)
        );
    }

    #[test]
    fn a_number_is_plain_digits_that_fit() {
        for (text, expected) in [
            ("5", Some(5)),
            ("0", Some(0)),
            ("007", Some(7)),
            (" 5", None),
            ("+5", None),
            ("-5", None),
            ("5 ", None),
            ("5\0", None),
            ("", None),
            ("abc", None),
            ("5,6", None),
            ("5.0", None),
            ("2147483647", Some(i32::MAX)),
            ("2147483648", None),
            ("\u{0663}", None),
        ] {
            let values = QueryValues {
                pairs: vec![("n".into(), text.into())],
                messages: Messages::new(stellarr_messages::Language::English),
            };
            assert_eq!(values.int("n").ok().flatten(), expected, "{text:?}");
        }
    }

    #[test]
    fn a_long_number_has_the_range_of_64_bits() {
        assert_eq!(
            read("n=9223372036854775807").long("n").unwrap(),
            Some(i64::MAX)
        );
        assert!(read("n=9223372036854775808").long("n").is_err());
        assert!(read("n=-1").long("n").is_err());
        assert!(read("n=%2B12").long("n").is_err());
        assert!(read("n=+12").long("n").is_err());
    }

    #[test]
    fn a_flag_is_true_or_false_and_nothing_else() {
        assert_eq!(read("f=true").flag("f").unwrap(), Some(true));
        assert_eq!(read("f=false").flag("f").unwrap(), Some(false));
        assert_eq!(read("g=true").flag("f").unwrap(), None);
        for bad in ["f=", "f=True", "f=FALSE", "f=1", "f=%20true", "f=true%00"] {
            assert!(read(bad).flag("f").is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_value_that_does_not_read_is_a_validation_problem_for_its_name() {
        use http_body_util::BodyExt;

        let refused = read("limit=abc").int("limit").unwrap_err();
        let response = axum::response::IntoResponse::into_response(refused);
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("\"errors\":{\"limit\":["), "{body}");
    }
}
