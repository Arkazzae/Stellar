//! Content codings of an answer: `gzip` and `deflate` are decoded, and
//! nothing else is offered.
//!
//! A request offers `gzip, deflate` unless its caller turned decoding off, in
//! which case it says `identity` and the body is handed over as it came.
//! Brotli and zstd are not offered because there is no decoder for them; a
//! server that sends one anyway makes reading the body fail, while the status
//! and headers stay readable.
//!
//! # What the decoder does
//!
//! - The codings of `Content-Encoding` are undone last to first, as many as
//!   [`MAX_CODINGS`] of them; `gzip, gzip` is two layers. `identity` in any
//!   case, and empty entries, are nothing. `x-gzip` is `gzip`.
//! - `deflate` is the zlib format by the standard, and some servers send raw
//!   DEFLATE under that name. Browsers, curl and the common libraries read
//!   the first two bytes: a valid zlib header means zlib, anything else raw.
//!   So does this client.
//! - Strict: a damaged stream, a bad checksum, a body that ends inside the
//!   stream and bytes after the stream (other than another gzip member) are
//!   errors, after the bytes that were good have been delivered.
//! - A body of no bytes at all, with a coding declared, is an empty body.
//!   Servers do send that for an empty page, and browsers read it as empty.
//! - An answer that has no body (to HEAD, 1xx, 204, 304) is not decoded
//!   whatever it declares: its coding describes what a GET would carry.
//! - When a body is decoded the answer's `Content-Encoding` and
//!   `Content-Length` are removed, since neither describes what the caller
//!   reads any more.
//!
//! # Memory
//!
//! Input is pushed in the pieces it arrives in and output is pulled in
//! pieces of at most [`PIECE`] bytes, so a body that inflates a thousandfold
//! is never held, in any layer: each layer is capped at what the caller's
//! ceiling leaves, and the last one stops the moment it would pass it.

use bytes::Bytes;
use http::header::CONTENT_ENCODING;
use http::{HeaderMap, Method, StatusCode};
use stellarr_deflate::{Format, PushDecoder, Step};

use crate::error::{Error, Result};

/// What a request offers when it decodes.
pub(crate) const ACCEPT_DECODED: &str = "gzip, deflate";

/// What a request says when it does not.
pub(crate) const ACCEPT_RAW: &str = "identity";

/// Most codings undone on one body. Each costs a window of memory and the
/// work of a pass; browsers and curl cap the chain too.
pub(crate) const MAX_CODINGS: usize = 4;

/// The most a read hands out at once.
const PIECE: usize = 64 * 1024;

/// What one layer hands the next, at most.
const STAGE: usize = 16 * 1024;

/// How the body of an answer is turned into what the caller reads.
pub(crate) enum Decoder {
    /// The bytes are what the server meant, or the caller wants them as they
    /// are.
    Pass,
    /// A coding this client cannot undo, or too many of them.
    Unsupported(&'static str),
    Chain(Box<Chain>),
}

/// What [`Decoder::pull`] found.
pub(crate) enum Pull {
    Data(Bytes),
    /// All that was pushed is used up.
    Wire,
    End,
}

pub(crate) struct Chain {
    /// Nearest the wire first.
    layers: Vec<Layer>,
    out: Vec<u8>,
    wire_seen: bool,
    /// The body had no bytes: there is nothing to decode and nothing wrong.
    empty: bool,
}

struct Layer {
    decoder: PushDecoder,
    /// Where the layer below hands over what this one reads.
    stage: Vec<u8>,
}

/// The codings the headers declare, in the order they were applied. `Err`
/// names why they cannot be undone.
fn codings(headers: &HeaderMap) -> std::result::Result<Vec<Format>, &'static str> {
    let mut found = Vec::new();
    for value in headers.get_all(CONTENT_ENCODING) {
        let Ok(value) = value.to_str() else {
            return Err("the answer's content coding is not text");
        };
        for coding in value.split(',') {
            let coding = coding.trim();
            if coding.is_empty() || coding.eq_ignore_ascii_case("identity") {
                continue;
            }
            if coding.eq_ignore_ascii_case("gzip") || coding.eq_ignore_ascii_case("x-gzip") {
                found.push(Format::Gzip);
            } else if coding.eq_ignore_ascii_case("deflate") {
                found.push(Format::ZlibOrRaw);
            } else {
                return Err("the answer is compressed in a way this client does not decode");
            }
        }
    }
    if found.len() > MAX_CODINGS {
        return Err("the answer is compressed too many times");
    }
    Ok(found)
}

/// Whether an answer to this request can have a body at all.
fn has_body(method: &Method, status: StatusCode) -> bool {
    *method != Method::HEAD
        && !status.is_informational()
        && status != StatusCode::NO_CONTENT
        && status != StatusCode::NOT_MODIFIED
}

impl Decoder {
    /// The decoder for an answer, and the headers adjusted to match it.
    /// `room` is how many bytes of decoded body the caller's ceiling leaves.
    pub(crate) fn for_response(
        method: &Method,
        status: StatusCode,
        headers: &mut HeaderMap,
        decode: bool,
        room: u64,
    ) -> Decoder {
        if !decode || !has_body(method, status) {
            return Decoder::Pass;
        }
        let codings = match codings(headers) {
            Ok(codings) => codings,
            Err(why) => return Decoder::Unsupported(why),
        };
        if codings.is_empty() {
            return Decoder::Pass;
        }
        // The first coding listed was applied first, so it is undone last,
        // and its layer is the one whose output the caller reads.
        let last = codings.len() - 1;
        let layers = codings
            .iter()
            .rev()
            .enumerate()
            .map(|(index, format)| {
                let limit = if index == last {
                    room
                } else {
                    // What a layer hands up is the next layer's compressed
                    // input, which can be a little larger than what that
                    // decodes to (stored blocks, headers).
                    room.saturating_add(room / 1024).saturating_add(4096)
                };
                Layer {
                    decoder: PushDecoder::new(*format).with_limit(limit),
                    stage: vec![0; STAGE],
                }
            })
            .collect();
        headers.remove(CONTENT_ENCODING);
        headers.remove(http::header::CONTENT_LENGTH);
        Decoder::Chain(Box::new(Chain {
            layers,
            out: vec![0; PIECE],
            wire_seen: false,
            empty: false,
        }))
    }
}

impl Chain {
    pub(crate) fn push(&mut self, data: &[u8]) {
        self.wire_seen = true;
        self.layers[0].decoder.push(data);
    }

    /// The body has no more bytes.
    pub(crate) fn finish_wire(&mut self) {
        if self.wire_seen {
            self.layers[0].decoder.finish();
        } else {
            self.empty = true;
        }
    }

    /// The next piece of the decoded body.
    pub(crate) fn pull(&mut self) -> Result<Pull> {
        if self.empty {
            return Ok(Pull::End);
        }
        let mut out = std::mem::take(&mut self.out);
        let top = self.layers.len() - 1;
        let result = self.pull_level(top, &mut out);
        let pulled = match &result {
            Ok(Level::Data(n)) => Some(Bytes::copy_from_slice(&out[..*n])),
            _ => None,
        };
        self.out = out;
        match result? {
            Level::Data(_) => Ok(Pull::Data(pulled.unwrap_or_default())),
            Level::Wire => Ok(Pull::Wire),
            Level::End => Ok(Pull::End),
        }
    }

    /// Reads from layer `level`, feeding it from the layer below when it
    /// runs dry.
    fn pull_level(&mut self, level: usize, out: &mut [u8]) -> Result<Level> {
        loop {
            match self.layers[level].decoder.read(out).map_err(damaged)? {
                Step::Data(n) => return Ok(Level::Data(n)),
                Step::End => return Ok(Level::End),
                Step::NeedInput => {}
            }
            if level == 0 {
                return Ok(Level::Wire);
            }
            let mut stage = std::mem::take(&mut self.layers[level].stage);
            let below = self.pull_level(level - 1, &mut stage);
            match &below {
                Ok(Level::Data(n)) => self.layers[level].decoder.push(&stage[..*n]),
                Ok(Level::End) => self.layers[level].decoder.finish(),
                _ => {}
            }
            self.layers[level].stage = stage;
            if matches!(below?, Level::Wire) {
                return Ok(Level::Wire);
            }
        }
    }
}

enum Level {
    Data(usize),
    Wire,
    End,
}

/// What a stream that cannot be decoded is called in an error: the reason
/// only, never a byte of it.
fn damaged(error: stellarr_deflate::Error) -> Error {
    match error {
        stellarr_deflate::Error::OutputLimit => Error::TooLarge,
        other => Error::Body(format!("the compressed answer is damaged: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append(CONTENT_ENCODING, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn kind(values: &[&str], method: Method, status: u16, decode: bool) -> &'static str {
        let mut map = headers(values);
        let decoder = Decoder::for_response(
            &method,
            StatusCode::from_u16(status).unwrap(),
            &mut map,
            decode,
            1000,
        );
        match decoder {
            Decoder::Pass => "pass",
            Decoder::Unsupported(_) => "unsupported",
            Decoder::Chain(chain) => match chain.layers.len() {
                1 => "one",
                2 => "two",
                _ => "many",
            },
        }
    }

    #[test]
    fn the_codings_that_are_nothing() {
        for values in [
            &[][..],
            &["identity"],
            &["Identity"],
            &["IDENTITY"],
            &["  identity  "],
            &[""],
            &[" , ,"],
            &["identity, identity"],
            &["identity", "identity"],
        ] {
            assert_eq!(kind(values, Method::GET, 200, true), "pass", "{values:?}");
        }
    }

    #[test]
    fn the_codings_that_are_decoded_in_any_case_and_any_spelling() {
        for (values, expected) in [
            (&["gzip"][..], "one"),
            (&["GZip"], "one"),
            (&["x-gzip"], "one"),
            (&["deflate"], "one"),
            (&["identity, gzip"], "one"),
            (&["gzip,identity"], "one"),
            (&["gzip, gzip"], "two"),
            (&["gzip", "deflate"], "two"),
            (&[" gzip ,\t deflate "], "two"),
            (&["gzip, gzip, gzip, gzip"], "many"),
        ] {
            assert_eq!(kind(values, Method::GET, 200, true), expected, "{values:?}");
        }
    }

    #[test]
    fn the_codings_that_are_not() {
        for values in [
            &["br"][..],
            &["zstd"],
            &["compress"],
            &["x-compress"],
            &["gzip, br"],
            &["br, gzip"],
            &["gzip;q=1"],
            &["gzipp"],
            &["gzip, gzip, gzip, gzip, gzip"],
        ] {
            assert_eq!(
                kind(values, Method::GET, 200, true),
                "unsupported",
                "{values:?}"
            );
        }
        let mut map = HeaderMap::new();
        map.append(
            CONTENT_ENCODING,
            HeaderValue::from_bytes(b"gz\xffip").unwrap(),
        );
        assert!(matches!(
            Decoder::for_response(&Method::GET, StatusCode::OK, &mut map, true, 10),
            Decoder::Unsupported(_)
        ));
    }

    #[test]
    fn an_answer_that_has_no_body_is_not_decoded_whatever_it_declares() {
        for (method, status) in [
            (Method::HEAD, 200),
            (Method::GET, 204),
            (Method::GET, 304),
            (Method::GET, 100),
            (Method::GET, 101),
        ] {
            assert_eq!(kind(&["br"], method.clone(), status, true), "pass");
            assert_eq!(kind(&["gzip"], method, status, true), "pass");
        }
        // An error answer has a body like any other.
        assert_eq!(kind(&["gzip"], Method::GET, 404, true), "one");
        assert_eq!(kind(&["gzip"], Method::POST, 500, true), "one");
    }

    #[test]
    fn turned_off_nothing_is_touched() {
        assert_eq!(kind(&["gzip"], Method::GET, 200, false), "pass");
        assert_eq!(kind(&["br"], Method::GET, 200, false), "pass");
    }

    #[test]
    fn a_decoded_answer_loses_the_headers_that_no_longer_fit_and_others_keep_them() {
        let mut map = headers(&["gzip"]);
        map.insert(http::header::CONTENT_LENGTH, HeaderValue::from_static("20"));
        map.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html"),
        );
        Decoder::for_response(&Method::GET, StatusCode::OK, &mut map, true, 100);
        assert!(map.get(CONTENT_ENCODING).is_none());
        assert!(map.get(http::header::CONTENT_LENGTH).is_none());
        assert!(map.get(http::header::CONTENT_TYPE).is_some());

        for (values, decode) in [(&["br"][..], true), (&["gzip"], false)] {
            let mut map = headers(values);
            map.insert(http::header::CONTENT_LENGTH, HeaderValue::from_static("20"));
            Decoder::for_response(&Method::GET, StatusCode::OK, &mut map, decode, 100);
            assert!(map.get(CONTENT_ENCODING).is_some());
            assert!(map.get(http::header::CONTENT_LENGTH).is_some());
        }
    }
}
