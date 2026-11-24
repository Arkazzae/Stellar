//! Reading the one field of the approval form.
//!
//! The page's own form is sent as a browser sends one, URL-encoded, but a
//! client may also send the other kind of form (`multipart/form-data`). The
//! name is matched exactly, a field sent twice is a bad request, and a request
//! that is not a form at all, or breaks the limits of one, is a failure of the
//! server's own (500) and not a refusal. The limits are 1024 fields, a name of
//! at most 2048 characters and a value of at most 4 MiB, and the body is at
//! most [`BODY_LIMIT`], since nothing but a token of 43 characters is ever
//! sent.

use axum::body::Body;
use axum::http::{HeaderMap, header};

use crate::http::multipart::{FormError, Kind, Multipart};
use crate::http::{ApiError, read_all};

/// The most a form of this route may be. The page's own is under a hundred
/// bytes.
pub const BODY_LIMIT: usize = 1024 * 1024;
const MAX_FIELDS: usize = 1024;
const MAX_NAME: usize = 2048;
const MAX_VALUE: usize = 4 * 1024 * 1024;

/// The name of the field that binds the form.
const FIELD: &str = "token";

/// How the body of the request writes its fields.
enum Encoding {
    UrlEncoded,
    Multipart,
}

fn kind_of(headers: &HeaderMap) -> Option<Encoding> {
    let content_type = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match media.as_str() {
        "application/x-www-form-urlencoded" => Some(Encoding::UrlEncoded),
        "multipart/form-data" => Some(Encoding::Multipart),
        _ => None,
    }
}

/// The value of the `token` field of the request's form.
pub async fn read_token(headers: &HeaderMap, body: Body) -> Result<String, ApiError> {
    match kind_of(headers) {
        None => Err(ApiError::internal("the request is not a form")),
        Some(Encoding::UrlEncoded) => {
            let bytes = read_all(headers, body, BODY_LIMIT).await.map_err(|error| match error {
                ApiError::TooLarge => ApiError::internal("the form is over its limit"),
                other => other,
            })?;
            token_of_urlencoded(&bytes)
        }
        Some(Encoding::Multipart) => token_of_multipart(headers, body).await,
    }
}

fn limit(what: &str) -> ApiError {
    ApiError::internal(format!("the form breaks a limit: {what}"))
}

/// What the field comes to: its value, empty when it was not sent, and a
/// refusal when it was sent more than once.
fn single(mut values: Vec<String>) -> Result<String, ApiError> {
    match values.len() {
        0 => Ok(String::new()),
        1 => Ok(values.remove(0)),
        _ => Err(ApiError::bad_request()),
    }
}

fn token_of_urlencoded(body: &[u8]) -> Result<String, ApiError> {
    let mut fields = 0;
    let mut values = Vec::new();
    for pair in body.split(|b| *b == b'&').filter(|pair| !pair.is_empty()) {
        let (name, value) = match pair.iter().position(|b| *b == b'=') {
            Some(at) => (&pair[..at], &pair[at + 1..]),
            None => (pair, &pair[pair.len()..]),
        };
        fields += 1;
        if fields > MAX_FIELDS {
            return Err(limit("too many fields"));
        }
        if name.len() > MAX_NAME {
            return Err(limit("a name is too long"));
        }
        if value.len() > MAX_VALUE {
            return Err(limit("a value is too long"));
        }
        if decode(name) == FIELD {
            values.push(decode(value));
        }
    }
    single(values)
}

/// A name or a value as a form writes it: a plus is a space, and `%` and two
/// hexadecimal digits is a byte. What is not hexadecimal after a `%` is kept
/// as written, and what the bytes do not make of UTF-8 is replaced.
fn decode(raw: &[u8]) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut at = 0;
    while at < raw.len() {
        match raw[at] {
            b'+' => bytes.push(b' '),
            b'%' if at + 2 < raw.len() && hex(raw[at + 1]).is_some() && hex(raw[at + 2]).is_some() => {
                bytes.push(hex(raw[at + 1]).unwrap_or(0) * 16 + hex(raw[at + 2]).unwrap_or(0));
                at += 2;
            }
            b => bytes.push(b),
        }
        at += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

async fn token_of_multipart(headers: &HeaderMap, body: Body) -> Result<String, ApiError> {
    let failed = |error: FormError| match error {
        FormError::TooSlow => ApiError::from(error),
        other => ApiError::internal(format!("the form could not be read: {other:?}")),
    };
    let mut form = Multipart::open(headers, body, BODY_LIMIT).map_err(failed)?;
    let mut values = Vec::new();
    let mut fields = 0;
    while let Some(part) = form.next_part().await.map_err(failed)? {
        let disposition = part.disposition().map_err(failed)?;
        // A file, or a part that is not form data, is not a field of the form.
        if disposition.kind != Kind::Field {
            continue;
        }
        fields += 1;
        if fields > MAX_FIELDS {
            return Err(limit("too many fields"));
        }
        if disposition.name.len() > MAX_NAME {
            return Err(limit("a name is too long"));
        }
        let wanted = disposition.name == FIELD;
        let mut value = Vec::new();
        while let Some(chunk) = form.chunk().await.map_err(failed)? {
            if value.len() + chunk.len() > MAX_VALUE {
                return Err(limit("a value is too long"));
            }
            if wanted {
                value.extend_from_slice(&chunk);
            }
        }
        if wanted {
            values.push(String::from_utf8_lossy(&value).into_owned());
        }
    }
    single(values)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(content_type: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(content_type).unwrap(),
        );
        headers
    }

    async fn token(content_type: &str, body: &str) -> Result<String, ApiError> {
        read_token(&headers(content_type), Body::from(body.to_string())).await
    }

    const URL: &str = "application/x-www-form-urlencoded";

    #[tokio::test]
    async fn the_field_is_found_by_its_exact_name_and_any_spelling_of_its_value() {
        assert_eq!(token(URL, "token=abc").await.unwrap(), "abc");
        assert_eq!(token(URL, "TOKEN=abc").await.unwrap(), "");
        assert_eq!(token(URL, "t%6Fken=a%2Db+c").await.unwrap(), "a-b c");
        assert_eq!(token(URL, "x=1&token=abc&y=2").await.unwrap(), "abc");
        assert_eq!(token(URL, "a=1;token=abc").await.unwrap(), "");
        assert_eq!(token(URL, "token=%zz%4").await.unwrap(), "%zz%4");
        assert_eq!(token(URL, "token=%FF").await.unwrap(), "\u{fffd}");
        assert_eq!(token("Application/X-WWW-Form-Urlencoded; charset=utf-8", "token=a").await.unwrap(), "a");
        assert_eq!(token(URL, "").await.unwrap(), "");
        assert_eq!(token(URL, "token").await.unwrap(), "");
    }

    #[tokio::test]
    async fn a_field_sent_twice_is_refused() {
        for body in ["token=a&token=b", "token=&token=b", "token=a&token="] {
            let refused = token(URL, body).await.unwrap_err();
            assert!(
                matches!(refused, ApiError::Status(status) if status == axum::http::StatusCode::BAD_REQUEST),
                "{body}"
            );
        }
        assert_eq!(token(URL, "token=a&Token=").await.unwrap(), "a");
    }

    #[tokio::test]
    async fn a_multipart_form_is_read_and_its_files_are_not_fields() {
        let body = "--B\r\nContent-Disposition: form-data; name=\"token\"; filename=\"t.txt\"\r\n\r\nfile\r\n--B\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n1\r\n--B\r\nContent-Disposition: form-data; name=\"token\"\r\n\r\nabc\r\n--B--\r\n";
        assert_eq!(
            token("multipart/form-data; boundary=B", body).await.unwrap(),
            "abc"
        );
    }

    #[tokio::test]
    async fn what_is_not_a_form_or_breaks_its_limits_is_the_servers_failure() {
        for (content_type, body) in [
            ("application/json", "{\"token\":\"a\"}"),
            ("text/plain", "token=a"),
            ("multipart/form-data", "x"),
            ("multipart/form-data; boundary=B", "garbage"),
        ] {
            let refused = token(content_type, body).await.unwrap_err();
            assert!(matches!(refused, ApiError::Internal(_)), "{content_type}");
        }
        let none = read_token(&HeaderMap::new(), Body::empty()).await.unwrap_err();
        assert!(matches!(none, ApiError::Internal(_)));

        let many = |n: usize| (0..n).map(|i| format!("f{i}=1")).collect::<Vec<_>>().join("&");
        assert!(token(URL, &format!("{}&token=a", many(1023))).await.is_ok());
        assert!(token(URL, &format!("{}&token=a", many(1024))).await.is_err());
        let name = |n: usize| "k".repeat(n);
        assert!(token(URL, &format!("{}=1&token=a", name(MAX_NAME))).await.is_ok());
        assert!(token(URL, &format!("{}=1&token=a", name(MAX_NAME + 1))).await.is_err());
        let value = |n: usize| "a".repeat(n);
        assert!(token(URL, &format!("x={}", value(MAX_VALUE))).await.is_err(), "over the body limit as a whole");
        assert!(token(URL, &format!("x={}", value(BODY_LIMIT + 1))).await.is_err());
        assert!(token(URL, &format!("x={}&token=a", value(BODY_LIMIT / 2))).await.is_ok());
    }
}
