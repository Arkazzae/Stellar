//! Reading a `multipart/form-data` body as it arrives, a piece at a time.
//!
//! A person who is signed in may send 128 MiB in one request, so nothing here
//! holds a part whole: the reader hands over the head of each part (its
//! headers) and then its bytes in the pieces the network delivered them in, and
//! the caller decides where they go. What the reader itself keeps is a few
//! kilobytes: the unread tail of the last piece, and one part's headers.
//!
//! The limits are the ones the first server's form reader had, and are
//! enforced while reading:
//!
//! - the boundary is at most 128 characters;
//! - a part has at most 16 headers, and their lines add up to less than 16 KiB;
//! - a body has at most 1024 parts;
//! - a request is at most 128 MiB, and one that declares more is refused
//!   before a byte of it is read;
//! - a body that arrives slower than 240 bytes a second, once it has had five
//!   seconds to get going, is cut off, as is one that sends nothing for 30
//!   seconds.
//!
//! What a request shows that the format forbids is a refusal ([`FormError`]):
//! a body that ends before the closing boundary, a header without a name, a
//! line that ends in a bare line feed. A preamble before the first boundary and
//! an epilogue after the last are ignored, and so is whatever follows a
//! boundary on its line, as the first server did.

use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, header};
use bytes::{Buf, Bytes, BytesMut};
use http_body_util::BodyExt;
use tokio::time::Instant;

use super::error::ApiError;

/// The longest boundary the reader takes.
pub const MAX_BOUNDARY_CHARACTERS: usize = 128;
/// The most headers one part has.
pub const MAX_PART_HEADERS: usize = 16;
/// What the header lines of one part may add up to.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// The most parts a body has.
pub const MAX_PARTS: usize = 1024;

/// How long the body may take to get going, and the window its speed is
/// measured over.
const GRACE: Duration = Duration::from_secs(5);
/// How long the client may send nothing at all.
const STALL: Duration = Duration::from_secs(30);
/// The speed under which a body is cut off.
const MINIMUM_BYTES_PER_SECOND: u64 = 240;

/// Why a body was not read to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormError {
    /// The request is not a form: a body that is neither multipart nor a form.
    NotAForm,
    /// The form breaks the format or one of its limits.
    Malformed,
    /// The request is larger than the server takes.
    TooLarge,
    /// The client sends too slowly, or has stopped sending.
    TooSlow,
}

impl From<FormError> for ApiError {
    fn from(error: FormError) -> ApiError {
        match error {
            FormError::NotAForm => ApiError::status(axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE),
            FormError::Malformed => ApiError::bad_request(),
            FormError::TooLarge => ApiError::TooLarge,
            FormError::TooSlow => ApiError::status(axum::http::StatusCode::REQUEST_TIMEOUT),
        }
    }
}

/// The head of a part: its header lines, in the order they came.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartHead {
    pub headers: Vec<(String, String)>,
}

impl PartHead {
    /// The value of a header, whatever the case of its name; several lines of
    /// the same name are joined with a comma.
    pub fn header(&self, name: &str) -> Option<String> {
        let mut found: Option<String> = None;
        for (have, value) in &self.headers {
            if have.eq_ignore_ascii_case(name) {
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

    /// What `Content-Disposition` makes of the part, or `Err` when there is
    /// none or it cannot be read: the form is refused as a whole.
    pub fn disposition(&self) -> Result<Disposition, FormError> {
        let text = self
            .header("content-disposition")
            .ok_or(FormError::Malformed)?;
        Disposition::parse(&text).ok_or(FormError::Malformed)
    }
}

/// What the `Content-Disposition` of a part says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A file: `form-data` with a file name.
    File,
    /// A value: `form-data` without one.
    Field,
    /// Any other disposition; the part is not part of the form.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disposition {
    pub kind: Kind,
    /// The name of the form field, as sent, without its quotes.
    pub name: String,
    /// The file name as sent: the extended form when there is one.
    pub file_name: Option<String>,
}

impl Disposition {
    /// `form-data; name="x"; filename="y"`: the type, then parameters, whose
    /// names are not case sensitive and whose values are tokens or quoted
    /// strings. The type itself has to be written `form-data`.
    pub fn parse(text: &str) -> Option<Disposition> {
        let (kind_text, mut rest) = match text.find(';') {
            Some(at) => (&text[..at], &text[at + 1..]),
            None => (text, ""),
        };
        let kind_text = kind_text.trim_matches(is_blank);
        if kind_text.is_empty() || !kind_text.bytes().all(is_token_byte) {
            return None;
        }
        let mut name: Option<String> = None;
        let mut plain: Option<String> = None;
        let mut extended: Option<String> = None;
        loop {
            rest = rest.trim_start_matches(is_blank);
            if rest.is_empty() {
                break;
            }
            if let Some(after) = rest.strip_prefix(';') {
                rest = after;
                continue;
            }
            let end = rest.find(['=', ';']).unwrap_or(rest.len());
            let parameter = rest[..end].trim_matches(is_blank);
            if parameter.is_empty() || !parameter.bytes().all(is_token_byte) {
                return None;
            }
            rest = &rest[end..];
            let mut value = "";
            if let Some(after) = rest.strip_prefix('=') {
                let after = after.trim_start_matches(is_blank);
                let (taken, left) = take_value(after)?;
                value = taken;
                rest = left;
            }
            let slot = if parameter.eq_ignore_ascii_case("name") {
                &mut name
            } else if parameter.eq_ignore_ascii_case("filename") {
                &mut plain
            } else if parameter.eq_ignore_ascii_case("filename*") {
                &mut extended
            } else {
                continue;
            };
            if slot.is_none() {
                *slot = Some(unquote(value).to_string());
            }
        }
        let extended = extended
            .filter(|value| !value.is_empty())
            .map(|value| decode_extended(&value).unwrap_or(value));
        let plain = plain.filter(|value| !value.is_empty());
        let file_name = extended.or(plain);
        let kind = if kind_text != "form-data" {
            Kind::Other
        } else if file_name.is_some() {
            Kind::File
        } else {
            Kind::Field
        };
        Some(Disposition {
            kind,
            name: name.unwrap_or_default(),
            file_name,
        })
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// A parameter's value: a quoted string, whose backslash escapes the next
/// character, or a token up to the next semicolon. The value is returned as
/// written, quotes included, with what is left after it.
fn take_value(text: &str) -> Option<(&str, &str)> {
    if let Some(inner) = text.strip_prefix('"') {
        let mut escaped = false;
        for (at, c) in inner.char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                let end = at + 2;
                return Some((&text[..end], &text[end..]));
            }
        }
        return None;
    }
    let end = text.find(';').unwrap_or(text.len());
    Some((text[..end].trim_end_matches(is_blank), &text[end..]))
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

/// `UTF-8''caf%C3%A9.png`: a character set, a language and percent-encoded
/// bytes. Only UTF-8 and Latin-1 are known.
fn decode_extended(value: &str) -> Option<String> {
    let (charset, rest) = value.split_once('\'')?;
    let (_, encoded) = rest.split_once('\'')?;
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let high = char::from(*bytes.get(at + 1)?).to_digit(16)?;
            let low = char::from(*bytes.get(at + 2)?).to_digit(16)?;
            out.push(u8::try_from(high * 16 + low).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    if charset.eq_ignore_ascii_case("utf-8") {
        Some(String::from_utf8_lossy(&out).into_owned())
    } else if charset.eq_ignore_ascii_case("iso-8859-1") {
        Some(out.iter().map(|byte| char::from(*byte)).collect())
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Before the first boundary: whatever there is, is a preamble.
    Preamble,
    /// Just after a boundary: the closing `--`, or the rest of its line, and
    /// then the headers of the next part.
    AfterBoundary,
    /// In the bytes of a part.
    Body,
    /// The closing boundary has been read.
    Finished,
}

/// A form being read.
pub struct Multipart {
    body: Body,
    buffer: BytesMut,
    /// `--boundary`, and `\r\n--boundary` for every boundary after the first.
    first: Vec<u8>,
    inner: Vec<u8>,
    state: State,
    parts: usize,
    ceiling: u64,
    received: u64,
    ended: bool,
    window_start: Instant,
    window_bytes: u64,
}

impl Multipart {
    /// Begins reading the body of a request whose headers are `headers`. A
    /// request that is not multipart form data, or names no boundary, or one
    /// that is too long, is refused here; so is one that declares a body over
    /// `ceiling`.
    pub fn open(headers: &HeaderMap, body: Body, ceiling: usize) -> Result<Multipart, FormError> {
        let declared = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .ok_or(FormError::NotAForm)?;
        let boundary = boundary_of(content_type)?;
        if declared.is_some_and(|length| length > ceiling as u64) {
            return Err(FormError::TooLarge);
        }
        let mut first = b"--".to_vec();
        first.extend_from_slice(boundary.as_bytes());
        let mut inner = b"\r\n".to_vec();
        inner.extend_from_slice(&first);
        Ok(Multipart {
            body,
            buffer: BytesMut::new(),
            first,
            inner,
            state: State::Preamble,
            parts: 0,
            ceiling: ceiling as u64,
            received: 0,
            ended: false,
            window_start: Instant::now(),
            window_bytes: 0,
        })
    }

    /// The head of the next part, after the rest of the one being read is
    /// passed over. `None` after the closing boundary.
    pub async fn next_part(&mut self) -> Result<Option<PartHead>, FormError> {
        while self.chunk().await?.is_some() {}
        loop {
            match self.state {
                State::Finished => return Ok(None),
                State::Preamble => {
                    let delimiter = self.first.clone();
                    self.skip_to(&delimiter).await?;
                    self.state = State::AfterBoundary;
                }
                State::AfterBoundary => {
                    if self.fill_to(2).await? < 2 {
                        return Err(FormError::Malformed);
                    }
                    if &self.buffer[..2] == b"--" {
                        self.state = State::Finished;
                        return Ok(None);
                    }
                    // Whatever else is on the line of a boundary is ignored.
                    let mut budget = MAX_HEADER_BYTES;
                    self.skip_line(&mut budget).await?;
                    let head = self.read_head().await?;
                    self.parts += 1;
                    if self.parts > MAX_PARTS {
                        return Err(FormError::Malformed);
                    }
                    self.state = State::Body;
                    return Ok(Some(head));
                }
                // The part was passed over above.
                State::Body => return Err(FormError::Malformed),
            }
        }
    }

    /// The next bytes of the current part, or `None` once it is over.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, FormError> {
        loop {
            if self.state != State::Body {
                return Ok(None);
            }
            if let Some(at) = find(&self.buffer, &self.inner) {
                let data = self.buffer.split_to(at).freeze();
                self.buffer.advance(self.inner.len());
                self.state = State::AfterBoundary;
                if data.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(data));
            }
            // What cannot be the start of a boundary is the part's.
            let keep = self.inner.len() - 1;
            if self.buffer.len() > keep {
                let give = self.buffer.len() - keep;
                return Ok(Some(self.buffer.split_to(give).freeze()));
            }
            if !self.pull().await? {
                return Err(FormError::Malformed);
            }
        }
    }

    /// Pulls the next piece of the body into the buffer. False at its end.
    async fn pull(&mut self) -> Result<bool, FormError> {
        if self.ended {
            return Ok(false);
        }
        loop {
            let frame = match tokio::time::timeout(STALL, self.body.frame()).await {
                Err(_) => return Err(FormError::TooSlow),
                Ok(None) => {
                    self.ended = true;
                    return Ok(false);
                }
                // The client went away in the middle of its body.
                Ok(Some(Err(_))) => return Err(FormError::Malformed),
                Ok(Some(Ok(frame))) => frame,
            };
            let Ok(data) = frame.into_data() else {
                // Trailers: not part of the form.
                continue;
            };
            if data.is_empty() {
                continue;
            }
            self.received += data.len() as u64;
            if self.received > self.ceiling {
                return Err(FormError::TooLarge);
            }
            self.window_bytes += data.len() as u64;
            let elapsed = self.window_start.elapsed();
            if elapsed >= GRACE {
                if self.window_bytes < MINIMUM_BYTES_PER_SECOND * elapsed.as_secs() {
                    return Err(FormError::TooSlow);
                }
                self.window_start = Instant::now();
                self.window_bytes = 0;
            }
            self.buffer.extend_from_slice(&data);
            return Ok(true);
        }
    }

    /// Pulls until `wanted` bytes are in the buffer or the body is over;
    /// says how many there are.
    async fn fill_to(&mut self, wanted: usize) -> Result<usize, FormError> {
        while self.buffer.len() < wanted {
            if !self.pull().await? {
                break;
            }
        }
        Ok(self.buffer.len())
    }

    /// Passes over everything up to and including the first `delimiter`.
    async fn skip_to(&mut self, delimiter: &[u8]) -> Result<(), FormError> {
        loop {
            if let Some(at) = find(&self.buffer, delimiter) {
                self.buffer.advance(at + delimiter.len());
                return Ok(());
            }
            let keep = delimiter.len() - 1;
            if self.buffer.len() > keep {
                let drop = self.buffer.len() - keep;
                self.buffer.advance(drop);
            }
            if !self.pull().await? {
                return Err(FormError::Malformed);
            }
        }
    }

    /// Passes over a line, its end included. A line longer than the budget
    /// is a refusal.
    async fn skip_line(&mut self, budget: &mut usize) -> Result<(), FormError> {
        loop {
            if let Some(at) = find(&self.buffer, b"\r\n") {
                if at > *budget {
                    return Err(FormError::Malformed);
                }
                self.buffer.advance(at + 2);
                return Ok(());
            }
            if self.buffer.len() > *budget + 1 {
                return Err(FormError::Malformed);
            }
            if !self.pull().await? {
                return Err(FormError::Malformed);
            }
        }
    }

    /// The header lines of a part, up to the empty line.
    async fn read_head(&mut self) -> Result<PartHead, FormError> {
        let mut head = PartHead::default();
        let mut used = 0usize;
        loop {
            // A line may use what the lines before it left, less its own end.
            let room = MAX_HEADER_BYTES - used;
            let line = self.read_line(room).await?;
            if line.is_empty() {
                return Ok(head);
            }
            used += line.len();
            if head.headers.len() >= MAX_PART_HEADERS {
                return Err(FormError::Malformed);
            }
            let text = String::from_utf8_lossy(&line).into_owned();
            let Some((name, value)) = text.split_once(':') else {
                return Err(FormError::Malformed);
            };
            if name.is_empty() {
                return Err(FormError::Malformed);
            }
            head.headers
                .push((name.to_string(), value.trim_matches(is_blank).to_string()));
        }
    }

    /// One line, without its end, shorter than `room`.
    async fn read_line(&mut self, room: usize) -> Result<Vec<u8>, FormError> {
        loop {
            if let Some(at) = find(&self.buffer, b"\r\n") {
                if at >= room {
                    return Err(FormError::Malformed);
                }
                let line = self.buffer.split_to(at).to_vec();
                self.buffer.advance(2);
                return Ok(line);
            }
            if self.buffer.len() > room {
                return Err(FormError::Malformed);
            }
            if !self.pull().await? {
                return Err(FormError::Malformed);
            }
        }
    }
}

/// The boundary a `Content-Type` names, when it is `multipart/form-data`.
/// `application/x-www-form-urlencoded` is a form too, but not one that carries
/// a file: it is read as an empty one.
fn boundary_of(content_type: &str) -> Result<String, FormError> {
    let mut parts = content_type.split(';');
    let media_type = parts.next().unwrap_or("").trim_matches(is_blank);
    if media_type.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        return Err(FormError::Malformed);
    }
    if !media_type.eq_ignore_ascii_case("multipart/form-data") {
        return Err(FormError::NotAForm);
    }
    for parameter in parts {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim_matches(is_blank).eq_ignore_ascii_case("boundary") {
            let value = unquote(value.trim_matches(is_blank));
            if value.is_empty() || value.chars().count() > MAX_BOUNDARY_CHARACTERS {
                return Err(FormError::Malformed);
            }
            return Ok(value.to_string());
        }
    }
    Err(FormError::Malformed)
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut from = 0;
    while let Some(at) = haystack[from..].iter().position(|byte| *byte == first) {
        let start = from + at;
        if haystack[start..].starts_with(needle) {
            return Some(start);
        }
        from = start + 1;
    }
    None
}

#[cfg(test)]
mod tests;
