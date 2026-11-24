//! What a channel or a section may be called, the way the first server
//! judged it: by UTF-16 units, with its own idea of white space, letters and
//! digits.

use stellarr_unicode::{is_letter_or_digit, to_lower};

/// The longest a name may be, in UTF-16 units.
pub const MAX_NAME: usize = 60;
/// The longest a topic may be, in UTF-16 units, once trimmed.
pub const MAX_TOPIC: usize = 1024;
/// The longest slow mode a text channel may have, in seconds.
pub const MAX_SLOW_MODE: i64 = 6 * 60 * 60;
/// The longest a channel may be hidden after it has gone quiet, in hours.
pub const MAX_HIDE_AFTER: i64 = 365 * 24;
/// The most people a voice channel may be limited to.
pub const MAX_USER_LIMIT: i64 = 99;

/// The text without the white space at either end. What counts as white space
/// is the same set the first server trimmed (the Unicode `White_Space`
/// characters), which is what `str::trim` takes off.
pub fn trim(text: &str) -> &str {
    text.trim()
}

pub(super) use stellarr_unicode::utf16::length;

/// A channel's name as stored: trimmed and in lower case, letter by letter.
pub fn normalize_channel(name: Option<&str>) -> String {
    name.map(|name| to_lower(trim(name))).unwrap_or_default()
}

/// A channel name is 1 to 60 units of letters, digits and hyphens. A
/// character outside the basic plane is two units that are neither, so it
/// never passes.
pub fn is_valid_channel_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&length(name))
        && name
            .chars()
            .all(|c| (u32::from(c) <= 0xFFFF && is_letter_or_digit(c)) || c == '-')
}

/// A section name is any text of 1 to 60 units once trimmed.
pub fn is_valid_section_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&length(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_name_is_lower_cased_trimmed_and_judged_by_letters_and_digits() {
        assert_eq!(normalize_channel(Some("  Foo-Bar\u{3000}")), "foo-bar");
        assert_eq!(normalize_channel(None), "");
        assert!(is_valid_channel_name("żółć-1"));
        assert!(!is_valid_channel_name("a b"));
        assert!(!is_valid_channel_name(""));
        assert!(!is_valid_channel_name("a_b"));
    }

    #[test]
    fn the_length_is_counted_in_utf16_units() {
        assert!(is_valid_channel_name(&"a".repeat(60)));
        assert!(!is_valid_channel_name(&"a".repeat(61)));
        // A letter outside the basic plane is two units, which are no letter.
        assert!(!is_valid_channel_name("\u{1D400}"));
        assert!(is_valid_section_name(&"\u{1D400}".repeat(30)));
        assert!(!is_valid_section_name(&"\u{1D400}".repeat(31)));
    }
}
