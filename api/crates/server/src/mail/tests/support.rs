//! An instance to arrange rows in, and the servers that stand in for the
//! providers.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::params;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::database::Database;
use crate::events::Unconnected;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::state::AppState;
use crate::testing::TempDir;
use crate::values::{Id, Timestamp};

pub const PUBLIC_URL: &str = "https://chat.test";

/// A state on a database of its own, in the Testing environment, with the
/// public address set; `extra` adds or replaces environment variables.
pub fn instance(extra: &[(&str, &str)]) -> (TempDir, AppState) {
    let folder = TempDir::new();
    let mut variables: Vec<(String, String)> = vec![
        ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
        (
            "STELLARR_DATA_ROOT".into(),
            folder.path().display().to_string(),
        ),
        ("Operations__PublicUrl".into(), PUBLIC_URL.into()),
    ];
    for (name, value) in extra {
        variables.retain(|(have, _)| have != name);
        variables.push((name.to_string(), value.to_string()));
    }
    let settings = Settings::load(&variables).expect("usable settings");
    let database = Database::open(&settings.database).expect("a new database");
    let state = AppState::new(
        settings,
        database,
        Arc::new(Unconnected),
        Arc::new(Unconnected),
        Sealer::from_key([5u8; 32]),
    );
    (folder, state)
}

/// The mail settings the operator saved, as the database holds them.
#[derive(Clone)]
pub struct Saved {
    pub mailing: bool,
    pub alerts: bool,
    pub from: &'static str,
    pub host: &'static str,
    pub port: u16,
    pub security: &'static str,
    pub username: &'static str,
    pub resend: Option<bool>,
    pub time_zone: &'static str,
}

impl Default for Saved {
    fn default() -> Saved {
        Saved {
            mailing: true,
            alerts: true,
            from: "noreply@chat.test",
            host: "127.0.0.1",
            port: 25,
            security: "none",
            username: "",
            resend: None,
            time_zone: "UTC",
        }
    }
}

/// Founds the instance server and saves `saved` as its settings.
pub async fn found(state: &AppState, saved: Saved) -> Id {
    let server = Id::random();
    state
        .database()
        .write(move |tx| -> rusqlite::Result<()> {
            tx.execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Orbit', ?2, 1, 0)",
                params![server, Timestamp::now()],
            )?;
            save(tx, server, &saved)
        })
        .await
        .unwrap();
    server
}

pub fn save(tx: &rusqlite::Connection, server: Id, saved: &Saved) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO \"OperationsSettings\"
            (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\",
             \"MailSmtpPort\", \"MailSmtpUsername\", \"MailSmtpSecurity\", \"MailResendEnabled\", \"InviteMailEnabled\",
             \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
             \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\")
         VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 1, ?10, 14, 30, 1, 1, ?11)",
        params![
            server,
            saved.time_zone,
            saved.mailing,
            saved.from,
            saved.host,
            saved.port,
            saved.username,
            saved.security,
            saved.resend,
            saved.alerts,
            Timestamp::now()
        ],
    )?;
    Ok(())
}

pub async fn resave(state: &AppState, server: Id, saved: Saved) {
    state
        .database()
        .write(move |tx| save(tx, server, &saved))
        .await
        .unwrap();
}

/// A person with an alert queued for `kind`, due at `due`. Returns the id of
/// the queued row.
pub async fn queue(state: &AppState, name: &str, address: &str, kind: &str, due: Timestamp) -> Id {
    let (name, address, kind) = (name.to_string(), address.to_string(), kind.to_string());
    state
        .database()
        .write(move |tx| -> rusqlite::Result<Id> {
            let (user, event, row) = (Id::random(), Id::random(), Id::random());
            tx.execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"Email\",
                                        \"NormalizedEmail\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\",
                                        \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, 'hash', ?5, NULL, 0, 0, 0, '#777777', ?6)",
                params![
                    user,
                    format!("u{}", user.to_db_text()),
                    user.to_db_text(),
                    name,
                    address,
                    Timestamp::now()
                ],
            )?;
            tx.execute(
                "INSERT INTO \"SecurityEvents\" (\"Id\", \"UserId\", \"Kind\", \"Detail\", \"IpAddress\", \"Device\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, '', '203.0.113.7', 'Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0', ?4)",
                params![event, user, kind, Timestamp::parse("2026-01-15 12:00:00").unwrap()],
            )?;
            tx.execute(
                "INSERT INTO \"SecurityNotifications\" (\"Id\", \"SecurityEventId\", \"UserId\", \"EmailAddress\", \"Attempts\",
                                                       \"CreatedAt\", \"NextAttemptAt\", \"LastAttemptAt\", \"SentAt\", \"LastError\")
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5, NULL, NULL, '')",
                params![row, event, user, address, due],
            )?;
            Ok(row)
        })
        .await
        .unwrap()
}

/// A row of the queue as the database holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub attempts: i64,
    pub next_attempt_at: Timestamp,
    pub last_attempt_at: Option<Timestamp>,
    pub sent_at: Option<Timestamp>,
    pub last_error: String,
}

pub async fn row(state: &AppState, id: Id) -> Queued {
    state
        .database()
        .read(move |db| -> rusqlite::Result<Queued> {
            db.query_row(
                "SELECT \"Attempts\", \"NextAttemptAt\", \"LastAttemptAt\", \"SentAt\", \"LastError\"
                 FROM \"SecurityNotifications\" WHERE \"Id\" = ?1",
                [id],
                |row| {
                    Ok(Queued {
                        attempts: row.get(0)?,
                        next_attempt_at: row.get(1)?,
                        last_attempt_at: row.get(2)?,
                        sent_at: row.get(3)?,
                        last_error: row.get(4)?,
                    })
                },
            )
        })
        .await
        .unwrap()
}

pub fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).expect("a moment")
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// ---- a mail server of the test's own ----

/// What a fake server is told to do.
#[derive(Clone, Default)]
pub struct Behaviour {
    /// The reply to RCPT TO; `250 2.1.5 OK` when empty.
    pub recipient_reply: &'static str,
    /// Accepts the connection and says nothing.
    pub silent: bool,
    /// Offers AUTH PLAIN and accepts this password; the login is recorded.
    pub password: Option<&'static str>,
    /// Speaks TLS from the first byte.
    pub tls: Option<Arc<rustls::ServerConfig>>,
}

#[derive(Default, Debug, Clone)]
pub struct Seen {
    pub connections: usize,
    pub logins: Vec<(String, String)>,
    pub lines: Vec<String>,
    pub mails: Vec<Mail>,
}

#[derive(Default, Debug, Clone)]
pub struct Mail {
    pub commands: Vec<String>,
    pub data: String,
}

pub struct FakeSmtp {
    pub port: u16,
    seen: Arc<Mutex<Seen>>,
    task: JoinHandle<()>,
}

impl Drop for FakeSmtp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeSmtp {
    pub async fn start(behaviour: Behaviour) -> FakeSmtp {
        FakeSmtp::start_on(0, behaviour).await
    }

    /// On `port`, which may be one a server was once on.
    pub async fn start_on(port: u16, behaviour: Behaviour) -> FakeSmtp {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = Arc::clone(&seen);
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let (behaviour, seen) = (behaviour.clone(), Arc::clone(&shared));
                tokio::spawn(async move {
                    seen.lock().unwrap().connections += 1;
                    match behaviour.tls.clone() {
                        Some(config) => {
                            if let Ok(stream) =
                                tokio_rustls::TlsAcceptor::from(config).accept(tcp).await
                            {
                                converse(stream, &behaviour, &seen).await;
                            }
                        }
                        None => converse(tcp, &behaviour, &seen).await,
                    }
                });
            }
        });
        FakeSmtp { port, seen, task }
    }

    pub fn seen(&self) -> Seen {
        self.seen.lock().unwrap().clone()
    }

    pub fn mails(&self) -> Vec<Mail> {
        self.seen().mails
    }
}

async fn converse<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    behaviour: &Behaviour,
    seen: &Mutex<Seen>,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    if behaviour.silent {
        tokio::time::sleep(Duration::from_secs(60)).await;
        return;
    }
    let mut mail = Mail::default();
    let _ = write.write_all(b"220 fake.test ESMTP\r\n").await;
    while let Ok(Some(line)) = lines.next_line().await {
        mail.commands.push(line.clone());
        seen.lock().unwrap().lines.push(line.clone());
        let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
        let answer: String = match verb.as_str() {
            "EHLO" if behaviour.password.is_some() => "250-fake.test\r\n250 AUTH PLAIN".into(),
            "EHLO" => "250-fake.test\r\n250 8BITMIME".into(),
            "AUTH" => {
                let _ = write.write_all(b"334 \r\n").await;
                let Ok(Some(credentials)) = lines.next_line().await else {
                    return;
                };
                let decoded = decode_base64(&credentials);
                let mut parts = decoded.split(|byte| *byte == 0).skip(1);
                let user = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
                let password =
                    String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
                let accepted = behaviour.password == Some(password.as_str());
                seen.lock().unwrap().logins.push((user, password));
                if accepted {
                    "235 2.7.0 Authentication successful".into()
                } else {
                    "535 5.7.8 Authentication credentials invalid".into()
                }
            }
            "RCPT" if behaviour.recipient_reply.is_empty() => "250 2.1.5 OK".into(),
            "RCPT" => behaviour.recipient_reply.into(),
            "DATA" => {
                let _ = write
                    .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                    .await;
                while let Ok(Some(body)) = lines.next_line().await {
                    if body == "." {
                        break;
                    }
                    mail.data.push_str(&body);
                    mail.data.push('\n');
                }
                seen.lock().unwrap().mails.push(std::mem::take(&mut mail));
                "250 2.0.0 Queued".into()
            }
            "QUIT" => {
                let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
                return;
            }
            _ => "250 OK".into(),
        };
        let _ = write.write_all(format!("{answer}\r\n").as_bytes()).await;
    }
}

fn decode_base64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.trim().bytes().filter(|byte| *byte != b'=') {
        let Some(value) = ALPHABET.iter().position(|letter| *letter == byte) else {
            continue;
        };
        bits = bits << 6 | value as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

/// The test certificates of the outbound crate: a root, and a server
/// certificate for `localhost` it signed, and another root nobody trusts.
pub mod certificates {
    use super::*;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    fn read(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../outbound/tests/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    pub fn roots() -> rustls::RootCertStore {
        stellarr_outbound::tls::roots_from_pem(&read("ca.crt.txt")).unwrap()
    }

    /// A server that presents the certificate of fixture `name`.
    pub fn server(name: &str) -> Arc<rustls::ServerConfig> {
        let chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(&read(&format!("{name}.crt.txt")))
                .collect::<Result<_, _>>()
                .unwrap();
        let key = PrivateKeyDer::from_pem_slice(&read(&format!("{name}.pk8.txt"))).unwrap();
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(config)
    }
}

// ---- a stand-in for Resend ----

/// What the stand-in answers to every request.
#[derive(Clone)]
pub struct Answer {
    pub status: u16,
    pub body: &'static str,
}

/// What it was asked.
#[derive(Debug, Clone)]
pub struct Asked {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Asked {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub struct FakeResend {
    pub port: u16,
    asked: Arc<Mutex<Vec<Asked>>>,
    task: JoinHandle<()>,
}

impl Drop for FakeResend {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeResend {
    pub async fn start(answer: Answer) -> FakeResend {
        FakeResend::serve(answer, None).await
    }

    pub async fn start_tls(answer: Answer, config: Arc<rustls::ServerConfig>) -> FakeResend {
        FakeResend::serve(answer, Some(config)).await
    }

    async fn serve(answer: Answer, tls: Option<Arc<rustls::ServerConfig>>) -> FakeResend {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&asked);
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let (answer, asked, tls) = (answer.clone(), Arc::clone(&shared), tls.clone());
                tokio::spawn(async move {
                    match tls {
                        Some(config) => {
                            if let Ok(stream) =
                                tokio_rustls::TlsAcceptor::from(config).accept(tcp).await
                            {
                                respond(stream, &answer, &asked).await;
                            }
                        }
                        None => respond(tcp, &answer, &asked).await,
                    }
                });
            }
        });
        FakeResend { port, asked, task }
    }

    pub fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

async fn respond<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    answer: &Answer,
    asked: &Mutex<Vec<Asked>>,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
        return;
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end().to_string();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body)
        .await
        .is_err()
    {
        return;
    }
    asked.lock().unwrap().push(Asked {
        request_line: request_line.trim_end().to_string(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    let text = format!(
        "HTTP/1.1 {} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        answer.status,
        answer.body.len(),
        answer.body
    );
    let _ = write.write_all(text.as_bytes()).await;
    let _ = write.shutdown().await;
}
