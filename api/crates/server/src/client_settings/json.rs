//! Reading a section as JSON and writing it back the way the first server
//! did, so that the text kept (and measured against the limit) is the same.
//!
//! The text is parsed as strict JSON, with at most 64 levels of nesting, and
//! written again compactly: members stay in the order they came in, repeated
//! names stay, and a number keeps the digits it was written with. A string is
//! written with every character that is not printable ASCII, and the
//! characters `"&'+<>` and the backtick, as `\uXXXX` with capital digits, so
//! that a character outside ASCII counts for six.

use crate::permissions::audit::string as quoted;

/// How deep arrays and objects may lie in one another.
const MAX_DEPTH: usize = 64;

/// The text of a body, read as a reader of text reads it: a byte order mark
/// names the encoding, and what is not valid in it becomes U+FFFD.
pub fn decode(bytes: &[u8]) -> String {
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, 0, 0, rest @ ..] => wide(rest, 4, u32::from_le_bytes),
        [0, 0, 0xFE, 0xFF, rest @ ..] => wide(rest, 4, u32::from_be_bytes),
        [0xFF, 0xFE, rest @ ..] => narrow(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => narrow(rest, u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn narrow(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = bytes.chunks_exact(2).map(|pair| unit([pair[0], pair[1]]));
    let mut text: String = char::decode_utf16(units)
        .map(|part| part.unwrap_or('\u{FFFD}'))
        .collect();
    if !bytes.len().is_multiple_of(2) {
        text.push('\u{FFFD}');
    }
    text
}

fn wide(bytes: &[u8], width: usize, scalar: fn([u8; 4]) -> u32) -> String {
    let mut text: String = bytes
        .chunks_exact(width)
        .map(|quad| char::from_u32(scalar([quad[0], quad[1], quad[2], quad[3]])).unwrap_or('\u{FFFD}'))
        .collect();
    if !bytes.len().is_multiple_of(width) {
        text.push('\u{FFFD}');
    }
    text
}

/// The text as one compact JSON object, or none when it is not JSON, not an
/// object, or nested too deeply.
pub fn object(text: &str) -> Option<String> {
    let mut reader = Reader { text: text.as_bytes(), at: 0 };
    reader.blank();
    if reader.peek()? != b'{' {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    reader.value(&mut out, 0)?;
    reader.blank();
    (reader.at == reader.text.len()).then_some(out)
}

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn blank(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn word(&mut self, word: &str, out: &mut String) -> Option<()> {
        if self.text[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            out.push_str(word);
            Some(())
        } else {
            None
        }
    }

    fn value(&mut self, out: &mut String, depth: usize) -> Option<()> {
        match self.peek()? {
            b'{' => self.object(out, depth + 1),
            b'[' => self.array(out, depth + 1),
            b'"' => {
                let text = self.string()?;
                out.push_str(&quoted(&text));
                Some(())
            }
            b't' => self.word("true", out),
            b'f' => self.word("false", out),
            b'n' => self.word("null", out),
            b'-' | b'0'..=b'9' => self.number(out),
            _ => None,
        }
    }

    fn object(&mut self, out: &mut String, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.at += 1;
        out.push('{');
        self.blank();
        if self.peek()? == b'}' {
            self.at += 1;
            out.push('}');
            return Some(());
        }
        loop {
            self.blank();
            if self.peek()? != b'"' {
                return None;
            }
            let name = self.string()?;
            out.push_str(&quoted(&name));
            self.blank();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            out.push(':');
            self.blank();
            self.value(out, depth)?;
            self.blank();
            match self.peek()? {
                b',' => {
                    self.at += 1;
                    out.push(',');
                }
                b'}' => {
                    self.at += 1;
                    out.push('}');
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self, out: &mut String, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.at += 1;
        out.push('[');
        self.blank();
        if self.peek()? == b']' {
            self.at += 1;
            out.push(']');
            return Some(());
        }
        loop {
            self.blank();
            self.value(out, depth)?;
            self.blank();
            match self.peek()? {
                b',' => {
                    self.at += 1;
                    out.push(',');
                }
                b']' => {
                    self.at += 1;
                    out.push(']');
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    /// A number, with the digits it was written with.
    fn number(&mut self, out: &mut String) -> Option<()> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => self.digits(),
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !self.peek()?.is_ascii_digit() {
                return None;
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !self.peek()?.is_ascii_digit() {
                return None;
            }
            self.digits();
        }
        // Whatever follows has to be a delimiter, which the caller checks.
        out.push_str(std::str::from_utf8(&self.text[start..self.at]).ok()?);
        Some(())
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
    }

    /// The characters of a string, with its escapes read.
    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut units: Vec<u16> = Vec::new();
        let mut text = String::new();
        loop {
            let byte = self.peek()?;
            match byte {
                b'"' => {
                    self.at += 1;
                    break;
                }
                0..=0x1F => return None,
                b'\\' => {
                    self.at += 1;
                    let escaped = self.peek()?;
                    self.at += 1;
                    let unit = match escaped {
                        b'"' => u16::from(b'"'),
                        b'\\' => u16::from(b'\\'),
                        b'/' => u16::from(b'/'),
                        b'b' => 0x08,
                        b'f' => 0x0C,
                        b'n' => 0x0A,
                        b'r' => 0x0D,
                        b't' => 0x09,
                        b'u' => {
                            let digits = self.text.get(self.at..self.at + 4)?;
                            let digits = std::str::from_utf8(digits).ok()?;
                            if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                                return None;
                            }
                            self.at += 4;
                            u16::from_str_radix(digits, 16).ok()?
                        }
                        _ => return None,
                    };
                    units.push(unit);
                }
                _ => {
                    // A run of plain characters, up to the next one that
                    // needs a look.
                    flush(&mut units, &mut text)?;
                    let start = self.at;
                    while self
                        .peek()
                        .is_some_and(|byte| byte != b'"' && byte != b'\\' && byte > 0x1F)
                    {
                        self.at += 1;
                    }
                    text.push_str(std::str::from_utf8(&self.text[start..self.at]).ok()?);
                }
            }
        }
        flush(&mut units, &mut text)?;
        Some(text)
    }
}

/// Moves the units of escapes into the text; a surrogate that has no partner
/// is no text.
fn flush(units: &mut Vec<u16>, text: &mut String) -> Option<()> {
    for part in char::decode_utf16(units.drain(..)) {
        text.push(part.ok()?);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(text: &str) -> Option<String> {
        object(text)
    }

    #[test]
    fn an_object_is_written_compactly_in_the_order_it_came() {
        assert_eq!(
            kept("{ \"b\" : 1 ,\n\"a\":[ true , null , \"x\" ], \"b\": {} }").as_deref(),
            Some("{\"b\":1,\"a\":[true,null,\"x\"],\"b\":{}}")
        );
        assert_eq!(kept(" { } ").as_deref(), Some("{}"));
    }

    #[test]
    fn a_number_keeps_the_digits_it_was_written_with() {
        assert_eq!(
            kept("{\"a\":[1.0,-0,1E5,2e+3,0.10,1e999,123456789012345678901234567890]}").as_deref(),
            Some("{\"a\":[1.0,-0,1E5,2e+3,0.10,1e999,123456789012345678901234567890]}")
        );
    }

    #[test]
    fn what_is_not_printable_ascii_is_written_as_an_escape() {
        assert_eq!(
            kept("{\"\u{e9}\":\"<a href='x'>&\u{1f680}\\n\\/\u{7f}\"}").as_deref(),
            Some("{\"\\u00E9\":\"\\u003Ca href=\\u0027x\\u0027\\u003E\\u0026\\uD83D\\uDE80\\n/\\u007F\"}")
        );
        assert_eq!(kept("{\"a\":\"\\u0041\\ud83d\\ude80\"}").as_deref(), Some("{\"a\":\"A\\uD83D\\uDE80\"}"));
    }

    #[test]
    fn only_an_object_is_taken_and_only_whole() {
        for bad in [
            "", " ", "[1]", "\"x\"", "1", "null", "{", "{\"a\"}", "{\"a\":}", "{\"a\":1,}", "{,}", "{'a':1}",
            "{\"a\":01}", "{\"a\":+1}", "{\"a\":.5}", "{\"a\":1.}", "{\"a\":1e}", "{\"a\":-}", "{\"a\":tru}",
            "{\"a\":NaN}", "{} {}", "{}x", "{\"a\":\"\u{1}\"}", "{\"a\":\"\\x\"}", "{\"a\":\"\\u12\"}",
            "{\"a\":\"\\ud800\"}", "{\"a\":\"\\udc00\"}", "{ /* c */ }", "\u{feff}{}", "{\"a\":1}\u{a0}",
            "{\"a\":[1,]}", "{\"a\":[,1]}",
        ] {
            assert_eq!(kept(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn nesting_may_be_sixty_four_deep_and_no_deeper() {
        let nested = |levels: usize| format!("{}{}{}", "{\"a\":".repeat(levels), "1", "}".repeat(levels));
        let arrays = |levels: usize| format!("{{\"a\":{}{}}}", "[".repeat(levels - 1), "]".repeat(levels - 1));
        assert!(kept(&nested(64)).is_some());
        assert!(kept(&nested(65)).is_none());
        assert!(kept(&arrays(64)).is_some());
        assert!(kept(&arrays(65)).is_none());
    }

    #[test]
    fn a_body_is_read_as_a_reader_of_text_reads_it() {
        assert_eq!(decode(b"\xEF\xBB\xBF{}"), "{}");
        assert_eq!(decode(b"\xFF\xFE{\0}\0"), "{}");
        assert_eq!(decode(b"\xFE\xFF\0{\0}"), "{}");
        assert_eq!(decode(b"\xFF\xFE\0\0{\0\0\0}\0\0\0"), "{}");
        assert_eq!(decode(b"\0\0\xFE\xFF\0\0\0{\0\0\0}"), "{}");
        assert_eq!(decode(b"{\"a\":\"\xFF\"}"), "{\"a\":\"\u{fffd}\"}");
    }
}
