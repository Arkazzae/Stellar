//! The framing of the hub protocol: a stream of text records, each ended by
//! the byte `0x1e`. Records do not line up with WebSocket messages: one
//! message may hold several records, or part of one.

use bytes::{Buf, Bytes, BytesMut};

/// The byte that ends a record.
pub const SEPARATOR: u8 = 0x1e;

/// A record would be longer than the limit allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong;

/// Collects incoming bytes and hands out whole records.
///
/// The limit counts a record with its separator, so a record of exactly
/// `max` bytes passes and one byte more does not. Bytes of a record that has
/// not ended yet count too, so a peer cannot make the buffer grow by never
/// sending the separator. (A record that has reached the limit without its
/// separator is waited for one more byte: the separator would make it too
/// long anyway, and the answer is the same.)
#[derive(Debug)]
pub struct RecordBuffer {
    buf: BytesMut,
    /// How much of `buf` is known to hold no separator.
    scanned: usize,
    max: usize,
}

impl RecordBuffer {
    pub fn new(max: usize) -> Self {
        Self {
            buf: BytesMut::new(),
            scanned: 0,
            max,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole record, without its separator.
    pub fn next_record(&mut self) -> Result<Option<Bytes>, TooLong> {
        match self.buf[self.scanned..]
            .iter()
            .position(|b| *b == SEPARATOR)
        {
            Some(offset) => {
                let end = self.scanned + offset;
                if end + 1 > self.max {
                    return Err(TooLong);
                }
                let record = self.buf.split_to(end).freeze();
                self.buf.advance(1);
                self.scanned = 0;
                Ok(Some(record))
            }
            None => {
                self.scanned = self.buf.len();
                if self.buf.len() > self.max {
                    return Err(TooLong);
                }
                Ok(None)
            }
        }
    }

    /// Bytes held that do not make a whole record yet.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

/// A record with its separator, as it goes on the wire.
pub fn frame_record(body: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(body.len() + 1);
    out.extend_from_slice(body);
    out.extend_from_slice(&[SEPARATOR]);
    out.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(buffer: &mut RecordBuffer) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(record) = buffer.next_record().unwrap() {
            out.push(record.to_vec());
        }
        out
    }

    #[test]
    fn several_records_in_one_push() {
        let mut buffer = RecordBuffer::new(100);
        buffer.push(b"one\x1etwo\x1e\x1ethree\x1e");
        assert_eq!(
            drain(&mut buffer),
            [
                b"one".to_vec(),
                b"two".to_vec(),
                b"".to_vec(),
                b"three".to_vec()
            ]
        );
        assert_eq!(buffer.pending(), 0);
    }

    #[test]
    fn a_record_in_pieces_waits_for_its_separator() {
        let mut buffer = RecordBuffer::new(100);
        for byte in b"{\"type\":6}" {
            buffer.push(&[*byte]);
            assert_eq!(buffer.next_record(), Ok(None));
        }
        buffer.push(&[SEPARATOR, b'x']);
        assert_eq!(drain(&mut buffer), [b"{\"type\":6}".to_vec()]);
        assert_eq!(buffer.pending(), 1, "the start of the next record is kept");
    }

    #[test]
    fn the_limit_counts_the_separator() {
        // A record of 9 bytes and its separator is 10: at the limit.
        let mut buffer = RecordBuffer::new(10);
        buffer.push(b"123456789\x1e");
        assert_eq!(buffer.next_record().unwrap().unwrap().len(), 9);
        let mut buffer = RecordBuffer::new(10);
        buffer.push(b"1234567890\x1e");
        assert_eq!(buffer.next_record(), Err(TooLong));
    }

    #[test]
    fn a_record_that_never_ends_is_refused_before_it_is_complete() {
        let mut buffer = RecordBuffer::new(10);
        buffer.push(b"123456789");
        assert_eq!(
            buffer.next_record(),
            Ok(None),
            "9 bytes may still end in a separator"
        );
        buffer.push(b"0");
        assert_eq!(
            buffer.next_record(),
            Ok(None),
            "ten bytes are at the limit, not past it"
        );
        buffer.push(b"1");
        assert_eq!(buffer.next_record(), Err(TooLong));
    }

    #[test]
    fn a_long_record_after_short_ones_is_still_caught() {
        let mut buffer = RecordBuffer::new(10);
        buffer.push(b"ab\x1e");
        buffer.push(&[b'x'; 40]);
        buffer.push(b"\x1e");
        assert_eq!(buffer.next_record().unwrap().unwrap().as_ref(), b"ab");
        assert_eq!(buffer.next_record(), Err(TooLong));
    }

    #[test]
    fn a_stream_cut_off_in_the_middle_yields_nothing() {
        let mut buffer = RecordBuffer::new(100);
        buffer.push(b"{\"type\":1,\"inv");
        assert_eq!(buffer.next_record(), Ok(None));
        assert_eq!(buffer.pending(), 14);
    }

    #[test]
    fn framing_adds_one_separator() {
        assert_eq!(frame_record(b"{}").as_ref(), b"{}\x1e");
    }

    #[test]
    fn scanning_does_not_start_over_for_every_byte() {
        // A hundred thousand single-byte pushes would be quadratic if each
        // call looked at the whole buffer again; it must stay fast.
        let mut buffer = RecordBuffer::new(1 << 20);
        for _ in 0..100_000 {
            buffer.push(b"x");
            assert_eq!(buffer.next_record(), Ok(None));
        }
        buffer.push(&[SEPARATOR]);
        assert_eq!(buffer.next_record().unwrap().unwrap().len(), 100_000);
    }
}
