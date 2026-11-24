//! Text as UTF-16 code units.
//!
//! Positions in a message are counted in UTF-16 code units because that is how
//! the web client, which writes and reads these messages, counts them, and
//! because a name's length limit and the way markup is cut are defined that
//! way. Text from the network is valid UTF-8, so the units always pair up; the
//! functions still treat a lone surrogate as what it is, a character that
//! belongs to no class, so that no input can make them panic.

use crate::{is_letter_or_digit, to_lower_char, to_upper_char};

/// The character shown for what could not be decoded.
pub const REPLACEMENT: char = '\u{FFFD}';

pub fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

pub fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

pub fn is_surrogate(unit: u16) -> bool {
    (0xD800..0xE000).contains(&unit)
}

/// The number of UTF-16 code units in the text.
pub fn length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The text as UTF-16 units.
pub fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// The longest start of `text` that fits in `max` UTF-16 units. A character of
/// two units that the limit falls in the middle of is left out whole, so what
/// comes back is never half a character.
pub fn prefix(text: &str, max: usize) -> &str {
    let mut used = 0;
    for (at, letter) in text.char_indices() {
        used += letter.len_utf16();
        if used > max {
            return &text[..at];
        }
    }
    text
}

/// Units back to text. A lone surrogate reads as the replacement character.
pub fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// The character that starts at `index` and how many units it takes, or `None`
/// at a lone surrogate or past the end.
pub fn scalar_at(text: &[u16], index: usize) -> Option<(char, usize)> {
    let first = *text.get(index)?;
    if !is_surrogate(first) {
        return char::from_u32(u32::from(first)).map(|c| (c, 1));
    }
    if is_high_surrogate(first) {
        let second = *text.get(index + 1)?;
        if is_low_surrogate(second) {
            let code = 0x10000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
            return char::from_u32(code).map(|c| (c, 2));
        }
    }
    None
}

/// The character that ends just before `index`, or `None`.
pub fn scalar_before(text: &[u16], index: usize) -> Option<char> {
    if index == 0 || index > text.len() {
        return None;
    }
    let last = text[index - 1];
    if !is_surrogate(last) {
        return char::from_u32(u32::from(last));
    }
    if is_low_surrogate(last) && index >= 2 && is_high_surrogate(text[index - 2]) {
        return scalar_at(text, index - 2).map(|(c, _)| c);
    }
    None
}

/// Whether the unit alone, outside any pair, is a letter or a digit.
pub fn unit_is_letter_or_digit(unit: u16) -> bool {
    char::from_u32(u32::from(unit)).is_some_and(is_letter_or_digit)
}

/// The characters JavaScript's `\s` matches. They end an address in both of
/// Stellarr's readers, so they end it here.
pub fn is_js_whitespace(unit: u16) -> bool {
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

/// ASCII punctuation: what a backslash can escape.
pub fn is_ascii_punctuation(unit: u16) -> bool {
    matches!(unit, 0x21..=0x2F | 0x3A..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E)
}

fn map_units(units: &[u16], map: fn(char) -> char) -> Vec<u16> {
    let mut out = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        match scalar_at(units, index) {
            Some((c, width)) => {
                let mut buffer = [0u16; 2];
                out.extend_from_slice(map(c).encode_utf16(&mut buffer));
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

/// Simple upper case, character by character: one character stays one
/// character. A lone surrogate stays as it is.
pub fn to_upper_units(units: &[u16]) -> Vec<u16> {
    map_units(units, to_upper_char)
}

/// Simple lower case, character by character, as [`to_upper_units`].
pub fn to_lower_units(units: &[u16]) -> Vec<u16> {
    map_units(units, to_lower_char)
}
