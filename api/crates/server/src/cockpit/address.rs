//! The e-mail address reader of the panel and of the mail settings: one plain
//! address, `local@domain`, and nothing around it.
//!
//! - The text holds exactly one `@`, and no white space, display name,
//!   comment, angle bracket or quote: `Ala <ala@example.test>`,
//!   `a@b, c@d` and `"a b"@c` are refused.
//! - The local part and the domain are each a dot-atom: ASCII letters, digits,
//!   ``!#$%&'*+-/=?^_`{|}~`` and dots, and any character outside ASCII. It
//!   must not be empty, start with a dot or hold two dots in a row, but it may
//!   end with a dot: `a.@b.` is taken.
//! - The domain may instead be a literal in square brackets that holds ASCII
//!   without white space, `[` and `]`, such as `[1.2.3.4]`.
//! - The address read out is the text with each part brought to Unicode
//!   canonical composition (NFC), and U+FFFE in either part refuses the text.

/// What was read out of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    /// The local part, `@` and the domain, in canonical composition.
    pub address: String,
}

/// Reads one plain address; `None` where the text is anything else.
pub fn try_create(text: &str) -> Option<Address> {
    let (local, domain) = text.split_once('@')?;
    if domain.contains('@') || !is_dot_atom(local) || !(is_dot_atom(domain) || is_literal(domain)) {
        return None;
    }
    Some(Address {
        address: format!("{}@{}", normalize(local)?, normalize(domain)?),
    })
}

/// A dot-atom: never empty, never starting with a dot or holding two in a row.
fn is_dot_atom(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('.')
        && !text.contains("..")
        && text
            .chars()
            .all(|c| !c.is_ascii() || c == '.' || is_atext(c))
}

/// `[` and `]` around ASCII that is not white space, `[`, `\` or `]`.
fn is_literal(text: &str) -> bool {
    text.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|inside| inside.chars().all(|c| c.is_ascii() && is_dtext(c)))
}

/// ASCII that may stand in a dot-atom besides the dot.
fn is_atext(c: char) -> bool {
    matches!(c,
        'a'..='z' | 'A'..='Z' | '0'..='9'
        | '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '/' | '='
        | '?' | '^' | '_' | '`' | '{' | '|' | '}' | '~')
}

/// ASCII that may stand in a domain literal: all but NUL, white space, `[`,
/// `\` and `]`.
fn is_dtext(c: char) -> bool {
    matches!(c, '\u{1}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '!'..='Z' | '^'..='\u{7F}')
}

/// A part of the address in Unicode canonical composition (NFC); `None` for
/// U+FFFE, which the normalization refuses as a broken text.
fn normalize(text: &str) -> Option<String> {
    if text.contains('\u{FFFE}') {
        return None;
    }
    Some(stellarr_unicode::nfc(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Option<String> {
        try_create(text).map(|found| found.address)
    }

    #[test]
    fn a_plain_address_is_read_out_as_it_is() {
        for text in [
            "ala@example.test",
            "a@b",
            "a@b.c",
            "a.b-c_d+e@sub.example.test",
            "a.@example.test",
            "a@example.test.",
            "a@[1.2.3.4]",
            "żółć@gęś.pl",
            "😀@😀.com",
            "{a}|~@b",
        ] {
            assert_eq!(read(text).as_deref(), Some(text), "{text}");
        }
    }

    #[test]
    fn a_name_a_comment_a_list_or_a_quote_is_no_plain_address() {
        for text in [
            "",
            "a",
            "a@",
            "@",
            "@example.test",
            ".a@example.test",
            "a..b@example.test",
            "a@.example.test",
            "a@example..test",
            "a@@example.test",
            "a@b@example.test",
            "a@exam ple.test",
            " a@example.test",
            "a@example.test ",
            "a b@example.test",
            "a@example.test\n",
            "\"a b\"@example.test",
            "\"a\"@example.test",
            "Name <a@example.test>",
            "<a@example.test>",
            "(comment) a@example.test",
            "a@example.test (comment)",
            "a(comment)@example.test",
            "a@example.test, b@example.test",
            "a@example.test b@example.test",
            "a@[1.2.3.4",
            "a@[1 2]",
            "a@[\\]]",
            "a@ex:ample,test",
            "a,b@example.test",
            "a＠b",
        ] {
            assert_eq!(read(text), None, "{text:?}");
        }
    }

    #[test]
    fn text_outside_ascii_is_taken_and_composed() {
        for (text, expected) in [
            ("e\u{301}@b", Some("é@b")),
            ("a@e\u{301}", Some("a@é")),
            ("a@\u{c5}\u{212b}A\u{30a}", Some("a@ÅÅÅ")),
            ("\u{1100}\u{1161}\u{11a8}@b", Some("\u{ac01}@b")),
            ("a@\u{212a}", Some("a@K")),
            ("a\u{fffe}@b", None),
            ("a@b\u{fffe}", None),
        ] {
            assert_eq!(read(text).as_deref(), expected, "{text:?}");
        }
    }

    #[test]
    fn long_texts_are_read_in_one_pass() {
        let long = format!("{}@{}", "x".repeat(4000), "y".repeat(4000));
        assert_eq!(read(&long), Some(long.clone()));
        let marks = format!("a@b{}", "\u{301}\u{323}".repeat(1500));
        let composed = format!("a@\u{1e05}{}{}", "\u{323}".repeat(1499), "\u{301}".repeat(1500));
        assert_eq!(read(&marks), Some(composed));
    }
}
