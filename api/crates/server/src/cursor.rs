//! How every cursor is written: a fixed run of numbers and ids as bytes, in
//! standard byte order, as URL-safe Base64 without padding.
//!
//! A client keeps a cursor as an opaque text and sends it back, so the form is
//! the server's own. What does not read back exactly (a text that is not
//! Base64, a run of the wrong length, a moment out of range) is no cursor.

use crate::encoding::{base64_url, from_base64_url};
use crate::values::{Id, Timestamp};

/// The bytes of a cursor being written.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    pub fn byte(mut self, value: u8) -> Writer {
        self.0.push(value);
        self
    }

    pub fn id(mut self, id: Id) -> Writer {
        self.0.extend_from_slice(id.as_bytes());
        self
    }

    /// A moment as its ticks of 100 ns since 1970, 64 bits, big-endian.
    pub fn moment(mut self, at: Timestamp) -> Writer {
        self.0.extend_from_slice(&at.ticks().to_be_bytes());
        self
    }

    pub fn int(mut self, value: i32) -> Writer {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    pub fn long(mut self, value: u64) -> Writer {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    pub fn finish(self) -> String {
        base64_url(&self.0)
    }
}

/// The bytes of a cursor being read.
pub struct Reader {
    bytes: Vec<u8>,
    at: usize,
}

impl Reader {
    /// None for a text that is not URL-safe Base64 without padding.
    pub fn open(text: &str) -> Option<Reader> {
        Some(Reader {
            bytes: from_base64_url(text)?,
            at: 0,
        })
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let found = self.bytes.get(self.at..self.at + N)?.try_into().ok()?;
        self.at += N;
        Some(found)
    }

    pub fn byte(&mut self) -> Option<u8> {
        Some(self.take::<1>()?[0])
    }

    pub fn id(&mut self) -> Option<Id> {
        self.take().map(Id::from_bytes)
    }

    pub fn moment(&mut self) -> Option<Timestamp> {
        Timestamp::from_ticks(i64::from_be_bytes(self.take()?))
    }

    pub fn int(&mut self) -> Option<i32> {
        self.take().map(i32::from_be_bytes)
    }

    pub fn long(&mut self) -> Option<u64> {
        self.take().map(u64::from_be_bytes)
    }

    /// Whether every byte was read: a cursor with something left over is none.
    pub fn done(self) -> Option<()> {
        (self.at == self.bytes.len()).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_is_read_back_in_order() {
        let id = Id::parse("11223344-5566-7788-99aa-bbccddeeff00").unwrap();
        let at = Timestamp::parse("2026-11-24 10:00:00.5").unwrap();
        let text = Writer::new()
            .byte(1)
            .id(id)
            .moment(at)
            .int(-3)
            .long(7)
            .finish();

        let mut read = Reader::open(&text).unwrap();
        assert_eq!(read.byte(), Some(1));
        assert_eq!(read.id(), Some(id));
        assert_eq!(read.moment(), Some(at));
        assert_eq!(read.int(), Some(-3));
        assert_eq!(read.long(), Some(7));
        assert!(read.done().is_some());
    }

    #[test]
    fn an_id_is_written_in_standard_byte_order() {
        let id = Id::parse("11223344-5566-7788-99aa-bbccddeeff00").unwrap();
        let bytes = from_base64_url(&Writer::new().id(id).finish()).unwrap();
        assert_eq!(&bytes[..4], &[0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn text_that_does_not_read_back_exactly_is_no_cursor() {
        let text = Writer::new().byte(1).id(Id::random()).finish();
        assert!(Reader::open(&format!("{text}=")).is_none(), "padding");
        assert!(Reader::open(&format!(" {text}")).is_none(), "space");
        let mut short = Reader::open(&text).unwrap();
        assert!(short.byte().is_some() && short.id().is_some());
        assert!(short.moment().is_none(), "past the end");
        let mut left = Reader::open(&text).unwrap();
        left.byte();
        assert!(left.done().is_none(), "something left over");
        let beyond = base64_url(&i64::MAX.to_be_bytes());
        assert!(
            Reader::open(&beyond).unwrap().moment().is_none(),
            "out of range"
        );
    }
}
