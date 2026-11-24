//! The tables against sources that do not share their origin: the Unicode
//! 13.0 database that ships with Python (categories) and with Perl (simple
//! upper case), copied into `tests/fixtures`, and the standard library's own
//! mapping, which is newer.
//!
//! The tables are of Unicode 16.0, so what 13.0 assigned must agree and the
//! rest, what 14.0 to 16.0 added, is counted and pinned. A table made from
//! another version fails these counts on purpose.

use std::collections::{BTreeMap, BTreeSet};

use stellarr_unicode::*;

fn ranges(text: &str) -> BTreeMap<String, Vec<(u32, u32)>> {
    let mut classes: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
    let mut current = String::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        if let Some(name) = line.strip_prefix("CLASS ") {
            current = name.to_owned();
        } else {
            let (a, b) = line.split_once(' ').unwrap();
            classes.entry(current.clone()).or_default().push((
                u32::from_str_radix(a, 16).unwrap(),
                u32::from_str_radix(b, 16).unwrap(),
            ));
        }
    }
    classes
}

fn members(ranges: &[(u32, u32)]) -> BTreeSet<u32> {
    ranges.iter().flat_map(|&(a, b)| a..=b).collect()
}

fn scalars() -> impl Iterator<Item = char> {
    (0..=0x10FFFFu32).filter_map(char::from_u32)
}

#[test]
fn categories_agree_with_unicode_13_for_everything_it_assigned() {
    let python = ranges(include_str!("fixtures/python-categories.txt"));
    let assigned = members(&python["ASSIGNED"]);
    // (class, test, code points added since 13.0 that the table has)
    type Class = (&'static str, fn(char) -> bool, usize);
    let classes: [Class; 5] = [
        ("LETTER", is_letter, 9787),
        ("DIGIT", is_decimal_digit, 110),
        ("NUMBER", is_number, 130),
        ("PUNCTUATION", is_punctuation, 57),
        ("SYMBOL", is_symbol, 950),
    ];
    for (name, test, added) in classes {
        let expected = members(&python[name]);
        let ours: BTreeSet<u32> = scalars().filter(|&c| test(c)).map(u32::from).collect();
        // Nothing that 13.0 put in the class is missing, and nothing 13.0
        // assigned to another class is in it.
        assert_eq!(expected.difference(&ours).count(), 0, "{name}: missing");
        let extra: Vec<u32> = ours.difference(&expected).copied().collect();
        assert!(
            extra.iter().all(|code| !assigned.contains(code)),
            "{name}: reclassified"
        );
        assert_eq!(extra.len(), added, "{name}: added since 13.0");
    }
}

#[test]
fn the_combined_questions_are_the_union_of_the_single_ones() {
    for c in scalars() {
        assert_eq!(is_letter_or_digit(c), is_letter(c) || is_decimal_digit(c));
        assert_eq!(is_letter_or_number(c), is_letter(c) || is_number(c));
        // Digits are numbers; no character is two of letter, punctuation, symbol.
        assert!(!is_decimal_digit(c) || is_number(c));
        assert!(u8::from(is_letter(c)) + u8::from(is_punctuation(c)) + u8::from(is_symbol(c)) <= 1);
        assert!(!is_number(c) || !is_letter(c));
    }
}

#[test]
fn no_surrogate_or_unassigned_private_character_is_in_any_class() {
    for code in [0xD7FFu32, 0xE000, 0xF8FF, 0xFFFE, 0xFFFF, 0x10FFFF, 0xE0080] {
        let c = char::from_u32(code).unwrap();
        assert!(
            !is_letter(c) && !is_number(c) && !is_punctuation(c) && !is_symbol(c),
            "{code:X}"
        );
    }
}

#[test]
fn spot_checks_of_each_class() {
    assert!(is_letter('a') && is_letter('Ż') && is_letter('𝒜') && is_letter('ǅ') && is_letter('ª'));
    assert!(!is_letter('1') && !is_letter('\u{0301}') && !is_letter('_'));
    assert!(
        is_decimal_digit('7')
            && is_decimal_digit('٣')
            && !is_decimal_digit('²')
            && !is_decimal_digit('Ⅷ')
    );
    assert!(is_number('²') && is_number('Ⅷ') && is_number('½') && !is_number('a'));
    assert!(is_letter_or_digit('٣') && !is_letter_or_digit('²'));
    assert!(is_letter_or_number('²') && is_letter_or_number('Ⅷ'));
    assert!(
        is_punctuation('.') && is_punctuation('‿') && is_punctuation('–') && !is_punctuation('+')
    );
    assert!(is_symbol('+') && is_symbol('€') && is_symbol('😀') && !is_symbol('.'));
    // Added in Unicode 16.0, the version of the data.
    assert!(is_letter('\u{A7DC}'));
    // Added in Unicode 17.0, which the data does not have.
    assert!(!is_letter('\u{A7CE}'));
}

fn perl_upper() -> BTreeMap<u32, u32> {
    include_str!("fixtures/perl-simple-upper.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|line| {
            let (a, b) = line.split_once(' ').unwrap();
            (
                u32::from_str_radix(a, 16).unwrap(),
                u32::from_str_radix(b, 16).unwrap(),
            )
        })
        .collect()
}

#[test]
fn upper_case_is_the_standard_simple_mapping_but_for_two_characters() {
    let standard = perl_upper();
    let assigned = members(&ranges(include_str!("fixtures/python-categories.txt"))["ASSIGNED"]);
    let mut exceptions = BTreeSet::new();
    let mut added = BTreeSet::new();
    for c in scalars() {
        let ours = u32::from(to_upper_char(c));
        let code = u32::from(c);
        match standard.get(&code) {
            Some(&theirs) if theirs == ours => {}
            Some(_) => {
                exceptions.insert(code);
            }
            None if ours == code => {}
            None => {
                added.insert(code);
            }
        }
    }
    // The standard maps both to ASCII; the first server leaves them be.
    assert_eq!(exceptions, BTreeSet::from([0x131, 0x17F]));
    assert_eq!(to_upper_char('ı'), 'ı');
    assert_eq!(to_upper_char('ſ'), 'ſ');
    assert_eq!(to_upper_char('i'), 'I');
    assert_eq!(to_upper_char('s'), 'S');
    // Mappings the standard of 13.0 does not have: 65 new characters, and two
    // old ones that 16.0 gave an upper case (U+019B and U+0264).
    let old: Vec<u32> = added
        .iter()
        .copied()
        .filter(|code| assigned.contains(code))
        .collect();
    assert_eq!(old, [0x19B, 0x264]);
    assert_eq!(added.len(), 67);
    assert_eq!(u32::from(to_upper_char('\u{19B}')), 0xA7DC);
    assert_eq!(u32::from(to_upper_char('\u{264}')), 0xA7CB);
}

#[test]
fn the_full_mapping_would_have_made_two_characters_and_this_makes_one() {
    // Titlecase forms and the Greek with iota subscript have a simple mapping
    // of their own; the full mapping is two characters.
    assert_eq!(to_upper_char('\u{1F80}'), '\u{1F88}');
    assert_eq!(to_upper_char('\u{1FB3}'), '\u{1FBC}');
    assert_eq!(to_upper_char('\u{1C6}'), '\u{1C4}');
    assert_eq!(to_upper_char('\u{1C5}'), '\u{1C4}');
    // No simple mapping at all.
    assert_eq!(to_upper_char('ß'), 'ß');
    assert_eq!(to_upper_char('\u{149}'), '\u{149}');
    assert_eq!(to_upper("Straße"), "STRAßE");
    assert_eq!(to_upper("ǆ ǅ"), "Ǆ Ǆ");
    assert_eq!(to_upper("ilıſ"), "ILıſ");
    for text in ["", "abc", "Żaneta", "straße", "ǅ", "𐐨x", "ᾀ"] {
        assert_eq!(
            to_upper(text).chars().count(),
            text.chars().count(),
            "{text:?}"
        );
    }
    assert_eq!(to_upper("żaneta@Example.pl"), "ŻANETA@EXAMPLE.PL");
    assert_eq!(to_upper("𐐨"), "𐐀");
}

#[test]
fn what_the_standard_library_maps_differently_is_the_two_exceptions_or_newer_than_the_data() {
    for c in scalars() {
        assert!(!is_letter(c) || c.is_alphabetic(), "{c:?}");
        let mut upper = c.to_uppercase();
        let (Some(theirs), None) = (upper.next(), upper.next()) else {
            continue;
        };
        if theirs != to_upper_char(c) && !matches!(c, 'ı' | 'ſ') {
            // The library knows a newer Unicode: it maps to a character that
            // the data does not know yet.
            assert!(!is_letter(theirs), "{c:?} -> {theirs:?}");
        }
    }
}
