//! The crate against the mail the real server sent. `oracle/<language>.json`
//! holds every message a running server delivered to a mail catcher while
//! `oracle/scenario.py` drove it (a password reset, a delivery test, a code,
//! alerts for several events, an address change and the two activity
//! notices), with what the catcher read back. Each message is built again
//! from the same inputs and must come out the same: subject, text, HTML,
//! sender, recipient, message id and the images with their content ids.
//!
//! Lines are compared with LF endings: the catcher returns what was on the
//! wire, which is CRLF.

use std::path::Path;

use serde_json::Value;
use stellarr_mail::{
    MailSettings, MailTransport, Message, SecurityEvent, delivery_test, direct_message,
    email_change, email_code, mention, password_reset, security_alert,
};
use stellarr_messages::{Language, Messages};
use stellarr_timezone::TimeZone;

const DISPLAY_NAME: &str = "Jo <b>Owner</b>";
const INSTANCE: &str = "Orbit & Co";
const PUBLIC_URL: &str = "http://127.0.0.1:5196";

fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_else(|| panic!("no {key}"))
}

/// The hexadecimal id after the last slash of a message id.
fn id_of(message_id: &str) -> u128 {
    let key = message_id
        .split('@')
        .next()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    u128::from_str_radix(key, 16).unwrap()
}

fn unix_of(iso: &str) -> i64 {
    // 2026-11-24T01:54:57.7074458Z
    let n = |from: usize, to: usize| iso[from..to].parse::<i64>().unwrap();
    let (year, month, day) = (n(0, 4), n(5, 7), n(8, 10));
    let shifted_year = year - i64::from(month <= 2);
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    days * 86_400 + n(11, 13) * 3600 + n(14, 16) * 60 + n(17, 19)
}

fn warsaw() -> TimeZone {
    let database = Path::new(env!("CARGO_MANIFEST_DIR")).join("../timezone/tests/zoneinfo");
    TimeZone::find_in(&database, "Europe/Warsaw").unwrap()
}

fn rebuild(capture: &Value, mail: &Value, messages: &Messages) -> Message {
    let message_id = text_of(mail, "message_id");
    let from = format!(
        "{} <{}>",
        mail["from"]["Name"].as_str().unwrap(),
        mail["from"]["Address"].as_str().unwrap()
    );
    let to = mail["to"][0]["Address"].as_str().unwrap();
    let text = lf(text_of(mail, "text"));
    let lines: Vec<&str> = text.lines().collect();
    let id = id_of(message_id);
    let display_name = capture["display_name"].as_str().unwrap_or(DISPLAY_NAME);
    let kind = message_id.split('/').next().unwrap();
    match kind {
        // greeting, blank, instruction, link
        "password-reset" => {
            password_reset(&from, to, display_name, INSTANCE, lines[3], id, messages)
        }
        "email-change" => email_change(&from, to, display_name, INSTANCE, lines[3], id, messages),
        // greeting, blank, instruction, code
        "email-code" => email_code(
            &from,
            to,
            DISPLAY_NAME,
            INSTANCE,
            PUBLIC_URL,
            lines[3],
            8,
            id,
            messages,
        ),
        "delivery-test" => {
            let transport = MailTransport::resolve(
                Some(&MailSettings {
                    smtp_host: "host.docker.internal".into(),
                    smtp_port: 51025,
                    smtp_security: "none".into(),
                    ..MailSettings::default()
                }),
                "",
                "",
            );
            delivery_test(&from, to, INSTANCE, PUBLIC_URL, &transport, id, messages)
        }
        "security-alert" | "security-event" => {
            let event = &capture["events"][format!("{id:032x}")];
            assert!(event.is_object(), "no event {id:032x}");
            security_alert(
                &from,
                to,
                DISPLAY_NAME,
                INSTANCE,
                PUBLIC_URL,
                &SecurityEvent {
                    id,
                    kind: text_of(event, "kind"),
                    ip_address: text_of(event, "ipAddress"),
                    device: text_of(event, "device"),
                    created_at: unix_of(text_of(event, "createdAt")),
                },
                &warsaw(),
                messages,
            )
        }
        "activity-notification" => {
            if text_of(mail, "subject").contains("wzmianka")
                || text_of(mail, "subject").contains("mention")
            {
                mention(&from, to, PUBLIC_URL, id, messages)
            } else {
                direct_message(&from, to, PUBLIC_URL, id, messages)
            }
        }
        other => panic!("unknown kind {other}"),
    }
    .unwrap()
}

fn first_difference(expected: &str, built: &str) -> String {
    let at = expected
        .char_indices()
        .zip(built.chars())
        .find(|((_, a), b)| a != b)
        .map_or(expected.len().min(built.len()), |((i, _), _)| i);
    let window = |s: &str| {
        s.chars()
            .skip(at.saturating_sub(60))
            .take(160)
            .collect::<String>()
    };
    format!(
        "at {at}\nserver: {:?}\nours:   {:?}",
        window(expected),
        window(built)
    )
}

fn check_language(file: &str, language: Language) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/oracle")
        .join(file);
    let capture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let messages = Messages::new(language);
    let mails = capture["mails"].as_array().unwrap();
    let mut kinds = Vec::new();
    for mail in mails {
        let built = rebuild(&capture, mail, &messages);
        let label = text_of(mail, "message_id");
        kinds.push(label.split('/').next().unwrap().to_owned());

        assert_eq!(built.subject, text_of(mail, "subject"), "{label} subject");
        assert_eq!(built.text, lf(text_of(mail, "text")), "{label} text");
        let expected_html = lf(text_of(mail, "html"));
        assert!(
            built.html == expected_html,
            "{label} html differs {}",
            first_difference(&expected_html, &built.html)
        );
        assert_eq!(
            format!("{}@stellarr", built.message_id.split('@').next().unwrap()),
            text_of(mail, "message_id")
        );
        assert_eq!(
            built.message_id,
            text_of(mail, "message_id"),
            "{label} message id"
        );
        assert_eq!(
            built.from.address(),
            mail["from"]["Address"].as_str().unwrap()
        );
        assert_eq!(
            built.from.name().unwrap_or(""),
            mail["from"]["Name"].as_str().unwrap()
        );
        assert_eq!(built.to.len(), 1);
        assert_eq!(
            built.to[0].address(),
            mail["to"][0]["Address"].as_str().unwrap()
        );

        let sent_images = mail["images"].as_array().unwrap();
        assert_eq!(built.images.len(), sent_images.len(), "{label} images");
        for (ours, theirs) in built.images.iter().zip(sent_images) {
            assert_eq!(ours.content_id, text_of(theirs, "content_id"), "{label}");
            assert_eq!(ours.file_name, text_of(theirs, "file_name"), "{label}");
            assert_eq!(
                ours.content_type,
                text_of(theirs, "content_type"),
                "{label}"
            );
            assert_eq!(
                ours.content.len() as u64,
                theirs["size"].as_u64().unwrap(),
                "{label}"
            );
        }
    }
    kinds.sort();
    kinds.dedup();
    if !capture["events"].as_object().unwrap().is_empty() {
        assert!(kinds.len() >= 6, "the capture holds only {kinds:?}");
    }
}

#[test]
fn every_message_the_polish_server_sent_is_built_again_byte_for_byte() {
    check_language("pl.json", Language::Polish);
}

#[test]
fn every_message_the_english_server_sent_is_built_again_byte_for_byte() {
    check_language("en.json", Language::English);
}

#[test]
fn a_name_full_of_characters_to_escape_is_escaped_the_way_the_server_does() {
    // No-break space, accented letters from U+00A0 to U+00FF, an emoji, the
    // markup characters and a letter past Latin-1.
    check_language("extra.json", Language::English);
}
