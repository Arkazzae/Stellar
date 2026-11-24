//! The path a request is for, as the first server's front read it, before any
//! step of the pipeline or the router looks at it.
//!
//! Every step that decides by path (the rate limit, the client gate, the
//! session rule, the body ceiling, the router) must decide on the same string,
//! or a request could be judged as one route and served as another. So the
//! target is normalised once, outermost, and the rest of the server only ever
//! sees the normalised one:
//!
//! 1. a request line of more than 8192 bytes, line break included, is a 414;
//! 2. `%XX` is decoded, except `%2F`, which stays as the three characters it
//!    was written as (so it never becomes a separator). A byte of 0x80 or more
//!    is decoded together with the ones that complete a UTF-8 character; what
//!    is not one stays as written, and so does a `%` that no two hex digits
//!    follow. `%00` is a 400;
//! 3. the dot segments `.` and `..` are removed, as RFC 3986 says, after
//!    decoding, so `%2e%2e` is one; `..` cannot go above the root;
//! 4. one slash at the end is dropped: `/api/x/` is `/api/x`. Two are not.
//!
//! The result is written back into the target in the one encoding that decodes
//! to it again, so the router and `Path` read the segments as the first server
//! had them: a literal `%2F` is written `%252F` and comes out of `Path` as
//! `%2F`.
//!
//! The query is not touched. Letter case is not either: routes are matched
//! with their case here (a known difference, see the README).

use axum::extract::Request;
use axum::http::uri::{PathAndQuery, Uri};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

/// The longest request line, `GET /path HTTP/1.1` and its line break.
const MAX_REQUEST_LINE: usize = 8192;

/// What is wrong with a target, answered before anything else is done with it.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// 400: a null character in the path.
    BadRequest,
    /// 414: the request line is too long.
    TooLong,
}

impl Refusal {
    fn status(&self) -> StatusCode {
        match self {
            Refusal::BadRequest => StatusCode::BAD_REQUEST,
            Refusal::TooLong => StatusCode::URI_TOO_LONG,
        }
    }
}

/// The outermost step: refuses a target the first server refused before
/// routing and rewrites the others to their normalised path. Nothing it
/// refuses is counted or logged, as the first server's front did not.
pub async fn middleware(mut request: Request, next: Next) -> Response {
    match normalise_request(&mut request) {
        Ok(()) => next.run(request).await,
        Err(refusal) => {
            let mut response = Response::new(axum::body::Body::empty());
            *response.status_mut() = refusal.status();
            response
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
            response
        }
    }
}

fn normalise_request(request: &mut Request) -> Result<(), Refusal> {
    let uri = request.uri();
    let target_length = if uri.authority().is_some() {
        uri.to_string().len()
    } else {
        uri.path_and_query()
            .map_or(0, |target| target.as_str().len())
    };
    // The method, a space, the target, a space, `HTTP/1.1`, the line break.
    if request.method().as_str().len() + 1 + target_length + 1 + 8 + 2 > MAX_REQUEST_LINE {
        return Err(Refusal::TooLong);
    }
    let path = uri.path();
    if !path.starts_with('/') {
        // `*`, or a target with no path: nothing to read.
        return Ok(());
    }
    let normalised = normalise(path)?;
    if normalised == path {
        return Ok(());
    }
    let target = match uri.query() {
        Some(query) => format!("{normalised}?{query}"),
        None => normalised,
    };
    let mut parts = std::mem::take(request.uri_mut()).into_parts();
    parts.path_and_query = Some(
        target
            .parse::<PathAndQuery>()
            .map_err(|_| Refusal::BadRequest)?,
    );
    *request.uri_mut() = Uri::from_parts(parts).map_err(|_| Refusal::BadRequest)?;
    Ok(())
}

/// The path as the router should read it, written in the encoding that
/// decodes to the normalised path.
pub fn normalise(path: &str) -> Result<String, Refusal> {
    let decoded = decode(path)?;
    let mut normalised = remove_dot_segments(&decoded);
    if normalised.len() > 1 && normalised.ends_with('/') {
        normalised.pop();
    }
    Ok(encode(&normalised))
}

/// The value of the two hex digits after the `%` at `at`, if they are there.
fn escape_at(bytes: &[u8], at: usize) -> Option<u8> {
    if bytes.get(at) != Some(&b'%') {
        return None;
    }
    let digit = |byte: u8| (byte as char).to_digit(16);
    let high = digit(*bytes.get(at + 1)?)?;
    let low = digit(*bytes.get(at + 2)?)?;
    Some((high * 16 + low) as u8)
}

fn decode(path: &str) -> Result<String, Refusal> {
    let bytes = path.as_bytes();
    let mut decoded = String::with_capacity(path.len());
    let mut at = 0;
    while at < bytes.len() {
        let Some(value) = escape_at(bytes, at) else {
            decoded.push(bytes[at] as char);
            at += 1;
            continue;
        };
        match value {
            0 => return Err(Refusal::BadRequest),
            // Kept as written: it must not become a separator.
            b'/' => {
                decoded.push_str(&path[at..at + 3]);
                at += 3;
            }
            0x01..=0x7f => {
                decoded.push(value as char);
                at += 3;
            }
            _ => match utf8_character(bytes, at, value) {
                Some((character, escapes)) => {
                    decoded.push(character);
                    at += 3 * escapes;
                }
                // Not the start of a character: left as it was written.
                None => {
                    decoded.push_str(&path[at..at + 3]);
                    at += 3;
                }
            },
        }
    }
    Ok(decoded)
}

/// The character that the escape at `at` (whose value is `lead`) starts, and
/// how many escapes it takes, when the escapes after it complete one.
fn utf8_character(bytes: &[u8], at: usize, lead: u8) -> Option<(char, usize)> {
    let length = match lead {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let mut sequence = Vec::with_capacity(length);
    for index in 0..length {
        sequence.push(escape_at(bytes, at + 3 * index)?);
    }
    let text = std::str::from_utf8(&sequence).ok()?;
    Some((text.chars().next()?, length))
}

/// RFC 3986, 5.2.4, for a path that starts with a slash.
fn remove_dot_segments(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let mut kept: Vec<&str> = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let last = index + 1 == segments.len();
        match *segment {
            "." => {
                if last {
                    kept.push("");
                }
            }
            ".." => {
                kept.pop();
                if last {
                    kept.push("");
                }
            }
            other => kept.push(other),
        }
    }
    format!("/{}", kept.join("/"))
}

/// Everything that is not a plain character of a path is written as `%XX`,
/// including the `%` itself.
fn encode(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
                    | b'@'
                    | b'/'
            )
        {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &str) -> String {
        normalise(path).unwrap()
    }

    #[test]
    fn a_slash_at_the_end_is_dropped_once() {
        assert_eq!(read("/api/x/"), "/api/x");
        assert_eq!(read("/api/x//"), "/api/x/");
        assert_eq!(read("/"), "/");
        assert_eq!(read("//"), "/");
        assert_eq!(read("//api/x"), "//api/x");
        assert_eq!(read("/api//x"), "/api//x");
    }

    #[test]
    fn dot_segments_go_before_routing_and_cannot_leave_the_root() {
        assert_eq!(read("/health/./live"), "/health/live");
        assert_eq!(read("/health/x/../live"), "/health/live");
        assert_eq!(read("/health/live/.."), "/health");
        assert_eq!(read("/health/live/."), "/health/live");
        assert_eq!(read("/.."), "/");
        assert_eq!(read("/../health/live"), "/health/live");
        assert_eq!(read("/health/../../health/live"), "/health/live");
        assert_eq!(read("/a/.../b"), "/a/.../b");
        assert_eq!(read("/a/..;/b"), "/a/..;/b");
    }

    #[test]
    fn escapes_are_decoded_before_the_dot_segments_are_looked_for() {
        assert_eq!(read("/%68ealth/li%76e"), "/health/live");
        assert_eq!(read("/health/%2e%2E/health/live"), "/health/live");
        assert_eq!(read("/health/live/.%2e/live"), "/health/live");
    }

    #[test]
    fn an_encoded_slash_stays_an_encoded_slash() {
        // It is never a separator, so it is not a dot segment's end either.
        assert_eq!(read("/health%2flive"), "/health%252flive");
        assert_eq!(read("/health/..%2Flive"), "/health/..%252Flive");
        assert_eq!(read("/a%2f"), "/a%252f");
    }

    #[test]
    fn a_decoded_character_is_written_back_so_that_it_decodes_to_itself() {
        assert_eq!(read("/a%25b"), "/a%25b");
        assert_eq!(read("/a%20b%3fc%23d"), "/a%20b%3Fc%23d");
        assert_eq!(read("/a%5cb"), "/a%5Cb");
        assert_eq!(read("/%c3%a9"), "/%C3%A9");
        assert_eq!(read("/a%2bb+c"), "/a+b+c");
        assert_eq!(read("/a%7eb"), "/a~b");
    }

    #[test]
    fn what_is_not_a_character_stays_as_written() {
        // Lone and truncated sequences, overlong forms, surrogates.
        assert_eq!(read("/x%c3y"), "/x%25c3y");
        assert_eq!(read("/x%c3%28y"), "/x%25c3(y");
        assert_eq!(read("/x%e2%82y"), "/x%25e2%2582y");
        assert_eq!(read("/x%c0%afy"), "/x%25c0%25afy");
        assert_eq!(read("/x%ed%a0%80y"), "/x%25ed%25a0%2580y");
        assert_eq!(read("/x%e2%82%acy"), "/x%E2%82%ACy");
        assert_eq!(read("/x%e2%82%ac%82y"), "/x%E2%82%AC%2582y");
        // An escape that is not one.
        assert_eq!(read("/x%zzy%"), "/x%25zzy%25");
        assert_eq!(read("/x%4"), "/x%254");
    }

    #[test]
    fn a_null_character_is_refused_and_other_controls_are_not() {
        assert_eq!(normalise("/a%00b"), Err(Refusal::BadRequest));
        assert_eq!(normalise("/%00"), Err(Refusal::BadRequest));
        assert_eq!(read("/a%2500b"), "/a%2500b");
        assert_eq!(read("/a%01b%7f"), "/a%01b%7F");
    }
}
