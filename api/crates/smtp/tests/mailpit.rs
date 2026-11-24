//! Every kind of message, sent to a real server and read back from it.
//!
//! The server is Mailpit, which keeps what it receives and shows it over
//! HTTP. These tests are ignored by default because they need containers;
//! `tests/mailpit.sh` starts them, runs the tests and removes them.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use stellarr_smtp::{Connection, ErrorKind, Options, Security, Stage, send};

const CA: &[u8] = include_bytes!("fixtures/ca.der");
const OTHER_CA: &[u8] = include_bytes!("fixtures/other-ca.der");
const USER: &str = "jo";
const PASSWORD: &str = "s3cr3t-pässword";

fn tls(ca: &[u8]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca.to_vec())).unwrap();
    Arc::new(
        ClientConfig::builder_with_provider(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        )
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}

/// `(smtp port, http port)` of a container, from the environment the script
/// sets.
fn ports(mode: &str) -> (u16, u16) {
    let var = format!("STELLARR_TEST_MAILPIT_{}", mode.to_uppercase());
    let value =
        std::env::var(&var).unwrap_or_else(|_| panic!("{var} is not set: run tests/mailpit.sh"));
    let (smtp, http) = value.split_once(',').expect("smtp,http");
    (smtp.parse().unwrap(), http.parse().unwrap())
}

fn connection(mode: &str, security: Security, login: bool) -> Connection {
    Connection {
        host: "localhost".into(),
        port: ports(mode).0,
        security,
        username: if login { USER.into() } else { String::new() },
        password: if login {
            PASSWORD.into()
        } else {
            String::new()
        },
    }
}

fn options() -> Options {
    Options {
        step_timeout: Duration::from_secs(10),
        total_timeout: Duration::from_secs(30),
        local_name: None,
    }
}

fn work_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smtp-mailpit");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
#[ignore = "needs the containers started by tests/mailpit.sh"]
async fn every_kind_of_message_arrives_intact_in_every_security_mode() {
    let dir = work_dir();
    for (mode, security, login) in [
        ("plain", Security::None, false),
        ("starttls", Security::StartTls, true),
        ("tls", Security::Tls, true),
    ] {
        let mode_dir = dir.join(mode);
        fs::create_dir_all(&mode_dir).unwrap();
        fs::write(
            mode_dir.join("http"),
            format!("127.0.0.1:{}", ports(mode).1),
        )
        .unwrap();
        for sample in common::samples() {
            let delivery = send(
                &connection(mode, security, login),
                tls(CA),
                &options(),
                &sample.message,
            )
            .await
            .unwrap_or_else(|e| panic!("{mode}/{}: {e}", sample.name));
            assert_eq!(delivery.message_id, sample.message.message_id);
            fs::write(
                mode_dir.join(format!("{}.eml", sample.name)),
                sample.message.encode().unwrap().bytes,
            )
            .unwrap();
            common::write_expectations(&sample, &mode_dir.join(format!("{}.expect", sample.name)));
        }
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/verify_mailpit.py");
    let output = Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(&dir)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stdout);
    eprintln!("{report}");
    assert!(
        output.status.success(),
        "{report}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "needs the containers started by tests/mailpit.sh"]
async fn failures_against_a_real_server_are_told_apart() {
    let message = &common::samples()[0].message;

    // A wrong password.
    let mut wrong = connection("starttls", Security::StartTls, true);
    wrong.password = "wrong".into();
    let error = send(&wrong, tls(CA), &options(), message)
        .await
        .unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Rejected, Stage::Auth),
        "{error}"
    );
    assert!(!error.is_transient(), "{error}");
    eprintln!("wrong password: {error}");

    // No login at all on a server that wants one.
    let error = send(
        &connection("starttls", Security::StartTls, false),
        tls(CA),
        &options(),
        message,
    )
    .await
    .unwrap_err();
    assert!(!error.is_transient(), "{error}");
    eprintln!("no login: {error}");

    // A certificate the client does not trust.
    let error = send(
        &connection("starttls", Security::StartTls, true),
        tls(OTHER_CA),
        &options(),
        message,
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Tls, Stage::Handshake),
        "{error}"
    );
    assert!(!error.is_transient());
    eprintln!("untrusted: {error}");

    // STARTTLS asked of a server that does not offer it.
    let error = send(
        &connection("plain", Security::StartTls, false),
        tls(CA),
        &options(),
        message,
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::StartTls),
        "{error}"
    );
    eprintln!("no starttls: {error}");

    // TLS from the first byte on a port that speaks plain SMTP: the server
    // waits for the client to speak first, so this ends at the timeout.
    let quick = Options {
        step_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(3),
        local_name: None,
    };
    let error = send(
        &connection("plain", Security::Tls, false),
        tls(CA),
        &quick,
        message,
    )
    .await
    .unwrap_err();
    assert_eq!(error.stage(), Stage::Handshake, "{error}");
    eprintln!("tls on a plain port: {error}");

    // Nothing is listening: a connect error, which may be retried.
    let closed = Connection {
        host: "127.0.0.1".into(),
        port: 1,
        security: Security::None,
        username: String::new(),
        password: String::new(),
    };
    let error = send(&closed, tls(CA), &options(), message)
        .await
        .unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Connect, Stage::Connect),
        "{error}"
    );
    assert!(error.is_transient());
    eprintln!("closed port: {error}");
}
