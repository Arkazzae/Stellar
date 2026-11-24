//! The client against a scripted server held in memory: what it says, what it
//! does with each answer, and every failure it reports.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use aws_lc_rs::{digest, hmac, pbkdf2};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use stellarr_smtp::{
    Connection, Delivery, Error, ErrorKind, InlineImage, Mailbox, Message, Options, Security,
    Stage, send_over,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;

const CA: &[u8] = include_bytes!("fixtures/ca.der");
const OTHER_CA: &[u8] = include_bytes!("fixtures/other-ca.der");
const SERVER: &[u8] = include_bytes!("fixtures/server.der");
const SERVER_KEY: &[u8] = include_bytes!("fixtures/server-key.der");

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

#[derive(Clone)]
enum Step {
    /// A line the server sends (a CRLF is added; put more in the text for a
    /// reply of several lines).
    Say(&'static str),
    /// A line the client must send, exactly.
    Expect(&'static str),
    /// A line the client must send, as far as its start.
    ExpectStart(&'static str),
    /// The message, up to the lone dot.
    Data,
    /// Bytes as they are.
    Raw(Vec<u8>),
    /// Turn the connection into TLS, as the server.
    Tls,
    /// Say nothing more, for a long time.
    Hang,
    /// Read what the client sends next, whatever it is, and drop it.
    ReadAny,
    /// A byte every 40 ms, never a line break.
    Drip,
    /// Read a little of the message and hang up.
    ReadSomeThenClose,
    Close,
}
use Step::*;

#[derive(Default)]
struct Captured {
    data: Option<Vec<u8>>,
    finished: bool,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

fn client_config(ca: &[u8]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca.to_vec())).unwrap();
    Arc::new(
        ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn acceptor() -> TlsAcceptor {
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(SERVER.to_vec())],
            PrivateKeyDer::try_from(SERVER_KEY.to_vec()).unwrap(),
        )
        .unwrap();
    TlsAcceptor::from(Arc::new(config))
}

struct Reader {
    io: Box<dyn Io>,
    buf: Vec<u8>,
}

impl Reader {
    async fn say(&mut self, text: impl AsRef<str>) {
        self.io
            .write_all(format!("{}\r\n", text.as_ref()).as_bytes())
            .await
            .unwrap();
    }

    async fn line(&mut self) -> Option<String> {
        loop {
            if let Some(end) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line: Vec<u8> = self.buf.drain(..end + 2).take(end).collect();
                return Some(String::from_utf8_lossy(&line).into_owned());
            }
            let mut chunk = [0u8; 4096];
            match self.io.read(&mut chunk).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
            }
        }
    }

    /// Everything up to the line that is a single dot, with the stuffing taken
    /// off.
    async fn data(&mut self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            if let Some(end) = self.buf.windows(5).position(|w| w == b"\r\n.\r\n") {
                out.extend(self.buf.drain(..end + 5).take(end + 2));
                break;
            }
            if self.buf.starts_with(b".\r\n") && out.is_empty() {
                self.buf.drain(..3);
                break;
            }
            let mut chunk = [0u8; 8192];
            match self.io.read(&mut chunk).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
            }
        }
        let mut plain = Vec::with_capacity(out.len());
        let mut start = true;
        for (i, &b) in out.iter().enumerate() {
            if start && b == b'.' && out.get(i + 1) == Some(&b'.') {
                start = false;
                continue;
            }
            start = b == b'\n';
            plain.push(b);
        }
        Some(plain)
    }
}

async fn serve(io: impl Io + 'static, steps: Vec<Step>, captured: Arc<Mutex<Captured>>) {
    let mut reader = Reader {
        io: Box::new(io),
        buf: Vec::new(),
    };
    for step in steps {
        match step {
            Say(text) => {
                let line = format!("{text}\r\n");
                if reader.io.write_all(line.as_bytes()).await.is_err() {
                    return;
                }
            }
            Expect(want) => match reader.line().await {
                Some(got) => assert_eq!(got, want),
                None => return,
            },
            ExpectStart(start) => match reader.line().await {
                Some(got) => assert!(
                    got.starts_with(start),
                    "{got:?} does not start with {start:?}"
                ),
                None => return,
            },
            Data => match reader.data().await {
                Some(data) => captured.lock().unwrap().data = Some(data),
                None => return,
            },
            Raw(bytes) => {
                if reader.io.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            Tls => {
                assert!(
                    reader.buf.is_empty(),
                    "the client sent bytes before the handshake"
                );
                match acceptor().accept(reader.io).await {
                    Ok(stream) => {
                        reader = Reader {
                            io: Box::new(stream),
                            buf: Vec::new(),
                        }
                    }
                    Err(_) => return,
                }
            }
            Hang => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            ReadAny => {
                let mut chunk = [0u8; 4096];
                match reader.io.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
            }
            Drip => loop {
                if reader.io.write_all(b"2").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            },
            ReadSomeThenClose => {
                let mut chunk = [0u8; 4096];
                let _ = reader.io.read(&mut chunk).await;
                return;
            }
            Close => return,
        }
    }
    captured.lock().unwrap().finished = true;
}

fn connection(security: Security) -> Connection {
    Connection {
        host: "localhost".into(),
        port: 587,
        security,
        username: String::new(),
        password: String::new(),
    }
}

fn with_login(mut c: Connection) -> Connection {
    c.username = "jo".into();
    c.password = "s3cr3t-pässword".into();
    c
}

fn ascii_login() -> Connection {
    let mut c = with_login(connection(Security::None));
    c.password = "pencil".into();
    c
}

fn options() -> Options {
    Options {
        step_timeout: Duration::from_secs(5),
        total_timeout: Duration::from_secs(10),
        local_name: Some("client.test".into()),
    }
}

fn message() -> Message {
    Message {
        from: Mailbox::parse("Stellarr <noreply@chat.test>").unwrap(),
        to: vec![Mailbox::parse("jo@example.test").unwrap()],
        subject: "Zmień hasło w Orbit & Co".into(),
        html: "<p>Zażółć gęślą jaźń</p>\n<p>.dot</p>".into(),
        text: "Zażółć gęślą jaźń\n.dot at the start\n.".into(),
        message_id: "password-reset/ab12@stellarr".into(),
        date: UNIX_EPOCH + Duration::from_secs(1_795_000_000),
        images: vec![InlineImage {
            content_id: "lockup@stellarr".into(),
            file_name: "lockup.png".into(),
            content_type: "image/png".into(),
            content: (0..=255u8).cycle().take(3000).collect(),
        }],
    }
}

async fn exchange(
    connection: &Connection,
    trust: &[u8],
    options: &Options,
    message: &Message,
    steps: Vec<Step>,
) -> (Result<Delivery, Error>, Arc<Mutex<Captured>>) {
    let captured = Arc::new(Mutex::new(Captured::default()));
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let mut server = tokio::spawn(serve(server_io, steps, Arc::clone(&captured)));
    let result = send_over(
        client_io,
        connection,
        client_config(trust),
        options,
        message,
    )
    .await;
    // After a failure the script may be waiting on purpose; its assertions
    // have had their chance by then.
    let wait = if result.is_ok() {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(300)
    };
    match tokio::time::timeout(wait, &mut server).await {
        Ok(Err(e)) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        Ok(_) => {}
        Err(_) => server.abort(),
    }
    (result, captured)
}

/// The steps of the opening up to the sender, for a server without extensions.
fn opening_plain() -> Vec<Step> {
    vec![
        Say("220 mail.test ready"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 8BITMIME"),
    ]
}

fn envelope_and_data() -> Vec<Step> {
    vec![
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 2.1.0 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("250 2.1.5 ok"),
        Expect("DATA"),
        Say("354 go on"),
        Data,
        Say("250 2.0.0 queued as 4711"),
        Expect("QUIT"),
        Say("221 bye"),
    ]
}

fn assert_data_is_the_message(captured: &Mutex<Captured>, message: &Message) {
    let captured = captured.lock().unwrap();
    assert!(captured.finished, "the script did not run to its end");
    let got = captured.data.as_ref().expect("the server saw no message");
    assert_eq!(
        String::from_utf8_lossy(got),
        String::from_utf8_lossy(&message.encode().unwrap().bytes)
    );
}

#[tokio::test]
async fn a_message_goes_through_a_plain_connection() {
    let mut script = opening_plain();
    script.extend(envelope_and_data());
    let message = message();
    let (result, captured) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message,
        script,
    )
    .await;
    let delivery = result.unwrap();
    assert_eq!(delivery.message_id, "password-reset/ab12@stellarr");
    assert_eq!(delivery.reply, "2.0.0 queued as 4711");
    assert_data_is_the_message(&captured, &message);
}

#[tokio::test]
async fn a_message_goes_through_tls_from_the_first_byte() {
    let mut script = vec![
        Tls,
        Say("220 mail.test ready"),
        Expect("EHLO client.test"),
        Say("250 mail.test"),
    ];
    script.extend(envelope_and_data());
    let message = message();
    let (result, captured) =
        exchange(&connection(Security::Tls), CA, &options(), &message, script).await;
    result.unwrap();
    assert_data_is_the_message(&captured, &message);
}

fn starttls_opening() -> Vec<Step> {
    vec![
        Say("220 mail.test ready"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 STARTTLS"),
        Expect("STARTTLS"),
        Say("220 2.0.0 go ahead"),
        Tls,
        Expect("EHLO client.test"),
    ]
}

#[tokio::test]
async fn a_message_goes_through_starttls() {
    let mut script = starttls_opening();
    script.push(Say("250 mail.test"));
    script.extend(envelope_and_data());
    let message = message();
    let (result, captured) = exchange(
        &connection(Security::StartTls),
        CA,
        &options(),
        &message,
        script,
    )
    .await;
    result.unwrap();
    assert_data_is_the_message(&captured, &message);
}

#[tokio::test]
async fn the_extensions_after_starttls_are_the_ones_that_count() {
    // The server offers AUTH only inside TLS; announcing SIZE shows in MAIL.
    let mut script = vec![
        Say("220 mail.test ready"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 STARTTLS"),
        Expect("STARTTLS"),
        Say("220 go"),
        Tls,
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250-AUTH PLAIN\r\n250 SIZE 10000000"),
        Expect("AUTH PLAIN"),
        Say("334 "),
        // base64 of "\0jo\0s3cr3t-pässword", checked with Python.
        Expect("AGpvAHMzY3IzdC1ww6Rzc3dvcmQ="),
        Say("235 2.7.0 welcome"),
    ];
    script.extend([
        ExpectStart("MAIL FROM:<noreply@chat.test> SIZE="),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("250 ok"),
        Expect("DATA"),
        Say("354 go"),
        Data,
        Say("250 ok"),
        Expect("QUIT"),
        Say("221 bye"),
    ]);
    let (result, captured) = exchange(
        &with_login(connection(Security::StartTls)),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    result.unwrap();
    assert!(captured.lock().unwrap().finished);
}

fn plain_server_with_auth(mechs: &'static str) -> Vec<Step> {
    vec![
        Say("220 mail.test ready"),
        Expect("EHLO client.test"),
        match mechs {
            "LOGIN" => Say("250-mail.test\r\n250 AUTH LOGIN"),
            "NONE" => Say("250 mail.test"),
            _ => Say("250-mail.test\r\n250 AUTH CRAM-MD5 PLAIN LOGIN"),
        },
    ]
}

#[tokio::test]
async fn login_is_used_when_it_is_all_the_server_offers() {
    let mut script = plain_server_with_auth("LOGIN");
    script.extend([
        Expect("AUTH LOGIN"),
        Say("334 VXNlcm5hbWU6"),
        Expect("am8="),
        Say("334 UGFzc3dvcmQ6"),
        Expect("czNjcjN0LXDDpHNzd29yZA=="),
        Say("235 ok"),
    ]);
    script.extend(envelope_and_data());
    let (result, captured) = exchange(
        &with_login(connection(Security::None)),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    result.unwrap();
    assert!(captured.lock().unwrap().finished);
}

#[tokio::test]
async fn plain_is_preferred_to_login_and_cram_md5_is_ignored() {
    let mut script = plain_server_with_auth("ALL");
    script.extend([
        Expect("AUTH PLAIN"),
        Say("334 "),
        ExpectStart("AGpvA"),
        Say("235 ok"),
    ]);
    script.extend(envelope_and_data());
    let (result, captured) = exchange(
        &with_login(connection(Security::None)),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    result.unwrap();
    assert!(captured.lock().unwrap().finished);
}

/// A server that does SCRAM-SHA-256 the way the RFC says, computed here from
/// the password with the same primitives but written separately from the
/// client.
async fn scram_server(
    io: impl Io + 'static,
    password: &str,
    wrong_signature: bool,
    captured: Arc<Mutex<Captured>>,
) {
    let mut reader = Reader {
        io: Box::new(io),
        buf: Vec::new(),
    };
    let b64 = |d: &[u8]| {
        use std::fmt::Write;
        // Tiny local encoder so that the test does not lean on the crate's.
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in d.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= c.len() {
                    s.write_char(A[(n >> (18 - 6 * i)) as usize & 63] as char)
                        .unwrap();
                } else {
                    s.push('=');
                }
            }
        }
        s
    };
    let un64 = |t: &str| -> Vec<u8> {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0);
        for b in t.bytes().filter(|&b| b != b'=') {
            acc = (acc << 6) | A.iter().position(|&a| a == b).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    };
    reader.say("220 mail.test ready").await;
    assert_eq!(reader.line().await.unwrap(), "EHLO client.test");
    reader
        .say("250-mail.test\r\n250 AUTH PLAIN SCRAM-SHA-256")
        .await;
    assert_eq!(reader.line().await.unwrap(), "AUTH SCRAM-SHA-256");
    reader.say("334 ").await;
    let first = String::from_utf8(un64(&reader.line().await.unwrap())).unwrap();
    let bare = first.strip_prefix("n,,").unwrap().to_owned();
    assert!(bare.starts_with("n=jo,r="));
    let client_nonce = bare.split("r=").nth(1).unwrap().to_owned();
    let nonce = format!("{client_nonce}server-part");
    let salt = b"0123456789abcdef";
    let server_first = format!("r={nonce},s={},i=4096", b64(salt));
    reader
        .say(format!("334 {}", b64(server_first.as_bytes())))
        .await;
    let last = String::from_utf8(un64(&reader.line().await.unwrap())).unwrap();
    let (without_proof, proof) = last.rsplit_once(",p=").unwrap();
    assert_eq!(without_proof, format!("c=biws,r={nonce}"));

    let mut salted = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        4096.try_into().unwrap(),
        salt,
        password.as_bytes(),
        &mut salted,
    );
    let sign = |key: &[u8], data: &[u8]| {
        hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data)
            .as_ref()
            .to_vec()
    };
    let client_key = sign(&salted, b"Client Key");
    let stored = digest::digest(&digest::SHA256, &client_key);
    let auth_message = format!("{bare},{server_first},{without_proof}");
    let signature = sign(stored.as_ref(), auth_message.as_bytes());
    let expected: Vec<u8> = client_key
        .iter()
        .zip(&signature)
        .map(|(a, b)| a ^ b)
        .collect();
    assert_eq!(un64(proof), expected, "the client's proof is wrong");
    let server_key = sign(&salted, b"Server Key");
    let mut v = sign(&server_key, auth_message.as_bytes());
    if wrong_signature {
        v[0] ^= 1;
    }
    reader
        .say(format!("334 {}", b64(format!("v={}", b64(&v)).as_bytes())))
        .await;
    if wrong_signature {
        return;
    }
    assert_eq!(reader.line().await.unwrap(), "");
    reader.say("235 2.7.0 welcome").await;
    assert_eq!(
        reader.line().await.unwrap(),
        "MAIL FROM:<noreply@chat.test>"
    );
    reader.say("250 ok").await;
    assert_eq!(reader.line().await.unwrap(), "RCPT TO:<jo@example.test>");
    reader.say("250 ok").await;
    assert_eq!(reader.line().await.unwrap(), "DATA");
    reader.say("354 go").await;
    captured.lock().unwrap().data = reader.data().await;
    reader.say("250 ok").await;
    assert_eq!(reader.line().await.unwrap(), "QUIT");
    captured.lock().unwrap().finished = true;
}

async fn scram_exchange(
    server_password: &'static str,
    wrong_signature: bool,
) -> Result<Delivery, Error> {
    let captured = Arc::new(Mutex::new(Captured::default()));
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let server = tokio::spawn(scram_server(
        server_io,
        server_password,
        wrong_signature,
        Arc::clone(&captured),
    ));
    let mut connection = connection(Security::None);
    connection.username = "jo".into();
    connection.password = "pencil".into();
    let result = send_over(
        client_io,
        &connection,
        client_config(CA),
        &options(),
        &message(),
    )
    .await;
    if result.is_ok() {
        server.await.unwrap();
        assert!(captured.lock().unwrap().finished);
    } else {
        let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
    }
    result
}

#[tokio::test]
async fn scram_sha_256_logs_in_against_a_server_that_checks_the_proof() {
    scram_exchange("pencil", false).await.unwrap();
}

#[tokio::test]
async fn scram_stops_when_the_server_cannot_prove_it_knows_the_password() {
    let error = scram_exchange("pencil", true).await.unwrap_err();
    assert_eq!(error.stage(), Stage::Auth);
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_server_that_offers_only_what_the_client_lacks_is_reported() {
    let mut script = vec![
        Say("220 mail.test ready"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 AUTH CRAM-MD5 DIGEST-MD5 XOAUTH2"),
    ];
    script.push(Close);
    let (result, _) = exchange(
        &with_login(connection(Security::None)),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::Auth)
    );
    assert!(!error.is_transient());
    assert!(error.to_string().contains("CRAM-MD5"), "{error}");
}

#[tokio::test]
async fn a_login_on_a_server_without_authentication_is_reported() {
    let mut script = plain_server_with_auth("NONE");
    script.push(Close);
    let (result, _) = exchange(
        &with_login(connection(Security::None)),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::Auth)
    );
}

#[tokio::test]
async fn refused_credentials_are_permanent_and_the_password_is_not_in_the_error() {
    let mut script = plain_server_with_auth("ALL");
    script.extend([
        Expect("AUTH PLAIN"),
        Say("334 "),
        ExpectStart("AGpvA"),
        Say("535 5.7.8 Authentication credentials invalid"),
    ]);
    let connection = with_login(connection(Security::None));
    let (result, _) = exchange(&connection, CA, &options(), &message(), script).await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage(), error.reply_code()),
        (ErrorKind::Rejected, Stage::Auth, Some(535))
    );
    assert!(!error.is_transient());
    assert!(!error.may_have_been_delivered());
    let shown = format!("{error} {error:?} {connection:?}");
    assert!(!shown.contains("s3cr3t"), "{shown}");
    assert!(
        error
            .to_string()
            .starts_with("SMTP localhost:587: authentication: 535"),
        "{error}"
    );
}

#[tokio::test]
async fn a_temporary_authentication_failure_may_be_retried() {
    let mut script = plain_server_with_auth("ALL");
    script.extend([
        Expect("AUTH PLAIN"),
        Say("334 "),
        ExpectStart("AGpvA"),
        Say("454 4.7.0 Temporary authentication failure"),
    ]);
    let (result, _) = exchange(
        &with_login(connection(Security::None)),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.stage(), Stage::Auth);
    assert!(error.is_transient());
}

#[tokio::test]
async fn a_rejected_recipient_is_permanent_and_names_the_address() {
    let mut script = opening_plain();
    script.extend([
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("550 5.1.1 No such user"),
    ]);
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage(), error.reply_code()),
        (ErrorKind::Rejected, Stage::Recipient, Some(550))
    );
    assert!(!error.is_transient());
    assert!(error.to_string().contains("jo@example.test"), "{error}");
}

#[tokio::test]
async fn a_busy_server_asks_to_try_later() {
    let mut script = opening_plain();
    script.extend([
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("451 4.7.1 Greylisted, try again later"),
    ]);
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.stage(), error.reply_code()),
        (Stage::Recipient, Some(451))
    );
    assert!(error.is_transient());
}

#[tokio::test]
async fn a_refused_sender_a_refused_data_command_and_a_refused_message_are_each_reported() {
    let after_envelope = |sender: &'static str, data: &'static str, end: &'static str| {
        let mut script = opening_plain();
        script.extend([Expect("MAIL FROM:<noreply@chat.test>"), Say(sender)]);
        if sender.starts_with('2') {
            script.extend([
                Expect("RCPT TO:<jo@example.test>"),
                Say("250 ok"),
                Expect("DATA"),
                Say(data),
            ]);
            if data.starts_with('3') {
                script.extend([Data, Say(end)]);
            }
        }
        script
    };
    for (script, stage, code, transient) in [
        (
            after_envelope("553 5.7.1 Sender not allowed", "", ""),
            Stage::Sender,
            553,
            false,
        ),
        (
            after_envelope("250 ok", "554 5.5.1 no", ""),
            Stage::Data,
            554,
            false,
        ),
        (
            after_envelope("250 ok", "354 go", "552 5.3.4 Message too big"),
            Stage::EndOfData,
            552,
            false,
        ),
        (
            after_envelope("250 ok", "354 go", "451 4.3.0 Local error"),
            Stage::EndOfData,
            451,
            true,
        ),
    ] {
        let (result, _) = exchange(
            &connection(Security::None),
            CA,
            &options(),
            &message(),
            script,
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(
            (error.stage(), error.reply_code(), error.is_transient()),
            (stage, Some(code), transient)
        );
        // A refusal is an answer: the message was not taken.
        assert!(!error.may_have_been_delivered());
    }
}

#[tokio::test]
async fn a_server_that_is_not_ready_is_transient() {
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        vec![Say("421 4.3.2 Service shutting down")],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.stage(), error.reply_code()),
        (Stage::Greeting, Some(421))
    );
    assert!(error.is_transient());
}

#[tokio::test]
async fn a_server_that_hangs_is_given_up_on_at_the_step_timeout() {
    let options = Options {
        step_timeout: Duration::from_millis(250),
        total_timeout: Duration::from_secs(30),
        local_name: Some("client.test".into()),
    };
    let started = Instant::now();
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options,
        &message(),
        vec![Say("220 hi"), Hang],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Timeout, Stage::Hello)
    );
    assert!(error.is_transient());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_server_that_drips_bytes_is_cut_off_by_the_deadline() {
    let options = Options {
        step_timeout: Duration::from_secs(30),
        total_timeout: Duration::from_millis(400),
        local_name: Some("client.test".into()),
    };
    let started = Instant::now();
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options,
        &message(),
        vec![Drip],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);
    assert!(error.is_transient());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn silence_after_the_message_leaves_it_unknown_whether_it_was_taken() {
    let options = Options {
        step_timeout: Duration::from_millis(250),
        total_timeout: Duration::from_secs(30),
        local_name: Some("client.test".into()),
    };
    let mut script = opening_plain();
    script.extend([
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("250 ok"),
        Expect("DATA"),
        Say("354 go"),
        Data,
        Hang,
    ]);
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options,
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Timeout, Stage::EndOfData)
    );
    assert!(error.is_transient());
    assert!(error.may_have_been_delivered());
}

#[tokio::test]
async fn a_connection_dropped_before_the_answer_to_the_message_is_unknown_too() {
    let mut script = opening_plain();
    script.extend([
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("250 ok"),
        Expect("DATA"),
        Say("354 go"),
        Data,
        Close,
    ]);
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Io, Stage::EndOfData)
    );
    assert!(error.is_transient() && error.may_have_been_delivered());
}

#[tokio::test]
async fn a_connection_dropped_in_the_middle_of_the_message_was_not_taken() {
    let mut big = message();
    big.images[0].content = vec![7; 3_000_000];
    let captured = Arc::new(Mutex::new(Captured::default()));
    // A small pipe, so that the client is still writing when the server leaves.
    let (client_io, server_io) = tokio::io::duplex(4096);
    let mut script = opening_plain();
    script.extend([
        ExpectStart("MAIL FROM:"),
        Say("250 ok"),
        ExpectStart("RCPT TO:"),
        Say("250 ok"),
        Expect("DATA"),
        Say("354 go"),
        ReadSomeThenClose,
    ]);
    let server = tokio::spawn(serve(server_io, script, captured));
    let result = send_over(
        client_io,
        &connection(Security::None),
        client_config(CA),
        &options(),
        &big,
    )
    .await;
    server.await.unwrap();
    let error = result.unwrap_err();
    assert_eq!((error.kind(), error.stage()), (ErrorKind::Io, Stage::Data));
    assert!(error.is_transient());
    assert!(!error.may_have_been_delivered());
}

#[tokio::test]
async fn an_immediate_hang_up_is_transient() {
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        vec![Close],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Io, Stage::Greeting)
    );
    assert!(error.is_transient());
}

#[tokio::test]
async fn garbage_is_a_protocol_error_that_will_not_get_better() {
    let garbage: Vec<Vec<Step>> = vec![
        vec![Say("HTTP/1.1 400 Bad Request")],
        vec![Raw(vec![
            0x16, 0x03, 0x01, 0x00, 0xa5, 0x01, 0x00, 0x00, 0xa1, 0x03, 0x03, b'\n',
        ])],
        vec![Raw(b"\xff\xfe\x00\x01 garbage\r\n".to_vec())],
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("250-a\r\n251 b"),
        ],
        vec![Say("220 hi"), Expect("EHLO client.test"), Say("2x0 a")],
        vec![Say("220 hi"), Expect("EHLO client.test"), Say("250x a")],
        vec![Say("999 nope")],
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("200 not what was asked"),
        ],
    ];
    for script in garbage {
        let (result, _) = exchange(
            &connection(Security::None),
            CA,
            &options(),
            &message(),
            script,
        )
        .await;
        let error = result.unwrap_err();
        assert!(!error.is_transient(), "{error}");
        assert!(
            matches!(error.kind(), ErrorKind::Protocol | ErrorKind::Rejected),
            "{error}"
        );
    }
}

#[tokio::test]
async fn lines_that_are_too_long_are_refused() {
    let long: &'static str = format!("220 {}", "a".repeat(5000)).leak();
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        vec![Say(long)],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Protocol, Stage::Greeting)
    );
    assert!(!error.is_transient());

    // No line break at all, and endless: the buffer must not grow without end.
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        vec![Raw(vec![b'x'; 200_000]), Hang],
    )
    .await;
    assert_eq!(result.unwrap_err().kind(), ErrorKind::Protocol);

    // A reply of endless lines.
    let many: Vec<u8> = "250-x\r\n".repeat(500).into_bytes();
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        vec![Say("220 hi"), Expect("EHLO client.test"), Raw(many), Hang],
    )
    .await;
    assert_eq!(result.unwrap_err().kind(), ErrorKind::Protocol);
}

#[tokio::test]
async fn a_server_without_ehlo_is_spoken_to_with_helo() {
    let mut script = vec![
        Say("220 old.test ready"),
        Expect("EHLO client.test"),
        Say("502 5.5.1 Command not implemented"),
        Expect("HELO client.test"),
        Say("250 old.test"),
    ];
    script.extend(envelope_and_data());
    let message = message();
    let (result, captured) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message,
        script,
    )
    .await;
    result.unwrap();
    assert_data_is_the_message(&captured, &message);
}

#[tokio::test]
async fn starttls_is_required_when_the_settings_say_so() {
    let (result, _) = exchange(
        &connection(Security::StartTls),
        CA,
        &options(),
        &message(),
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("250 mail.test"),
            Close,
        ],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::StartTls)
    );
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_starttls_the_server_refuses_is_reported_with_its_code() {
    let (result, _) = exchange(
        &connection(Security::StartTls),
        CA,
        &options(),
        &message(),
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("250-mail.test\r\n250 STARTTLS"),
            Expect("STARTTLS"),
            Say("454 4.7.0 TLS not available due to temporary reason"),
        ],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.stage(), error.reply_code()),
        (Stage::StartTls, Some(454))
    );
    assert!(error.is_transient());
}

#[tokio::test]
async fn a_starttls_whose_handshake_never_happens_is_a_tls_error() {
    // The server says 220 and then goes on in the clear, which is garbage
    // to a TLS client.
    let (result, _) = exchange(
        &connection(Security::StartTls),
        CA,
        &options(),
        &message(),
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("250-mail.test\r\n250 STARTTLS"),
            Expect("STARTTLS"),
            Say("220 go ahead"),
            ReadAny,
            Raw(b"250 this is not a handshake\r\n".repeat(20)),
            Hang,
        ],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Tls, Stage::Handshake)
    );
}

#[tokio::test]
async fn a_server_that_hangs_up_during_the_handshake_is_transient() {
    let (result, _) = exchange(
        &connection(Security::StartTls),
        CA,
        &options(),
        &message(),
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("250-mail.test\r\n250 STARTTLS"),
            Expect("STARTTLS"),
            Say("220 go ahead"),
            Close,
        ],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Tls, Stage::Handshake)
    );
    assert!(error.is_transient());
}

#[tokio::test]
async fn a_certificate_nobody_trusts_is_refused_for_good() {
    for (security, script) in [
        (
            Security::StartTls,
            vec![
                Say("220 hi"),
                Expect("EHLO client.test"),
                Say("250-mail.test\r\n250 STARTTLS"),
                Expect("STARTTLS"),
                Say("220 go ahead"),
                Tls,
            ],
        ),
        (Security::Tls, vec![Tls]),
    ] {
        let (result, _) = exchange(
            &connection(security),
            OTHER_CA,
            &options(),
            &message(),
            script,
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(
            (error.kind(), error.stage()),
            (ErrorKind::Tls, Stage::Handshake),
            "{error}"
        );
        assert!(!error.is_transient(), "{error}");
    }
}

#[tokio::test]
async fn the_name_in_the_certificate_must_match_the_host() {
    let mut wrong = connection(Security::Tls);
    wrong.host = "elsewhere.test".into();
    let (result, _) = exchange(&wrong, CA, &options(), &message(), vec![Tls]).await;
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Tls);
    assert!(!error.is_transient());
}

#[tokio::test]
async fn bytes_smuggled_in_before_the_handshake_are_not_believed() {
    // The 220 and a second reply arrive together: the second was sent before
    // TLS and must not be taken as if it came through it.
    let (result, _) = exchange(
        &connection(Security::StartTls),
        CA,
        &options(),
        &message(),
        vec![
            Say("220 hi"),
            Expect("EHLO client.test"),
            Say("250-mail.test\r\n250 STARTTLS"),
            Expect("STARTTLS"),
            Say("220 go ahead\r\n250 AUTH PLAIN injected"),
            Hang,
        ],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Protocol, Stage::StartTls)
    );
}

#[tokio::test]
async fn smtputf8_is_needed_for_a_non_ascii_local_part_and_used_when_there() {
    let mut message = message();
    message.to = vec![Mailbox::parse("zażółć@example.test").unwrap()];
    let (result, _) = exchange(&connection(Security::None), CA, &options(), &message, {
        let mut s = opening_plain();
        s.push(Close);
        s
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::Sender)
    );

    let script = vec![
        Say("220 hi"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 SMTPUTF8"),
        Expect("MAIL FROM:<noreply@chat.test> SMTPUTF8"),
        Say("250 ok"),
        Expect("RCPT TO:<zażółć@example.test>"),
        Say("250 ok"),
        Expect("DATA"),
        Say("354 go"),
        Data,
        Say("250 ok"),
        Expect("QUIT"),
    ];
    let (result, captured) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message,
        script,
    )
    .await;
    result.unwrap();
    let data = captured.lock().unwrap().data.clone().unwrap();
    assert!(
        String::from_utf8(data)
            .unwrap()
            .contains("zażółć@example.test")
    );
}

#[tokio::test]
async fn a_message_larger_than_the_server_allows_is_not_sent() {
    let script = vec![
        Say("220 hi"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 SIZE 1000"),
        Close,
    ];
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message(),
        script,
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::Data)
    );
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_message_that_cannot_be_built_fails_before_anything_is_said() {
    let mut bad = message();
    bad.subject = "x\r\nBcc: y@z.test".into();
    let (result, captured) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &bad,
        vec![Say("220 hi")],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::InvalidMessage, Stage::Message)
    );
    assert!(!error.is_transient());
    assert!(!captured.lock().unwrap().finished || captured.lock().unwrap().data.is_none());
}

#[tokio::test]
async fn several_recipients_each_get_a_rcpt() {
    let mut message = message();
    message
        .to
        .push(Mailbox::parse("Ala <ala@example.test>").unwrap());
    let script = vec![
        Say("220 hi"),
        Expect("EHLO client.test"),
        Say("250 mail.test"),
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<ala@example.test>"),
        Say("251 will forward"),
        Expect("DATA"),
        Say("354 go"),
        Data,
        Say("250 ok"),
        Expect("QUIT"),
    ];
    let (result, captured) = exchange(
        &connection(Security::None),
        CA,
        &options(),
        &message,
        script,
    )
    .await;
    result.unwrap();
    assert_data_is_the_message(&captured, &message);
}

#[tokio::test]
async fn a_scram_the_server_cannot_do_for_this_user_falls_back_to_plain() {
    let script = vec![
        Say("220 hi"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 AUTH PLAIN SCRAM-SHA-256 SCRAM-SHA-1"),
        Expect("AUTH SCRAM-SHA-256"),
        Say("535 5.7.8 no SCRAM secret for this user"),
        // SCRAM-SHA-1 is not tried as well: it would be the same refusal.
        Expect("AUTH PLAIN"),
        Say("334 "),
        ExpectStart("AGpvA"),
        Say("235 ok"),
        Expect("MAIL FROM:<noreply@chat.test>"),
        Say("250 ok"),
        Expect("RCPT TO:<jo@example.test>"),
        Say("250 ok"),
        Expect("DATA"),
        Say("354 go"),
        Data,
        Say("250 ok"),
        Expect("QUIT"),
    ];
    let mut login = with_login(connection(Security::None));
    login.password = "pencil".into();
    let (result, captured) = exchange(&login, CA, &options(), &message(), script).await;
    result.unwrap();
    assert!(captured.lock().unwrap().finished);
}

#[tokio::test]
async fn when_every_mechanism_is_refused_the_last_refusal_is_reported() {
    let script = vec![
        Say("220 hi"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 AUTH PLAIN SCRAM-SHA-256"),
        Expect("AUTH SCRAM-SHA-256"),
        Say("535 5.7.8 no"),
        Expect("AUTH PLAIN"),
        Say("334 "),
        ExpectStart("AGpvA"),
        Say("535 5.7.8 bad credentials"),
    ];
    let (result, _) = exchange(&ascii_login(), CA, &options(), &message(), script).await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.stage(), error.reply_code()),
        (Stage::Auth, Some(535))
    );
    assert!(error.to_string().contains("bad credentials"), "{error}");
    assert!(!error.is_transient());
}

#[tokio::test]
async fn a_temporary_failure_of_scram_is_not_papered_over_with_plain() {
    let script = vec![
        Say("220 hi"),
        Expect("EHLO client.test"),
        Say("250-mail.test\r\n250 AUTH PLAIN SCRAM-SHA-256"),
        Expect("AUTH SCRAM-SHA-256"),
        Say("454 4.7.0 try later"),
    ];
    let (result, _) = exchange(&ascii_login(), CA, &options(), &message(), script).await;
    let error = result.unwrap_err();
    assert!(error.is_transient());
    assert_eq!(error.reply_code(), Some(454));
}

#[tokio::test]
async fn a_host_that_is_no_tls_name_is_fine_without_tls() {
    let mut plain = connection(Security::None);
    plain.host = "bad host!".into();
    let mut script = opening_plain();
    script.extend(envelope_and_data());
    let (result, _) = exchange(&plain, CA, &options(), &message(), script).await;
    result.unwrap();

    let mut tls = connection(Security::Tls);
    tls.host = "bad host!".into();
    let (result, _) = exchange(&tls, CA, &options(), &message(), vec![Close]).await;
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.stage()),
        (ErrorKind::Unsupported, Stage::Handshake)
    );
}

#[tokio::test]
async fn the_deadline_is_reported_as_the_exchange_running_out_of_time() {
    let options = Options {
        step_timeout: Duration::from_secs(30),
        total_timeout: Duration::from_millis(200),
        local_name: Some("client.test".into()),
    };
    // The greeting arrives; the deadline runs out while waiting for the rest.
    let (result, _) = exchange(
        &connection(Security::None),
        CA,
        &options,
        &message(),
        vec![Say("220 hi"), Hang],
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Timeout);
    assert!(!error.to_string().contains("0.0 seconds"), "{error}");
}
