//! The cursor that pages a channel's history.
//!
//! A cursor is a snapshot ceiling and the last message delivered, so that
//! deleting any delivered row, the anchor included, cannot move the next page
//! past an unread message. Clients keep cursors across restarts and across
//! the switch between servers, so the bytes are the first server's: a version
//! byte, the channel, the snapshot and the last message's moment as 64-bit
//! counts of 100 ns from the year 1, big-endian, and the last message's id,
//! the ids in the byte order of that server's UUID type (the first three
//! groups little-endian). Written as base 64 with the URL alphabet and no
//! padding.
//!
//! The first server also read a shorter, older form that holds a position
//! instead of a message; it is read as a restart of its snapshot.

use crate::values::{Id, Timestamp};

const VERSION: u8 = 1;
const LEGACY_SIZE: usize = 28;
const SIZE: usize = 49;

/// Ticks between 0001-01-01 and 1970-01-01.
const EPOCH_TICKS: i64 = 621_355_968_000_000_000;
/// The last moment the year-1 count can hold, 9999-12-31T23:59:59.9999999.
const LAST_TICKS: i64 = 3_155_378_975_999_999_999;

/// The page a client asks for: where its snapshot ends and, after the first
/// page, the key of the last message it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryCursor {
    pub scope: Id,
    pub snapshot: Timestamp,
    pub before: Option<(Timestamp, Id)>,
}

/// What reading a cursor came to. The first server's decoder let some
/// malformed texts through its validity check and then failed on them, which
/// the client saw as a server error; that is kept, so that the same text means
/// the same thing on both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Cursor(HistoryCursor),
    Invalid,
    Fault,
}

/// A UUID's bytes in the order of the first server's own type: the first three
/// groups swapped to little-endian.
fn to_net(id: &Id) -> [u8; 16] {
    let b = id.as_bytes();
    [
        b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15],
    ]
}

fn from_net(b: &[u8]) -> Id {
    Id::from_bytes([
        b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15],
    ])
}

fn year_one_ticks(at: Timestamp) -> i64 {
    at.ticks() + EPOCH_TICKS
}

fn from_year_one_ticks(ticks: i64) -> Option<Timestamp> {
    if !(0..=LAST_TICKS).contains(&ticks) {
        return None;
    }
    Timestamp::from_ticks(ticks - EPOCH_TICKS)
}

impl HistoryCursor {
    /// The text a client sends back to get the page after this one. Only a
    /// cursor that has a last message can be written.
    pub fn encode(&self) -> Option<String> {
        let (before_at, before_id) = self.before?;
        let mut bytes = [0u8; SIZE];
        bytes[0] = VERSION;
        bytes[1..17].copy_from_slice(&to_net(&self.scope));
        bytes[17..25].copy_from_slice(&year_one_ticks(self.snapshot).to_be_bytes());
        bytes[25..33].copy_from_slice(&year_one_ticks(before_at).to_be_bytes());
        bytes[33..].copy_from_slice(&to_net(&before_id));
        Some(base64url(&bytes))
    }

    pub fn decode(text: &str) -> Reading {
        let (bytes, undecodable) = match unbase64url(text) {
            Unbased::Bytes(bytes, undecodable) => (bytes, undecodable),
            Unbased::Invalid => return Reading::Invalid,
        };
        if undecodable && matches!(bytes.len(), LEGACY_SIZE | SIZE) {
            return Reading::Fault;
        }
        match bytes.len() {
            LEGACY_SIZE => {
                let snapshot = i64::from_be_bytes(bytes[16..24].try_into().unwrap_or_default());
                let offset = i32::from_be_bytes(bytes[24..28].try_into().unwrap_or_default());
                match from_year_one_ticks(snapshot) {
                    Some(snapshot) if offset >= 0 => Reading::Cursor(HistoryCursor {
                        scope: from_net(&bytes[..16]),
                        snapshot,
                        before: None,
                    }),
                    _ => Reading::Invalid,
                }
            }
            SIZE => {
                let ticks = i64::from_be_bytes(bytes[17..25].try_into().unwrap_or_default());
                let before = i64::from_be_bytes(bytes[25..33].try_into().unwrap_or_default());
                let (Some(snapshot), Some(last)) =
                    (from_year_one_ticks(ticks), from_year_one_ticks(before))
                else {
                    return Reading::Invalid;
                };
                if bytes[0] != VERSION || before > ticks {
                    return Reading::Invalid;
                }
                Reading::Cursor(HistoryCursor {
                    scope: from_net(&bytes[1..17]),
                    snapshot,
                    before: Some((last, from_net(&bytes[33..]))),
                })
            }
            _ => Reading::Invalid,
        }
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut word = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            word |= u32::from(*byte) << (16 - 8 * index);
        }
        let digits = chunk.len() + 1;
        for index in 0..digits {
            out.push(char::from(
                ALPHABET[((word >> (18 - 6 * index)) & 63) as usize],
            ));
        }
    }
    out
}

enum Unbased {
    /// The bytes, and whether the first server's decoder would have thrown on
    /// the text after its check let it through.
    Bytes(Vec<u8>, bool),
    Invalid,
}

fn digit(c: char) -> Option<u32> {
    ALPHABET
        .iter()
        .position(|a| char::from(*a) == c)
        .map(|p| p as u32)
}

/// Reads base 64 with the URL alphabet the way the first server's library
/// did: white space (space, tab, carriage return, line feed) anywhere is
/// skipped, up to two `=` may close the text and then the count of all the
/// characters must be a multiple of four, one that leaves a single digit over
/// is no text at all, and the bits past the last whole byte must be zero.
fn unbase64url(text: &str) -> Unbased {
    let mut digits: Vec<u32> = Vec::with_capacity(text.len());
    let mut padding = 0usize;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\r' | '\n') {
            continue;
        }
        if c == '=' {
            padding += 1;
            continue;
        }
        if padding > 0 {
            return Unbased::Invalid;
        }
        match digit(c) {
            Some(value) => digits.push(value),
            None => return Unbased::Invalid,
        }
    }
    if padding > 2 || digits.len() % 4 == 1 {
        return Unbased::Invalid;
    }
    // The library's check let one pad after two digits through and its
    // decoder then threw.
    let undecodable = match (digits.len() % 4, padding) {
        (_, 0) | (2, 2) | (3, 1) => false,
        (2, 1) => true,
        _ => return Unbased::Invalid,
    };
    let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
    for group in digits.chunks(4) {
        let mut word = 0u32;
        for (index, value) in group.iter().enumerate() {
            word |= value << (18 - 6 * index);
        }
        let produced = match group.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => 0,
        };
        for index in 0..produced {
            bytes.push((word >> (16 - 8 * index)) as u8);
        }
        if group.len() < 4 && word & (0x00FF_FFFF >> (8 * produced)) != 0 {
            return Unbased::Invalid;
        }
    }
    Unbased::Bytes(bytes, undecodable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        Timestamp::parse(text).unwrap()
    }

    #[test]
    fn a_cursor_is_the_same_bytes_the_first_server_wrote() {
        // Written by the first server's own code for these values.
        let cursor = HistoryCursor {
            scope: Id::parse("11223344-5566-7788-99aa-bbccddeeff00").unwrap(),
            snapshot: Timestamp::from_ticks(638_355_968_000_000_000 - EPOCH_TICKS).unwrap(),
            before: Some((
                Timestamp::from_ticks(638_355_967_999_999_995 - EPOCH_TICKS).unwrap(),
                Id::parse("aabbccdd-eeff-0011-2233-445566778899").unwrap(),
            )),
        };

        let text = cursor.encode().unwrap();

        assert_eq!(
            text,
            "AUQzIhFmVYh3maq7zN3u_wAI2-Ve6OQAAAjb5V7o4__73cy7qv_uEQAiM0RVZneImQ"
        );
        assert_eq!(HistoryCursor::decode(&text), Reading::Cursor(cursor));
    }

    #[test]
    fn a_cursor_without_a_last_message_cannot_be_written() {
        let cursor = HistoryCursor {
            scope: Id::random(),
            snapshot: at("2026-11-24 10:00:00"),
            before: None,
        };

        assert_eq!(cursor.encode(), None);
    }

    #[test]
    fn the_older_form_restarts_its_snapshot() {
        let reading = HistoryCursor::decode("RDMiEWZViHeZqrvM3e7_AAjb5V7o5AAAAAAAAw");

        let Reading::Cursor(cursor) = reading else {
            panic!("{reading:?}");
        };
        assert_eq!(
            cursor.scope,
            Id::parse("11223344-5566-7788-99aa-bbccddeeff00").unwrap()
        );
        assert_eq!(cursor.before, None);
        // A negative position is no cursor.
        assert_eq!(
            HistoryCursor::decode("RDMiEWZViHeZqrvM3e7_AAjb5V7o5AAA_____Q"),
            Reading::Invalid
        );
    }

    #[test]
    fn text_that_is_not_a_cursor_is_refused() {
        let valid = "AUQzIhFmVYh3maq7zN3u_wAI2-Ve6OQAAAjb5V7o4__73cy7qv_uEQAiM0RVZneImQ";
        for text in [
            "",
            "A",
            // Bits past the last byte that are not zero.
            "AUQzIhFmVYh3maq7zN3u_wAI2-Ve6OQAAAjb5V7o4__73cy7qv_uEQAiM0RVZneImB",
            // The standard alphabet.
            "AUQzIhFmVYh3maq7zN3u/wAI2+Ve6OQAAAjb5V7o4//73cy7qv/uEQAiM0RVZneImQ",
            // A byte too many, or one too few.
            &format!("{valid}A"),
            &valid[..valid.len() - 1],
            &format!("{valid}AA"),
        ] {
            assert_eq!(HistoryCursor::decode(text), Reading::Invalid, "{text:?}");
        }
    }

    #[test]
    fn white_space_and_whole_padding_are_let_through_and_a_single_pad_is_a_fault() {
        let valid = "AUQzIhFmVYh3maq7zN3u_wAI2-Ve6OQAAAjb5V7o4__73cy7qv_uEQAiM0RVZneImQ";
        let expected = HistoryCursor::decode(valid);
        assert!(matches!(expected, Reading::Cursor(_)));

        assert_eq!(HistoryCursor::decode(&format!(" {valid}")), expected);
        assert_eq!(
            HistoryCursor::decode(&format!("{}\n {}", &valid[..20], &valid[20..])),
            expected
        );
        assert_eq!(HistoryCursor::decode(&format!("{valid}==")), expected);
        assert_eq!(HistoryCursor::decode(&format!("{valid}=")), Reading::Fault);
    }

    #[test]
    fn a_snapshot_before_the_last_message_or_out_of_range_is_refused() {
        let scope = Id::random();
        let late = HistoryCursor {
            scope,
            snapshot: at("2026-11-24 10:00:00"),
            before: Some((at("2026-11-24 10:00:01"), Id::random())),
        };

        assert_eq!(
            HistoryCursor::decode(&late.encode().unwrap()),
            Reading::Invalid
        );
    }
}
