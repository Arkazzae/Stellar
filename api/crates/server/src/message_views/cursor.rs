//! Where a reader stands in one result of the index: the moment it was read as
//! of, how many messages have been delivered, and what the result is of
//! (channel, kind and a hash of the text), which a later request must repeat.
//!
//! A client keeps the cursor it was given and sends it back; it is written by
//! [`crate::cursor`].

use aws_lc_rs::digest;

use crate::cursor::{Reader, Writer};
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

/// A hash of the text a result is of, so that a cursor means nothing next to
/// another text. Nothing and an empty text are the same.
pub fn query_hash(query: Option<&str>) -> u64 {
    let hashed = digest::digest(&digest::SHA256, query.unwrap_or("").as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&hashed.as_ref()[..8]);
    u64::from_be_bytes(first)
}

impl IndexCursor {
    pub fn encode(&self) -> String {
        Writer::new()
            .id(self.channel)
            .moment(self.snapshot)
            .int(self.offset)
            .byte(self.kind.byte())
            .long(self.query_hash)
            .finish()
    }

    /// None for text that is not a cursor of this form.
    pub fn decode(text: &str) -> Option<IndexCursor> {
        let mut read = Reader::open(text)?;
        let channel = read.id()?;
        let snapshot = read.moment()?;
        let offset = read.int()?;
        let kind = Kind::from_byte(read.byte()?)?;
        let query_hash = read.long()?;
        read.done()?;
        (offset >= 0).then_some(IndexCursor {
            channel,
            snapshot,
            offset,
            kind,
            query_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{base64_url, from_base64_url};

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
    fn the_channel_is_written_in_standard_byte_order() {
        let bytes = from_base64_url(&sample().encode()).unwrap();
        assert_eq!(&bytes[..4], &[0x24, 0xf0, 0x8f, 0x99]);
        assert_eq!(bytes.len(), 37);
    }

    #[test]
    fn padding_space_and_other_lengths_are_no_cursor() {
        let written = sample().encode();
        assert!(IndexCursor::decode(&format!("{written}=")).is_none());
        assert!(IndexCursor::decode(&format!("{written}==")).is_none());
        assert!(IndexCursor::decode(&format!(" {written}")).is_none());
        assert!(IndexCursor::decode(&format!("{}\n{}", &written[..20], &written[20..])).is_none());
        assert!(IndexCursor::decode(&written[1..]).is_none());
        assert!(IndexCursor::decode(&format!("{written}AA")).is_none());
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
