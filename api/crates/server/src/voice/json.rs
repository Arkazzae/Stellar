//! JSON text written by hand where its exact shape matters: the bodies sent to
//! the media server, which the first server wrote with the escaping of its
//! default encoder, and a hub event whose member is a document's own text.
//!
//! Strings carry everything that is not printable ASCII, and the characters a
//! page could mistake for markup (`"&'+<>` and the backtick), as `\uXXXX`
//! with capital hex digits.

use std::fmt::Write;

/// A JSON string with its quotes.
pub fn text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    write_text(&mut out, value);
    out
}

fn write_text(out: &mut String, value: &str) {
    out.push('"');
    for unit in value.encode_utf16() {
        match unit {
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0c => out.push_str("\\f"),
            0x0d => out.push_str("\\r"),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e if !matches!(unit, 0x22 | 0x26 | 0x27 | 0x2b | 0x3c | 0x3e | 0x60) => {
                out.push(char::from(u8::try_from(unit).unwrap_or(b'?')));
            }
            _ => {
                let _ = write!(out, "\\u{unit:04X}");
            }
        }
    }
    out.push('"');
}

/// A document written again without the whitespace between its tokens and
/// with its strings escaped as [`text`] does. Numbers, names, the order of
/// members and repeated members stay as they were. `document` must be valid
/// JSON, which the caller has checked.
pub fn compact(document: &str) -> String {
    let mut out = String::with_capacity(document.len());
    let mut characters = document.chars();
    while let Some(character) = characters.next() {
        match character {
            ' ' | '\t' | '\r' | '\n' => {}
            '"' => {
                let mut decoded = String::new();
                read_string(&mut characters, &mut decoded);
                write_text(&mut out, &decoded);
            }
            other => out.push(other),
        }
    }
    out
}

/// Reads the rest of a string, after its opening quote, with its escapes
/// resolved.
fn read_string(characters: &mut std::str::Chars<'_>, into: &mut String) {
    let mut high: Option<u16> = None;
    while let Some(character) = characters.next() {
        let unit = match character {
            '"' => break,
            '\\' => match characters.next() {
                Some('n') => 0x0a,
                Some('t') => 0x09,
                Some('r') => 0x0d,
                Some('b') => 0x08,
                Some('f') => 0x0c,
                Some('u') => {
                    let digits: String = characters.by_ref().take(4).collect();
                    u16::from_str_radix(&digits, 16).unwrap_or(0xfffd)
                }
                Some(other) => u16::try_from(u32::from(other)).unwrap_or(0xfffd),
                None => break,
            },
            other => {
                let mut buffer = [0u16; 2];
                let units = other.encode_utf16(&mut buffer);
                if units.len() == 2 {
                    into.push(other);
                    continue;
                }
                units[0]
            }
        };
        match (high.take(), unit) {
            (Some(first), 0xdc00..=0xdfff) => {
                let pair = [first, unit];
                into.extend(char::decode_utf16(pair).map(|c| c.unwrap_or('\u{fffd}')));
            }
            (Some(_), _) => {
                into.push('\u{fffd}');
                push_unit(into, &mut high, unit);
            }
            (None, _) => push_unit(into, &mut high, unit),
        }
    }
    if high.is_some() {
        into.push('\u{fffd}');
    }
}

fn push_unit(into: &mut String, high: &mut Option<u16>, unit: u16) {
    match unit {
        0xd800..=0xdbff => *high = Some(unit),
        0xdc00..=0xdfff => into.push('\u{fffd}'),
        _ => into.push(char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}')),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_escaped_as_the_first_servers_writer_escapes_them() {
        assert_eq!(text("a=ice-pwd:Zk+x/y"), "\"a=ice-pwd:Zk\\u002Bx/y\"");
        assert_eq!(text("<&>'`\""), "\"\\u003C\\u0026\\u003E\\u0027\\u0060\\u0022\"");
        assert_eq!(text("v=0\r\ns=-\n"), "\"v=0\\r\\ns=-\\n\"");
        assert_eq!(text("żółw 😀"), "\"\\u017C\\u00F3\\u0142w \\uD83D\\uDE00\"");
        assert_eq!(text("a\\b\u{1}"), "\"a\\\\b\\u0001\"");
    }

    #[test]
    fn a_document_loses_its_whitespace_and_keeps_everything_else() {
        let source = "{ \"candidate\" : \"a+b\" ,\n\t\"n\": 1.50e2, \"list\": [ true , null, \"\\u00e9\\n\" ], \"n\": 2 }";

        assert_eq!(
            compact(source),
            "{\"candidate\":\"a\\u002Bb\",\"n\":1.50e2,\"list\":[true,null,\"\\u00E9\\n\"],\"n\":2}"
        );
    }

    #[test]
    fn escapes_that_form_a_pair_come_out_as_one_character() {
        assert_eq!(compact("\"\\ud83d\\ude00\""), "\"\\uD83D\\uDE00\"");
        assert_eq!(compact("\"\u{1f600}\""), "\"\\uD83D\\uDE00\"");
        assert_eq!(compact("\"\\/\\\"\\\\\""), "\"/\\u0022\\\\\"");
    }

    #[test]
    fn a_lone_surrogate_becomes_the_replacement_character() {
        assert_eq!(compact("\"\\ud800x\""), "\"\\uFFFDx\"");
        assert_eq!(compact("\"\\udc00\""), "\"\\uFFFD\"");
    }
}
