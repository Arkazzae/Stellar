//! Turning what a page wrote into text that is safe to store and show.

use crate::entity_table::ENTITIES;

/// The longest name any reference has. A name longer than this is copied
/// through without being looked up.
const LONGEST_ENTITY_NAME: usize = 8;

/// Decodes character references. What is recognised, and what is not:
///
/// - `&name;` for the named references of HTML 4 and `&apos;`, in exactly
///   that case. An unknown name stays as written.
/// - `&#123;` and `&#x7B;`, which may carry spaces and a sign in the decimal
///   form. A value that is not a character (a surrogate, past U+10FFFF, too
///   large) leaves the text as written. Zero becomes U+FFFD.
/// - A reference without its semicolon is not one, and a second `&` before
///   the semicolon starts over from there.
///
/// One pass: `&amp;lt;` is `&lt;`, not `<`.
pub fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        // The reference ends at the first ';' or '&', whichever comes first.
        let end = after.find([';', '&']);
        match end {
            Some(end) if after.as_bytes()[end] == b';' => {
                let body = &after[..end];
                if let Some(decoded) = decode_one(body) {
                    out.push(decoded);
                } else {
                    // Unknown, or not a number: as written, in one piece.
                    out.push('&');
                    out.push_str(body);
                    out.push(';');
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_one(body: &str) -> Option<char> {
    if let Some(number) = body.strip_prefix('#').filter(|n| !n.is_empty()) {
        return decode_number(number);
    }
    if body.len() > LONGEST_ENTITY_NAME {
        return None;
    }
    ENTITIES
        .binary_search_by(|(name, _)| name.cmp(&body))
        .ok()
        .and_then(|at| char::from_u32(u32::from(ENTITIES[at].1)))
}

fn decode_number(number: &str) -> Option<char> {
    let value = match number.strip_prefix(['x', 'X']) {
        Some(hex) => parse_unsigned(hex, 16, false)?,
        None => parse_unsigned(number, 10, true)?,
    };
    match value {
        0 => Some('\u{FFFD}'),
        _ => char::from_u32(value),
    }
}

/// The digits of a number the way a lenient reader takes them: in decimal
/// the whole may be padded with spaces and start with a sign (minus only
/// before zero); in hex it is the digits alone. `None` when it is not a
/// number or does not fit 32 bits.
fn parse_unsigned(text: &str, radix: u32, lenient: bool) -> Option<u32> {
    let is_space = |c: char| matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ');
    let mut text = text;
    let mut negative = false;
    if lenient {
        text = text.trim_matches(is_space);
        if let Some(rest) = text.strip_prefix('+') {
            text = rest;
        } else if let Some(rest) = text.strip_prefix('-') {
            negative = true;
            text = rest;
        }
    }
    if text.is_empty() || !text.bytes().all(|b| (b as char).is_digit(radix)) {
        return None;
    }
    // Digits past what fits in 32 bits are only zeros when the value fits.
    let value = text.bytes().try_fold(0u32, |acc, b| {
        acc.checked_mul(radix)?
            .checked_add((b as char).to_digit(radix)?)
    })?;
    (!negative || value == 0).then_some(value)
}

/// What a card shows of a field: references decoded, runs of whitespace
/// collapsed to one space, the text cut to `limit` UTF-16 code units with an
/// ellipsis, and nothing left that is not text. `None` when nothing is left.
///
/// The limit counts UTF-16 code units because that is what the clients that
/// show it count, and a cut never separates a character from what it is made
/// of.
pub fn clean_text(value: Option<&str>, limit: usize) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    let decoded = decode_entities(value);
    let mut text = String::with_capacity(decoded.len().min(limit * 3));
    // A NUL is not text, and in a stored value it ends the string for some
    // of what reads it.
    for word in decoded
        .split(char::is_whitespace)
        .map(|w| w.replace('\0', ""))
        .filter(|w| !w.is_empty())
    {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&word);
    }
    if text.is_empty() {
        return None;
    }
    let units: usize = text.chars().map(char::len_utf16).sum();
    if units <= limit {
        return Some(text);
    }
    let mut kept = 0;
    let mut cut = 0;
    for (at, c) in text.char_indices() {
        kept += c.len_utf16();
        if kept > limit {
            break;
        }
        cut = at + c.len_utf8();
    }
    let mut cut_text = text[..cut].trim_end().to_owned();
    cut_text.push('…');
    Some(cut_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_every_form_of_reference() {
        for (input, expected) in [
            ("a &amp; b", "a & b"),
            ("&lt;b&gt; &quot;x&quot; &apos;y&apos;", "<b> \"x\" 'y'"),
            ("&nbsp;", "\u{a0}"),
            ("&eacute;&Eacute;&euro;&hearts;", "éÉ€♥"),
            ("&#65;&#x42;&#X43;", "ABC"),
            ("&#128512;&#x1F600;", "😀😀"),
            ("&# 65 ;", "A"),
            ("&#+65;", "A"),
            ("&#0;", "\u{FFFD}"),
            ("&#x0;", "\u{FFFD}"),
            ("&#-0;", "\u{FFFD}"),
        ] {
            assert_eq!(decode_entities(input), expected, "{input}");
        }
    }

    #[test]
    fn leaves_what_is_not_a_reference_as_written() {
        for input in [
            "&amp",
            "&amp x",
            "&",
            "&;",
            "&#;",
            "&#",
            "&unknown;",
            "&AMP;",
            "&Amp;",
            "&#xZZ;",
            "&#-5;",
            "&#55296;",
            "&#xD800;",
            "&#1114112;",
            "&#99999999999;",
            "&#x110000;",
            "&#x 41;",
            "&#12a;",
            "&notanentityname;",
            "a & b",
            "&&",
        ] {
            assert_eq!(decode_entities(input), input, "{input}");
        }
    }

    #[test]
    fn a_second_ampersand_starts_over() {
        assert_eq!(decode_entities("&amp&amp;"), "&amp&");
        assert_eq!(decode_entities("&&amp;"), "&&");
        assert_eq!(decode_entities("&lt;&amp;lt;"), "<&lt;");
    }

    #[test]
    fn a_long_run_of_ampersands_is_linear() {
        let text = "&".repeat(1_000_000) + &"&#x41".repeat(100_000);
        let started = std::time::Instant::now();
        assert_eq!(decode_entities(&text), text);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn collapses_whitespace_and_decodes() {
        assert_eq!(
            clean_text(
                Some("  Opis   ze\n spacjami &amp; więcej&nbsp;&nbsp;tu "),
                400
            )
            .as_deref(),
            Some("Opis ze spacjami & więcej tu")
        );
    }

    #[test]
    fn nothing_left_is_nothing() {
        assert_eq!(clean_text(None, 10), None);
        assert_eq!(clean_text(Some(" \n\t"), 10), None);
        assert_eq!(clean_text(Some("&nbsp;&#32;"), 10), None);
        assert_eq!(clean_text(Some("\0\0"), 10), None);
    }

    #[test]
    fn removes_nul_but_keeps_the_rest() {
        assert_eq!(clean_text(Some("a\0b c\0"), 10).as_deref(), Some("ab c"));
    }

    #[test]
    fn cuts_with_an_ellipsis_and_never_inside_a_character() {
        let long = "a".repeat(500);
        let cut = clean_text(Some(&long), 200).unwrap();
        assert_eq!(cut.chars().count(), 201);
        assert!(cut.ends_with('…'));

        let emoji = format!("{}😀 and more", "a".repeat(199));
        assert_eq!(
            clean_text(Some(&emoji), 200).unwrap(),
            format!("{}…", "a".repeat(199))
        );
        // A text of exactly the limit is kept whole.
        assert_eq!(clean_text(Some(&"b".repeat(200)), 200).unwrap().len(), 200);
        // Counted in UTF-16 units: 100 emoji are 200 units.
        assert_eq!(
            clean_text(Some(&"😀".repeat(100)), 200)
                .unwrap()
                .chars()
                .count(),
            100
        );
        assert_eq!(
            clean_text(Some(&"😀".repeat(101)), 200)
                .unwrap()
                .chars()
                .count(),
            101
        );
    }

    #[test]
    fn the_cut_drops_the_space_before_the_ellipsis() {
        let text = format!("{} {}", "a".repeat(199), "b".repeat(50));
        assert_eq!(
            clean_text(Some(&text), 200).unwrap(),
            format!("{}…", "a".repeat(199))
        );
    }
}
