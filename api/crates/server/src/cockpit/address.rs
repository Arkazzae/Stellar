//! The e-mail address reader of the first server, ported decision for
//! decision.
//!
//! Both servers work on copies of one database, so an address one of them
//! takes must be taken by the other, and what one reads out of a text the
//! other must read too. The first server asks its runtime's own address
//! reader in three places: the e-mail of a new account, the sender address of
//! the mail settings (both valid when the reader takes the text), and the
//! recipient of a test mail (valid when the reader takes it and the address
//! it reads out is the text itself, ignoring case). That reader is far more
//! lenient than the grammar of a mail address, and this module answers as it
//! does, including where it is surprising.
//!
//! # How the reader goes
//!
//! It reads from the end of the text towards the start, in this order:
//!
//! 1. whitespace and comments (text in nested parentheses);
//! 2. an optional `>`, which then requires a `<` before the local part;
//! 3. the domain: a dot-atom, or a domain literal in square brackets;
//! 4. whitespace and comments, then `@`, then whitespace and comments;
//! 5. the local part: a dot-atom or a quoted string;
//! 6. whitespace and comments, and the `<` if a `>` was read (then only
//!    whitespace);
//! 7. whatever is left is a display name: a quoted string with only
//!    whitespace and comments around it, or else any text that a quoted
//!    string could hold, taken without the quotes.
//!
//! The rules that surprise:
//!
//! - A carriage return or a line feed anywhere refuses the text.
//! - A dot-atom is ASCII letters, digits, ``!#$%&'*+-/=?^_`{|}~`` and dots,
//!   and any character outside ASCII. It must not be empty, start with a dot
//!   or hold two dots in a row, but it may end with a dot: `a.@b.` is taken.
//! - The address read out is the local part, `@` and the domain as they were
//!   written (quotes and brackets kept, whitespace and comments around them
//!   dropped), each brought to Unicode canonical composition (NFC). The display
//!   name is not part of it: `Ala <ala@example.test>`, `x y@z` and
//!   `a@b <c@d>` read out `ala@example.test`, `y@z` and `c@d`.
//! - Whitespace may stand around the `@`: `a @ b` reads out `a@b`.
//! - A dot-atom local part must follow whitespace, `)`, `"`, `<` (when the
//!   text ended with `>`) or the start of the text, so `"x"a@b` is taken with
//!   the display name `x`, but `,a@b` is not.
//! - Inside a quoted string, a domain literal or a comment, a backslash makes
//!   the next character plain, but spaces and tabs are skipped before an
//!   escape is looked for, so `"a\ b"@c` is refused. A domain literal holds
//!   ASCII only.
//! - In a comment, `(` with no comment open is refused, and `)` opens one
//!   even after a backslash.
//! - The local part, the domain and the display name are each normalized, and
//!   normalization refuses U+FFFE, so U+FFFE in any of them refuses the text
//!   (it may stand in a comment around the address).
//!
//! The normalization is `stellarr_unicode::nfc`, whose data came from the
//! system library of the first server's container.

/// What the first server's address reader read out of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    /// The local part, `@` and the domain, as the first server's reader gives
    /// the address back: without a display name or comments, in canonical
    /// composition.
    pub address: String,
}

/// Reads a text the way the first server's address reader does; `None` where
/// it refuses the text.
pub fn try_create(text: &str) -> Option<Address> {
    // The reader refuses an empty text before reading, and any line break
    // before reading a single address.
    if text.is_empty() || text.contains(['\r', '\n']) {
        return None;
    }
    let data: Vec<char> = text.chars().collect();
    let (local, domain) = Reader { data: &data }.address()?;
    Some(Address {
        address: format!("{local}@{domain}"),
    })
}

/// The text being read, one character a place. The first server's reader
/// walks UTF-16 units, but it treats every unit outside ASCII alike and every
/// character it looks for is ASCII, so walking characters decides the same.
///
/// Positions are signed: the reader goes from the last character towards the
/// start, and `-1` means it went past the start.
struct Reader<'a> {
    data: &'a [char],
}

impl Reader<'_> {
    /// The character at a position the reader is about to look at. Every
    /// caller has checked the position is inside the text; outside it, a NUL
    /// stands in, which no rule takes.
    fn at(&self, index: isize) -> char {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.data.get(index))
            .copied()
            .unwrap_or('\0')
    }

    /// The characters from `first` to `last`, both included.
    fn text(&self, first: isize, last: isize) -> String {
        let from = usize::try_from(first).unwrap_or(0);
        let to = usize::try_from(last.saturating_add(1)).unwrap_or(0);
        self.data
            .get(from..to.min(self.data.len()))
            .map(|part| part.iter().collect())
            .unwrap_or_default()
    }

    /// The local part and the domain, read from the end of the text.
    fn address(&self) -> Option<(String, String)> {
        let last = isize::try_from(self.data.len()).ok()? - 1;
        let mut index = self.cfws_with_more(last)?;
        let bracketed = self.at(index) == '>';
        if bracketed {
            index -= 1;
        }
        let (domain, index) = self.domain(index)?;
        if self.at(index) != '@' {
            return None;
        }
        let (local, mut index) = self.local_part(index - 1, bracketed)?;
        if bracketed {
            if index >= 0 && self.at(index) == '<' {
                // Comments may belong to the display name, so only whitespace
                // is skipped here.
                index = self.fws(index - 1);
            } else {
                return None;
            }
        }
        if index >= 0 {
            self.display_name(index)?;
        }
        Some((local, domain))
    }

    /// The domain, ending at `index`, and where the reader stands after it and
    /// the whitespace and comments before it.
    fn domain(&self, index: isize) -> Option<(String, isize)> {
        let end = self.cfws_with_more(index)?;
        let start = if self.at(end) == ']' {
            self.domain_literal(end)?
        } else {
            self.dot_atom(end)?
        };
        let domain = normalize(&self.text(start + 1, end))?;
        Some((domain, self.cfws_with_more(start)?))
    }

    /// The local part, ending at `index`, and where the reader stands after it
    /// and the whitespace and comments before it.
    fn local_part(&self, index: isize, bracketed: bool) -> Option<(String, isize)> {
        let end = self.cfws_with_more(index)?;
        let start = if self.at(end) == '"' {
            self.quoted(end)?
        } else {
            let start = self.dot_atom(end)?;
            if start >= 0 {
                let before = self.at(start);
                let parted = matches!(before, ' ' | '\t' | '\r' | '\n' | ')' | '"')
                    || (bracketed && before == '<');
                if !parted {
                    return None;
                }
            }
            start
        };
        let local = normalize(&self.text(start + 1, end))?;
        Some((local, self.cfws(start)?))
    }

    /// Checks what is left before the address, from `index` back to the start.
    /// The name itself is not kept: only whether it can be normalized matters.
    fn display_name(&self, index: isize) -> Option<()> {
        let first = self.cfws(index)?;
        let name = if first >= 0 && self.at(first) == '"' {
            // A quoted name: the comments around it are not part of it, and
            // nothing but whitespace and comments may come before it.
            let before = self.quoted(first)?;
            let name = self.text(before + 2, first - 1);
            if self.cfws(before)? >= 0 {
                return None;
            }
            name
        } else {
            // An unquoted name runs to the start of the text and takes the
            // comments at its end as text, so it is read again from `index`.
            let start = self.unquoted(index)?;
            self.text(start + 1, index)
        };
        normalize(&name).map(|_| ())
    }

    /// Spaces and tabs, read backwards. Folded lines never get here: the text
    /// was refused for its line break.
    fn fws(&self, mut index: isize) -> isize {
        while index >= 0 && matches!(self.at(index), ' ' | '\t') {
            index -= 1;
        }
        index
    }

    /// Whitespace and comments, read backwards; where the reader stands after
    /// them, or `None` for a comment that is not closed or not opened.
    fn cfws(&self, index: isize) -> Option<isize> {
        let mut depth = 0usize;
        let mut index = self.fws(index);
        while index >= 0 {
            let escaped = self.escaped(index, true)?;
            let c = self.at(index);
            if depth > 0 && escaped > 0 {
                index -= escaped;
            } else if c == ')' {
                depth += 1;
                index -= 1;
            } else if c == '(' {
                depth = depth.checked_sub(1)?;
                index -= 1;
            } else if depth > 0 && (!c.is_ascii() || is_ctext(c)) {
                index -= 1;
            } else if depth > 0 {
                return None;
            } else {
                break;
            }
            index = self.fws(index);
        }
        (depth == 0).then_some(index)
    }

    /// As [`Self::cfws`], where more of the address must follow: going past
    /// the start of the text refuses it.
    fn cfws_with_more(&self, index: isize) -> Option<isize> {
        self.cfws(index).filter(|&index| index >= 0)
    }

    /// How many characters an escape ending at `index` covers (the character
    /// and the backslashes before it), or 0 when the character is not escaped.
    /// A run of backslashes escapes the character when it is odd. `None` when
    /// the escaped character is outside ASCII where only ASCII may be.
    fn escaped(&self, index: isize, unicode: bool) -> Option<isize> {
        if index <= 0 || self.at(index - 1) != '\\' {
            return Some(0);
        }
        let mut backslashes = 0;
        let mut at = index - 1;
        while at >= 0 && self.at(at) == '\\' {
            backslashes += 1;
            at -= 1;
        }
        if backslashes % 2 == 0 {
            return Some(0);
        }
        if !unicode && !self.at(index).is_ascii() {
            return None;
        }
        Some(backslashes + 1)
    }

    /// A dot-atom ending at `index`; the position of the first character
    /// before it.
    fn dot_atom(&self, end: isize) -> Option<isize> {
        let mut index = end;
        while index >= 0 {
            let c = self.at(index);
            if c.is_ascii() {
                let doubled = c == '.' && index > 0 && self.at(index - 1) == '.';
                if (c != '.' && !is_atext(c)) || doubled {
                    break;
                }
            }
            index -= 1;
        }
        let doubled = index > 0 && self.at(index) == '.' && self.at(index - 1) == '.';
        if index == end || doubled || self.at(index + 1) == '.' {
            return None;
        }
        Some(index)
    }

    /// A domain literal whose `]` is at `index`; the position before its `[`.
    fn domain_literal(&self, index: isize) -> Option<isize> {
        let mut index = index - 1;
        loop {
            index = self.fws(index);
            if index < 0 {
                return None;
            }
            let escaped = self.escaped(index, false)?;
            let c = self.at(index);
            if escaped > 0 {
                index -= escaped;
            } else if c == '[' {
                return Some(index - 1);
            } else if !c.is_ascii() || !is_dtext(c) {
                return None;
            } else {
                index -= 1;
            }
            if index < 0 {
                return None;
            }
        }
    }

    /// A quoted string whose closing quote is at `index`; the position before
    /// its opening quote.
    fn quoted(&self, index: isize) -> Option<isize> {
        let mut index = index - 1;
        loop {
            index = self.fws(index);
            if index < 0 {
                return None;
            }
            let escaped = self.escaped(index, true)?;
            let c = self.at(index);
            if escaped > 0 {
                index -= escaped;
            } else if c == '"' {
                return Some(index - 1);
            } else if c.is_ascii() && !is_qtext(c) {
                return None;
            } else {
                index -= 1;
            }
            if index < 0 {
                return None;
            }
        }
    }

    /// What a quoted string could hold, without the quotes, from `index` back
    /// to the start of the text; the position before it (always `-1`).
    fn unquoted(&self, mut index: isize) -> Option<isize> {
        loop {
            index = self.fws(index);
            if index < 0 {
                return Some(index);
            }
            let escaped = self.escaped(index, true)?;
            let c = self.at(index);
            if escaped > 0 {
                index -= escaped;
            } else if c.is_ascii() && !is_qtext(c) {
                return None;
            } else {
                index -= 1;
            }
            if index < 0 {
                return Some(index);
            }
        }
    }
}

/// ASCII that may stand in a dot-atom besides the dot.
fn is_atext(c: char) -> bool {
    matches!(c,
        'a'..='z' | 'A'..='Z' | '0'..='9'
        | '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '/' | '='
        | '?' | '^' | '_' | '`' | '{' | '|' | '}' | '~')
}

/// ASCII that may stand unescaped in a quoted string: all but NUL, line
/// breaks, `"` and `\`.
fn is_qtext(c: char) -> bool {
    matches!(c, '\u{1}'..='\u{9}' | '\u{B}' | '\u{C}' | '\u{E}'..='!' | '#'..='[' | ']'..='\u{7F}')
}

/// ASCII that may stand unescaped in a domain literal: all but NUL,
/// whitespace, `[`, `\` and `]`.
fn is_dtext(c: char) -> bool {
    matches!(c, '\u{1}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '!'..='Z' | '^'..='\u{7F}')
}

/// ASCII that may stand unescaped in a comment: all but NUL, whitespace,
/// parentheses and `\`.
fn is_ctext(c: char) -> bool {
    matches!(c,
        '\u{1}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}'
        | '!'..='\'' | '*'..='[' | ']'..='\u{7F}')
}

/// A part of the address in Unicode canonical composition (NFC), as the first
/// server's runtime makes it through the system library; `None` where the
/// runtime refuses to: for U+FFFE, which it takes for a broken text (a lone
/// surrogate, the other case, cannot be in a Rust string).
fn normalize(text: &str) -> Option<String> {
    if text.contains('\u{FFFE}') {
        return None;
    }
    Some(stellarr_unicode::nfc(text))
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn read(text: &str) -> Option<String> {
        try_create(text).map(|found| found.address)
    }

    /// Each case was put to the first server's own reader, and the answer
    /// (refused, or the address it read out) is the one it gave.
    fn agree(cases: &[(&str, Option<&str>)]) {
        for &(text, expected) in cases {
            assert_eq!(read(text).as_deref(), expected, "{text:?}");
        }
    }

    #[test]
    fn plain_addresses_and_dots() {
        agree(&[
            ("ala@example.test", Some("ala@example.test")),
            ("a@b", Some("a@b")),
            ("a@b.c", Some("a@b.c")),
            ("a.b.c.d@e.f.g.h", Some("a.b.c.d@e.f.g.h")),
            ("a.@b", Some("a.@b")),
            ("a.b.@c", Some("a.b.@c")),
            ("a@b.", Some("a@b.")),
            ("a@b.c.", Some("a@b.c.")),
            (".a@b", None),
            (".@b", None),
            ("a..b@c", None),
            ("a@.b", None),
            ("a@b..c", None),
            ("a@.", None),
            ("", None),
            (" ", None),
            ("a", None),
            ("@", None),
            ("@b", None),
            ("a@", None),
            ("a@b@c", None),
            ("a@@b", None),
            ("a@b@", None),
            ("\u{7f}@b", None),
            ("\\a@b", None),
            ("a@b;", None),
            ("name: a@b;", None),
        ]);
    }

    #[test]
    fn display_names_are_dropped() {
        agree(&[
            ("Ala <ala@example.test>", Some("ala@example.test")),
            ("\"Ala Kot\" <ala@example.test>", Some("ala@example.test")),
            ("Ala Kot <ala@example.test>", Some("ala@example.test")),
            ("<ala@example.test>", Some("ala@example.test")),
            ("< a@b >", Some("a@b")),
            ("\"y\"<a@b>", Some("a@b")),
            ("y<a@b>", Some("a@b")),
            ("a<b@c>", Some("b@c")),
            ("\"\" <a@b>", Some("a@b")),
            ("Ala, Kot <a@b>", Some("a@b")),
            ("\"Ala, Kot\" <a@b>", Some("a@b")),
            // Without brackets the reader still takes a name in front.
            ("x y@z", Some("y@z")),
            ("x y z@w", Some("z@w")),
            ("\"x\"a@b", Some("a@b")),
            ("x\"a\"@b", Some("\"a\"@b")),
            (", a@b", Some("a@b")),
            ("\\\\ a@b", Some("a@b")),
            // An unquoted name may be anything a quoted string could hold.
            ("a@b <c@d>", Some("c@d")),
            ("a@b, c@d", Some("c@d")),
            ("(x) \"y\" <a@b>", Some("a@b")),
            ("\"y\" (x) <a@b>", Some("a@b")),
            ("\"y\" z <a@b>", None),
            ("z \"y\" <a@b>", None),
            ("\\\" <a@b>", None),
            ("\"x\\\" <a@b>", None),
            ("x\0 <a@b>", None),
            ("a@b>", None),
            ("<a@b", None),
            ("<<a@b>>", None),
            ("<>", None),
            ("<@>", None),
            ("<a b@c>", None),
            ("<a@b> c", None),
            ("<a@b>,", None),
            (",a@b", None),
            ("a@b,", None),
            ("a@b,c@d", None),
        ]);
    }

    #[test]
    fn comments_and_whitespace_around_the_parts() {
        agree(&[
            ("ala@example.test (comment)", Some("ala@example.test")),
            ("(comment) ala@example.test", Some("ala@example.test")),
            ("ala(comment)@example.test", Some("ala@example.test")),
            ("ala@(comment)example.test", Some("ala@example.test")),
            ("a (c) @ (d) b", Some("a@b")),
            ("a @ b", Some("a@b")),
            ("<(c)a@b(d)>", Some("a@b")),
            ("<a@b> (c)", Some("a@b")),
            ("(a(b)c) d@e", Some("d@e")),
            ("((a@b)) c@d", Some("c@d")),
            ("(a\\(b) d@e", Some("d@e")),
            ("(a\\)b) d@e", Some("d@e")),
            ("(a\\\\b) d@e", Some("d@e")),
            ("(\\a) d@e", Some("d@e")),
            ("(a\\\0) d@e", Some("d@e")),
            ("(\u{7}) a@b", Some("a@b")),
            ("(ż) d@e", Some("d@e")),
            ("d@e (\\ż)", Some("d@e")),
            ("a@b (\\\\)", Some("a@b")),
            ("ala@example(c).test", None),
            ("(\0) d@e", None),
            ("a@b(\0)", None),
            ("( a@b", None),
            ("a@b )", None),
            ("a@b (", None),
            ("a@b (c))", None),
            (")a@b", None),
            ("(a@b)", None),
            ("a@b \\)", None),
            ("a@b (\\)", None),
        ]);
    }

    #[test]
    fn quoted_local_parts_and_domain_literals_keep_their_marks() {
        agree(&[
            ("\"a b\"@c", Some("\"a b\"@c")),
            ("\"a\\\"b\"@c", Some("\"a\\\"b\"@c")),
            ("\"a\\\\b\"@c", Some("\"a\\\\b\"@c")),
            ("\"a\\xb\"@c", Some("\"a\\xb\"@c")),
            ("\"\"@b", Some("\"\"@b")),
            ("\"\\\"\"@b", Some("\"\\\"\"@b")),
            ("\"\\ż\"@b", Some("\"\\ż\"@b")),
            ("\"\t\"@b", Some("\"\t\"@b")),
            ("\"\\\0\"@b", Some("\"\\\0\"@b")),
            ("\"a@b\"@c", Some("\"a@b\"@c")),
            ("<\"a@b\"@c>", Some("\"a@b\"@c")),
            ("a@[1.2.3.4]", Some("a@[1.2.3.4]")),
            ("a@[ 1 . 2 ]", Some("a@[ 1 . 2 ]")),
            ("a@[a b]", Some("a@[a b]")),
            ("a@[\\]]", Some("a@[\\]]")),
            ("a@[]", Some("a@[]")),
            // Spaces are skipped before an escape is looked for.
            ("\"a\\ b\"@c", None),
            ("a@[a\\ b]", None),
            ("\"\\\"@b", None),
            ("\"a\"b\"@c", None),
            ("\"\0\"@b", None),
            ("\"a@b\"", None),
            ("a@[ż]", None),
            ("a@[\\ż]", None),
        ]);
    }

    #[test]
    fn line_breaks_refuse_the_text_anywhere() {
        agree(&[
            ("a@b\r\n", None),
            ("\"a\r\n b\"@c", None),
            ("(a\r\n b) c@d", None),
            ("\"a\\\rb\"@c", None),
            ("\"a\\\nb\"@c", None),
        ]);
    }

    #[test]
    fn text_outside_ascii_is_taken_and_composed() {
        agree(&[
            ("żółć@gęś.pl", Some("żółć@gęś.pl")),
            ("İ@İ.tr", Some("İ@İ.tr")),
            ("ß@straße.de", Some("ß@straße.de")),
            ("ａ@ｂ．ｃ", Some("ａ@ｂ．ｃ")),
            ("a\u{a0}b@c", Some("a\u{a0}b@c")),
            ("a@b\u{3002}c", Some("a@b\u{3002}c")),
            ("😀@😀.com", Some("😀@😀.com")),
            ("a\u{ffff}@b", Some("a\u{ffff}@b")),
            // Canonical composition, part by part.
            ("e\u{301}@b", Some("é@b")),
            ("a@e\u{301}", Some("a@é")),
            ("a@\u{c5}\u{212b}A\u{30a}", Some("a@ÅÅÅ")),
            ("a@b\u{301}\u{323}", Some("a@\u{1e05}\u{301}")),
            ("a@a\u{307}\u{323}", Some("a@\u{1ea1}\u{307}")),
            ("a@\u{b47}\u{b3e}", Some("a@\u{b4b}")),
            ("\u{1100}\u{1161}\u{11a8}@b", Some("\u{ac01}@b")),
            ("a@\u{ac00}\u{11a8}", Some("a@\u{ac01}")),
            ("a@\u{344}", Some("a@\u{308}\u{301}")),
            ("a@\u{958}", Some("a@\u{915}\u{93c}")),
            ("a@\u{212a}", Some("a@K")),
            ("a@\u{37e}", Some("a@;")),
            ("a@\u{1fef}", Some("a@`")),
            ("a\u{1fbe}@b", Some("a\u{3b9}@b")),
            ("<\u{338}a@b>", Some("\u{338}a@b")),
            ("a@b>\u{338}", None),
            ("a＠b", None),
            // Normalization refuses U+FFFE in a part, not in a comment.
            ("a\u{fffe}@b", None),
            ("a@b\u{fffe}", None),
            ("\"a\u{fffe}\"@b", None),
            ("x\u{fffe} <a@b>", None),
            ("\"x\u{fffe}\" <a@b>", None),
            ("x (\u{fffe}) <a@b>", None),
            ("(\u{fffe}) a@b", Some("a@b")),
            ("a@b (\u{fffe})", Some("a@b")),
            ("\"y\" (\u{fffe}) <a@b>", Some("a@b")),
        ]);
    }

    #[test]
    fn long_and_deep_texts_are_read_in_one_pass() {
        let long = format!("{}@{}", "x".repeat(4000), "y".repeat(4000));
        assert_eq!(read(&long), Some(long.clone()));
        let nested = format!("{}{} a@b", "(".repeat(3000), ")".repeat(3000));
        assert_eq!(read(&nested).as_deref(), Some("a@b"));
        let escapes = format!("a@b ({})", "\\".repeat(5000));
        assert_eq!(read(&escapes).as_deref(), Some("a@b"));
        let odd = format!("a@b ({})", "\\".repeat(5001));
        assert_eq!(read(&odd), None);
        let quoted = format!("\"{}\"@b", "\\".repeat(5000));
        assert_eq!(read(&quoted), Some(quoted.clone()));
        let marks = format!("a@b{}", "\u{301}\u{323}".repeat(1500));
        let composed = format!("a@\u{1e05}{}{}", "\u{323}".repeat(1499), "\u{301}".repeat(1500));
        assert_eq!(read(&marks), Some(composed));
        let syllables = format!("a@{}", "\u{1100}\u{1161}\u{11a8}".repeat(1000));
        assert_eq!(read(&syllables), Some(format!("a@{}", "\u{ac01}".repeat(1000))));
    }

    /// The line the witness program prints for an input.
    fn answer(text: &str) -> String {
        let Some(found) = try_create(text) else {
            return "invalid".to_owned();
        };
        let mut line = String::from("valid \"");
        for unit in found.address.encode_utf16() {
            match char::from_u32(u32::from(unit)) {
                Some('"') => line.push_str("\\\""),
                Some('\\') => line.push_str("\\\\"),
                Some(c @ ' '..='~') => line.push(c),
                _ => {
                    let _ = write!(line, "\\u{unit:04x}");
                }
            }
        }
        line.push('"');
        line
    }

    /// Asks this reader everything the first server's reader was asked.
    ///
    /// Not part of the ordinary run: it needs a file of inputs, one JSON
    /// string a line, and the answers the witness program in
    /// `tests/cockpit-address-witness` gave for them (`valid "<address>"` or
    /// `invalid`, a line each). Build the witness outside the repository and
    /// run it with the inputs:
    ///
    /// ```sh
    /// cp tests/cockpit-address-witness/* "$work/witness/"
    /// dotnet build "$work/witness" -c Release -o "$work/witness/out"
    /// dotnet "$work/witness/out/witness.dll" < inputs.jsonl > answers.txt
    /// ADDRESS_WITNESS_INPUTS=inputs.jsonl ADDRESS_WITNESS_ANSWERS=answers.txt \
    ///   acargo test -p stellarr-api --lib cockpit::address -- --ignored --nocapture
    /// ```
    ///
    /// A runtime whose system library follows a Unicode newer than 15.1
    /// composes some characters assigned since, which this reader leaves
    /// alone as the first server's container does; only inputs holding them
    /// differ.
    #[test]
    #[ignore = "needs the inputs and the first server's answers; see the comment above"]
    fn the_first_servers_reader_agrees_on_every_input() {
        let inputs = std::fs::read_to_string(std::env::var("ADDRESS_WITNESS_INPUTS").unwrap()).unwrap();
        let answers = std::fs::read_to_string(std::env::var("ADDRESS_WITNESS_ANSWERS").unwrap()).unwrap();
        assert_eq!(inputs.lines().count(), answers.lines().count(), "one answer an input");
        let mut differing = Vec::new();
        let mut agreeing = 0usize;
        for (line, expected) in inputs.lines().zip(answers.lines()) {
            let input: String = serde_json::from_str(line).unwrap();
            let got = answer(&input);
            if got == expected {
                agreeing += 1;
            } else {
                differing.push(format!("{line}\n  first: {expected}\n  here:  {got}"));
            }
        }
        eprintln!("{agreeing} agree, {} differ", differing.len());
        let shown: Vec<_> = differing.iter().take(40).cloned().collect();
        assert!(differing.is_empty(), "{} differ:\n{}", differing.len(), shown.join("\n"));
    }
}

