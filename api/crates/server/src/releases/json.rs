//! A reader for the JSON an operator writes into a manifest.
//!
//! It is strict where the first server's reader was strict, because a manifest
//! that one reader accepts and the other refuses would turn a working release
//! into a broken one on the day the servers are swapped: no comments, no
//! trailing commas, nesting no deeper than 64, and numbers kept as the text
//! they were written in, so that `1.0` is told from `1`. The members of an
//! object keep their order, since a name written twice is decided by the last
//! one and a name matches without regard to letter case.

/// The deepest an object or an array may sit inside others.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as written.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// The whole text as one value, or None when it is not JSON.
pub fn parse(text: &str) -> Option<Value> {
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
    };
    reader.skip_space();
    let value = reader.value(0)?;
    reader.skip_space();
    (reader.at == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, word: &[u8]) -> Option<()> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Some(())
        } else {
            None
        }
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        match self.peek()? {
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => self.string().map(Value::String),
            b't' => self.expect(b"true").map(|()| Value::Bool(true)),
            b'f' => self.expect(b"false").map(|()| Value::Bool(false)),
            b'n' => self.expect(b"null").map(|()| Value::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn object(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.at += 1;
        let mut members = Vec::new();
        self.skip_space();
        if self.peek()? == b'}' {
            self.at += 1;
            return Some(Value::Object(members));
        }
        loop {
            self.skip_space();
            if self.peek()? != b'"' {
                return None;
            }
            let name = self.string()?;
            self.skip_space();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            self.skip_space();
            let value = self.value(depth)?;
            members.push((name, value));
            self.skip_space();
            match self.peek()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Some(Value::Object(members));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek()? == b']' {
            self.at += 1;
            return Some(Value::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value(depth)?);
            self.skip_space();
            match self.peek()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(Value::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
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
        // The slice is made of ASCII only.
        Some(Value::Number(
            String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned(),
        ))
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.bytes.get(self.at..self.at + 4)?;
        let mut value = 0;
        for digit in digits {
            value = value * 16 + char::from(*digit).to_digit(16)?;
        }
        self.at += 4;
        Some(value)
    }

    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            // Copy the run that needs no care in one piece; it ends at a
            // quote, a backslash or a control character, all of which are
            // ASCII, so the run is whole characters.
            let start = self.at;
            while self
                .peek()
                .is_some_and(|b| b != b'"' && b != b'\\' && b >= 0x20)
            {
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).ok()?);
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first) {
                                self.expect(b"\\u")?;
                                let second = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return None;
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else {
                                first
                            };
                            // A lone low surrogate is no character.
                            out.push(char::from_u32(code)?);
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Value {
        Value::String(value.to_string())
    }

    #[test]
    fn every_kind_of_value_is_read() {
        let read = parse(
            " {\"a\": [1, -2.5e3, true, false, null, \"x\"], \"b\": {}, \"a\": []}\r\n",
        )
        .unwrap();

        assert_eq!(
            read,
            Value::Object(vec![
                (
                    "a".into(),
                    Value::Array(vec![
                        Value::Number("1".into()),
                        Value::Number("-2.5e3".into()),
                        Value::Bool(true),
                        Value::Bool(false),
                        Value::Null,
                        text("x"),
                    ])
                ),
                ("b".into(), Value::Object(vec![])),
                ("a".into(), Value::Array(vec![])),
            ])
        );
    }

    #[test]
    fn strings_have_their_escapes_read() {
        assert_eq!(parse(r#""a\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00z""#), Some(text("a\"\\/\u{8}\u{c}\n\r\té\u{1F600}z")));
        assert_eq!(parse("\"żółć\""), Some(text("żółć")));
    }

    #[test]
    fn what_json_does_not_allow_is_refused() {
        for bad in [
            "",
            "  ",
            "{",
            "}",
            "{\"a\":1,}",
            "[1,]",
            "[,1]",
            "{a:1}",
            "{'a':1}",
            "{\"a\" 1}",
            "{\"a\":1 \"b\":2}",
            "{/*c*/}",
            "// c\n{}",
            "01",
            "+1",
            "1.",
            ".5",
            "1e",
            "-",
            "tru",
            "nul",
            "NaN",
            "\"a",
            "\"a\nb\"",
            "\"\\x\"",
            "\"\\u12\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "\"\\ud800\\u0041\"",
            "{} {}",
            "{}x",
            "\u{feff}{}",
            "\u{a0}{}",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn nesting_is_allowed_down_to_sixty_four_levels() {
        let nested = |levels: usize| format!("{}{}", "[".repeat(levels), "]".repeat(levels));

        assert!(parse(&nested(64)).is_some());
        assert!(parse(&nested(65)).is_none());
        assert!(parse(&format!("{{\"a\":{}}}", nested(63))).is_some());
        assert!(parse(&format!("{{\"a\":{}}}", nested(64))).is_none());
        // Far deeper than the stack could follow, and still just refused.
        assert!(parse(&nested(100_000)).is_none());
    }
}
