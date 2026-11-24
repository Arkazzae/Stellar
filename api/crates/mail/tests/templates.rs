//! What the branded messages say and carry, apart from how a provider sends
//! them.

use std::path::Path;

use stellarr_mail::{
    MailSettings, MailTransport, Message, SecurityEvent, delivery_test, describe_event,
    direct_message, email_change, email_code, event_kinds, mention, password_reset, security_alert,
};
use stellarr_messages::{Language, Messages};
use stellarr_timezone::TimeZone;

const FROM: &str = "Stellarr <noreply@chat.test>";
const TO: &str = "jo@example.test";
const PUBLIC_URL: &str = "https://chat.test";
/// 2026-11-20 13:05:00 UTC
const EVENT_TIME: i64 = 1_795_179_900;
const FIREFOX: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0";

fn polish() -> Messages {
    Messages::new(Language::Polish)
}

fn english() -> Messages {
    Messages::new(Language::English)
}

fn warsaw() -> TimeZone {
    let database = Path::new(env!("CARGO_MANIFEST_DIR")).join("../timezone/tests/zoneinfo");
    TimeZone::find_in(&database, "Europe/Warsaw").unwrap()
}

fn event(kind: &str) -> SecurityEvent<'_> {
    SecurityEvent {
        id: 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
        kind,
        ip_address: "203.0.113.42",
        device: FIREFOX,
        created_at: EVENT_TIME,
    }
}

fn resend_transport() -> MailTransport {
    MailTransport::resolve(
        Some(&MailSettings {
            resend_enabled: Some(true),
            ..MailSettings::default()
        }),
        "k",
        "",
    )
}

fn all_templates(messages: &Messages) -> Vec<(&'static str, Message)> {
    vec![
        (
            "password-reset",
            password_reset(
                FROM,
                TO,
                "Jo",
                "Orbit",
                "https://chat.test/reset?token=a",
                1,
                messages,
            )
            .unwrap(),
        ),
        (
            "email-change",
            email_change(
                FROM,
                TO,
                "Jo",
                "Orbit",
                "https://chat.test/confirm?token=a",
                2,
                messages,
            )
            .unwrap(),
        ),
        (
            "email-code",
            email_code(
                FROM, TO, "Jo", "Orbit", PUBLIC_URL, "123456", 8, 3, messages,
            )
            .unwrap(),
        ),
        (
            "security-alert",
            security_alert(
                FROM,
                TO,
                "Jo",
                "Orbit",
                PUBLIC_URL,
                &event(event_kinds::LOGIN_SUCCEEDED),
                &TimeZone::utc(),
                messages,
            )
            .unwrap(),
        ),
        (
            "delivery-test",
            delivery_test(
                FROM,
                TO,
                "Orbit",
                PUBLIC_URL,
                &resend_transport(),
                4,
                messages,
            )
            .unwrap(),
        ),
        (
            "mention",
            mention(FROM, TO, PUBLIC_URL, 5, messages).unwrap(),
        ),
        (
            "direct-message",
            direct_message(FROM, TO, PUBLIC_URL, 6, messages).unwrap(),
        ),
    ]
}

/// Every `src="cid:..."` of the page.
fn cid_references(html: &str) -> Vec<String> {
    let mut found: Vec<String> = html
        .split("src=\"cid:")
        .skip(1)
        .map(|rest| rest[..rest.find('"').unwrap()].to_owned())
        .collect();
    found.sort();
    found.dedup();
    found
}

#[test]
fn every_image_the_html_shows_travels_with_the_message_and_nothing_else() {
    for (name, mail) in all_templates(&polish()) {
        let shown = cid_references(&mail.html);
        let mut carried: Vec<String> = mail.images.iter().map(|i| i.content_id.clone()).collect();
        carried.sort();

        assert_eq!(carried, shown, "{name}");
        assert!(
            mail.images
                .iter()
                .any(|i| i.content_id == "lockup@stellarr"),
            "{name}"
        );
        // Nothing is fetched when the mail is opened: every source and link
        // is a cid or the instance's own address.
        for attribute in ["src=\"", "href=\""] {
            for rest in mail.html.split(attribute).skip(1) {
                let value = &rest[..rest.find('"').unwrap()];
                let local = value.starts_with("cid:") || value.starts_with("https://chat.test");
                assert!(local, "{name}: {value}");
            }
        }
    }
}

#[test]
fn every_message_is_in_the_instances_language_and_has_a_plain_text_part() {
    for (name, mail) in all_templates(&polish()) {
        assert!(
            mail.html.starts_with("<!doctype html>\n<html lang=\"pl\""),
            "{name}"
        );
        assert!(!mail.text.is_empty(), "{name}");
        assert!(!mail.text.contains('<'), "{name}");
    }
    for (name, mail) in all_templates(&english()) {
        assert!(
            mail.html.starts_with("<!doctype html>\n<html lang=\"en\""),
            "{name}"
        );
    }
}

#[test]
fn the_images_are_embedded_pngs() {
    let mail = mention(FROM, TO, PUBLIC_URL, 1, &english()).unwrap();
    let direct = direct_message(FROM, TO, PUBLIC_URL, 1, &english()).unwrap();

    for image in mail.images.iter().chain(&direct.images) {
        assert_eq!(image.content_type, "image/png");
        assert_eq!(&image.content[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }
    let ids: Vec<&str> = mail.images.iter().map(|i| i.content_id.as_str()).collect();
    assert_eq!(ids, ["lockup@stellarr", "mention@stellarr"]);
    let ids: Vec<&str> = direct
        .images
        .iter()
        .map(|i| i.content_id.as_str())
        .collect();
    assert_eq!(ids, ["lockup@stellarr", "direct-message@stellarr"]);
}

#[test]
fn a_security_alert_names_the_event_the_time_in_the_instances_zone_and_the_device() {
    let mail = security_alert(
        FROM,
        TO,
        "Jo",
        "Orbit",
        PUBLIC_URL,
        &event(event_kinds::LOGIN_SUCCEEDED),
        &warsaw(),
        &polish(),
    )
    .unwrap();

    assert_eq!(mail.subject, "Aktywność zabezpieczeń w Orbit");
    assert!(mail.html.contains("Udane logowanie"));
    // 13:05 UTC is 14:05 in Warsaw in winter; the month is in the genitive.
    assert!(
        mail.html
            .contains("20 listopada 2026, 14:05 (Europe/Warsaw)")
    );
    assert!(mail.html.contains("Firefox na Linux"));
    assert!(mail.html.contains("203.0.113.42"));
    assert!(!mail.html.contains("login-succeeded"));
    assert!(
        mail.text.contains(
            "Zdarzenie: Udane logowanie\nKiedy: 20 listopada 2026, 14:05 (Europe/Warsaw)\n"
        )
    );
    assert!(mail.text.contains("Urządzenie: Firefox na Linux"));
}

#[test]
fn an_event_without_a_name_of_its_own_is_shown_by_its_kind() {
    assert_eq!(
        describe_event("something-new", &english()).unwrap(),
        "Event: something-new"
    );
    assert_eq!(
        describe_event(event_kinds::PASSWORD_CHANGED, &english()).unwrap(),
        "Password changed"
    );
}

#[test]
fn an_alert_leaves_out_the_device_and_address_it_does_not_know() {
    let unknown = SecurityEvent {
        device: "curl/8.0",
        ip_address: "",
        ..event(event_kinds::PASSWORD_CHANGED)
    };

    let mail = security_alert(
        FROM,
        TO,
        "Jo",
        "Orbit",
        PUBLIC_URL,
        &unknown,
        &TimeZone::utc(),
        &english(),
    )
    .unwrap();

    assert!(mail.html.contains("Password changed"));
    assert!(!mail.html.contains("Device"));
    assert!(!mail.html.contains("IP address"));
}

#[test]
fn a_code_mail_shows_the_code_and_how_long_it_lasts() {
    let mail = email_code(
        FROM,
        TO,
        "Jo",
        "Orbit",
        PUBLIC_URL,
        "482913",
        8,
        1,
        &english(),
    )
    .unwrap();

    assert_eq!(mail.subject, "Your verification code for Orbit");
    assert!(mail.html.contains(">482913</td>"));
    assert!(mail.html.contains("valid for 8 min"));
    assert!(mail.text.contains("\n482913\n"));
    assert!(!mail.html.contains("Security activity"));
}

#[test]
fn a_delivery_test_names_the_way_it_went() {
    let smtp = MailTransport::resolve(
        Some(&MailSettings {
            smtp_host: "mail.chat.test".into(),
            smtp_port: 587,
            smtp_username: "u".into(),
            ..MailSettings::default()
        }),
        "",
        "secret",
    );
    let resend = MailTransport::resolve(
        Some(&MailSettings {
            resend_enabled: Some(true),
            ..MailSettings::default()
        }),
        "re_key",
        "",
    );

    let through_smtp = delivery_test(FROM, TO, "Orbit", PUBLIC_URL, &smtp, 1, &english()).unwrap();
    let through_resend =
        delivery_test(FROM, TO, "Orbit", PUBLIC_URL, &resend, 2, &english()).unwrap();

    assert!(through_smtp.html.contains("SMTP, mail.chat.test:587"));
    assert!(!through_smtp.html.contains("secret"));
    assert!(!through_smtp.html.contains("Resend"));
    assert!(through_resend.html.contains("Resend"));
    assert!(!through_resend.html.contains("re_key"));
}

#[test]
fn activity_mail_names_no_instance_and_says_how_to_stop_it() {
    let mail = direct_message(FROM, TO, PUBLIC_URL, 1, &english()).unwrap();

    assert!(mail.html.contains("Settings › Notifications"));
    assert!(mail.text.contains("Settings › Notifications"));
    assert!(!mail.html.contains("Sent by the"));
    assert!(!mail.html.contains("class=\"instance\""));
}

#[test]
fn the_default_instance_name_is_not_repeated_beside_the_lockup() {
    let url = "https://chat.test/reset";
    let plain = password_reset(FROM, TO, "Jo", "Stellarr", url, 1, &english()).unwrap();
    let named = password_reset(FROM, TO, "Jo", "Orbit", url, 2, &english()).unwrap();

    assert!(!plain.html.contains("class=\"instance\""));
    assert!(named.html.contains("class=\"instance\""));
    assert!(named.html.contains(">Orbit</td>"));
}

#[test]
fn the_password_reset_mail_escapes_names_in_its_html_only() {
    let token_id = 0xfeed_u128;

    let mail = password_reset(
        "Stellarr <noreply@chat.test>",
        "jo@example.test",
        "<b>Jo</b>",
        "Orbit & Co",
        "https://chat.test/reset-password?token=abc&x=1",
        token_id,
        &polish(),
    )
    .unwrap();

    assert!(mail.html.contains("&lt;b&gt;Jo&lt;/b&gt;"));
    assert!(!mail.html.contains("<b>Jo</b>"));
    assert!(mail.html.contains("Orbit &amp; Co"));
    assert!(
        mail.html
            .contains("href=\"https://chat.test/reset-password?token=abc&amp;x=1\"")
    );
    assert!(mail.text.contains("<b>Jo</b>"));
    assert!(
        mail.text
            .contains("https://chat.test/reset-password?token=abc&x=1")
    );
    assert_eq!(mail.subject, "Zmień hasło w Orbit & Co");
    // The same message is the same delivery, whichever attempt sends it.
    assert_eq!(
        mail.message_id,
        "password-reset/0000000000000000000000000000feed@stellarr"
    );
}

#[test]
fn an_address_that_is_not_one_is_refused() {
    assert!(
        password_reset(
            "Stellarr <noreply@chat.test>",
            "not an address",
            "Jo",
            "Orbit",
            "https://chat.test/r",
            1,
            &english()
        )
        .is_err()
    );
    assert!(password_reset("", TO, "Jo", "Orbit", "https://chat.test/r", 1, &english()).is_err());
}

#[test]
fn a_link_without_an_authority_leaves_the_origin_out() {
    let mail = password_reset(FROM, TO, "Jo", "Orbit", "/reset?token=a", 1, &english()).unwrap();

    assert!(mail.text.ends_with("Sent by the Orbit server"));
    assert!(
        !mail
            .html
            .contains("target=\"_blank\" style=\"color:#9aa0ad")
    );
}
