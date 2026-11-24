use serde_json::{Value, json};

use super::*;

fn parse(text: &str) -> Result<Incoming, Malformed> {
    parse_message(text.as_bytes())
}

fn invocation(text: &str) -> Invocation {
    match parse(text).unwrap() {
        Incoming::Invocation(invocation) => invocation,
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_invocation_with_an_id_and_arguments() {
    let got = invocation(
        r#"{"type":1,"invocationId":"7","target":"SetPresence","arguments":["online",2,null,{"a":[1]}]}"#,
    );
    assert_eq!(got.id.as_deref(), Some("7"));
    assert_eq!(got.target, "SetPresence");
    assert_eq!(
        got.arguments,
        [json!("online"), json!(2), Value::Null, json!({"a": [1]})]
    );
    assert_eq!((got.stream_ids, got.streaming), (0, false));
}

#[test]
fn keys_may_come_in_any_order_and_unknown_keys_are_skipped() {
    let got = invocation(r#"{"arguments":[],"zzz":{"deep":[1,2]},"target":"T","type":1}"#);
    assert_eq!((got.id, got.target.as_str()), (None, "T"));
}

#[test]
fn a_send_has_no_id_and_an_empty_id_is_none_too() {
    assert_eq!(
        invocation(r#"{"type":1,"target":"T","arguments":[]}"#).id,
        None
    );
    assert_eq!(
        invocation(r#"{"type":1,"invocationId":"","target":"T","arguments":[]}"#).id,
        None
    );
}

#[test]
fn a_streaming_invocation_is_recognised_and_stream_ids_are_counted() {
    let got = invocation(
        r#"{"type":4,"invocationId":"1","target":"T","arguments":[],"streamIds":["a","b"]}"#,
    );
    assert!(got.streaming);
    assert_eq!(got.stream_ids, 2);
    assert_eq!(
        invocation(r#"{"type":1,"target":"T","arguments":[],"streamIds":null}"#).stream_ids,
        0
    );
}

#[test]
fn close_is_recognised_whatever_it_carries() {
    assert_eq!(parse(r#"{"type":7}"#), Ok(Incoming::Close));
    assert_eq!(
        parse(r#"{"type":7,"error":"x","allowReconnect":true}"#),
        Ok(Incoming::Close)
    );
}

#[test]
fn messages_the_server_has_no_use_for_are_skipped() {
    for text in [
        r#"{"type":6}"#,
        r#"{"type":2,"invocationId":"1","item":1}"#,
        r#"{"type":3,"invocationId":"1","result":1}"#,
        r#"{"type":5,"invocationId":"1"}"#,
        r#"{"type":8,"sequenceId":1}"#,
        r#"{"type":9,"sequenceId":1}"#,
        r#"{"type":0}"#,
        r#"{"type":99}"#,
        r#"{"type":-1}"#,
    ] {
        assert_eq!(parse(text), Ok(Incoming::Ignored), "{text}");
    }
}

#[test]
fn what_cannot_be_read_is_malformed() {
    for text in [
        "",
        "{not json",
        "[1,2]",
        "7",
        "null",
        r#"{"invocationId":"1","target":"T","arguments":[]}"#,
        r#"{"type":"1","target":"T","arguments":[]}"#,
        r#"{"type":1.5,"target":"T","arguments":[]}"#,
        r#"{"type":1,"arguments":[]}"#,
        r#"{"type":1,"target":null,"arguments":[]}"#,
        r#"{"type":1,"target":5,"arguments":[]}"#,
        r#"{"type":1,"target":"T"}"#,
        r#"{"type":1,"target":"T","arguments":{}}"#,
        r#"{"type":1,"target":"T","arguments":null}"#,
        r#"{"type":1,"invocationId":5,"target":"T","arguments":[]}"#,
        r#"{"type":1,"invocationId":null,"target":"T","arguments":[]}"#,
        r#"{"type":1,"target":"T","arguments":[],"streamIds":[1]}"#,
        r#"{"type":1,"target":"T","arguments":[],"streamIds":"a"}"#,
        r#"{"type":1,"target":"T","arguments":[]} trailing"#,
        r#"{"type":1,"target":"T","arguments":["#,
    ] {
        assert!(parse(text).is_err(), "{text:?}");
    }
}

#[test]
fn bytes_that_are_not_utf8_are_malformed() {
    assert!(parse_message(&[b'{', 0xff, b'}']).is_err());
}

#[test]
fn deeply_nested_input_is_refused_not_crashed_on() {
    let deep = format!(
        r#"{{"type":1,"target":"T","arguments":[{}1{}]}}"#,
        "[".repeat(100_000),
        "]".repeat(100_000)
    );
    assert!(parse(&deep).is_err());
}

#[test]
fn large_numbers_and_unicode_arguments_survive() {
    let got = invocation(r#"{"type":1,"target":"T","arguments":[9007199254740993,"ł 😀"]}"#);
    assert_eq!(got.arguments[0], json!(9007199254740993u64));
    assert_eq!(got.arguments[1], json!("\u{142} \u{1f600}"));
}

#[test]
fn handshake_accepts_json_in_any_case_and_any_whole_version() {
    for text in [
        r#"{"protocol":"json","version":1}"#,
        r#"{"protocol":"JSON","version":1}"#,
        r#"{"protocol":"json","version":0}"#,
        r#"{"protocol":"json","version":2}"#,
        r#"{"version":1,"protocol":"json","extra":[1]}"#,
    ] {
        assert_eq!(parse_handshake(text.as_bytes()), Ok(()), "{text}");
    }
}

#[test]
fn handshake_names_a_protocol_it_does_not_speak() {
    assert_eq!(
        parse_handshake(br#"{"protocol":"messagepack","version":1}"#),
        Err("The protocol 'messagepack' is not supported.".into())
    );
}

#[test]
fn a_handshake_that_cannot_be_read_gets_the_general_error() {
    for text in [
        "",
        "garbage",
        "[]",
        r#"{"version":1}"#,
        r#"{"protocol":"json"}"#,
        r#"{"protocol":null,"version":1}"#,
        r#"{"protocol":1,"version":1}"#,
        r#"{"protocol":"json","version":1.5}"#,
        r#"{"protocol":"json","version":"1"}"#,
        r#"{"protocol":"json","version":null}"#,
        r#"{"protocol":"messagepack","version":"x"}"#,
    ] {
        assert_eq!(
            parse_handshake(text.as_bytes()),
            Err(HANDSHAKE_FAILED.into()),
            "{text:?}"
        );
    }
}

#[test]
fn the_handshake_answers() {
    assert_eq!(handshake_response(None).as_ref(), b"{}\x1e");
    assert_eq!(
        handshake_response(Some("The protocol 'x\"y' is not supported.")).as_ref(),
        b"{\"error\":\"The protocol 'x\\\"y' is not supported.\"}\x1e"
    );
}

#[test]
fn completions_are_what_the_client_library_reads() {
    assert_eq!(
        completion_result("7", &Value::Null).as_ref(),
        b"{\"type\":3,\"invocationId\":\"7\",\"result\":null}\x1e"
    );
    assert_eq!(
        completion_result("a\"b", &json!({"x": [1, "\u{142}"]})).as_ref(),
        "{\"type\":3,\"invocationId\":\"a\\\"b\",\"result\":{\"x\":[1,\"\u{142}\"]}}\x1e"
            .as_bytes()
    );
    assert_eq!(
        completion_error("1", "Failed to invoke 'Nope' due to an error on the server.").as_ref(),
        b"{\"type\":3,\"invocationId\":\"1\",\"error\":\"Failed to invoke 'Nope' due to an error on the server.\"}\x1e"
    );
}

#[test]
fn ping_and_close_records() {
    assert_eq!(ping().as_ref(), b"{\"type\":6}\x1e");
    assert_eq!(close(None, false).as_ref(), b"{\"type\":7}\x1e");
    assert_eq!(
        close(Some(CLOSED_WITH_ERROR), true).as_ref(),
        b"{\"type\":7,\"error\":\"Connection closed with an error.\",\"allowReconnect\":true}\x1e"
    );
    assert_eq!(
        close(None, true).as_ref(),
        b"{\"type\":7,\"allowReconnect\":true}\x1e"
    );
}

#[test]
fn a_server_invocation_has_no_id() {
    let bytes = invocation_record();
    assert_eq!(bytes.as_ref(), b"{\"type\":1,\"target\":\"PresenceChanged\",\"arguments\":[{\"userId\":\"u\",\"presence\":\"online\"}]}\x1e");
    assert_eq!(
        super::invocation("Empty", &Vec::<u8>::new())
            .unwrap()
            .as_ref(),
        b"{\"type\":1,\"target\":\"Empty\",\"arguments\":[]}\x1e"
    );
}

fn invocation_record() -> bytes::Bytes {
    #[derive(Serialize)]
    struct Update {
        #[serde(rename = "userId")]
        user_id: &'static str,
        presence: &'static str,
    }
    super::invocation(
        "PresenceChanged",
        &(Update {
            user_id: "u",
            presence: "online",
        },),
    )
    .unwrap()
}

#[test]
fn negotiation_versions() {
    let v1: Value = serde_json::from_str(&negotiate_response(Some("1"), "id", "token")).unwrap();
    assert_eq!(
        v1,
        json!({
            "negotiateVersion": 1,
            "connectionId": "id",
            "connectionToken": "token",
            "availableTransports": [{"transport": "WebSockets", "transferFormats": ["Text", "Binary"]}],
        })
    );
    let v0: Value = serde_json::from_str(&negotiate_response(None, "id", "token")).unwrap();
    assert_eq!(v0["negotiateVersion"], 0);
    assert!(v0.get("connectionToken").is_none());
    // A newer client is answered with the newest version this server knows.
    let v9: Value = serde_json::from_str(&negotiate_response(Some("9"), "id", "token")).unwrap();
    assert_eq!(v9["negotiateVersion"], 1);
    let low: Value = serde_json::from_str(&negotiate_response(Some("-3"), "id", "token")).unwrap();
    assert_eq!(low["negotiateVersion"], 0);
}

#[test]
fn negotiation_refuses_a_version_that_is_not_a_number() {
    assert_eq!(
        negotiate_response(Some("abc"), "id", "token"),
        r#"{"error":"The client requested a non-integer protocol version."}"#
    );
    assert!(negotiate_response(Some(""), "id", "token").contains("non-integer"));
}

#[test]
fn negotiation_keys_are_in_the_order_the_client_expects_them() {
    let text = negotiate_response(Some("1"), "id", "token");
    assert!(text.starts_with(r#"{"negotiateVersion":1,"connectionId":"id","connectionToken":"token","availableTransports":"#));
}
