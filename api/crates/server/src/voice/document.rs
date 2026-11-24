//! Reading what browsers send the hub as JSON: the object that carries a
//! signal, and the document inside it, which the hub reads as the first
//! server's JSON reader did.
//!
//! - A member of the object is found without regard to letter case, and the
//!   last of two of the same name counts.
//! - A member that is missing or `null` is nothing, for a text. A whole number
//!   that is missing reads as 0 and one that is `null` is no number.
//! - A member of the wrong kind makes the call fail as one whose arguments do
//!   not fit, which tells the client nothing more.
//! - A document is read to a depth of 64 containers, no more, and has no
//!   comments, no trailing commas and nothing before or after it.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;
use stellarr_hub::HubError;

/// How deep a document may nest.
const MAX_DEPTH: usize = 64;

/// The members of a JSON object, in the order they were written, repeated
/// names included.
#[derive(Debug, Default)]
pub struct Members(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Members, D::Error> {
        struct Reader;
        impl<'de> Visitor<'de> for Reader {
            type Value = Members;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some((name, value)) = map.next_entry::<String, Value>()? {
                    members.push((name, value));
                }
                Ok(Members(members))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Members, E> {
                Err(E::custom("not an object"))
            }
        }
        deserializer.deserialize_map(Reader)
    }
}

impl Members {
    /// The last member of this name, the name compared exactly.
    pub fn exact(&self, name: &str) -> Option<&Value> {
        self.0
            .iter()
            .rev()
            .find(|(found, _)| found == name)
            .map(|(_, value)| value)
    }

    /// How many members carry this name, compared exactly.
    pub fn count(&self, name: &str) -> usize {
        self.0.iter().filter(|(found, _)| found == name).count()
    }

    /// The last member whose name is this one in any letter case.
    fn loose(&self, name: &str) -> Option<&Value> {
        self.0
            .iter()
            .rev()
            .find(|(found, _)| found.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }

    /// A text member: nothing when it is missing or null.
    pub fn text(&self, name: &str) -> Result<Option<String>, HubError> {
        match self.loose(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(HubError::InvalidArguments),
        }
    }

    /// A 32-bit whole number member: 0 when it is missing.
    pub fn number(&self, name: &str) -> Result<i32, HubError> {
        match self.loose(name) {
            None => Ok(0),
            Some(Value::Number(number)) => number
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or(HubError::InvalidArguments),
            Some(_) => Err(HubError::InvalidArguments),
        }
    }
}

/// The argument of a method that takes one object: none for `null`, and a
/// call that does not fit for anything that is not an object.
pub fn object(argument: Value) -> Result<Option<Members>, HubError> {
    match argument {
        Value::Null => Ok(None),
        Value::Object(_) => Members::deserialize(argument)
            .map(Some)
            .map_err(|_| HubError::InvalidArguments),
        _ => Err(HubError::InvalidArguments),
    }
}

/// The text of a signal's payload read as a JSON document: any value, which
/// is none when the text is not one.
pub fn parse(text: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(text).ok()?;
    (depth(&value) <= MAX_DEPTH).then_some(value)
}

/// The text of a signal's payload read as a JSON object, keeping the repeated
/// names.
pub fn parse_members(text: &str) -> Option<ParsedPayload> {
    let value = parse(text)?;
    Some(match value {
        Value::Object(_) => ParsedPayload::Object(serde_json::from_str(text).ok()?),
        Value::Null => ParsedPayload::Null,
        _ => ParsedPayload::Other,
    })
}

/// What a payload document is.
#[derive(Debug)]
pub enum ParsedPayload {
    Object(Members),
    Null,
    Other,
}

fn depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Value::Object(members) => 1 + members.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_member_is_found_in_any_letter_case_and_the_last_of_a_name_counts() {
        let members = object(json!({"Kind": "a"})).unwrap().unwrap();
        assert_eq!(members.text("kind").unwrap(), Some("a".to_string()));

        let repeated: Members = serde_json::from_str("{\"muted\":true,\"muted\":false}").unwrap();
        assert_eq!(repeated.exact("muted"), Some(&Value::Bool(false)));
        assert_eq!(repeated.count("muted"), 2);
        assert_eq!(repeated.exact("Muted"), None);
    }

    #[test]
    fn texts_and_numbers_are_read_as_the_first_server_read_them() {
        let members = object(json!({"a": null, "b": 5, "c": "x", "d": 1.5, "e": "1", "f": 3000000000_i64}))
            .unwrap()
            .unwrap();

        assert_eq!(members.text("a").unwrap(), None);
        assert_eq!(members.text("missing").unwrap(), None);
        assert_eq!(members.text("c").unwrap(), Some("x".to_string()));
        assert_eq!(members.text("b"), Err(HubError::InvalidArguments));
        assert_eq!(members.number("missing").unwrap(), 0);
        assert_eq!(members.number("b").unwrap(), 5);
        for name in ["a", "c", "d", "e", "f"] {
            assert_eq!(members.number(name), Err(HubError::InvalidArguments), "{name}");
        }
    }

    #[test]
    fn only_an_object_or_nothing_is_an_argument_that_fits() {
        assert!(object(Value::Null).unwrap().is_none());
        for other in [json!("x"), json!(5), json!(true), json!([]), json!([{}])] {
            assert_eq!(object(other).unwrap_err(), HubError::InvalidArguments);
        }
    }

    #[test]
    fn a_document_nests_to_64_containers_and_no_deeper() {
        let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        assert!(parse(&nested(64)).is_some());
        assert!(parse(&nested(65)).is_none());
        assert!(parse("").is_none());
        assert!(parse("{\"a\":1,}").is_none());
        assert!(parse("{\"a\":1} x").is_none());
        assert!(parse("\u{feff}{}").is_none());
        assert!(matches!(parse_members("{}"), Some(ParsedPayload::Object(_))));
        assert!(matches!(parse_members("null"), Some(ParsedPayload::Null)));
        assert!(matches!(parse_members("[]"), Some(ParsedPayload::Other)));
        assert!(parse_members("{").is_none());
    }
}
