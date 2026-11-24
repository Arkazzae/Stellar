//! Where a reader stands in one result of the index: the moment it was read as
//! of, how many messages have been delivered, and what the result is of
//! (channel, kind and a hash of the text), which a later request must repeat.
//!
//! A client keeps the cursor it was given and sends it back, so the form is
//! fixed: 37 bytes, written in URL-safe Base64 without padding.

use aws_lc_rs::digest;

use crate::encoding::{base64_url, from_base64_url};
use crate::values::{Id, Timestamp};

/// Which messages of a channel an index read is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    All,
    Media,
    Links,
}

impl Kind {
    fn byte(self) -> u8 {
        match self {
            Kind::All => 0,
            Kind::Media => 1,
            Kind::Links => 2,
        }
    }

    fn from_byte(byte: u8) -> Option<Kind> {
        match byte {
            0 => Some(Kind::All),
            1 => Some(Kind::Media),
            2 => Some(Kind::Links),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexCursor {
    pub channel: Id,
    pub snapshot: Timestamp,
    pub offset: i32,
    pub kind: Kind,
    pub query_hash: u64,
}

const SIZE: usize = 16 + 8 + 4 + 1 + 8;
/// Ticks between 0001-01-01, where the cursor counts from, and 1970-01-01.
const TICKS_TO_UNIX_EPOCH: i64 = 621_355_968_000_000_000;

/// A hash of the text a result is of, so that a cursor means nothing next to
/// another text. Nothing and an empty text are the same.
pub fn query_hash(query: Option<&str>) -> u64 {
    let hashed = digest::digest(&digest::SHA256, query.unwrap_or("").as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&hashed.as_ref()[..8]);
    u64::from_be_bytes(first)
}

/// The sixteen bytes of an id in the order the cursor keeps them: the first
/// three groups with their bytes reversed.
fn mixed_endian(id: &[u8; 16]) -> [u8; 16] {
    let mut out = *id;
    out[..4].reverse();
    out[4..6].reverse();
    out[6..8].reverse();
    out
}

impl IndexCursor {
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(SIZE);
        bytes.extend_from_slice(&mixed_endian(self.channel.as_bytes()));
        bytes.extend_from_slice(&(self.snapshot.ticks() + TICKS_TO_UNIX_EPOCH).to_be_bytes());
        bytes.extend_from_slice(&self.offset.to_be_bytes());
        bytes.push(self.kind.byte());
        bytes.extend_from_slice(&self.query_hash.to_be_bytes());
        base64_url(&bytes)
    }

    /// None for text that is not a cursor of this form.
    pub fn decode(text: &str) -> Option<IndexCursor> {
        let bytes = from_base64_url(unpadded(text)?)?;
        if bytes.len() != SIZE {
            return None;
        }
        let number = |from: usize, len: usize| -> Option<[u8; 8]> {
            let mut buffer = [0u8; 8];
            buffer[..len].copy_from_slice(bytes.get(from..from + len)?);
            Some(buffer)
        };
        let mut channel = [0u8; 16];
        channel.copy_from_slice(&bytes[..16]);
        let ticks = i64::from_be_bytes(number(16, 8)?);
        let offset = i32::from_be_bytes(number(24, 4)?[..4].try_into().ok()?);
        let kind = Kind::from_byte(bytes[28])?;
        let query_hash = u64::from_be_bytes(number(29, 8)?);
        // The runtime's moments run from the first of January of the year 1 to
        // the last moment of the year 9999.
        if !(0..=3_155_378_975_999_999_999).contains(&ticks) || offset < 0 {
            return None;
        }
        Some(IndexCursor {
            channel: Id::from_bytes(mixed_endian(&channel)),
            snapshot: Timestamp::from_ticks(ticks - TICKS_TO_UNIX_EPOCH)?,
            offset,
            kind,
            query_hash,
        })
    }
}

/// The Base64 without its padding, which the first server's reader let a
/// cursor carry as well: none, or enough `=` to make a whole number of groups
/// of four.
fn unpadded(text: &str) -> Option<&str> {
    let body = text.trim_end_matches('=');
    let padding = text.len() - body.len();
    match padding {
        0 => Some(body),
        1 | 2 if (body.len() + padding) % 4 == 0 => Some(body),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IndexCursor {
        IndexCursor {
            channel: Id::parse("24f08f99-6377-480c-afb8-c61fe5da5b36").unwrap(),
            snapshot: Timestamp::parse("2026-11-24 07:59:05.775861").unwrap(),
            offset: 3,
            kind: Kind::Media,
            query_hash: query_hash(Some("lampa")),
        }
    }

    #[test]
    fn a_cursor_comes_back_as_it_went() {
        let cursor = sample();
        assert_eq!(IndexCursor::decode(&cursor.encode()), Some(cursor));
    }

    #[test]
    fn the_form_is_the_first_servers() {
        // 37 bytes of Base64 without padding; the channel's first three
        // groups are written reversed.
        let written = sample().encode();
        assert_eq!(written.len(), 50);
        let bytes = from_base64_url(&written).unwrap();
        assert_eq!(&bytes[..4], &[0x99, 0x8f, 0xf0, 0x24]);
        assert_eq!(&bytes[24..28], &3i32.to_be_bytes());
        assert_eq!(bytes[28], 1);
    }

    #[test]
    fn padding_is_let_through_and_nothing_else_is() {
        let written = sample().encode();
        assert!(IndexCursor::decode(&format!("{written}==")).is_some());
        assert!(IndexCursor::decode(&format!("{written}=")).is_none());
        assert!(IndexCursor::decode(&format!("{written}===")).is_none());
        assert!(IndexCursor::decode(&written[1..]).is_none());
        assert!(IndexCursor::decode("").is_none());
    }

    #[test]
    fn a_negative_offset_or_an_unknown_kind_is_not_a_cursor() {
        let mut bytes = from_base64_url(&sample().encode()).unwrap();
        bytes[28] = 3;
        assert!(IndexCursor::decode(&base64_url(&bytes)).is_none());
        bytes[28] = 0;
        bytes[24] = 0xFF;
        assert!(IndexCursor::decode(&base64_url(&bytes)).is_none());
    }

    #[test]
    fn nothing_and_an_empty_text_hash_alike() {
        assert_eq!(query_hash(None), query_hash(Some("")));
        assert_ne!(query_hash(Some("a")), query_hash(Some("b")));
    }
}
