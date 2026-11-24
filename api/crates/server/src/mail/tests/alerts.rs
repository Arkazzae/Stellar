use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::support::{
    Behaviour, FakeSmtp, PUBLIC_URL, Saved, at, found, free_port, instance, queue, resave, row,
};
use crate::mail::security_alerts::{BATCH, MAX_ATTEMPTS, dispatch};
use crate::state::AppState;
use crate::values::Timestamp;

const NOW: &str = "2026-01-15 12:30:00";

fn now() -> Timestamp {
    at(NOW)
}

fn later(minutes: u64) -> Timestamp {
    now().plus(Duration::from_secs(minutes * 60))
}

/// An instance whose alerts go to `server`.
async fn alerting_to(
    port: u16,
    extra: &[(&str, &str)],
) -> (crate::testing::TempDir, AppState, crate::values::Id) {
    let (folder, state) = instance(extra);
    let server = found(
        &state,
        Saved {
            port,
            ..Saved::default()
        },
    )
    .await;
    (folder, state, server)
}

#[tokio::test]
async fn a_due_alert_is_sent_and_recorded() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    let sent = dispatch(&state, now()).await.unwrap();

    assert_eq!(sent, 1);
    let mail = &smtp.mails()[0];
    assert!(
        mail.commands
            .contains(&"MAIL FROM:<noreply@chat.test>".to_string()),
        "{:?}",
        mail.commands
    );
    assert!(
        mail.commands
            .contains(&"RCPT TO:<jo@example.test>".to_string())
    );
    assert!(
        mail.data.contains("From: Stellarr <noreply@chat.test>"),
        "{}",
        mail.data
    );
    assert!(mail.data.contains("To: jo@example.test"));
    assert!(mail.data.contains("Subject: "), "{}", mail.data);
    assert!(
        mail.data.contains("Message-ID: <security-event/"),
        "{}",
        mail.data
    );
    let recorded = row(&state, alert).await;
    assert_eq!(recorded.attempts, 1);
    assert_eq!(recorded.sent_at, Some(now()));
    assert_eq!(recorded.last_attempt_at, Some(now()));
    assert_eq!(recorded.last_error, "");
}

#[tokio::test]
async fn the_alert_is_written_in_the_instances_language_and_zone() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state) = instance(&[("STELLARR_LANGUAGE", "pl")]);
    found(
        &state,
        Saved {
            port: smtp.port,
            time_zone: "Europe/Warsaw",
            ..Saved::default()
        },
    )
    .await;
    queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    dispatch(&state, now()).await.unwrap();

    let data = &smtp.mails()[0].data;
    // The subject is a word of Polish, so it travels encoded.
    assert!(data.contains("Subject: =?utf-8?"), "{data}");
    // 12:00 UTC is one o'clock in Warsaw in January.
    assert!(data.contains("13:00 (Europe/Warsaw)"), "{data}");
}

#[tokio::test]
async fn a_zone_the_host_does_not_know_is_utc() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state) = instance(&[]);
    found(
        &state,
        Saved {
            port: smtp.port,
            time_zone: "Nowhere/Land",
            ..Saved::default()
        },
    )
    .await;
    queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    dispatch(&state, now()).await.unwrap();

    assert!(
        smtp.mails()[0].data.contains("12:00 (UTC)"),
        "{}",
        smtp.mails()[0].data
    );
}

#[tokio::test]
async fn alerts_that_are_switched_off_send_nothing_and_wait_an_hour() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, server) = alerting_to(smtp.port, &[]).await;
    resave(
        &state,
        server,
        Saved {
            port: smtp.port,
            alerts: false,
            ..Saved::default()
        },
    )
    .await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    let sent = dispatch(&state, now()).await.unwrap();

    assert_eq!(sent, 0);
    assert_eq!(smtp.seen().connections, 0);
    let recorded = row(&state, alert).await;
    assert_eq!(
        (
            recorded.attempts,
            recorded.sent_at,
            recorded.last_attempt_at
        ),
        (0, None, None)
    );
    assert_eq!(recorded.next_attempt_at, later(60));
}

#[tokio::test]
async fn with_no_saved_settings_alerts_are_off() {
    let (_folder, state) = instance(&[("Mail__Enabled", "true")]);
    state
        .database()
        .write(|tx| {
            tx.execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Orbit', ?2, 1, 0)",
                rusqlite::params![crate::values::Id::random(), Timestamp::now()],
            )
        })
        .await
        .unwrap();
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    assert_eq!(dispatch(&state, now()).await.unwrap(), 0);

    assert_eq!(row(&state, alert).await.next_attempt_at, later(60));
}

#[tokio::test]
async fn mail_that_is_off_or_not_configured_sends_nothing_and_waits_an_hour() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    for saved in [
        Saved {
            port: smtp.port,
            mailing: false,
            ..Saved::default()
        },
        Saved {
            port: smtp.port,
            host: "",
            ..Saved::default()
        },
        Saved {
            port: smtp.port,
            from: "",
            ..Saved::default()
        },
    ] {
        let (_folder, state) = instance(&[("Mail__FromAddress", "")]);
        found(&state, saved).await;
        let alert = queue(
            &state,
            "Jo",
            "jo@example.test",
            "login-succeeded",
            at("2026-01-15 12:00:00"),
        )
        .await;

        assert_eq!(dispatch(&state, now()).await.unwrap(), 0);

        let recorded = row(&state, alert).await;
        assert_eq!((recorded.attempts, recorded.sent_at), (0, None));
        assert_eq!(recorded.next_attempt_at, later(60));
    }
    assert_eq!(smtp.seen().connections, 0);
}

#[tokio::test]
async fn a_refusing_server_is_a_failed_try_with_what_it_said_and_a_longer_wait_each_time() {
    let smtp = FakeSmtp::start(Behaviour {
        recipient_reply: "550 5.1.1 No such mailbox",
        ..Behaviour::default()
    })
    .await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    assert_eq!(dispatch(&state, now()).await.unwrap(), 0);
    let first = row(&state, alert).await;
    assert_eq!(dispatch(&state, later(5)).await.unwrap(), 0);
    let second = row(&state, alert).await;

    assert_eq!((first.attempts, first.sent_at), (1, None));
    assert!(
        first.last_error.contains("No such mailbox"),
        "{}",
        first.last_error
    );
    assert_eq!(first.last_attempt_at, Some(now()));
    assert_eq!(first.next_attempt_at, later(5));
    assert_eq!(second.attempts, 2);
    assert_eq!(second.next_attempt_at, later(15));
}

#[tokio::test]
async fn the_wait_grows_by_five_minutes_a_try_up_to_an_hour() {
    let smtp = FakeSmtp::start(Behaviour {
        recipient_reply: "550 no",
        ..Behaviour::default()
    })
    .await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;
    let mut waits = Vec::new();

    for tries in 1..=MAX_ATTEMPTS {
        let due = row(&state, alert).await.next_attempt_at;
        dispatch(&state, due).await.unwrap();
        let recorded = row(&state, alert).await;
        assert_eq!(recorded.attempts, tries);
        waits.push(due.until(recorded.next_attempt_at).as_secs() / 60);
    }

    assert_eq!(waits[..4], [5, 10, 15, 20]);
    assert_eq!(waits[11], 60);
    assert!(waits[11..].iter().all(|wait| *wait == 60));
}

#[tokio::test]
async fn a_row_that_has_been_tried_twenty_times_is_left_alone() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;
    state
        .database()
        .write(move |tx| {
            tx.execute(
                "UPDATE \"SecurityNotifications\" SET \"Attempts\" = 20 WHERE \"Id\" = ?1",
                [alert],
            )
        })
        .await
        .unwrap();

    assert_eq!(dispatch(&state, later(600)).await.unwrap(), 0);

    assert_eq!(smtp.seen().connections, 0);
    let recorded = row(&state, alert).await;
    assert_eq!(
        (
            recorded.attempts,
            recorded.sent_at,
            recorded.last_attempt_at
        ),
        (20, None, None)
    );
}

#[tokio::test]
async fn rows_that_are_not_due_or_already_sent_are_not_touched() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let future = queue(&state, "Jo", "jo@example.test", "login-succeeded", later(1)).await;
    let done = queue(
        &state,
        "Al",
        "al@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;
    state
        .database()
        .write(move |tx| {
            tx.execute(
                "UPDATE \"SecurityNotifications\" SET \"SentAt\" = \"CreatedAt\" WHERE \"Id\" = ?1",
                [done],
            )
        })
        .await
        .unwrap();

    assert_eq!(dispatch(&state, now()).await.unwrap(), 0);

    assert_eq!(smtp.seen().connections, 0);
    assert_eq!(row(&state, future).await.attempts, 0);
    assert_eq!(row(&state, done).await.attempts, 0);
}

#[tokio::test]
async fn one_refused_alert_does_not_hold_back_the_others() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let first = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;
    // An address the message cannot be written for.
    let broken = queue(
        &state,
        "Al",
        "not an address",
        "login-succeeded",
        at("2026-01-15 12:01:00"),
    )
    .await;
    let last = queue(
        &state,
        "Bo",
        "bo@example.test",
        "password-changed",
        at("2026-01-15 12:02:00"),
    )
    .await;

    let sent = dispatch(&state, now()).await.unwrap();

    assert_eq!(sent, 2);
    assert!(row(&state, first).await.sent_at.is_some());
    assert!(row(&state, last).await.sent_at.is_some());
    let failed = row(&state, broken).await;
    assert_eq!((failed.attempts, failed.sent_at), (1, None));
    assert!(!failed.last_error.is_empty());
    assert_eq!(failed.next_attempt_at, later(5));
}

#[tokio::test]
async fn a_batch_is_the_oldest_twenty_five() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let mut alerts = Vec::new();
    for index in 0..BATCH + 3 {
        let due = at("2026-01-15 11:00:00").plus(Duration::from_secs(index as u64));
        alerts.push(queue(&state, "Jo", "jo@example.test", "login-succeeded", due).await);
    }

    assert_eq!(dispatch(&state, now()).await.unwrap(), BATCH);
    assert_eq!(dispatch(&state, now()).await.unwrap(), 3);

    for alert in alerts {
        assert!(row(&state, alert).await.sent_at.is_some());
    }
}

#[tokio::test]
async fn a_server_that_is_not_there_is_retried_and_the_alert_goes_once_when_it_is_back() {
    let port = free_port();
    let (_folder, state, _) = alerting_to(port, &[]).await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    assert_eq!(dispatch(&state, now()).await.unwrap(), 0);
    let failed = row(&state, alert).await;
    assert_eq!((failed.attempts, failed.sent_at), (1, None));
    assert!(!failed.last_error.is_empty());
    assert_eq!(failed.next_attempt_at, later(5));
    // Not yet due: nothing is tried.
    assert_eq!(dispatch(&state, later(4)).await.unwrap(), 0);
    assert_eq!(row(&state, alert).await.attempts, 1);

    let smtp = FakeSmtp::start_on(port, Behaviour::default()).await;
    assert_eq!(dispatch(&state, later(5)).await.unwrap(), 1);
    assert_eq!(dispatch(&state, later(60)).await.unwrap(), 0);

    assert_eq!(smtp.mails().len(), 1);
    let sent = row(&state, alert).await;
    assert_eq!(
        (sent.attempts, sent.sent_at, sent.last_error.as_str()),
        (2, Some(later(5)), "")
    );
}

#[tokio::test]
async fn a_long_error_is_cut_to_five_hundred_characters() {
    let smtp = FakeSmtp::start(Behaviour {
        recipient_reply: Box::leak(format!("550 {}", "x".repeat(900)).into_boxed_str()),
        ..Behaviour::default()
    })
    .await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    dispatch(&state, now()).await.unwrap();

    assert_eq!(row(&state, alert).await.last_error.chars().count(), 500);
}

#[tokio::test]
async fn the_password_set_in_the_panel_is_what_the_server_is_logged_in_with() {
    let smtp = FakeSmtp::start(Behaviour {
        password: Some("panel pass "),
        ..Behaviour::default()
    })
    .await;
    let (_folder, state) = instance(&[("STELLARR_SMTP_PASSWORD", "environment")]);
    found(
        &state,
        Saved {
            port: smtp.port,
            username: "mailer",
            ..Saved::default()
        },
    )
    .await;
    let sealer = state.sealer().clone();
    state
        .database()
        .write(move |tx| {
            crate::secrets::set(tx, &sealer, &crate::secrets::SMTP_PASSWORD, "panel pass ")
        })
        .await
        .unwrap();
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    assert_eq!(dispatch(&state, now()).await.unwrap(), 1);

    assert_eq!(
        smtp.seen().logins,
        [("mailer".to_string(), "panel pass ".to_string())]
    );
    assert!(row(&state, alert).await.sent_at.is_some());
}

#[tokio::test]
async fn a_wrong_password_is_a_failed_try_that_does_not_show_it() {
    let smtp = FakeSmtp::start(Behaviour {
        password: Some("right"),
        ..Behaviour::default()
    })
    .await;
    let (_folder, state) = instance(&[("STELLARR_SMTP_PASSWORD", "wrong-password-marker")]);
    found(
        &state,
        Saved {
            port: smtp.port,
            username: "mailer",
            ..Saved::default()
        },
    )
    .await;
    let alert = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;

    assert_eq!(dispatch(&state, now()).await.unwrap(), 0);

    let failed = row(&state, alert).await;
    assert!(failed.last_error.contains("535"), "{}", failed.last_error);
    assert!(!failed.last_error.contains("wrong-password-marker"));
}

#[tokio::test]
async fn the_worker_is_told_to_stop_between_messages() {
    let smtp = FakeSmtp::start(Behaviour::default()).await;
    let (_folder, state, _) = alerting_to(smtp.port, &[]).await;
    let first = queue(
        &state,
        "Jo",
        "jo@example.test",
        "login-succeeded",
        at("2026-01-15 12:00:00"),
    )
    .await;
    let second = queue(
        &state,
        "Al",
        "al@example.test",
        "login-succeeded",
        at("2026-01-15 12:01:00"),
    )
    .await;
    let asked = AtomicUsize::new(0);

    // Stopping is noticed before the second message: the first one is sent
    // and recorded, the second is left for the next start.
    let sent = crate::mail::security_alerts::run(
        &state,
        || asked.fetch_add(1, Ordering::SeqCst) >= 1,
        now(),
    )
    .await
    .unwrap();

    assert_eq!(sent, 1);
    assert!(row(&state, first).await.sent_at.is_some());
    let waiting = row(&state, second).await;
    assert_eq!((waiting.attempts, waiting.sent_at), (0, None));
    assert_eq!(smtp.mails().len(), 1);
    let _ = PUBLIC_URL;
}
