use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};

use crate::HubError;

pub(crate) const SUBPROTOCOL: &str = "stellarr.v1";
pub(crate) const READY: &[u8] = br#"{"type":"ready"}"#;
pub(crate) const PING: &[u8] = br#"{"type":"ping"}"#;
pub(crate) const SESSION_ENDED: u16 = 4401;
pub(crate) const GOING_AWAY: u16 = 1001;
pub(crate) const INTERNAL_ERROR: u16 = 1011;
pub(crate) const RESTART: u16 = 1012;
pub(crate) const OVERLOADED: u16 = 1013;
const MAX_CALL_ID: u64 = (1 << 53) - 1;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Incoming {
    Call {
        id: u64,
        method: String,
        args: Vec<Value>,
    },
    Send {
        method: String,
        args: Vec<Value>,
    },
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Incoming, &'static str> {
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err("expected a JSON object");
    }
    match serde_json::from_slice(bytes) {
        Ok(Incoming::Call { id, .. }) if !(1..=MAX_CALL_ID).contains(&id) => {
            Err("the call id is out of range")
        }
        Ok(message) => Ok(message),
        Err(_) => Err("the message does not match the protocol"),
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Outgoing<'a> {
    Result {
        id: u64,
        value: &'a Value,
    },
    Error {
        id: u64,
        code: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<&'a str>,
    },
    Event {
        name: &'a str,
        data: &'a RawValue,
    },
}

pub(crate) fn event(name: &str, data: &RawValue) -> Bytes {
    encode(Outgoing::Event { name, data })
}

/// Identifies a complete presence snapshot that can supersede an older one.
pub(crate) fn presence_user(name: &str, data: &RawValue) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PresenceSnapshot {
        #[serde(rename = "userId")]
        user_id: String,
        #[serde(rename = "presence")]
        _presence: String,
    }

    if name != "PresenceChanged" || !data.get().trim_start().starts_with('{') {
        return None;
    }
    serde_json::from_str::<PresenceSnapshot>(data.get())
        .ok()
        .map(|snapshot| snapshot.user_id)
}

pub(crate) fn completion(id: u64, outcome: &Result<Value, HubError>) -> Bytes {
    match outcome {
        Ok(value) => encode(Outgoing::Result { id, value }),
        Err(error) => {
            let (code, message) = match error {
                HubError::Visible(message) => ("visible", Some(message.as_str())),
                HubError::UnknownMethod => ("unknown_method", None),
                HubError::InvalidArguments => ("invalid_arguments", None),
                HubError::Internal(_) => ("internal", None),
            };
            encode(Outgoing::Error { id, code, message })
        }
    }
}

fn encode(message: Outgoing<'_>) -> Bytes {
    serde_json::to_vec(&message)
        .expect("JSON values serialise")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calls_accept_reordered_fields_unicode_and_json_numbers() {
        let call = r#"{"args":[9007199254740991,"ż"],"method":"Echo","future":true,"id":9007199254740991,"type":"call"}"#;
        let Incoming::Call { id, method, args } = parse(call.as_bytes()).unwrap() else {
            panic!("expected a call")
        };
        assert_eq!(id, MAX_CALL_ID);
        assert_eq!(method, "Echo");
        assert_eq!(args, [json!(MAX_CALL_ID), json!("ż")]);
    }

    #[test]
    fn malformed_or_excessively_nested_json_is_rejected() {
        assert!(parse(b"{\"type\":\"send\",\"method\":\"\xFF\",\"args\":[]}").is_err());
        assert!(parse(br#"["send","Echo",[]]"#).is_err());
        let nested = format!(
            r#"{{"type":"send","method":"Echo","args":[{}null{}]}}"#,
            "[".repeat(200),
            "]".repeat(200),
        );
        assert!(parse(nested.as_bytes()).is_err());
    }

    #[test]
    fn completions_include_only_an_explicitly_visible_error_message() {
        for (outcome, expected) in [
            (
                Ok(Value::Null),
                json!({"type":"result","id":7,"value":null}),
            ),
            (
                Err(HubError::visible("Please try again.")),
                json!({"type":"error","id":7,"code":"visible","message":"Please try again."}),
            ),
            (
                Err(HubError::internal("private detail")),
                json!({"type":"error","id":7,"code":"internal"}),
            ),
        ] {
            let encoded = completion(7, &outcome);
            assert_eq!(serde_json::from_slice::<Value>(&encoded).unwrap(), expected);
        }
    }
}
