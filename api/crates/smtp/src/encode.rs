//! The encodings of mail: base64, quoted-printable, RFC 2047 words and the
//! folding of header lines.

use crate::error::Error;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The longest line the recommendation allows in a header, without the line
/// break.
const FOLD_AT: usize = 78;

/// The hard limit of RFC 5322 for any line.
pub(crate) const MAX_LINE: usize = 998;

pub(crate) fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Strict base64 with padding, the form SASL exchanges use. Anything else,
/// whitespace included, is refused.
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let value = |c: u8| ALPHABET.iter().position(|&a| a == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let groups = bytes.len() / 4;
    for (index, group) in bytes.chunks(4).enumerate() {
        let last = index + 1 == groups;
        let padding = group.iter().rev().take_while(|&&c| c == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in group.iter().enumerate() {
            let v = if i >= 4 - padding { 0 } else { value(c)? };
            n = (n << 6) | v;
        }
        let triple = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&triple[..3 - padding]);
        // The unused bits of the last group must be zero.
        if padding == 1 && n & 0xff != 0 || padding == 2 && n & 0xffff != 0 {
            return None;
        }
    }
    Some(out)
}

/// Base64 in lines of 76 characters, joined by CRLF, with no break at the end.
pub(crate) fn base64_wrapped(data: &[u8]) -> Vec<u8> {
    let encoded = base64(data);
    let mut out = Vec::with_capacity(encoded.len() + encoded.len() / 76 * 2);
    for (index, line) in encoded.as_bytes().chunks(76).enumerate() {
        if index > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(line);
    }
    out
}

/// Quoted-printable of text whose line breaks are all CRLF. Lines stay within
/// 76 characters, trailing white space is encoded, and no break is added at
/// the end.
pub(crate) fn quoted_printable(input: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(input.len() + input.len() / 8);
    let mut line = 0usize;
    let mut i = 0usize;
    while i < input.len() {
        let b = input[i];
        if b == b'\r' && input.get(i + 1) == Some(&b'\n') {
            out.extend_from_slice(b"\r\n");
            line = 0;
            i += 2;
            continue;
        }
        let at_line_end =
            i + 1 == input.len() || (input[i + 1] == b'\r' && input.get(i + 2) == Some(&b'\n'));
        let literal =
            (33..=126).contains(&b) && b != b'=' || (b == b' ' || b == b'\t') && !at_line_end;
        let width = if literal { 1 } else { 3 };
        if line + width > 75 {
            out.extend_from_slice(b"=\r\n");
            line = 0;
        }
        if literal {
            out.push(b);
        } else {
            out.extend_from_slice(&[b'=', HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]]);
        }
        line += width;
        i += 1;
    }
    out
}

/// Text as RFC 2047 encoded words, UTF-8 and base64, each word on a line of
/// its own so that the header stays within the recommended line length.
pub(crate) fn encoded_words(text: &str) -> String {
    // 39 bytes become 52 characters; with the 12 of the frame a word is 64.
    const CHUNK: usize = 39;
    let mut words = Vec::new();
    let mut chunk = String::new();
    for ch in text.chars() {
        if chunk.len() + ch.len_utf8() > CHUNK {
            words.push(format!("=?utf-8?b?{}?=", base64(chunk.as_bytes())));
            chunk.clear();
        }
        chunk.push(ch);
    }
    if !chunk.is_empty() || words.is_empty() {
        words.push(format!("=?utf-8?b?{}?=", base64(chunk.as_bytes())));
    }
    words.join("\r\n ")
}

/// Refuses what would end a header line early or break the message apart.
pub(crate) fn check_header_text(what: &str, text: &str) -> Result<(), Error> {
    if text.chars().any(|c| c.is_control() && c != '\t') {
        return Err(Error::invalid(format!(
            "{what} contains a control character"
        )));
    }
    Ok(())
}

/// `Name: value` with the line break at the end. A value that holds no fold of
/// its own is folded at spaces to lines of at most 78 characters where it can
/// be; a word longer than a line stays whole. A value that came out of
/// [`encoded_words`] is folded already and is left as it is.
pub(crate) fn header(name: &str, value: &str) -> Result<String, Error> {
    let mut out = format!("{name}:");
    if value.contains("\r\n ") {
        out.push(' ');
        out.push_str(value);
    } else {
        let mut line = out.len();
        // Each word keeps the space before it, so a fold goes in front of it
        // and unfolding gives the value back exactly.
        let spaced = format!(" {value}");
        let mut start = 0;
        let mut words = Vec::new();
        for (index, ch) in spaced.char_indices() {
            if ch == ' ' && index > start {
                words.push(&spaced[start..index]);
                start = index;
            }
        }
        words.push(&spaced[start..]);
        for word in words {
            if line + word.len() > FOLD_AT && line > name.len() + 1 {
                out.push_str("\r\n");
                line = 0;
            }
            out.push_str(word);
            line += word.len();
        }
    }
    if out.split("\r\n").any(|physical| physical.len() > MAX_LINE) {
        return Err(Error::invalid(format!(
            "the {name} header has a line longer than {MAX_LINE} bytes"
        )));
    }
    out.push_str("\r\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).as_deref(), Some(plain.as_bytes()));
        }
    }

    #[test]
    fn base64_decode_refuses_what_is_not_canonical() {
        for bad in [
            "Zg=",
            "Zg",
            "Z===",
            "Zm9v Zg==",
            "Zm9=v",
            "Zh==",
            "Zm9vYg==Zm9v",
            "Zm9*",
        ] {
            assert_eq!(base64_decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn wrapped_base64_has_lines_of_76_and_no_trailing_break() {
        let data = vec![0xAB; 100];
        let wrapped = String::from_utf8(base64_wrapped(&data)).unwrap();
        let lines: Vec<&str> = wrapped.split("\r\n").collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 76);
        assert!(!wrapped.ends_with('\n'));
        assert_eq!(base64_decode(&lines.concat()).unwrap(), data);
    }

    fn unqp(encoded: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < encoded.len() {
            if encoded[i] == b'=' {
                if encoded[i + 1..].starts_with(b"\r\n") {
                    i += 3;
                    continue;
                }
                let hex = std::str::from_utf8(&encoded[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(encoded[i]);
                i += 1;
            }
        }
        out
    }

    #[test]
    fn quoted_printable_round_trips_and_keeps_lines_short() {
        let text = "Zażółć gęślą jaźń = 100%\r\ntrailing space \r\n\ttab at end\t\r\nx".repeat(30);
        let encoded = quoted_printable(text.as_bytes());
        assert!(
            encoded.split(|&b| b == b'\n').all(|l| l.len() <= 77),
            "a line is too long"
        );
        assert_eq!(unqp(&encoded), text.as_bytes());
        let shown = String::from_utf8(encoded).unwrap();
        assert!(
            !shown.contains(" \r\n"),
            "white space before a break must be encoded"
        );
        assert!(!shown.contains("\t\r\n"));
    }

    #[test]
    fn quoted_printable_encodes_a_lone_return_and_control_bytes() {
        assert_eq!(quoted_printable(b"a\rb\x00c"), b"a=0Db=00c");
    }

    #[test]
    fn quoted_printable_of_a_very_long_line_breaks_it_softly() {
        let encoded = quoted_printable("x".repeat(5000).as_bytes());
        assert!(encoded.split(|&b| b == b'\n').all(|l| l.len() <= 77));
        assert_eq!(unqp(&encoded), "x".repeat(5000).as_bytes());
    }

    #[test]
    fn encoded_words_split_on_characters_and_stay_short() {
        let text = "Zażółć gęślą jaźń, zażółć gęślą jaźń, zażółć gęślą jaźń";
        let coded = encoded_words(text);
        let mut decoded = Vec::new();
        for word in coded.split("\r\n ") {
            assert!(word.len() <= 75, "{word}");
            let inner = word
                .strip_prefix("=?utf-8?b?")
                .unwrap()
                .strip_suffix("?=")
                .unwrap();
            decoded.extend(base64_decode(inner).unwrap());
        }
        assert_eq!(String::from_utf8(decoded).unwrap(), text);
    }

    #[test]
    fn a_header_folds_at_spaces_and_unfolds_to_the_same_text() {
        let value = "word ".repeat(40);
        let value = value.trim_end();
        let line = header("Subject", value).unwrap();
        assert!(line.split("\r\n").all(|l| l.len() <= 78));
        assert!(line.ends_with("\r\n"));
        let unfolded = line.trim_end().replace("\r\n", "");
        assert_eq!(unfolded, format!("Subject: {value}"));
    }

    #[test]
    fn a_header_with_one_long_word_stays_on_one_line() {
        let word = "a".repeat(500);
        let line = header("Subject", &word).unwrap();
        assert_eq!(line, format!("Subject: {word}\r\n"));
        assert!(header("Subject", &"a".repeat(1000)).is_err());
    }

    #[test]
    fn control_characters_are_refused_in_header_text() {
        assert!(check_header_text("x", "a\r\nBcc: z").is_err());
        assert!(check_header_text("x", "a\0b").is_err());
        assert!(check_header_text("x", "a\tb").is_ok());
    }
}
