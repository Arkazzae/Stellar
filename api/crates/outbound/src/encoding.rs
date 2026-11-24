//! Content codings of an answer. This is the one place where decoding a
//! compressed answer is added.
//!
//! Until then every request says `Accept-Encoding: identity`, and an answer
//! that is compressed anyway is not handed over as if it were the plain
//! bytes: reading its body fails, while its status and headers stay
//! readable. To add a coding: list it in [`ACCEPT_ENCODING`], return a
//! [`Decoder`] for it from [`Decoder::for_headers`], and run every chunk
//! through [`Decoder::feed`]. The ceiling on the size of the answer is
//! counted on the bytes from the wire, before this step, so that a small
//! body that inflates enormously cannot grow past what the caller can hold:
//! the decoder must also be given a ceiling for what it produces.

use http::HeaderMap;
use http::header::CONTENT_ENCODING;

use crate::error::{Error, Result};

/// What every request offers.
pub(crate) const ACCEPT_ENCODING: &str = "identity";

/// Turns the bytes of the body into what the caller reads.
pub(crate) enum Decoder {
    /// The body is the bytes the server meant.
    Identity,
    /// A coding this client cannot undo.
    Unsupported,
}

impl Decoder {
    pub(crate) fn for_headers(headers: &HeaderMap) -> Result<Decoder> {
        let mut decoder = Decoder::Identity;
        for value in headers.get_all(CONTENT_ENCODING) {
            let value = value.to_str().unwrap_or("?");
            for coding in value.split(',') {
                let coding = coding.trim();
                if !coding.is_empty() && !coding.eq_ignore_ascii_case("identity") {
                    decoder = Decoder::Unsupported;
                }
            }
        }
        Ok(decoder)
    }

    pub(crate) fn feed(&mut self, chunk: bytes::Bytes) -> Result<bytes::Bytes> {
        match self {
            Decoder::Identity => Ok(chunk),
            Decoder::Unsupported => Err(Error::Body(
                "the answer is compressed and this client does not decode it".into(),
            )),
        }
    }
}
