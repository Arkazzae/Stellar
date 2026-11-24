//! How the first server read text, where it differs from Rust's own habits:
//! what counts as white space, and lengths in UTF-16 units.
//!
//! A limit written as "at most 64 characters" was a limit on UTF-16 units, so
//! a flag counts four and a family seven. The limits here count the same way.

/// Whether the character is white space as the first server's runtime knew
/// it: the controls from tab to carriage return, the space separators, the
/// line and paragraph separators and U+0085. The list is spelled out because
/// the standard library's own list follows a newer Unicode.
pub fn is_white_space(c: char) -> bool {
    matches!(
        u32::from(c),
        0x09..=0x0D
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// The text without the white space at either end.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_white_space)
}

/// How long the text is in UTF-16 units.
pub fn units(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The first `count` UTF-16 units of the text. A cut inside a pair leaves a
/// lone half, which cannot be written as text: it comes out as the replacement
/// character, as it did from the first server.
pub fn first_units(text: &str, count: usize) -> String {
    let mut kept = String::new();
    let mut seen = 0;
    for c in text.chars() {
        let next = seen + c.len_utf16();
        if next > count {
            if seen < count {
                kept.push('\u{FFFD}');
            }
            break;
        }
        kept.push(c);
        seen = next;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_space_is_the_first_servers_list() {
        let listed: Vec<u32> = (0..0x1_0000u32)
            .filter(|&u| char::from_u32(u).is_some_and(is_white_space))
            .collect();
        let mut expected: Vec<u32> = (9..=13).collect();
        expected.extend([32, 133, 160, 5760]);
        expected.extend(8192..=8202);
        expected.extend([8232, 8233, 8239, 8287, 12288]);
        assert_eq!(listed, expected);
    }

    #[test]
    fn separators_of_the_file_and_group_kind_are_not_white_space() {
        for unit in 0x1C..=0x1F {
            assert!(!is_white_space(char::from_u32(unit).unwrap()));
        }
        assert!(!is_white_space('\u{200B}'));
        assert!(!is_white_space('\u{FEFF}'));
    }

    #[test]
    fn lengths_count_utf16_units() {
        assert_eq!(units("a\u{1F44D}"), 3);
        assert_eq!(units("zażółć"), 6);
        assert_eq!(first_units("a\u{1F44D}b", 2), "a\u{FFFD}");
        assert_eq!(first_units("a\u{1F44D}b", 3), "a\u{1F44D}");
        assert_eq!(first_units("abc", 9), "abc");
    }
}
