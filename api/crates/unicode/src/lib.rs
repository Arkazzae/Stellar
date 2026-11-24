//! Unicode general categories, and simple upper- and lower-casing, with the
//! answers the first Stellarr server gave on Linux, and the UTF-16 view of
//! text that message code works in.
//!
//! The standard library cannot stand in for this. It has no way to ask for a
//! general category, and its "alphabetic" and "numeric" take in marks and
//! other kinds of number than the ones a user name is made of. Its casing is
//! the full mapping, which turns one character into two (`ß` into `SS`) where
//! the simple mapping keeps it one, and it follows a newer Unicode than the
//! first server did. Names and e-mail addresses are compared by the simple
//! mapping, character by character, so that comparing two texts never changes
//! how long either is.
//!
//! # Where the data comes from
//!
//! Everything in `tables.rs` was read out of the first server's own platform:
//! its runtime, running on Linux in the container the server is shipped in,
//! was asked about every code point from U+0000 to U+10FFFF other than the
//! surrogates. The tables are what that platform answered, not what a
//! standard says, and the two parts come from different places:
//!
//! - **Categories** come from the runtime's own character data, which is
//!   version 16.0 of the Unicode Standard and does not depend on the
//!   platform (a dump on macOS gave the same ranges).
//!   - **Letter** is General_Category `L*` (Lu, Ll, Lt, Lm, Lo).
//!   - **Decimal digit** is `Nd`; **number** is `N*` (Nd, Nl, No).
//!   - **Punctuation** is `P*` and **symbol** is `S*`.
//! - **Casing** comes from the system's ICU library, as the runtime calls it
//!   for its invariant culture. The ICU of the container (version 74) follows
//!   version 15.1 of Unicode, one version older than the categories, so a
//!   character added in 16.0 is a letter here and upper-cases to itself. The
//!   mapping is ICU's simple one (`u_toupper`, `u_tolower`: field 12 and field
//!   13 of `UnicodeData.txt`) with two exceptions the runtime makes itself, to
//!   keep what Windows does:
//!   - U+0131 LATIN SMALL LETTER DOTLESS I is not upper-cased: it stays `ı`
//!     (the standard says `I`).
//!   - U+0130 LATIN CAPITAL LETTER I WITH DOT ABOVE is not lower-cased: it
//!     stays `İ` (the standard says `i`).
//!
//!   U+017F LATIN SMALL LETTER LONG S is not an exception: it upper-cases to
//!   `S`, as the standard says. (A dump on macOS leaves it alone, which is why
//!   the data was taken on the platform the server runs on.)
//!
//! Every place in the first server that changes case calls the runtime's
//! invariant upper-casing or lower-casing of a string, which goes through the
//! same code as the one of a single character, so there is one function for
//! each direction. The data is frozen on purpose: a name that was a name stays
//! one, and two names that were the same stay the same, whatever Unicode adds
//! later. A character added after the version of a table is in no class of it
//! and maps to itself.
//!
//! # Making the data again
//!
//! When the first server's platform is no longer there to ask, make the tables
//! from the Unicode Character Database, `UnicodeData.txt` (unicode.org, `ucd`):
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
//! [`nfc`] puts a text in canonical composition, as the first server's
//! platform does through its ICU library. Its data, `normalization_tables.rs`,
//! is not written by hand: `generator/` is a small program that asks that
//! library, on the machine it runs on, for the combining class of every code
//! point, the one-level canonical decomposition of every character and which
//! pairs compose back (all but the composition exclusions). It has to be run
//! in the image the server is shipped in, whose ICU is the reference, and not
//! on a development machine, whose ICU follows another Unicode:
//!
//! ```sh
//! dotnet build generator -c Release -o /tmp/generator
//! docker run --rm -v /tmp/generator:/gen:ro --entrypoint dotnet \
//!     stellarr-api-dev:local /gen/generator.dll > src/normalization_tables.rs
//! ```
//!
//! The tables of this version came from ICU 74.2 in that image, which follows
//! version 15.1 of Unicode.
//!

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

/// The simple upper case of one character, or the character itself, as the
/// first server's invariant upper-casing gives it.
pub fn to_upper_char(c: char) -> char {
    map(UPPER, c)
}

/// Upper case, character by character: the result has as many characters as
/// the text, and `ß` stays `ß`.
pub fn to_upper(text: &str) -> String {
    text.chars().map(to_upper_char).collect()
}

/// The simple lower case of one character, or the character itself, as the
/// first server's invariant lower-casing gives it.
pub fn to_lower_char(c: char) -> char {
    map(LOWER, c)
}

/// Lower case, character by character: the result has as many characters as
/// the text, and `İ` stays `İ`.
pub fn to_lower(text: &str) -> String {
    text.chars().map(to_lower_char).collect()
}
