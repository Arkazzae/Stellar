//! Every kind of message, built here, sent with the SMTP client to a real
//! server and read back from it. The server is Mailpit, which keeps what it
//! receives and shows it over HTTP.
//!
//! The test is ignored by default because it needs the container. Start one,
//! for example `docker run -d -p 127.0.0.1:1025:1025 -p 127.0.0.1:8025:8025
//! axllent/mailpit`, and run
//! `STELLARR_TEST_MAILPIT=1025,8025 cargo test -p stellarr-mail --test mailpit -- --ignored`.

use std::sync::Arc;
use std::time::Duration;

use rustls::{ClientConfig, RootCertStore};
use serde_json::Value;
use stellarr_mail::{
    MailSettings, MailTransport, Message, SecurityEvent, delivery_test, direct_message,
    email_change, email_code, event_kinds, mention, password_reset, security_alert,
};
use stellarr_messages::{Language, Messages};
use stellarr_smtp::{Connection, Options, Security, send};
use stellarr_timezone::TimeZone;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const FIREFOX: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0";

fn ports() -> (u16, u16) {
    let value = std::env::var("STELLARR_TEST_MAILPIT").expect(
        "STELLARR_TEST_MAILPIT is not set: it is the SMTP and the HTTP port, comma between",
    );
    let (smtp, http) = value.split_once(',').expect("smtp,http");
    (smtp.parse().unwrap(), http.parse().unwrap())
}

/// One HTTP request to the catcher; the answer's body.
async fn http(port: u16, method: &str, path: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    let split = answer.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&answer[..split]).to_lowercase();
    let body = answer[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        return unchunk(&body);
    }
    body
}

fn unchunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n").unwrap();
        let size =
            usize::from_str_radix(std::str::from_utf8(&body[..line_end]).unwrap().trim(), 16)
                .unwrap();
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&body[line_end + 2..line_end + 2 + size]);
        body = &body[line_end + 2 + size + 2..];
    }
}

async fn json(port: u16, path: &str) -> Value {
    serde_json::from_slice(&http(port, "GET", path).await).unwrap()
}

fn all_kinds(messages: &Messages) -> Vec<Message> {
    let from = "Stellarr <noreply@chat.test>";
    let to = "jo@example.test";
    let url = "https://chat.test";
    let transport = MailTransport::resolve(
        Some(&MailSettings {
            smtp_host: "mail.chat.test".into(),
            ..MailSettings::default()
        }),
        "",
        "",
    );
    vec![
        password_reset(
            from,
            to,
            "Zoë <Jo>",
            "Orbit & Co",
            "https://chat.test/reset?token=a&b=c",
            1,
            messages,
        )
        .unwrap(),
        email_change(
            from,
            to,
            "Jo",
            "Orbit",
            "https://chat.test/confirm?token=a",
            2,
            messages,
        )
        .unwrap(),
        email_code(from, to, "Jo", "Orbit", url, "123456", 8, 3, messages).unwrap(),
        security_alert(
            from,
            to,
            "Jo",
            "Orbit",
            url,
            &SecurityEvent {
                id: 4,
                kind: event_kinds::LOGIN_SUCCEEDED,
                ip_address: "203.0.113.42",
                device: FIREFOX,
                created_at: 1_795_179_900,
            },
            &TimeZone::utc(),
            messages,
        )
        .unwrap(),
        delivery_test(from, to, "Orbit", url, &transport, 5, messages).unwrap(),
        mention(from, to, url, 6, messages).unwrap(),
        direct_message(from, to, url, 7, messages).unwrap(),
    ]
}

#[tokio::test]
#[ignore = "needs a Mailpit container, see the top of the file"]
async fn every_kind_of_message_arrives_intact() {
    let (smtp, api) = ports();
    let tls = Arc::new(
        ClientConfig::builder_with_provider(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        )
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth(),
    );
    let connection = Connection {
        host: "127.0.0.1".into(),
        port: smtp,
        security: Security::None,
        username: String::new(),
        password: String::new(),
    };
    let options = Options {
        step_timeout: Duration::from_secs(10),
        total_timeout: Duration::from_secs(30),
        local_name: None,
    };
    http(api, "DELETE", "/api/v1/messages").await;

    let mut sent = Vec::new();
    for language in [Language::English, Language::Polish] {
        for mut mail in all_kinds(&Messages::new(language)) {
            // The same key in both languages would be the same message.
            mail.message_id = format!("{}-{}", language.code(), mail.message_id);
            let delivery = send(&connection, tls.clone(), &options, &mail)
                .await
                .unwrap();
            assert_eq!(delivery.message_id, mail.message_id);
            sent.push(mail);
        }
    }

    let list = json(api, "/api/v1/messages?limit=100").await;
    assert_eq!(list["total"].as_u64().unwrap(), sent.len() as u64);
    for mail in &sent {
        let summary = list["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["MessageID"] == mail.message_id.as_str())
            .unwrap_or_else(|| panic!("{} did not arrive", mail.message_id));
        let read = json(
            api,
            &format!("/api/v1/message/{}", summary["ID"].as_str().unwrap()),
        )
        .await;
        let label = &mail.message_id;

        assert_eq!(read["Subject"], mail.subject.as_str(), "{label}");
        assert_eq!(
            read["Text"].as_str().unwrap().replace("\r\n", "\n"),
            mail.text,
            "{label}"
        );
        assert_eq!(
            read["HTML"].as_str().unwrap().replace("\r\n", "\n"),
            mail.html,
            "{label}"
        );
        assert_eq!(read["From"]["Address"], mail.from.address(), "{label}");
        assert_eq!(read["From"]["Name"], mail.from.name().unwrap(), "{label}");
        assert_eq!(read["To"][0]["Address"], mail.to[0].address(), "{label}");
        let inline = read["Inline"].as_array().unwrap();
        assert_eq!(inline.len(), mail.images.len(), "{label}");
        for image in &mail.images {
            let part = inline
                .iter()
                .find(|p| p["ContentID"] == image.content_id.as_str())
                .unwrap_or_else(|| panic!("{label}: no {}", image.content_id));
            assert_eq!(part["FileName"], image.file_name.as_str());
            assert_eq!(part["ContentType"], image.content_type.as_str());
            let bytes = http(
                api,
                "GET",
                &format!(
                    "/api/v1/message/{}/part/{}",
                    summary["ID"].as_str().unwrap(),
                    part["PartID"].as_str().unwrap()
                ),
            )
            .await;
            assert_eq!(bytes, image.content, "{label}: {}", image.content_id);
        }
    }
}
