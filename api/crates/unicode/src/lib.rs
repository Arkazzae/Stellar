//! Unicode general categories, simple casing and UTF-16 text handling.
//!
//! The standard library cannot stand in for this. It has no way to ask for a
//! general category, and its "alphabetic" and "numeric" take in marks and
//! other kinds of number than the ones a user name is made of. Its casing is
//! the full mapping, which turns one character into two (`ß` into `SS`) where
//! the simple mapping keeps it one. Names and e-mail addresses use the simple
//! mapping, character by character, so that comparing two texts never changes
//! how long either is.
//!
//! # Where the data comes from
//!
//! `tables.rs` pins the categories and casing of every code point from
//! U+0000 through U+10FFFF except surrogates. The two tables use different
//! Unicode versions:
//!
//! - **Categories** use version 16.0 of the Unicode Standard.
//!   - **Letter** is General_Category `L*` (Lu, Ll, Lt, Lm, Lo).
//!   - **Decimal digit** is `Nd`; **number** is `N*` (Nd, Nl, No).
//!   - **Punctuation** is `P*` and **symbol** is `S*`.
//! - **Casing** uses ICU 74 and Unicode 15.1, one version older than the
//!   categories, so a
//!   character added in 16.0 is a letter here and upper-cases to itself. The
//!   mapping is ICU's simple one (`u_toupper`, `u_tolower`: field 12 and field
//!   13 of `UnicodeData.txt`) with two exceptions to match Windows invariant casing:
//!   - U+0131 LATIN SMALL LETTER DOTLESS I is not upper-cased: it stays `ı`
//!     (the standard says `I`).
//!   - U+0130 LATIN CAPITAL LETTER I WITH DOT ABOVE is not lower-cased: it
//!     stays `İ` (the standard says `i`).
//!
//!   U+017F LATIN SMALL LETTER LONG S upper-cases to `S` as in Unicode 15.1.
//!
//! The data is frozen so upgrades to Unicode cannot change which names we
//! accept or compare as equal. A character added after the version of a
//! table is in none of its classes and maps to itself.
//!
//! # Making the data again
//!
//! To regenerate the tables, use `UnicodeData.txt` from the Unicode Character Database:
//!
//! 1. Categories, of version 16.0: take field 2, the General_Category, for
//!    every code point (expanding the `First>`/`Last>` range pairs), and list
//!    the ranges whose category is in each class above. Leave out the
//!    surrogates.
//! 2. Upper case, of version 15.1: take field 12, Simple_Uppercase_Mapping,
//!    for every code point that has one, and drop the entry for U+0131.
//! 3. Lower case, of version 15.1: take field 13, Simple_Lowercase_Mapping,
//!    the same way, and drop the entry for U+0130.
//! 4. Check the result against `tests/fixtures`, which hold the same questions
//!    answered by Unicode 13: nothing that 13 assigned may differ, but for
//!    the exceptions the tests name.
//!
//! The tests pin the number of code points added since 13, so that a table
//! made from another version is noticed at once and changed on purpose.
//!
//! # Normalization
//!
//! [`nfc`] puts a text in canonical composition. Its data,
//! `normalization_tables.rs`, is not written by hand: the example
//! `make_normalization_tables` makes it from two files of the Unicode
//! Character Database, version 15.1.0. The generated tables match ICU 74.2
//! entry for entry. Fetch the inputs from
//!
//! - `https://www.unicode.org/Public/15.1.0/ucd/UnicodeData.txt`
//! - `https://www.unicode.org/Public/15.1.0/ucd/CompositionExclusions.txt`
//!
//! into one folder, and run, from the `api` folder:
//!
//! ```sh
//! cargo run -p stellarr-unicode --example make_normalization_tables -- <folder> \
//!     > crates/unicode/src/normalization_tables.rs
//! ```
//!
//! The program checks the SHA-256 of each file against the one it has written
//! in it and refuses another version. The standard's own conformance file,
//! `NormalizationTest.txt` of the same version (same address), is checked by
//! the ignored test `normalization_test_file`, which takes the same folder:
//!
//! ```sh
//! UCD_FOLDER=<folder> cargo test -p stellarr-unicode --test normalization -- --ignored
//! ```
//!
//! A new version of Unicode is a decision, like the tables of casing: change
//! the version and the checksums in the example on purpose, and the tests.

mod normalization;
mod normalization_tables;
mod tables;
pub mod utf16;

pub use normalization::nfc;

use tables::{DIGIT, LETTER, LOWER, NUMBER, PUNCTUATION, SYMBOL, UPPER};

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

/// A letter: General_Category `L*`.
pub fn is_letter(c: char) -> bool {
    in_table(LETTER, u32::from(c))
}

/// A decimal digit: General_Category `Nd`.
pub fn is_decimal_digit(c: char) -> bool {
    in_table(DIGIT, u32::from(c))
}

/// A number of any kind: General_Category `N*` (digits, letter numbers such as
/// Roman numerals, and other numbers such as fractions).
pub fn is_number(c: char) -> bool {
    in_table(NUMBER, u32::from(c))
}

/// A letter or a decimal digit.
pub fn is_letter_or_digit(c: char) -> bool {
    is_letter(c) || is_decimal_digit(c)
}

/// A letter or a number of any kind. User names are made of these.
pub fn is_letter_or_number(c: char) -> bool {
    is_letter(c) || is_number(c)
}

/// Punctuation: General_Category `P*`.
pub fn is_punctuation(c: char) -> bool {
    in_table(PUNCTUATION, u32::from(c))
}

/// A symbol: General_Category `S*`.
pub fn is_symbol(c: char) -> bool {
    in_table(SYMBOL, u32::from(c))
}

fn map(table: &[(u32, u32)], c: char) -> char {
    let code = u32::from(c);
    table
        .binary_search_by_key(&code, |&(from, _)| from)
        .ok()
        .and_then(|at| char::from_u32(table[at].1))
        .unwrap_or(c)
}

/// The simple uppercase mapping for a character. Unmapped characters stay as they are.
pub fn to_upper_char(c: char) -> char {
    map(UPPER, c)
}

/// Upper case, character by character: the result has as many characters as
/// the text, and `ß` stays `ß`.
pub fn to_upper(text: &str) -> String {
    text.chars().map(to_upper_char).collect()
}

/// The simple lowercase mapping for a character. Unmapped characters stay as they are.
pub fn to_lower_char(c: char) -> char {
    map(LOWER, c)
}

/// Lower case, character by character: the result has as many characters as
/// the text, and `İ` stays `İ`.
pub fn to_lower(text: &str) -> String {
    text.chars().map(to_lower_char).collect()
}
