//! What leaves the server for Resend, and what the server makes of its
//! answers.

use std::time::SystemTime;

use stellarr_mail::{Message, ResendError, ResendRequest, read_resend_answer};
use stellarr_smtp::{InlineImage, Mailbox};

fn message() -> Message {
    Message {
        from: Mailbox::parse("Stellarr <noreply@chat.test>").unwrap(),
        to: vec![Mailbox::parse("jo@example.test").unwrap()],
        subject: "Temat".into(),
        html: "<p>Treść</p>".into(),
        text: "Treść".into(),
        message_id: "message-1@stellarr".into(),
        date: SystemTime::UNIX_EPOCH,
        images: Vec::new(),
    }
}

fn body(request: &ResendRequest) -> serde_json::Value {
    serde_json::from_str(&request.json).unwrap()
}

#[test]
fn the_message_goes_to_resend_with_the_key_and_an_idempotency_key() {
    let request = ResendRequest::new(&message(), "re_test").unwrap();

    assert_eq!(ResendRequest::URL, "https://api.resend.com/emails");
    assert_eq!(request.authorization, "Bearer re_test");
    assert_eq!(request.idempotency_key, "message-1");
    let body = body(&request);
    assert_eq!(body["from"], "Stellarr <noreply@chat.test>");
    assert_eq!(body["to"], serde_json::json!(["jo@example.test"]));
    assert_eq!(body["subject"], "Temat");
    assert_eq!(body["html"], "<p>Treść</p>");
    assert_eq!(body["text"], "Treść");
    assert!(body.get("attachments").is_none());
}

#[test]
fn inline_images_go_to_resend_as_attachments_with_their_content_id() {
    let mut mail = message();
    mail.images.push(InlineImage {
        content_id: "tile@stellarr".into(),
        file_name: "tile.png".into(),
        content_type: "image/png".into(),
        content: vec![1, 2, 3],
    });

    let body = body(&ResendRequest::new(&mail, "re_test").unwrap());

    let attachments = body["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["content"], "AQID");
    assert_eq!(attachments[0]["filename"], "tile.png");
    assert_eq!(attachments[0]["content_type"], "image/png");
    assert_eq!(attachments[0]["content_id"], "tile@stellarr");
}

#[test]
fn the_attachment_bytes_survive_every_padding_length() {
    for (bytes, expected) in [
        (&b""[..], ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"fooba", "Zm9vYmE="),
        (b"foobar", "Zm9vYmFy"),
    ] {
        let mut mail = message();
        mail.images.push(InlineImage {
            content_id: "a@b".into(),
            file_name: "a.bin".into(),
            content_type: "application/octet-stream".into(),
            content: bytes.to_vec(),
        });
        let body = body(&ResendRequest::new(&mail, "k").unwrap());
        assert_eq!(body["attachments"][0]["content"], expected);
    }
}

#[test]
fn a_sender_with_punctuation_in_the_name_is_quoted() {
    let mut mail = message();
    mail.from = Mailbox::parse("\"Orbit, Co\" <noreply@chat.test>").unwrap();

    let body = body(&ResendRequest::new(&mail, "k").unwrap());

    assert_eq!(body["from"], "\"Orbit, Co\" <noreply@chat.test>");
}

#[test]
fn a_refused_message_is_reported_with_the_providers_status_and_reason() {
    let failure = read_resend_answer(422, b"invalid from address").unwrap_err();

    assert_eq!(
        failure,
        ResendError::Rejected {
            status: 422,
            detail: "invalid from address".into()
        }
    );
    assert_eq!(
        failure.to_string(),
        "Resend rejected the message (422): invalid from address"
    );
}

#[test]
fn a_long_refusal_is_cut_and_a_binary_one_does_not_panic() {
    let long = vec![b'x'; 100_000];
    let ResendError::Rejected { detail, .. } = read_resend_answer(500, &long).unwrap_err() else {
        panic!("a refusal");
    };
    assert_eq!(detail.len(), 1000);
    assert!(read_resend_answer(500, &[0xff, 0xfe, 0x00]).is_err());
}

#[test]
fn the_id_of_the_answer_is_the_providers_id_for_the_message() {
    assert_eq!(
        read_resend_answer(200, br#"{"id":"email-42"}"#).unwrap(),
        "email-42"
    );
    assert_eq!(
        read_resend_answer(201, br#"{"id":"email-42","x":1}"#).unwrap(),
        "email-42"
    );
}

#[test]
fn an_answer_without_a_message_id_is_a_failure() {
    for body in [
        &b"{}"[..],
        b"",
        b"not json",
        br#"{"id":7}"#,
        br#"{"id":null}"#,
        b"[]",
        b"\"id\"",
    ] {
        assert_eq!(
            read_resend_answer(200, body),
            Err(ResendError::NoMessageId),
            "{}",
            String::from_utf8_lossy(body)
        );
    }
}

#[test]
fn without_an_api_key_nothing_is_sent() {
    assert_eq!(
        ResendRequest::new(&message(), " ").unwrap_err(),
        ResendError::NoApiKey
    );
    assert_eq!(
        ResendRequest::new(&message(), "").unwrap_err(),
        ResendError::NoApiKey
    );
}

#[test]
fn printing_a_request_leaves_the_key_out() {
    let request = ResendRequest::new(&message(), "re_secret").unwrap();

    assert!(!format!("{request:?}").contains("re_secret"));
}
