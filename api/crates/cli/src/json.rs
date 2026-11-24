//! JSON as the first server's tool wrote it: members in the order they were
//! given, indented by two spaces, and every character outside printable ASCII
//! and `"&'+<>` and the backtick written as `\uXXXX`. Scripts parse it, so the
//! shape and the order are the contract.

use stellarr_api::values::Timestamp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn object<const N: usize>(members: [(&str, Json); N]) -> Json {
        Json::Object(
            members
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        )
    }

    pub fn from_members(members: Vec<(String, Json)>) -> Json {
        Json::Object(members)
    }

    pub fn text(text: impl Into<String>) -> Json {
        Json::Text(text.into())
    }

    pub fn optional_text(text: Option<impl Into<String>>) -> Json {
        text.map_or(Json::Null, |text| Json::Text(text.into()))
    }

    pub fn optional_bool(value: Option<bool>) -> Json {
        value.map_or(Json::Null, Json::Bool)
    }

    pub fn array(items: impl IntoIterator<Item = Json>) -> Json {
        Json::Array(items.into_iter().collect())
    }

    /// The document, indented, without a final line break.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Int(number) => out.push_str(&number.to_string()),
            Json::Text(text) => write_string(out, text),
            Json::Array(items) if items.is_empty() => out.push_str("[]"),
            Json::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    out.push_str(if index == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Json::Object(members) if members.is_empty() => out.push_str("{}"),
            Json::Object(members) => {
                out.push('{');
                for (index, (name, value)) in members.iter().enumerate() {
                    out.push_str(if index == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    write_string(out, name);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            ' '..='~' if !matches!(c, '"' | '&' | '\'' | '+' | '<' | '>' | '`') => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04X}"));
                }
            }
        }
    }
    out.push('"');
}

impl From<bool> for Json {
    fn from(value: bool) -> Json {
        Json::Bool(value)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Json {
        Json::Int(value)
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Json {
        Json::Int(value as i64)
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Json {
        Json::Text(value.to_string())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Json {
        Json::Text(value)
    }
}

/// The moment as JSON: no fraction when it is whole seconds, otherwise the
/// fraction without trailing zeros, always with its `Z`.
pub fn moment(at: Timestamp) -> Json {
    Json::Text(at.to_wire_text())
}

/// The moment in a line of text, as `{moment:O}` writes it: seven digits of
/// fraction always, and the `Z`.
pub fn round_trip(at: Timestamp) -> String {
    let text = at.to_wire_text();
    let base = text.trim_end_matches('Z');
    let (whole, fraction) = base.split_once('.').unwrap_or((base, ""));
    format!("{whole}.{fraction:0<7}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_indented_and_escaped_as_the_first_tool_wrote_it() {
        let document = Json::object([
            ("ok", true.into()),
            ("text", "a \"b\" <c> & 'd' + `e` é 😀 \\ \n".into()),
            ("empty", Json::Array(vec![])),
            ("none", Json::Null),
            (
                "nested",
                Json::object([
                    ("n", 3i64.into()),
                    ("list", Json::array([1i64.into(), "x".into()])),
                ]),
            ),
        ]);

        assert_eq!(
            document.pretty(),
            "{\n  \"ok\": true,\n  \"text\": \"a \\u0022b\\u0022 \\u003Cc\\u003E \\u0026 \\u0027d\\u0027 \\u002B \\u0060e\\u0060 \\u00E9 \\uD83D\\uDE00 \\\\ \\n\",\n  \"empty\": [],\n  \"none\": null,\n  \"nested\": {\n    \"n\": 3,\n    \"list\": [\n      1,\n      \"x\"\n    ]\n  }\n}"
        );
    }

    #[test]
    fn a_moment_has_seven_digits_in_a_line_and_as_few_as_it_needs_in_json() {
        let at = Timestamp::parse("2026-11-24 10:40:02.2787074").unwrap();
        let whole = Timestamp::parse("2026-11-24 10:40:02").unwrap();
        let tenth = Timestamp::parse("2026-11-24 10:40:02.5").unwrap();

        assert_eq!(round_trip(at), "2026-11-24T10:40:02.2787074Z");
        assert_eq!(round_trip(whole), "2026-11-24T10:40:02.0000000Z");
        assert_eq!(round_trip(tenth), "2026-11-24T10:40:02.5000000Z");
        assert_eq!(moment(whole), Json::text("2026-11-24T10:40:02Z"));
        assert_eq!(moment(tenth), Json::text("2026-11-24T10:40:02.5Z"));
    }
}
