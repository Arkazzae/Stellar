//! Every kind of mail, in both languages, against files that hold what each
//! message must be. `golden/<language>.json` keeps, for every message, the
//! subject, text, HTML, sender, recipient, message id and the images with
//! their content ids, together with the inputs the message is made from (the
//! person's name, the security events, the link or code in the text). Each
//! message is built again from those inputs and must come out the same.
//!
//! Lines are compared with LF endings: the files hold what goes on the wire,
//! which is CRLF.
//!
//! When the mail is meant to change, rewrite the files from what the code
//! builds, and read the difference before committing it:
//!
//! ```sh
//! STELLARR_MAIL_GOLDEN=update acargo test -p stellarr-mail --test golden
//! ```

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
        "at {at}\nexpected: {:?}\nbuilt:    {:?}",
        window(expected),
        window(built)
    )
}

/// The golden entry of a message: `mail` with what was built put in place of
/// what it held, written the way it goes on the wire.
fn golden_of(mail: &Value, built: &Message) -> Value {
    let wire = |text: &str| text.replace('\n', "\r\n");
    let mut entry = mail.clone();
    entry["subject"] = built.subject.clone().into();
    entry["text"] = wire(&built.text).into();
    entry["html"] = wire(&built.html).into();
    entry["message_id"] = built.message_id.clone().into();
    entry["from"]["Name"] = built.from.name().unwrap_or("").into();
    entry["from"]["Address"] = built.from.address().into();
    entry["to"][0]["Address"] = built.to[0].address().into();
    entry["images"] = built
        .images
        .iter()
        .map(|image| {
            serde_json::json!({
                "content_id": image.content_id,
                "file_name": image.file_name,
                "content_type": image.content_type,
                "size": image.content.len(),
            })
        })
        .collect();
    entry
}

fn check_language(file: &str, language: Language) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file);
    let mut capture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let messages = Messages::new(language);
    let mails = capture["mails"].as_array().unwrap().clone();
    if std::env::var("STELLARR_MAIL_GOLDEN").is_ok_and(|mode| mode == "update") {
        let rewritten: Vec<Value> = mails
            .iter()
            .map(|mail| golden_of(mail, &rebuild(&capture, mail, &messages)))
            .collect();
        capture["mails"] = rewritten.into();
        let mut text = serde_json::to_string_pretty(&capture).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let mut kinds = Vec::new();
    for mail in &mails {
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
fn every_polish_message_is_built_again_byte_for_byte() {
    check_language("pl.json", Language::Polish);
}

#[test]
fn every_english_message_is_built_again_byte_for_byte() {
    check_language("en.json", Language::English);
}

#[test]
fn a_name_full_of_characters_to_escape_is_escaped_in_the_message() {
    // No-break space, accented letters from U+00A0 to U+00FF, an emoji, the
    // markup characters and a letter past Latin-1.
    check_language("extra.json", Language::English);
}
