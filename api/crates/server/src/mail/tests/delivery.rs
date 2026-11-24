use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use stellarr_mail::{MailSettings, MailTransport, Message};
use stellarr_outbound::{AddressPolicy, TableResolver};
use stellarr_smtp::{InlineImage, Mailbox, Security};

use super::support::{Answer, Behaviour, FakeResend, FakeSmtp, certificates, free_port};
use crate::mail::{MailDelivery, Outcome, Wiring};

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

fn smtp_transport(port: u16, security: Security) -> MailTransport {
    smtp_transport_to("127.0.0.1", port, security, "", "")
}

fn smtp_transport_to(
    host: &str,
    port: u16,
    security: Security,
    user: &str,
    password: &str,
) -> MailTransport {
    let mut transport = MailTransport::resolve(
        Some(&MailSettings {
            smtp_host: host.into(),
            smtp_port: port,
            smtp_security: "none".into(),
            smtp_username: user.into(),
            resend_enabled: Some(false),
        }),
        "",
        password,
    );
    transport.smtp.as_mut().unwrap().security = security;
    transport
}

fn resend_transport(key: &str) -> MailTransport {
    MailTransport::resolve(
        Some(&MailSettings {
            resend_enabled: Some(true),
            ..MailSettings::default()
        }),
        key,
        "",
    )
}

// ---- SMTP ----

#[tokio::test]
async fn the_message_goes_to_the_server_with_its_envelope_and_body() {
    let server = FakeSmtp::start(Behaviour::default()).await;

    let outcome = MailDelivery::new()
        .send(&message(), &smtp_transport(server.port, Security::None))
        .await;

    assert_eq!(
        outcome,
        Outcome::Sent {
            id: "message-1@stellarr".into()
        }
    );
    let mail = &server.mails()[0];
    assert!(
        mail.commands
            .contains(&"MAIL FROM:<noreply@chat.test>".to_string())
    );
    assert!(
        mail.commands
            .contains(&"RCPT TO:<jo@example.test>".to_string())
    );
    assert!(server.seen().lines.contains(&"QUIT".to_string()));
    assert!(
        mail.data.contains("Message-ID: <message-1@stellarr>"),
        "{}",
        mail.data
    );
    assert!(mail.data.contains("Subject: Temat"), "{}", mail.data);
}

#[tokio::test]
async fn inline_images_travel_next_to_the_html_with_their_content_id() {
    let server = FakeSmtp::start(Behaviour::default()).await;
    let mut mail = message();
    mail.html = r#"<img src="cid:tile@stellarr">"#.into();
    mail.images.push(InlineImage {
        content_id: "tile@stellarr".into(),
        file_name: "tile.png".into(),
        content_type: "image/png".into(),
        content: vec![1, 2, 3],
    });

    MailDelivery::new()
        .send(&mail, &smtp_transport(server.port, Security::None))
        .await;

    let data = &server.mails()[0].data;
    assert!(
        data.to_ascii_lowercase().contains("multipart/related"),
        "{data}"
    );
    assert!(
        data.to_ascii_lowercase()
            .contains("content-id: <tile@stellarr>"),
        "{data}"
    );
    assert!(data.contains("image/png"));
    assert!(data.contains("AQID"));
}

#[tokio::test]
async fn a_refused_recipient_is_refused_for_good_with_the_servers_reply_and_address() {
    let server = FakeSmtp::start(Behaviour {
        recipient_reply: "550 5.1.1 No such mailbox",
        ..Behaviour::default()
    })
    .await;

    let outcome = MailDelivery::new()
        .send(&message(), &smtp_transport(server.port, Security::None))
        .await;

    let Outcome::Refused { reason } = outcome else {
        panic!("{outcome:?}")
    };
    assert!(reason.contains("No such mailbox"), "{reason}");
    assert!(
        reason.contains(&format!("127.0.0.1:{}", server.port)),
        "{reason}"
    );
}

#[tokio::test]
async fn a_server_that_asks_to_come_back_later_is_worth_another_try() {
    let server = FakeSmtp::start(Behaviour {
        recipient_reply: "450 4.2.0 Try again later",
        ..Behaviour::default()
    })
    .await;

    let outcome = MailDelivery::new()
        .send(&message(), &smtp_transport(server.port, Security::None))
        .await;

    assert!(matches!(outcome, Outcome::TryAgain { .. }), "{outcome:?}");
}

#[tokio::test]
async fn a_server_that_never_answers_fails_within_the_timeout() {
    let server = FakeSmtp::start(Behaviour {
        silent: true,
        ..Behaviour::default()
    })
    .await;
    let delivery = MailDelivery::wired(Wiring {
        roots: None,
        resend_url: "http://127.0.0.1:1/emails".into(),
        resend_policy: AddressPolicy::Anywhere,
        resolver: None,
        smtp_timeout: Some(Duration::from_millis(500)),
    });
    let started = Instant::now();

    let outcome = delivery
        .send(&message(), &smtp_transport(server.port, Security::None))
        .await;

    assert!(matches!(outcome, Outcome::TryAgain { .. }), "{outcome:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn no_server_listening_is_worth_another_try() {
    let outcome = MailDelivery::new()
        .send(&message(), &smtp_transport(free_port(), Security::None))
        .await;

    assert!(matches!(outcome, Outcome::TryAgain { .. }), "{outcome:?}");
}

#[tokio::test]
async fn a_login_is_made_with_the_password_and_a_wrong_one_is_refused() {
    let server = FakeSmtp::start(Behaviour {
        password: Some("right pass"),
        ..Behaviour::default()
    })
    .await;
    let delivery = MailDelivery::new();

    let wrong = delivery
        .send(
            &message(),
            &smtp_transport_to("127.0.0.1", server.port, Security::None, "mailer", "wrong"),
        )
        .await;
    let right = delivery
        .send(
            &message(),
            &smtp_transport_to(
                "127.0.0.1",
                server.port,
                Security::None,
                "mailer",
                "right pass",
            ),
        )
        .await;

    let Outcome::Refused { reason } = wrong else {
        panic!("{wrong:?}")
    };
    assert!(reason.contains("535"), "{reason}");
    assert!(!reason.contains("wrong"), "{reason}");
    assert!(right.is_sent(), "{right:?}");
    assert_eq!(server.seen().mails.len(), 1);
}

#[tokio::test]
async fn a_transport_that_is_not_ready_sends_nothing() {
    let server = FakeSmtp::start(Behaviour::default()).await;
    let mut transport = smtp_transport(server.port, Security::None);
    transport.ready = false;
    let blank_key = MailTransport {
        ready: true,
        ..resend_transport("")
    };

    let delivery = MailDelivery::new();

    assert_eq!(
        delivery.send(&message(), &transport).await,
        Outcome::NotConfigured
    );
    assert_eq!(
        delivery.send(&message(), &blank_key).await,
        Outcome::NotConfigured
    );
    assert_eq!(server.seen().connections, 0);
}

#[tokio::test]
async fn over_tls_from_the_first_byte_the_server_must_be_one_the_roots_trust() {
    let trusted = FakeSmtp::start(Behaviour {
        tls: Some(certificates::server("server")),
        ..Behaviour::default()
    })
    .await;
    let untrusted = FakeSmtp::start(Behaviour {
        tls: Some(certificates::server("untrusted")),
        ..Behaviour::default()
    })
    .await;
    let delivery = MailDelivery::wired(Wiring {
        roots: Some(certificates::roots()),
        resend_url: "http://127.0.0.1:1/emails".into(),
        resend_policy: AddressPolicy::Anywhere,
        resolver: None,
        smtp_timeout: None,
    });

    let good = delivery
        .send(
            &message(),
            &smtp_transport_to("localhost", trusted.port, Security::Tls, "", ""),
        )
        .await;
    let bad = delivery
        .send(
            &message(),
            &smtp_transport_to("localhost", untrusted.port, Security::Tls, "", ""),
        )
        .await;

    assert!(good.is_sent(), "{good:?}");
    assert!(!bad.is_sent(), "{bad:?}");
    assert!(untrusted.mails().is_empty());
}

// ---- Resend ----

fn resend_at(url: &str, policy: AddressPolicy) -> MailDelivery {
    MailDelivery::wired(Wiring {
        roots: None,
        resend_url: url.into(),
        resend_policy: policy,
        // The service's name leads to the stand-in, not to the internet.
        resolver: Some(Arc::new(TableResolver(vec![(
            "api.resend.test".into(),
            vec!["127.0.0.1".parse().unwrap()],
        )]))),
        smtp_timeout: None,
    })
}

async fn resend_answering(status: u16, body: &'static str) -> (FakeResend, MailDelivery) {
    let server = FakeResend::start(Answer { status, body }).await;
    let delivery = resend_at(
        &format!("http://api.resend.test:{}/emails", server.port),
        AddressPolicy::Anywhere,
    );
    (server, delivery)
}

#[tokio::test]
async fn the_message_goes_to_resend_with_the_key_and_an_idempotency_key() {
    let (server, delivery) = resend_answering(200, r#"{"id":"email-42"}"#).await;

    let outcome = delivery
        .send(&message(), &resend_transport(" re_test "))
        .await;

    assert_eq!(
        outcome,
        Outcome::Sent {
            id: "email-42".into()
        }
    );
    let asked = &server.asked()[0];
    assert_eq!(asked.request_line, "POST /emails HTTP/1.1");
    assert_eq!(asked.header("authorization"), Some("Bearer re_test"));
    assert_eq!(asked.header("idempotency-key"), Some("message-1"));
    assert_eq!(asked.header("content-type"), Some("application/json"));
    assert_eq!(asked.header("accept"), Some("application/json"));
    assert_eq!(asked.header("user-agent"), Some("Stellarr/2.0"));
    let body: serde_json::Value = serde_json::from_str(&asked.body).unwrap();
    assert_eq!(body["from"], "Stellarr <noreply@chat.test>");
    assert_eq!(body["to"], serde_json::json!(["jo@example.test"]));
    assert_eq!(body["subject"], "Temat");
    assert_eq!(body["html"], "<p>Treść</p>");
    assert_eq!(body["text"], "Treść");
    assert!(body.get("attachments").is_none());
}

#[tokio::test]
async fn inline_images_go_to_resend_as_attachments_with_their_content_id() {
    let (server, delivery) = resend_answering(200, r#"{"id":"email-42"}"#).await;
    let mut mail = message();
    mail.images.push(InlineImage {
        content_id: "tile@stellarr".into(),
        file_name: "tile.png".into(),
        content_type: "image/png".into(),
        content: vec![1, 2, 3],
    });

    delivery.send(&mail, &resend_transport("re_test")).await;

    let body: serde_json::Value = serde_json::from_str(&server.asked()[0].body).unwrap();
    let attachment = &body["attachments"][0];
    assert_eq!(attachment["content"], "AQID");
    assert_eq!(attachment["filename"], "tile.png");
    assert_eq!(attachment["content_type"], "image/png");
    assert_eq!(attachment["content_id"], "tile@stellarr");
}

#[tokio::test]
async fn a_message_resend_will_not_take_is_refused_with_its_status_and_reason() {
    for status in [400, 401, 403, 404, 422] {
        let (_server, delivery) = resend_answering(status, "invalid from address").await;

        let outcome = delivery
            .send(&message(), &resend_transport("re_test"))
            .await;

        let Outcome::Refused { reason } = outcome else {
            panic!("{status}: {outcome:?}")
        };
        assert!(reason.contains(&status.to_string()), "{reason}");
        assert!(reason.contains("invalid from address"), "{reason}");
    }
}

#[tokio::test]
async fn a_service_that_asks_for_time_or_is_down_is_worth_another_try() {
    for status in [408, 409, 429, 500, 502, 503] {
        let (_server, delivery) = resend_answering(status, "later").await;

        let outcome = delivery
            .send(&message(), &resend_transport("re_test"))
            .await;

        assert!(
            matches!(outcome, Outcome::TryAgain { .. }),
            "{status}: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn an_answer_without_a_message_id_is_a_failure_worth_another_try() {
    for body in ["{}", r#"{"id":7}"#, "not json", ""] {
        let (_server, delivery) = resend_answering(200, body).await;

        let outcome = delivery
            .send(&message(), &resend_transport("re_test"))
            .await;

        assert!(
            matches!(outcome, Outcome::TryAgain { .. }),
            "{body:?}: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn without_an_api_key_nothing_is_sent() {
    let server = FakeResend::start(Answer {
        status: 200,
        body: r#"{"id":"x"}"#,
    })
    .await;
    let delivery = resend_at(
        &format!("http://api.resend.test:{}/emails", server.port),
        AddressPolicy::Anywhere,
    );

    let outcome = delivery.send(&message(), &resend_transport(" ")).await;

    assert_eq!(outcome, Outcome::NotConfigured);
    assert!(server.asked().is_empty());
}

#[tokio::test]
async fn a_service_that_cannot_be_reached_is_worth_another_try() {
    let delivery = resend_at(
        &format!("http://api.resend.test:{}/emails", free_port()),
        AddressPolicy::Anywhere,
    );

    let outcome = delivery
        .send(&message(), &resend_transport("re_test"))
        .await;

    assert!(matches!(outcome, Outcome::TryAgain { .. }), "{outcome:?}");
}

#[tokio::test]
async fn the_service_is_only_ever_at_a_public_address() {
    let server = FakeResend::start(Answer {
        status: 200,
        body: r#"{"id":"x"}"#,
    })
    .await;
    let delivery = resend_at(
        &format!("http://api.resend.test:{}/emails", server.port),
        AddressPolicy::PublicOnly,
    );

    let outcome = delivery
        .send(&message(), &resend_transport("re_test"))
        .await;

    assert!(matches!(outcome, Outcome::Refused { .. }), "{outcome:?}");
    assert!(server.asked().is_empty());
}

#[tokio::test]
async fn the_key_goes_over_tls_to_a_service_the_roots_trust_and_to_nobody_else() {
    let trusted = FakeResend::start_tls(
        Answer {
            status: 200,
            body: r#"{"id":"email-1"}"#,
        },
        certificates::server("server"),
    )
    .await;
    let untrusted = FakeResend::start_tls(
        Answer {
            status: 200,
            body: r#"{"id":"email-2"}"#,
        },
        certificates::server("untrusted"),
    )
    .await;
    let delivery_for = |port: u16| {
        MailDelivery::wired(Wiring {
            roots: Some(certificates::roots()),
            resend_url: format!("https://localhost:{port}/emails"),
            resend_policy: AddressPolicy::Anywhere,
            resolver: None,
            smtp_timeout: None,
        })
    };

    let good = delivery_for(trusted.port)
        .send(&message(), &resend_transport("re_secret"))
        .await;
    let bad = delivery_for(untrusted.port)
        .send(&message(), &resend_transport("re_secret"))
        .await;

    assert_eq!(
        good,
        Outcome::Sent {
            id: "email-1".into()
        }
    );
    assert!(!bad.is_sent(), "{bad:?}");
    assert!(untrusted.asked().is_empty());
    if let Some(reason) = bad.reason() {
        assert!(!reason.contains("re_secret"), "{reason}");
    }
}

#[test]
fn an_outcome_never_prints_more_than_it_was_given() {
    assert!(Outcome::Sent { id: "a".into() }.is_sent());
    assert_eq!(Outcome::Off.reason(), None);
    assert_eq!(Outcome::TryAgain { reason: "r".into() }.reason(), Some("r"));
}

// ---- the whole of sending one message ----

mod deliver {
    use super::*;
    use crate::mail::deliver;
    use crate::mail::tests::support::{Saved, found, instance, resave};

    fn build(mailing: &crate::mail::Mailing) -> Result<Message, stellarr_mail::Error> {
        let mut mail = message();
        mail.from = Mailbox::parse(&mailing.sender).unwrap();
        Ok(mail)
    }

    #[tokio::test]
    async fn mail_that_is_off_is_off_and_nothing_is_built_or_sent() {
        let server = FakeSmtp::start(Behaviour::default()).await;
        let (_folder, state) = instance(&[]);
        found(
            &state,
            Saved {
                port: server.port,
                mailing: false,
                ..Saved::default()
            },
        )
        .await;
        let built = std::cell::Cell::new(false);

        let outcome = deliver(&state, |mailing| {
            built.set(true);
            build(mailing)
        })
        .await
        .unwrap();

        assert_eq!(outcome, Outcome::Off);
        assert!(!built.get());
        assert_eq!(server.seen().connections, 0);
    }

    #[tokio::test]
    async fn mail_that_lacks_something_is_not_configured() {
        let (_folder, state) = instance(&[]);
        let server = found(
            &state,
            Saved {
                host: "",
                ..Saved::default()
            },
        )
        .await;
        assert_eq!(
            deliver(&state, build).await.unwrap(),
            Outcome::NotConfigured
        );

        resave(
            &state,
            server,
            Saved {
                from: "",
                host: "mail.chat.test",
                ..Saved::default()
            },
        )
        .await;
        assert_eq!(
            deliver(&state, build).await.unwrap(),
            Outcome::NotConfigured
        );
    }

    #[tokio::test]
    async fn a_message_is_written_with_what_the_setup_gives_and_sent() {
        let server = FakeSmtp::start(Behaviour::default()).await;
        let (_folder, state) = instance(&[("Mail__FromName", "Orbit")]);
        found(
            &state,
            Saved {
                port: server.port,
                ..Saved::default()
            },
        )
        .await;

        let outcome = deliver(&state, build).await.unwrap();

        assert_eq!(
            outcome,
            Outcome::Sent {
                id: "message-1@stellarr".into()
            }
        );
        assert!(
            server.mails()[0]
                .data
                .contains("From: Orbit <noreply@chat.test>"),
            "{}",
            server.mails()[0].data
        );
    }

    #[tokio::test]
    async fn a_message_that_cannot_be_written_is_refused() {
        let server = FakeSmtp::start(Behaviour::default()).await;
        let (_folder, state) = instance(&[]);
        found(
            &state,
            Saved {
                port: server.port,
                ..Saved::default()
            },
        )
        .await;

        let outcome = deliver(&state, |_| {
            Mailbox::parse("not an address")
                .map(|_| message())
                .map_err(Into::into)
        })
        .await
        .unwrap();

        assert!(matches!(outcome, Outcome::Refused { .. }), "{outcome:?}");
        assert_eq!(server.seen().connections, 0);
    }
}

// ---- nothing secret in the log ----

/// What the log says while `work` runs, at the most detailed level.
async fn logged_during<F: std::future::Future>(work: F) -> (F::Output, String) {
    use std::io::Write;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
        type Writer = Sink;
        fn make_writer(&'a self) -> Sink {
            self.clone()
        }
    }
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(sink.clone())
        .finish();
    // The test runs on one thread, so what the work logs reaches this
    // subscriber and no other test's.
    let _guard = tracing::subscriber::set_default(subscriber);
    let output = work.await;
    let text = String::from_utf8_lossy(&sink.0.lock().unwrap()).into_owned();
    (output, text)
}

#[tokio::test]
async fn neither_the_resend_key_nor_the_smtp_password_nor_the_message_is_logged() {
    let (resend, delivery) = resend_answering(422, "refused for a reason").await;
    let smtp = FakeSmtp::start(Behaviour {
        password: Some("right"),
        ..Behaviour::default()
    })
    .await;
    let mut mail = message();
    mail.text = "a body nobody may read in a log".into();

    let ((by_resend, by_smtp), log) = logged_during(async {
        let by_resend = delivery
            .send(&mail, &resend_transport("re_secret_marker"))
            .await;
        let transport = smtp_transport_to(
            "127.0.0.1",
            smtp.port,
            Security::None,
            "mailer",
            "smtp_secret_marker",
        );
        (by_resend, MailDelivery::new().send(&mail, &transport).await)
    })
    .await;

    assert!(!by_resend.is_sent() && !by_smtp.is_sent());
    assert!(!resend.asked().is_empty());
    for secret in [
        "re_secret_marker",
        "smtp_secret_marker",
        "a body nobody",
        "jo@example.test",
    ] {
        assert!(!log.contains(secret), "the log holds {secret}:\n{log}");
    }
    // What the log does say shows the level was low enough to catch it.
    assert!(!log.is_empty());
}
