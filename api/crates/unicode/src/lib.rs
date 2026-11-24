//! Unicode general categories and simple upper-casing, with the answers the
//! first Stellarr server gave, and the UTF-16 view of text that message code
//! works in.
//!
//! The standard library cannot stand in for this. It has no way to ask for a
//! general category, and its "alphabetic" and "numeric" take in marks and
//! other kinds of number than the ones a user name is made of. Its upper-casing
//! is the full mapping, which turns one character into two (`ß` into `SS`)
//! where the simple mapping keeps it one. Names and e-mail addresses are
//! compared by the simple mapping, character by character, so that comparing
//! two texts never changes how long either is.
//!
//! # Where the data comes from
//!
//! Everything in `tables.rs` was read out of the character data of the runtime
//! of the first server, version 16.0 of the Unicode Standard, by asking that
//! runtime about every code point from U+0000 to U+10FFFF other than the
//! surrogates:
//!
//! - **Letter** is General_Category `L*` (Lu, Ll, Lt, Lm, Lo).
//! - **Decimal digit** is `Nd`; **number** is `N*` (Nd, Nl, No).
//! - **Punctuation** is `P*` and **symbol** is `S*`.
//! - **Upper case** is the Simple_Uppercase_Mapping, field 12 of
//!   `UnicodeData.txt`, with two exceptions, below.
//!
//! The data is frozen at that version on purpose: a name that was a name stays
//! one, and two names that were the same stay the same, whatever Unicode adds
//! later. A character added to Unicode after 16.0 is in no class here and
//! upper-cases to itself.
//!
//! ## The two exceptions to the standard simple mapping
//!
//! The first server leaves two characters as they are where the standard maps
//! them to ASCII, because its runtime never lets a non-ASCII character
//! upper-case to an ASCII one:
//!
//! - U+0131 LATIN SMALL LETTER DOTLESS I stays `ı` (the standard says `I`).
//! - U+017F LATIN SMALL LETTER LONG S stays `ſ` (the standard says `S`).
//!
//! These are the only two. Every other mapping in the table is the standard's,
//! including the titlecase forms that the full mapping would turn into two
//! characters (U+1F80 `ᾀ` becomes U+1F88 `ᾈ`, as its simple mapping says).
//! The tests check this against an independent copy of Unicode 13.
//!
//! # Making the data again
//!
//! When the first server's runtime is no longer there to ask, make the tables
//! from the Unicode Character Database of version 16.0 (unicode.org, `ucd`):
//!
//! 1. From `UnicodeData.txt`, take field 2, the General_Category, for every
//!    code point (expanding the `First>`/`Last>` range pairs), and list the
//!    ranges whose category is in each class above. Leave out the surrogates.
//! 2. From the same file take field 12, Simple_Uppercase_Mapping, for every
//!    code point that has one, and drop the entries for U+0131 and U+017F.
//! 3. Check the result against `tests/fixtures`, which hold the same questions
//!    answered by Unicode 13: nothing that 13 assigned may differ.
//!
//! The tests pin the number of code points added since 13, so that a table
//! made from another version is noticed at once and changed on purpose.

mod tables;
pub mod utf16;

use tables::{DIGIT, LETTER, NUMBER, PUNCTUATION, SYMBOL, UPPER};

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

/// The simple upper case of one character, or the character itself.
pub fn to_upper_char(c: char) -> char {
    let code = u32::from(c);
    UPPER
        .binary_search_by_key(&code, |&(from, _)| from)
        .ok()
        .and_then(|at| char::from_u32(UPPER[at].1))
        .unwrap_or(c)
}

/// Upper case, character by character: the result has as many characters as
/// the text, and `ß` stays `ß`.
pub fn to_upper(text: &str) -> String {
    text.chars().map(to_upper_char).collect()
}
