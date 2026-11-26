//! Answering for a file on disk: what a browser or a download manager expects
//! of one, without ever holding the file in memory. Any group that serves a
//! file (releases, uploads, assets) answers with [`answer`].
//!
//! The pipeline sets no caching headers of its own: `Cache-Control` and the
//! like are the handler's to put on the response it gets back. The file is
//! opened by the handler, on a blocking thread (`tokio::fs` does that for
//! you), and what this module reads of it afterwards is read the same way.
//!
//! The file is saved under the name the publisher gave it, a download that was
//! interrupted continues with a range request, and a copy the client already
//! has is not sent again. The moment the file is dated with is the one its
//! manifest gives, not the time it was copied to the disk.

use std::io::{self, SeekFrom};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use bytes::Bytes;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::http_date;
use crate::values::Timestamp;

/// How much of the file is read at a time, and how many reads wait for a slow
/// client: a download holds a few chunks of memory however big the file is.
const CHUNK: usize = 64 * 1024;
const CHUNKS_IN_FLIGHT: usize = 2;

pub struct Download {
    /// The file as opened, whose length is `length`.
    pub file: File,
    pub length: u64,
    pub content_type: &'static str,
    /// The name a browser saves the file under.
    pub file_name: String,
    pub modified: Timestamp,
}

/// What the conditions of a request say about sending the file at all. When
/// several are given the heaviest wins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    Unspecified,
    NotModified,
    Process,
    PreconditionFailed,
}

pub async fn answer(headers: &HeaderMap, download: Download) -> Response {
    let modified = download.modified.unix_seconds();
    let mut response = Response::new(Body::empty());
    put(
        &mut response,
        header::LAST_MODIFIED,
        &http_date_of(modified),
    );

    match verdict(headers, modified) {
        Verdict::NotModified => {
            *response.status_mut() = StatusCode::NOT_MODIFIED;
            return response;
        }
        Verdict::PreconditionFailed => {
            *response.status_mut() = StatusCode::PRECONDITION_FAILED;
            return response;
        }
        Verdict::Unspecified | Verdict::Process => {}
    }
    put(&mut response, header::ACCEPT_RANGES, "bytes");
    put(&mut response, header::CONTENT_TYPE, download.content_type);
    put(
        &mut response,
        header::CONTENT_DISPOSITION,
        &content_disposition(&download.file_name),
    );

    let (start, length) = match requested_range(headers, modified, download.length) {
        Range::Whole => (0, download.length),
        Range::Part(first, last) => {
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            put(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes {first}-{last}/{}", download.length),
            );
            (first, last - first + 1)
        }
        Range::Unsatisfiable => {
            *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
            put(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes */{}", download.length),
            );
            put(&mut response, header::CONTENT_LENGTH, "0");
            return response;
        }
    };
    put(&mut response, header::CONTENT_LENGTH, &length.to_string());
    *response.body_mut() = stream(download.file, start, length);
    response
}

fn put(response: &mut Response, name: header::HeaderName, value: &str) {
    // Every value here is made of printable ASCII.
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

fn http_date_of(unix_seconds: i64) -> String {
    Timestamp::from_unix_seconds(unix_seconds).map_or_else(String::new, Timestamp::to_http_date)
}

/// The value of a header that is there exactly once. A header written on two
/// lines is read as a list, which is none of the single values these
/// conditions are.
fn single(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    let mut lines = headers.get_all(name).iter();
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return None;
    };
    line.to_str().ok()
}

/// The date a header holds, none when it is not there or not a date.
fn date_header(headers: &HeaderMap, name: header::HeaderName) -> Option<i64> {
    http_date::parse(single(headers, name)?)
}

/// There is no entity tag on these files, so the conditions on one have
/// nothing to compare with and are left out.
fn verdict(headers: &HeaderMap, modified: i64) -> Verdict {
    let now = Timestamp::now().unix_seconds();
    let mut verdict = Verdict::Unspecified;
    // A date in the future says nothing: no copy can be that new.
    if let Some(since) = date_header(headers, header::IF_MODIFIED_SINCE)
        && since <= now
    {
        verdict = verdict.max(if since < modified {
            Verdict::Process
        } else {
            Verdict::NotModified
        });
    }
    if let Some(since) = date_header(headers, header::IF_UNMODIFIED_SINCE)
        && since <= now
    {
        verdict = verdict.max(if modified > since {
            Verdict::PreconditionFailed
        } else {
            Verdict::Process
        });
    }
    verdict
}

pub(crate) enum Range {
    Whole,
    /// First and last byte, both within the file.
    Part(u64, u64),
    Unsatisfiable,
}

/// The one range a request asks for. Several ranges, and anything that does
/// not read as a range, are answered with the whole file.
fn requested_range(headers: &HeaderMap, modified: i64, length: u64) -> Range {
    // `If-Range` with a date holds the range to a file that has not changed
    // since; with an entity tag it has nothing to compare with.
    if let Some(since) = date_header(headers, header::IF_RANGE)
        && modified > since
    {
        return Range::Whole;
    }
    range_for(headers, length)
}

pub(crate) fn range_for(headers: &HeaderMap, length: u64) -> Range {
    if length == 0 {
        return Range::Whole;
    }
    let mut lines = headers.get_all(header::RANGE).iter();
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return Range::Whole;
    };
    let Ok(line) = line.to_str() else {
        return Range::Whole;
    };
    if line.contains(',') {
        return Range::Whole;
    }
    let Some((from, to)) = parse_range(line) else {
        return Range::Whole;
    };
    match (from, to) {
        (Some(from), to) if from < length => {
            Range::Part(from, to.map_or(length - 1, |to| to.min(length - 1)))
        }
        (Some(_), _) => Range::Unsatisfiable,
        (None, Some(last)) if last > 0 => Range::Part(length.saturating_sub(last), length - 1),
        (None, _) => Range::Unsatisfiable,
    }
}

/// `bytes=0-99`, `bytes=100-` and `bytes=-500`. The unit is any word, spaces
/// and tabs around the parts are let through, and each number fits in 63 bits.
fn parse_range(text: &str) -> Option<(Option<u64>, Option<u64>)> {
    let blank = |c: char| c == ' ' || c == '\t';
    let (unit, spec) = text.split_once('=')?;
    let unit = unit.trim_matches(blank);
    if unit.is_empty() || !unit.bytes().all(is_token_byte) {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    let number = |part: &str| -> Option<Option<u64>> {
        let part = part.trim_matches(blank);
        if part.is_empty() {
            return Some(None);
        }
        // More than nineteen digits is no number, however many are zeros.
        if part.len() > 19 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<i64>().ok().map(|n| Some(n as u64))
    };
    let (from, to) = (number(from)?, number(to)?);
    match (from, to) {
        (None, None) => None,
        (Some(from), Some(to)) if from > to => None,
        _ => Some((from, to)),
    }
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// `attachment`, with the name as the headers of every browser read it: a
/// plain form for old ones, which keeps ASCII and puts an underscore for
/// every other character, and the percent-encoded UTF-8 one of RFC 8187.
pub fn content_disposition(name: &str) -> String {
    let plain: String = name
        .encode_utf16()
        .map(|unit| match unit {
            0x20..=0x7E => char::from(unit as u8),
            _ => '_',
        })
        .collect();
    let simple = !plain.is_empty() && plain.bytes().all(is_token_byte);
    let mut text = String::from("attachment; filename=");
    if simple {
        text.push_str(&plain);
    } else {
        text.push('"');
        for c in plain.chars() {
            if c == '"' || c == '\\' {
                text.push('\\');
            }
            text.push(c);
        }
        text.push('"');
    }
    text.push_str("; filename*=UTF-8''");
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            text.push(char::from(byte));
        } else {
            text.push_str(&format!("%{byte:02X}"));
        }
    }
    text
}

/// A body that reads `length` bytes from `start` as the client takes them.
fn stream(mut file: File, start: u64, length: u64) -> Body {
    let (sender, body) = super::stream::channel(CHUNKS_IN_FLIGHT);
    tokio::spawn(async move {
        if start > 0
            && let Err(error) = file.seek(SeekFrom::Start(start)).await
        {
            let _ = sender.send(Err(error)).await;
            return;
        }
        let mut left = length;
        while left > 0 {
            let mut chunk = vec![0u8; usize::try_from(left).map_or(CHUNK, |left| left.min(CHUNK))];
            let mut filled = 0;
            while filled < chunk.len() {
                match file.read(&mut chunk[filled..]).await {
                    Ok(0) => {
                        // The file got shorter than it was when it was checked.
                        let _ = sender.send(Err(io::ErrorKind::UnexpectedEof.into())).await;
                        return;
                    }
                    Ok(read) => filled += read,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                }
            }
            left -= filled as u64;
            // The client has gone: stop reading.
            if sender.send(Ok(Bytes::from(chunk))).await.is_err() {
                return;
            }
        }
    });
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_is_written_for_old_and_new_browsers() {
        for (name, expected) in [
            (
                "plain.bin",
                "attachment; filename=plain.bin; filename*=UTF-8''plain.bin",
            ),
            (
                "a b.bin",
                "attachment; filename=\"a b.bin\"; filename*=UTF-8''a%20b.bin",
            ),
            (
                "ü.bin",
                "attachment; filename=_.bin; filename*=UTF-8''%C3%BC.bin",
            ),
            (
                "a\"b.bin",
                "attachment; filename=\"a\\\"b.bin\"; filename*=UTF-8''a%22b.bin",
            ),
            (
                "a;b.bin",
                "attachment; filename=\"a;b.bin\"; filename*=UTF-8''a%3Bb.bin",
            ),
            (
                "a%b.bin",
                "attachment; filename=a%b.bin; filename*=UTF-8''a%25b.bin",
            ),
            (
                "a'b.bin",
                "attachment; filename=a'b.bin; filename*=UTF-8''a%27b.bin",
            ),
            (
                "a\\b.bin",
                "attachment; filename=\"a\\\\b.bin\"; filename*=UTF-8''a%5Cb.bin",
            ),
            (
                "~x.bin",
                "attachment; filename=~x.bin; filename*=UTF-8''~x.bin",
            ),
            (
                "日本.bin",
                "attachment; filename=__.bin; filename*=UTF-8''%E6%97%A5%E6%9C%AC.bin",
            ),
            (
                "😀",
                "attachment; filename=__; filename*=UTF-8''%F0%9F%98%80",
            ),
            ("a\tb", "attachment; filename=a_b; filename*=UTF-8''a%09b"),
            (
                "a\u{7f}b",
                "attachment; filename=a_b; filename*=UTF-8''a%7Fb",
            ),
            ("@x", "attachment; filename=\"@x\"; filename*=UTF-8''%40x"),
            ("a*b", "attachment; filename=a*b; filename*=UTF-8''a%2Ab"),
            (
                "a!b$c&d^e`f|g",
                "attachment; filename=a!b$c&d^e`f|g; filename*=UTF-8''a!b$c&d^e`f|g",
            ),
            (
                "a(b)",
                "attachment; filename=\"a(b)\"; filename*=UTF-8''a%28b%29",
            ),
        ] {
            assert_eq!(content_disposition(name), expected, "{name:?}");
        }
    }

    #[test]
    fn range_parser_handles_supported_and_invalid_byte_ranges() {
        let read = |text: &str| parse_range(text);

        assert_eq!(read("bytes=0-5"), Some((Some(0), Some(5))));
        assert_eq!(read(" bytes = 0 - 5 "), Some((Some(0), Some(5))));
        assert_eq!(read("BYTES=00-05"), Some((Some(0), Some(5))));
        assert_eq!(read("items=1-2"), Some((Some(1), Some(2))));
        assert_eq!(read("bytes=5-"), Some((Some(5), None)));
        assert_eq!(read("bytes=-5"), Some((None, Some(5))));
        assert_eq!(
            read("bytes=0-9223372036854775807"),
            Some((Some(0), Some(i64::MAX as u64)))
        );
        for refused in [
            "",
            "bytes",
            "bytes=",
            "bytes=-",
            "bytes=1",
            "bytes=5-2",
            "bytes=+1-5",
            "bytes=1-+5",
            "bytes=1-2-3",
            "bytes=x0-5",
            "bytes=0-5x",
            "bytes==0-5",
            "bytes 0-5",
            "=1-2",
            "by tes=1-2",
            "bytes=0-9223372036854775808",
            "bytes=-9223372036854775808",
            "bytes=\"0-5\"",
            "bytes=0000000000000000000000000-5",
            "bytes=0-00000000000000000000000005",
        ] {
            assert_eq!(read(refused), None, "{refused:?}");
        }
    }

    fn range_of(line: &str, length: u64) -> String {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, line.parse().unwrap());
        match requested_range(&headers, 0, length) {
            Range::Whole => "whole".into(),
            Range::Part(first, last) => format!("{first}-{last}"),
            Range::Unsatisfiable => "none".into(),
        }
    }

    #[test]
    fn a_range_is_fitted_to_the_file() {
        for (line, expected) in [
            ("bytes=0-9", "0-9"),
            ("bytes=1020-", "1020-1027"),
            ("bytes=-5", "1023-1027"),
            ("bytes=-5000", "0-1027"),
            ("bytes=0-99999", "0-1027"),
            ("bytes=1027-1027", "1027-1027"),
            ("bytes=1028-", "none"),
            ("bytes=2000-2001", "none"),
            ("bytes=-0", "none"),
            ("bytes=0-1,5-9", "whole"),
            ("bytes=abc", "whole"),
            ("bytes=5-2", "whole"),
        ] {
            assert_eq!(range_of(line, 1028), expected, "{line}");
        }
    }

    #[test]
    fn a_range_asked_for_twice_is_not_one() {
        let mut headers = HeaderMap::new();
        headers.append(header::RANGE, "bytes=0-5".parse().unwrap());
        headers.append(header::RANGE, "bytes=7-9".parse().unwrap());

        assert!(matches!(requested_range(&headers, 0, 100), Range::Whole));
    }

    fn headers_of(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(name.clone(), value.parse().unwrap());
        }
        headers
    }

    const MODIFIED: i64 = 1_795_169_472;

    #[test]
    fn the_conditions_of_a_request_decide_whether_the_file_is_sent() {
        let on =
            |name: header::HeaderName, date: &str| verdict(&headers_of(&[(name, date)]), MODIFIED);

        assert!(
            on(header::IF_MODIFIED_SINCE, "Fri, 20 Nov 2026 10:11:12 GMT") == Verdict::NotModified
        );
        assert!(on(header::IF_MODIFIED_SINCE, "Fri, 20 Nov 2026 10:11:11 GMT") == Verdict::Process);
        assert!(on(header::IF_MODIFIED_SINCE, "not a date") == Verdict::Unspecified);
        assert!(
            on(header::IF_UNMODIFIED_SINCE, "Fri, 20 Nov 2026 10:11:11 GMT")
                == Verdict::PreconditionFailed
        );
        assert!(
            on(header::IF_UNMODIFIED_SINCE, "Fri, 20 Nov 2026 10:11:12 GMT") == Verdict::Process
        );
        assert!(
            on(header::IF_MODIFIED_SINCE, "Fri, 20 Nov 2090 10:11:12 GMT") == Verdict::Unspecified
        );
        // The heaviest wins: a request that is modified for one condition and
        // not for the other is served.
        let both = headers_of(&[
            (header::IF_MODIFIED_SINCE, "Fri, 20 Nov 2026 10:11:12 GMT"),
            (header::IF_UNMODIFIED_SINCE, "Fri, 20 Nov 2026 10:11:12 GMT"),
        ]);
        assert!(verdict(&both, MODIFIED) == Verdict::Process);
    }
}
