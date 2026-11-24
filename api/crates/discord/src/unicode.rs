//! Character classes, and the UTF-16 view of text that the message
//! rewriting works in.
//!
//! Positions in a message are counted in UTF-16 code units because that is
//! how the web client, which writes and reads these messages, counts them, and
//! because a name's length limit and the way markup is cut are defined that
//! way. Text from Discord is valid UTF-8, so the units always pair up; the
//! functions still treat a lone surrogate as what it is, a character that
//! belongs to no class, so that no input can make them panic.

use crate::unicode_tables::{DIGIT, LETTER, NUMBER, PUNCTUATION, SYMBOL};

fn in_table(table: &[(u32, u32)], code: u32) -> bool {
    table
        .binary_search_by(|&(first, last)| {
            if code < first {
                std::cmp::Ordering::Greater
            } else if code > last {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Letters (categories L*) and decimal digits (Nd).
pub(crate) fn is_letter_or_digit(code: u32) -> bool {
    in_table(LETTER, code) || in_table(DIGIT, code)
}

/// Letters and numbers of every kind (L* and N*).
pub(crate) fn is_letter_or_number(code: u32) -> bool {
    in_table(LETTER, code) || in_table(NUMBER, code)
}

/// Punctuation (categories P*).
pub(crate) fn is_punctuation(code: u32) -> bool {
    in_table(PUNCTUATION, code)
}

/// Symbols (categories S*).
pub(crate) fn is_symbol(code: u32) -> bool {
    in_table(SYMBOL, code)
}

/// The character that stands for one Unicode scalar value that could not be
/// decoded.
pub(crate) const REPLACEMENT: u32 = 0xFFFD;

pub(crate) fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

pub(crate) fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

pub(crate) fn is_surrogate(unit: u16) -> bool {
    (0xD800..0xE000).contains(&unit)
}

/// The scalar value that starts at `index` and how many units it takes, or
/// `None` at a lone surrogate or past the end.
pub(crate) fn scalar_at(text: &[u16], index: usize) -> Option<(u32, usize)> {
    let first = *text.get(index)?;
    if !is_surrogate(first) {
        return Some((u32::from(first), 1));
    }
    if is_high_surrogate(first) {
        let second = *text.get(index + 1)?;
        if is_low_surrogate(second) {
            let code = 0x10000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
            return Some((code, 2));
        }
    }
    None
}

/// The scalar value that ends just before `index`, or `None`.
pub(crate) fn scalar_before(text: &[u16], index: usize) -> Option<u32> {
    if index == 0 || index > text.len() {
        return None;
    }
    let last = text[index - 1];
    if !is_surrogate(last) {
        return Some(u32::from(last));
    }
    if is_low_surrogate(last) && index >= 2 && is_high_surrogate(text[index - 2]) {
        return scalar_at(text, index - 2).map(|(code, _)| code);
    }
    None
}

/// Whether the unit alone, outside any pair, is a letter or a digit.
pub(crate) fn unit_is_letter_or_digit(unit: u16) -> bool {
    !is_surrogate(unit) && is_letter_or_digit(u32::from(unit))
}

/// The characters JavaScript's `\s` matches. They end an address in both of
/// Stellarr's readers, so they end it here.
pub(crate) fn is_js_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

pub(crate) fn is_ascii_punctuation(unit: u16) -> bool {
    matches!(unit, 0x21..=0x2F | 0x3A..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E)
}

/// The text as UTF-16 units.
pub(crate) fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Units back to text. A lone surrogate, which cannot occur in what this
/// crate builds from valid text, would read as the replacement character.
pub(crate) fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// Upper case by simple case mapping, character by character: a character
/// whose upper case is more than one character stays as it is. Names are
/// compared in this form.
pub(crate) fn simple_upper(units: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        match scalar_at(units, index) {
            Some((code, width)) => {
                let mapped = char::from_u32(code).and_then(|c| {
                    let mut upper = c.to_uppercase();
                    match (upper.next(), upper.next()) {
                        (Some(one), None) => Some(one),
                        _ => None,
                    }
                });
                match mapped {
                    Some(c) => {
                        let mut buffer = [0u16; 2];
                        out.extend_from_slice(c.encode_utf16(&mut buffer));
                    }
                    None => out.extend_from_slice(&units[index..index + width]),
                }
                index += width;
            }
            None => {
                out.push(units[index]);
                index += 1;
            }
        }
    }
    out
}
