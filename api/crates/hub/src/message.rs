//! The messages of the hub protocol in its JSON form, as this application
//! uses it, and the negotiation that comes before them.
//!
//! The client invokes methods, with or without an id to answer to, and may
//! close or ping. The server answers invocations with completions, invokes
//! methods on the client without an id, pings and closes. Streaming (item,
//! cancel), acknowledgements and sequence numbers belong to features this
//! server never turns on and the client never asks for; the server skips
//! them when they arrive, and answers a streaming invocation with an error.

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;

use crate::record::frame_record;

/// What the handshake answers when the request cannot be read.
pub const HANDSHAKE_FAILED: &str = "An unexpected error occurred during connection handshake.";
/// What a handshake that never arrived is answered with.
pub const HANDSHAKE_CANCELED: &str = "Handshake was canceled.";
/// What a close message says when the connection ended on an error whose
/// details the client may not see.
pub const CLOSED_WITH_ERROR: &str = "Connection closed with an error.";

const TYPE_INVOCATION: i64 = 1;
const TYPE_STREAM_INVOCATION: i64 = 4;
const TYPE_CLOSE: i64 = 7;

/// The version of the negotiation answer this server speaks, and the highest
/// it will agree to.
const NEGOTIATE_VERSION: i64 = 1;

/// The client's first record: which protocol it wants to speak. The version
/// must be a whole number but any is accepted, as the protocol has only one.
pub fn parse_handshake(record: &[u8]) -> Result<(), String> {
    let Ok(Value::Object(request)) = serde_json::from_slice::<Value>(record) else {
        return Err(HANDSHAKE_FAILED.into());
    };
    let (Some(Value::String(protocol)), Some(version)) =
        (request.get("protocol"), request.get("version"))
    else {
        return Err(HANDSHAKE_FAILED.into());
    };
    if version.as_i64().is_none() {
        return Err(HANDSHAKE_FAILED.into());
    }
    if protocol.eq_ignore_ascii_case("json") {
        Ok(())
    } else {
        Err(format!("The protocol '{protocol}' is not supported."))
    }
}

#[derive(Serialize)]
struct Failure<'a> {
    error: &'a str,
}

/// The answer to the handshake: `{}` when it is accepted, else the error.
pub fn handshake_response(error: Option<&str>) -> Bytes {
    match error {
        None => frame_record(b"{}"),
        Some(text) => {
            frame_record(&serde_json::to_vec(&Failure { error: text }).expect("strings serialise"))
        }
    }
}

/// An invocation from the client.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    /// The id the completion must carry; none when the client does not wait.
    pub id: Option<String>,
    pub target: String,
    pub arguments: Vec<Value>,
    /// How many upload streams the client announced.
    pub stream_ids: usize,
    /// True for a streaming invocation, which this server cannot serve.
    pub streaming: bool,
}

/// What the client sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Invocation(Invocation),
    /// The client closes the connection.
    Close,
    /// A message of a kind the server has no use for: ping, stream item,
    /// completion, cancel, acknowledgement, sequence, or a type it has never
    /// heard of.
    Ignored,
}

/// A record that cannot be read as a message; the connection is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed(pub &'static str);

/// Reads one record sent by the client.
///
/// Messages the server ignores are still read for their shape, because a
/// client that sends one with a part missing or of the wrong kind is broken,
/// and is told so by closing the connection.
pub fn parse_message(record: &[u8]) -> Result<Incoming, Malformed> {
    let Ok(value) = serde_json::from_slice::<Value>(record) else {
        return Err(Malformed("the record is not JSON"));
    };
    let Value::Object(mut message) = value else {
        return Err(Malformed("the record is not an object"));
    };
    let kind = message
        .get("type")
        .and_then(Value::as_i64)
        .and_then(|kind| i32::try_from(kind).ok())
        .ok_or(Malformed("the message has no type"))?;
    let has_string = |message: &serde_json::Map<String, Value>, key: &str| {
        matches!(message.get(key), Some(Value::String(_)))
    };
    match i64::from(kind) {
        TYPE_CLOSE => {
            let error_ok = matches!(
                message.get("error"),
                None | Some(Value::Null | Value::String(_))
            );
            let reconnect_ok = matches!(
                message.get("allowReconnect"),
                None | Some(Value::Null | Value::Bool(_))
            );
            if error_ok && reconnect_ok {
                Ok(Incoming::Close)
            } else {
                Err(Malformed("the close message is malformed"))
            }
        }
        TYPE_INVOCATION | TYPE_STREAM_INVOCATION => {
            let target = match message.remove("target") {
                Some(Value::String(target)) => target,
                _ => return Err(Malformed("the invocation has no target")),
            };
            let arguments = match message.remove("arguments") {
                Some(Value::Array(arguments)) => arguments,
                _ => return Err(Malformed("the invocation has no arguments")),
            };
            // An empty id is as good as none: nobody can wait for it.
            let id = match message.remove("invocationId") {
                None => None,
                Some(Value::String(id)) if id.is_empty() => None,
                Some(Value::String(id)) => Some(id),
                Some(_) => return Err(Malformed("the invocation id is not a string")),
            };
            let stream_ids = match message.remove("streamIds") {
                None => 0,
                Some(Value::Array(ids)) if ids.iter().all(Value::is_string) => ids.len(),
                Some(_) => return Err(Malformed("the stream ids are not strings")),
            };
            match message.get("headers") {
                None => {}
                Some(Value::Object(headers)) if headers.values().all(Value::is_string) => {}
                Some(_) => return Err(Malformed("the headers are not strings")),
            }
            Ok(Incoming::Invocation(Invocation {
                id,
                target,
                arguments,
                stream_ids,
                streaming: i64::from(kind) == TYPE_STREAM_INVOCATION,
            }))
        }
        // A stream item: it names its stream and carries an item.
        2 if has_string(&message, "invocationId") && message.contains_key("item") => {
            Ok(Incoming::Ignored)
        }
        // A completion: it names its call and holds a result or an error, not both.
        3 if has_string(&message, "invocationId")
            && !(message.contains_key("result") && message.contains_key("error")) =>
        {
            Ok(Incoming::Ignored)
        }
        // A cancellation.
        5 if has_string(&message, "invocationId") => Ok(Incoming::Ignored),
        // An acknowledgement, or a sequence number.
        8 | 9 if message.get("sequenceId").is_some_and(Value::is_i64) => Ok(Incoming::Ignored),
        2 | 3 | 5 | 8 | 9 => Err(Malformed("the message is missing a part it needs")),
        // A ping, and types nobody has defined.
        _ => Ok(Incoming::Ignored),
    }
}

/// The answer to an invocation that succeeded. A method without a result
/// answers with `null`.
pub fn completion_result(id: &str, result: &Value) -> Bytes {
    #[derive(Serialize)]
    struct Completion<'a> {
        #[serde(rename = "type")]
        kind: u8,
        #[serde(rename = "invocationId")]
        id: &'a str,
        result: &'a Value,
    }
    frame_record(
        &serde_json::to_vec(&Completion {
            kind: 3,
            id,
            result,
        })
        .expect("a value serialises"),
    )
}

/// The answer to an invocation that failed.
pub fn completion_error(id: &str, error: &str) -> Bytes {
    #[derive(Serialize)]
    struct Completion<'a> {
        #[serde(rename = "type")]
        kind: u8,
        #[serde(rename = "invocationId")]
        id: &'a str,
        error: &'a str,
    }
    frame_record(
        &serde_json::to_vec(&Completion { kind: 3, id, error }).expect("strings serialise"),
    )
}

pub fn ping() -> Bytes {
    frame_record(br#"{"type":6}"#)
}

/// The server closes the connection. `allow_reconnect` tells the client
/// whether to start over by itself; without it a client stops for good.
pub fn close(error: Option<&str>, allow_reconnect: bool) -> Bytes {
    #[derive(Serialize)]
    struct Close<'a> {
        #[serde(rename = "type")]
        kind: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<&'a str>,
        #[serde(rename = "allowReconnect", skip_serializing_if = "std::ops::Not::not")]
        allow_reconnect: bool,
    }
    frame_record(
        &serde_json::to_vec(&Close {
            kind: 7,
            error,
            allow_reconnect,
        })
        .expect("strings serialise"),
    )
}

/// A call the server makes on the client: no id, so no answer.
pub fn invocation<A: Serialize + ?Sized>(
    target: &str,
    arguments: &A,
) -> Result<Bytes, serde_json::Error> {
    #[derive(Serialize)]
    struct Invoke<'a, A: ?Sized> {
        #[serde(rename = "type")]
        kind: u8,
        target: &'a str,
        arguments: &'a A,
    }
    let body = serde_json::to_vec(&Invoke {
        kind: 1,
        target,
        arguments,
    })?;
    Ok(frame_record(&body))
}

/// The answer to `POST .../negotiate`, as JSON, for a client that asked for
/// `requested` (the `negotiateVersion` query value, if any). Only the
/// WebSocket transport is offered: the other two cannot be served, and a
/// client that finds WebSockets among the transports uses them.
///
/// `connection_id` names the connection; a version 1 answer also carries a
/// separate `connection_token`, which is what the transport then connects with.
pub fn negotiate_response(
    requested: Option<&str>,
    connection_id: &str,
    connection_token: &str,
) -> String {
    #[derive(Serialize)]
    struct Transport {
        transport: &'static str,
        #[serde(rename = "transferFormats")]
        transfer_formats: [&'static str; 2],
    }
    #[derive(Serialize)]
    struct Answer<'a> {
        #[serde(rename = "negotiateVersion")]
        version: i64,
        #[serde(rename = "connectionId")]
        connection_id: &'a str,
        #[serde(rename = "connectionToken", skip_serializing_if = "Option::is_none")]
        connection_token: Option<&'a str>,
        #[serde(rename = "availableTransports")]
        transports: [Transport; 1],
    }
    let version = match requested {
        None => 0,
        Some(text) => match text.parse::<i64>() {
            Ok(version) => version.clamp(0, NEGOTIATE_VERSION),
            Err(_) => {
                return serde_json::to_string(&Failure {
                    error: "The client requested a non-integer protocol version.",
                })
                .expect("strings serialise");
            }
        },
    };
    let answer = Answer {
        version,
        connection_id,
        connection_token: (version > 0).then_some(connection_token),
        transports: [Transport {
            transport: "WebSockets",
            transfer_formats: ["Text", "Binary"],
        }],
    };
    serde_json::to_string(&answer).expect("strings serialise")
}

#[cfg(test)]
mod tests;
