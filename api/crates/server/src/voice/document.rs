//! Reading what browsers send the hub as JSON: the object that carries a
//! signal, and the document inside it.
//!
//! - An argument is an object or `null`; the members are read by their exact
//!   names, a member of the wrong kind makes the call fail as one whose
//!   arguments do not fit, which tells the client nothing more.
//! - A document inside a signal is read to a depth of 64 containers, no more,
//!   and has no comments, no trailing commas, no byte order mark, no member
//!   named twice and nothing before or after it.

use serde::de::DeserializeOwned;
use serde_json::Value;
use stellarr_hub::HubError;

/// How deep a document may nest.
const MAX_DEPTH: usize = 64;

/// The argument of a method that takes one object: none for `null`, and a
/// call that does not fit for anything that is not an object of the right
/// shape.
pub fn argument<T: DeserializeOwned>(argument: Value) -> Result<Option<T>, HubError> {
    match argument {
        Value::Null => Ok(None),
        Value::Object(_) => serde_json::from_value(argument)
            .map(Some)
            .map_err(|_| HubError::InvalidArguments),
        _ => Err(HubError::InvalidArguments),
    }
}

/// The text of a signal's payload read as a document of type `T`: none when it
/// is not one, or nests too deep.
pub fn read<T: DeserializeOwned>(text: &str) -> Option<T> {
    if nests_deeper_than(text, MAX_DEPTH) {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// Whether more than `limit` arrays and objects stand inside one another.
/// Text that is not JSON may answer either way; the reader refuses it.
fn nests_deeper_than(text: &str, limit: usize) -> bool {
    let (mut depth, mut inside, mut escaped) = (0usize, false, false);
    for byte in text.bytes() {
        if inside {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => inside = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => inside = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Signal {
        kind: Option<String>,
        protocol_version: Option<i32>,
    }

    #[test]
    fn a_member_is_found_by_its_exact_name_and_a_repeated_one_is_refused() {
        let signal: Signal = argument(json!({"Kind": "a", "kind": "b"})).unwrap().unwrap();
        assert_eq!(signal.kind.as_deref(), Some("b"));
        assert!(read::<Signal>("{\"kind\":\"a\",\"kind\":\"b\"}").is_none());
    }

    #[test]
    fn texts_and_numbers_are_read_as_they_are_written() {
        let signal: Signal = argument(json!({"kind": null, "protocolVersion": 5})).unwrap().unwrap();
        assert_eq!(signal.kind, None);
        assert_eq!(signal.protocol_version, Some(5));
        for bad in [
            json!({"kind": 5}),
            json!({"protocolVersion": 1.5}),
            json!({"protocolVersion": "1"}),
            json!({"protocolVersion": 3000000000_i64}),
        ] {
            assert_eq!(argument::<Signal>(bad).err(), Some(HubError::InvalidArguments));
        }
    }

    #[test]
    fn only_an_object_or_nothing_is_an_argument_that_fits() {
        assert!(argument::<Signal>(Value::Null).unwrap().is_none());
        for other in [json!("x"), json!(5), json!(true), json!([]), json!([{}])] {
            assert_eq!(argument::<Signal>(other).err(), Some(HubError::InvalidArguments));
        }
    }

    #[test]
    fn a_document_nests_to_64_containers_and_no_deeper() {
        let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        assert!(read::<Value>(&nested(64)).is_some());
        assert!(read::<Value>(&nested(65)).is_none());
        assert!(read::<Value>("[\"[[[[\"]").is_some());
        assert!(read::<Value>("").is_none());
        assert!(read::<Value>("{\"a\":1,}").is_none());
        assert!(read::<Value>("{\"a\":1} x").is_none());
        assert!(read::<Value>("\u{feff}{}").is_none());
    }
}
