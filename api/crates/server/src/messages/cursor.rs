//! The cursor that pages a channel's history.
//!
//! A cursor is a snapshot ceiling and the last message delivered, so that
//! deleting any delivered row, the anchor included, cannot move the next page
//! past an unread message. It is a version byte, the channel (or
//! conversation), the snapshot, and the last message's moment and id, written
//! by [`crate::cursor`].

use crate::cursor::{Reader, Writer};
use crate::values::{Id, Timestamp};

const VERSION: u8 = 1;

/// The page a client asks for: where its snapshot ends and, after the first
/// page, the key of the last message it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryCursor {
    pub scope: Id,
    pub snapshot: Timestamp,
    pub before: Option<(Timestamp, Id)>,
}

impl HistoryCursor {
    /// The text a client sends back to get the page after this one. Only a
    /// cursor that has a last message can be written.
    pub fn encode(&self) -> Option<String> {
        let (before_at, before_id) = self.before?;
        Some(
            Writer::new()
                .byte(VERSION)
                .id(self.scope)
                .moment(self.snapshot)
                .moment(before_at)
                .id(before_id)
                .finish(),
        )
    }

    /// None for a text that is not a cursor.
    pub fn decode(text: &str) -> Option<HistoryCursor> {
        let mut read = Reader::open(text)?;
        if read.byte()? != VERSION {
            return None;
        }
        let scope = read.id()?;
        let snapshot = read.moment()?;
        let last = read.moment()?;
        let last_id = read.id()?;
        read.done()?;
        // The last message is inside the snapshot.
        (last <= snapshot).then_some(HistoryCursor {
            scope,
            snapshot,
            before: Some((last, last_id)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).unwrap()
    }

    fn sample() -> HistoryCursor {
        HistoryCursor {
            scope: Id::parse("11223344-5566-7788-99aa-bbccddeeff00").unwrap(),
            snapshot: at("2026-11-24 10:00:00.5"),
            before: Some((at("2026-11-24 09:59:59.1234567"), Id::random())),
        }
    }

    #[test]
    fn a_cursor_comes_back_as_it_went() {
        let cursor = sample();
        assert_eq!(HistoryCursor::decode(&cursor.encode().unwrap()), Some(cursor));
    }

    #[test]
    fn a_cursor_without_a_last_message_cannot_be_written() {
        let cursor = HistoryCursor {
            before: None,
            ..sample()
        };

        assert_eq!(cursor.encode(), None);
    }

    #[test]
    fn text_that_is_not_a_cursor_is_refused() {
        let valid = sample().encode().unwrap();
        for text in [
            "",
            "A",
            "not a cursor",
            // The standard alphabet, padding, space and a byte too many or too few.
            &valid.replace('-', "+"),
            &format!("{valid}="),
            &format!(" {valid}"),
            &format!("{valid}A"),
            &format!("{valid}AA"),
            &valid[..valid.len() - 1],
            // The form of an older cursor that held a position.
            "RDMiEWZViHeZqrvM3e7_AAjb5V7o5AAAAAAAAw",
        ] {
            assert_eq!(HistoryCursor::decode(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_snapshot_before_the_last_message_is_refused() {
        let late = HistoryCursor {
            snapshot: at("2026-11-24 10:00:00"),
            before: Some((at("2026-11-24 10:00:01"), Id::random())),
            ..sample()
        };

        assert_eq!(HistoryCursor::decode(&late.encode().unwrap()), None);
    }
}
