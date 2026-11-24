//! The client over a real TCP connection to a small server of its own, as the
//! application uses it: `send` connects, so these cover what the in-memory
//! script cannot, the connecting.

use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use rustls::{ClientConfig, RootCertStore};
use stellarr_smtp::{
    Connection, ErrorKind, InlineImage, Mailbox, Message, Options, Security, Stage, send,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

fn tls() -> Arc<ClientConfig> {
    Arc::new(
        ClientConfig::builder_with_provider(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        )
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth(),
    )
}

fn message() -> Message {
    Message {
        from: Mailbox::parse("Stellarr <sender@example.net>").unwrap(),
        to: vec![Mailbox::parse("person@example.net").unwrap()],
        subject: "Test from Orbit".into(),
        html: "<p>Hello</p>".into(),
        text: "Hello".into(),
        message_id: "msg-1@stellarr".into(),
        date: UNIX_EPOCH + Duration::from_secs(1_795_000_000),
        images: vec![],
    }
}

fn connection(port: u16) -> Connection {
    Connection {
        host: "127.0.0.1".into(),
        port,
        security: Security::None,
        username: String::new(),
        password: String::new(),
    }
}

#[derive(Default)]
struct Seen {
    commands: Vec<String>,
    data: String,
}

/// A plain SMTP server for one connection: it answers each command, keeps the
/// commands and the message, and can refuse the recipient or stay silent.
async fn server(
    recipient_reply: &'static str,
    silent: bool,
) -> (u16, tokio::task::JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let mut seen = Seen::default();
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        if silent {
            tokio::time::sleep(Duration::from_secs(60)).await;
            return seen;
        }
        write.write_all(b"220 fake.test ESMTP\r\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            seen.commands.push(line.clone());
            let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
            match verb.as_str() {
                "EHLO" => write
                    .write_all(b"250-fake.test\r\n250 8BITMIME\r\n")
                    .await
                    .unwrap(),
                "RCPT" => write
                    .write_all(format!("{recipient_reply}\r\n").as_bytes())
                    .await
                    .unwrap(),
                "DATA" => {
                    write
                        .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                        .await
                        .unwrap();
                    while let Ok(Some(body)) = lines.next_line().await {
                        if body == "." {
                            break;
                        }
                        seen.data.push_str(&body);
                        seen.data.push('\n');
                    }
                    write.write_all(b"250 2.0.0 Queued\r\n").await.unwrap();
                }
                "QUIT" => {
                    write.write_all(b"221 2.0.0 Bye\r\n").await.unwrap();
                    break;
                }
                _ => write.write_all(b"250 OK\r\n").await.unwrap(),
            }
        }
        seen
    });
    (port, handle)
}

#[tokio::test]
async fn the_message_goes_to_the_server_with_its_envelope_and_body() {
    let (port, seen) = server("250 2.1.5 OK", false).await;
    let delivery = send(&connection(port), tls(), &Options::default(), &message())
        .await
        .unwrap();
    let seen = seen.await.unwrap();
    assert_eq!(delivery.message_id, "msg-1@stellarr");
    assert!(
        seen.commands
            .contains(&"MAIL FROM:<sender@example.net>".to_string())
    );
    assert!(
        seen.commands
            .contains(&"RCPT TO:<person@example.net>".to_string())
    );
    assert!(seen.data.contains("Subject: Test from Orbit"));
    assert!(seen.data.contains("Message-ID: <msg-1@stellarr>"));
    assert!(seen.commands.contains(&"QUIT".to_string()));
    // Without a name of its own the client announces its address.
    assert!(
        seen.commands
            .iter()
            .any(|c| c.starts_with("EHLO [127.0.0.1]")),
        "{:?}",
        seen.commands
    );
}

#[tokio::test]
async fn inline_images_travel_next_to_the_html_with_their_content_id() {
    let (port, seen) = server("250 2.1.5 OK", false).await;
    let mut message = message();
    message.html = r#"<img src="cid:tile@stellarr">"#.into();
    message.images = vec![InlineImage {
        content_id: "tile@stellarr".into(),
        file_name: "tile.png".into(),
        content_type: "image/png".into(),
        content: vec![1, 2, 3],
    }];
    send(&connection(port), tls(), &Options::default(), &message)
        .await
        .unwrap();
    let data = seen.await.unwrap().data;
    assert!(data.contains("multipart/related"));
    assert!(data.contains("Content-Id: <tile@stellarr>"));
    assert!(data.contains("image/png"));
    assert!(data.contains("AQID"));
}

#[tokio::test]
async fn a_refused_recipient_is_reported_with_the_servers_reply_and_the_endpoint() {
    let (port, _seen) = server("550 5.1.1 No such mailbox", false).await;
    let error = send(&connection(port), tls(), &Options::default(), &message())
        .await
        .unwrap_err();
    let shown = error.to_string();
    assert!(shown.contains("No such mailbox"), "{shown}");
    assert!(shown.contains(&format!("127.0.0.1:{port}")), "{shown}");
}

#[tokio::test]
async fn a_server_that_never_answers_fails_within_the_timeout() {
    let (port, _seen) = server("250 OK", true).await;
    let options = Options {
        step_timeout: Duration::from_millis(500),
        total_timeout: Duration::from_millis(500),
        local_name: None,
    };
    let started = Instant::now();
    let error = send(&connection(port), tls(), &options, &message())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn no_server_listening_is_a_connect_failure_that_may_be_retried() {
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let error = send(&connection(port), tls(), &Options::default(), &message())
        .await
        .unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Connect, Stage::Connect)
    );
    assert!(error.is_transient());
}

#[tokio::test]
async fn a_name_that_does_not_resolve_is_a_connect_failure_too() {
    let mut nowhere = connection(25);
    nowhere.host = "no-such-host.invalid".into();
    let error = send(&nowhere, tls(), &Options::default(), &message())
        .await
        .unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Connect, Stage::Connect)
    );
    let mut empty = connection(25);
    empty.host = " ".into();
    let error = send(&empty, tls(), &Options::default(), &message())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Connect);
    assert!(!error.is_transient());
}
