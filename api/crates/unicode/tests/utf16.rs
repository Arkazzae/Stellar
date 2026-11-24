use stellarr_unicode::utf16::*;

#[test]
fn pairs_and_lone_surrogates() {
    let sample = units("a😀b");
    assert_eq!(sample.len(), 4);
    assert_eq!(scalar_at(&sample, 0), Some(('a', 1)));
    assert_eq!(scalar_at(&sample, 1), Some(('😀', 2)));
    assert_eq!(scalar_at(&sample, 2), None);
    assert_eq!(scalar_at(&sample, 3), Some(('b', 1)));
    assert_eq!(scalar_at(&sample, 4), None);
    assert_eq!(scalar_before(&sample, 3), Some('😀'));
    assert_eq!(scalar_before(&sample, 2), None);
    assert_eq!(scalar_before(&sample, 0), None);
    assert_eq!(scalar_before(&sample, 99), None);
    assert_eq!(scalar_at(&[0xD800], 0), None);
    assert_eq!(scalar_at(&[0xDC00, 0xD800], 0), None);
    assert_eq!(scalar_at(&[0xD800, 0x41], 0), None);
    assert_eq!(text(&[0xD800, 0x41]), "\u{FFFD}A");
    assert!(
        unit_is_letter_or_digit(0x41)
            && !unit_is_letter_or_digit(0xD83D)
            && !unit_is_letter_or_digit(0x2E)
    );
}

#[test]
fn upper_casing_units_keeps_pairs_and_lone_surrogates() {
    assert_eq!(text(&to_upper_units(&units("ab𐐨ıſ"))), "AB𐐀ıS");
    assert_eq!(to_upper_units(&[0x61, 0xD800, 0x62]), [0x41, 0xD800, 0x42]);
    assert_eq!(to_upper_units(&[]), Vec::<u16>::new());
    assert_eq!(text(&to_lower_units(&units("ABİ𐐀"))), "abİ𐐨");
    assert_eq!(to_lower_units(&[0x41, 0xDC00, 0x42]), [0x61, 0xDC00, 0x62]);
}

#[test]
fn the_character_sets_of_the_readers() {
    for unit in [
        0x09u16, 0x0A, 0x0D, 0x20, 0xA0, 0x1680, 0x2003, 0x2028, 0x3000, 0xFEFF,
    ] {
        assert!(is_js_whitespace(unit), "{unit:X}");
    }
    for unit in [0x08u16, 0x0E, 0x85, 0x200B, 0x180E, 0x41] {
        assert!(!is_js_whitespace(unit), "{unit:X}");
    }
    let punctuation: String = (0u16..128)
        .filter(|&u| is_ascii_punctuation(u))
        .map(|u| char::from(u as u8))
        .collect();
    assert_eq!(punctuation, "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~");
    assert!(
        is_high_surrogate(0xD800)
            && is_low_surrogate(0xDFFF)
            && is_surrogate(0xDC00)
            && !is_surrogate(0xE000)
    );
    assert_eq!(REPLACEMENT, '\u{FFFD}');
}
