use stellarr_mail::Provider;

use super::support::{PUBLIC_URL, Saved, found, instance, resave};
use crate::mail::setup::{self, MailSetup, Mailing, Switches};
use crate::secrets;

async fn find(state: &crate::state::AppState) -> MailSetup {
    let reading = state.clone();
    state
        .database()
        .read(move |db| setup::find(db, reading.settings(), reading.sealer()))
        .await
        .unwrap()
}

fn ready(setup: MailSetup) -> Mailing {
    match setup {
        MailSetup::Ready(mailing) => *mailing,
        other => panic!("mail should be ready, but is {other:?}"),
    }
}

#[tokio::test]
async fn an_instance_without_a_server_is_not_configured() {
    let (_folder, state) = instance(&[]);

    assert_eq!(find(&state).await, MailSetup::NotConfigured);
}

#[tokio::test]
async fn without_saved_settings_the_environment_decides_whether_mail_is_on() {
    let (_folder, off) = instance(&[("Mail__Enabled", "false")]);
    found_without_settings(&off).await;
    let (_folder, on) = instance(&[
        ("Mail__Enabled", "true"),
        ("Mail__FromAddress", "env@chat.test"),
        ("STELLARR_SMTP_PASSWORD", "x"),
    ]);
    found_without_settings(&on).await;

    assert_eq!(find(&off).await, MailSetup::Off);
    // On, with a sender, and nothing to send by: no SMTP host, no key.
    assert_eq!(find(&on).await, MailSetup::NotConfigured);
}

async fn found_without_settings(state: &crate::state::AppState) {
    state
        .database()
        .write(|tx| {
            tx.execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Orbit', ?2, 1, 0)",
                rusqlite::params![crate::values::Id::random(), crate::values::Timestamp::now()],
            )
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn the_saved_switch_wins_over_the_environments() {
    let (_folder, state) = instance(&[("Mail__Enabled", "true")]);
    let server = found(
        &state,
        Saved {
            mailing: false,
            host: "mail.chat.test",
            ..Saved::default()
        },
    )
    .await;

    assert_eq!(find(&state).await, MailSetup::Off);

    resave(
        &state,
        server,
        Saved {
            host: "mail.chat.test",
            ..Saved::default()
        },
    )
    .await;
    assert!(matches!(find(&state).await, MailSetup::Ready(_)));
}

#[tokio::test]
async fn a_ready_setup_names_the_sender_the_instance_and_the_transport() {
    let (_folder, state) = instance(&[("Mail__FromName", " Orbit Chat ")]);
    found(
        &state,
        Saved {
            host: " mail.chat.test ",
            port: 465,
            security: "TLS",
            username: " mailer ",
            from: " noreply@chat.test ",
            time_zone: "Europe/Warsaw",
            ..Saved::default()
        },
    )
    .await;

    let mailing = ready(find(&state).await);

    assert_eq!(mailing.sender, "Orbit Chat <noreply@chat.test>");
    assert_eq!(mailing.public_url, PUBLIC_URL);
    assert_eq!(mailing.instance_name, "Orbit");
    assert_eq!(mailing.time_zone, "Europe/Warsaw");
    assert_eq!(mailing.transport.provider, Provider::Smtp);
    let smtp = mailing.transport.smtp.unwrap();
    assert_eq!(
        (smtp.host.as_str(), smtp.port, smtp.username.as_str()),
        ("mail.chat.test", 465, "mailer")
    );
    assert_eq!(
        mailing.switches,
        Switches {
            invitations: true,
            recovery: true,
            alerts: true
        }
    );
}

#[tokio::test]
async fn a_sender_without_a_name_is_the_bare_address() {
    let (_folder, state) = instance(&[("Mail__FromName", "  ")]);
    found(
        &state,
        Saved {
            host: "mail.chat.test",
            ..Saved::default()
        },
    )
    .await;

    assert_eq!(ready(find(&state).await).sender, "noreply@chat.test");
}

#[tokio::test]
async fn the_environments_sender_stands_in_for_one_the_operator_never_saved() {
    let (_folder, state) = instance(&[
        ("Mail__FromAddress", "env@chat.test"),
        ("Mail__FromName", ""),
    ]);
    found(
        &state,
        Saved {
            from: "  ",
            host: "mail.chat.test",
            ..Saved::default()
        },
    )
    .await;

    assert_eq!(ready(find(&state).await).sender, "env@chat.test");
}

#[tokio::test]
async fn mail_is_not_configured_without_a_sender_an_address_or_a_transport() {
    let (_folder, no_sender) = instance(&[("Mail__FromAddress", "")]);
    found(
        &no_sender,
        Saved {
            from: "",
            host: "mail.chat.test",
            ..Saved::default()
        },
    )
    .await;
    let (_folder, no_address) = instance(&[("Operations__PublicUrl", "")]);
    found(
        &no_address,
        Saved {
            host: "mail.chat.test",
            ..Saved::default()
        },
    )
    .await;
    let (_folder, no_host) = instance(&[]);
    found(
        &no_host,
        Saved {
            host: "",
            ..Saved::default()
        },
    )
    .await;

    for state in [no_sender, no_address, no_host] {
        assert_eq!(find(&state).await, MailSetup::NotConfigured);
    }
}

#[tokio::test]
async fn a_resend_key_set_in_the_panel_makes_resend_the_transport() {
    let (_folder, state) = instance(&[]);
    found(
        &state,
        Saved {
            host: "",
            resend: None,
            ..Saved::default()
        },
    )
    .await;
    assert_eq!(find(&state).await, MailSetup::NotConfigured);
    let sealer = state.sealer().clone();
    state
        .database()
        .write(move |tx| secrets::set(tx, &sealer, &secrets::RESEND_API_KEY, " re_panel "))
        .await
        .unwrap();

    let mailing = ready(find(&state).await);

    assert_eq!(mailing.transport.provider, Provider::Resend);
    assert_eq!(mailing.transport.resend_api_key, "re_panel");
}

#[tokio::test]
async fn the_smtp_password_set_in_the_panel_goes_with_the_transport() {
    let (_folder, state) = instance(&[("STELLARR_SMTP_PASSWORD", "from-the-environment")]);
    found(
        &state,
        Saved {
            host: "mail.chat.test",
            username: "mailer",
            ..Saved::default()
        },
    )
    .await;
    let from_environment = ready(find(&state).await).transport.smtp.unwrap().password;
    let sealer = state.sealer().clone();
    state
        .database()
        .write(move |tx| secrets::set(tx, &sealer, &secrets::SMTP_PASSWORD, "from-the-panel"))
        .await
        .unwrap();

    let from_panel = ready(find(&state).await).transport.smtp.unwrap().password;

    assert_eq!(from_environment, "from-the-environment");
    assert_eq!(from_panel, "from-the-panel");
}

#[tokio::test]
async fn a_password_that_cannot_be_opened_leaves_mail_working_without_it() {
    let (_folder, state) = instance(&[]);
    found(
        &state,
        Saved {
            host: "mail.chat.test",
            username: "mailer",
            ..Saved::default()
        },
    )
    .await;
    state
        .database()
        .write(|tx| {
            tx.execute(
                "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES ('mail.smtp-password', 'CfDJ8-from-the-first-server', ?1)",
                [crate::values::Timestamp::now()],
            )
        })
        .await
        .unwrap();

    let smtp = ready(find(&state).await).transport.smtp.unwrap();

    assert_eq!(smtp.password, "");
}

#[tokio::test]
async fn the_switches_default_to_invitations_and_recovery_on_and_alerts_off() {
    let (_folder, state) = instance(&[]);
    let read = |state: crate::state::AppState| async move {
        state.database().read(setup::switches).await.unwrap()
    };

    assert_eq!(
        read(state.clone()).await,
        Switches {
            invitations: true,
            recovery: true,
            alerts: false
        }
    );
    found(
        &state,
        Saved {
            alerts: false,
            ..Saved::default()
        },
    )
    .await;
    assert!(!read(state.clone()).await.alerts);
}
