//! The security alerts, from a sign-in to a mail server, with the real binary.
//!
//! A sign-in queues an alert for an account whose address is verified; the
//! worker of a second process, started on the same data folder, sends it.
//! Starting the worker's process after the sign-in is what makes it look at
//! the queue at once: it looks when it starts, and then every thirty seconds.
//!
//! The tests at the end that name Mailpit need a container and are ignored:
//! `docker run -d -p 127.0.0.1:1025:1025 -p 127.0.0.1:8025:8025
//! axllent/mailpit:v1.34.0`, then
//! `STELLARR_TEST_MAILPIT=1025,8025 cargo test -p stellarr-api --test api mail::mailpit -- --ignored`.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use stellarr_api::seal::Sealer;
use stellarr_api::secrets;
use stellarr_api::values::{Id, Timestamp};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::support::{Client, PASSWORD, Person, Request, Server, TempDir};

const WORKER: &str = ("Testing__Workers", "security-notifications").1;
const PUBLIC_URL: &str = "https://chat.test";

// ---- a mail server that keeps what it is given ----

#[derive(Default, Debug, Clone)]
struct Seen {
    connections: usize,
    logins: Vec<(String, String)>,
    mails: Vec<String>,
}

struct SmtpServer {
    port: u16,
    seen: Arc<Mutex<Seen>>,
    task: JoinHandle<()>,
}

impl Drop for SmtpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SmtpServer {
    /// A server that wants `password` from anybody who logs in.
    async fn start(port: u16, password: &'static str) -> SmtpServer {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let shared = Arc::clone(&seen);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = Arc::clone(&shared);
                tokio::spawn(async move {
                    seen.lock().unwrap().connections += 1;
                    converse(stream, password, &seen).await;
                });
            }
        });
        SmtpServer { port, seen, task }
    }

    fn seen(&self) -> Seen {
        self.seen.lock().unwrap().clone()
    }
}

async fn converse(stream: tokio::net::TcpStream, password: &str, seen: &Mutex<Seen>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let _ = write.write_all(b"220 fake.test ESMTP\r\n").await;
    while let Ok(Some(line)) = lines.next_line().await {
        let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
        let answer = match verb.as_str() {
            "EHLO" => "250-fake.test\r\n250 AUTH PLAIN".to_string(),
            "AUTH" => {
                let _ = write.write_all(b"334 \r\n").await;
                let Ok(Some(credentials)) = lines.next_line().await else { return };
                let decoded = base64(&credentials);
                let mut parts = decoded.split(|byte| *byte == 0).skip(1);
                let user = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
                let given = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
                let accepted = given == password;
                seen.lock().unwrap().logins.push((user, given));
                if accepted { "235 2.7.0 Authentication successful" } else { "535 5.7.8 Authentication credentials invalid" }.to_string()
            }
            "DATA" => {
                let _ = write.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await;
                let mut data = String::new();
                while let Ok(Some(body)) = lines.next_line().await {
                    if body == "." {
                        break;
                    }
                    data.push_str(&body);
                    data.push('\n');
                }
                seen.lock().unwrap().mails.push(data);
                "250 2.0.0 Queued".to_string()
            }
            "QUIT" => {
                let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
                return;
            }
            _ => "250 OK".to_string(),
        };
        let _ = write.write_all(format!("{answer}\r\n").as_bytes()).await;
    }
}

fn base64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut out, mut bits, mut count) = (Vec::new(), 0u32, 0);
    for byte in text.trim().bytes().filter(|byte| *byte != b'=') {
        let Some(value) = ALPHABET.iter().position(|letter| *letter == byte) else { continue };
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

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..800 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{what} did not happen in twenty seconds");
}

// ---- an instance with an alert in its queue ----

/// A data folder with an owner whose address is verified, and the alert that
/// their second sign-in queued. No process runs on it.
async fn with_a_queued_alert(username: &str) -> TempDir {
    let folder = TempDir::new();
    let server = Server::start_in(folder.path(), &[]).await;
    let owner: Person = server.found(username).await;
    server
        .database()
        .execute("UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\"", [])
        .unwrap();
    server.sign_in(&owner).await;
    let queued: i64 = server
        .database()
        .query_row("SELECT count(*) FROM \"SecurityNotifications\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(queued, 1);
    assert!(server.stop().await);
    folder
}

/// What the operator saved: where the mail goes, and whether it may.
struct Saved {
    port: u16,
    username: &'static str,
    mailing: bool,
    alerts: bool,
}

fn open(folder: &Path) -> Connection {
    let connection = Connection::open(folder.join("stellarr.db")).unwrap();
    connection.busy_timeout(Duration::from_secs(10)).unwrap();
    connection
}

fn configure(folder: &Path, saved: &Saved) {
    let connection = open(folder);
    let server: Id = connection
        .query_row("SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1", [], |row| row.get(0))
        .unwrap();
    connection
        .execute(
            "INSERT OR REPLACE INTO \"OperationsSettings\"
                (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\",
                 \"MailSmtpPort\", \"MailSmtpUsername\", \"MailSmtpSecurity\", \"MailResendEnabled\", \"InviteMailEnabled\",
                 \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
                 \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\")
             VALUES (?1, '', 'Europe/Warsaw', ?2, 'noreply@chat.test', '127.0.0.1', ?3, ?4, 'none', 0, 1, 1, ?5, 14, 30, 1, 1, ?6)",
            params![server, saved.mailing, saved.port, saved.username, saved.alerts, Timestamp::now()],
        )
        .unwrap();
}

/// Stores a secret the way the operator's panel will: through the module, with
/// the key of that data folder.
fn set_secret(folder: &Path, definition: &secrets::SecretDefinition, value: &str) {
    let sealer = Sealer::load_or_create(&folder.join("keys")).unwrap();
    secrets::set(&open(folder), &sealer, definition, value).unwrap();
}

/// The one process that works the queue, on `folder`.
async fn start_worker(folder: &Path, extra: &[(&str, &str)]) -> Server {
    let mut variables = vec![("Testing__Workers", "security-notifications"), ("Operations__PublicUrl", PUBLIC_URL)];
    variables.extend_from_slice(extra);
    Server::start_in(folder, &variables).await
}

#[derive(Debug, PartialEq, Eq)]
struct Row {
    attempts: i64,
    next_attempt_at: Option<Timestamp>,
    sent_at: Option<Timestamp>,
    last_error: String,
}

fn row(folder: &Path) -> Row {
    open(folder)
        .query_row(
            "SELECT \"Attempts\", \"NextAttemptAt\", \"SentAt\", \"LastError\" FROM \"SecurityNotifications\"",
            [],
            |row| Ok(Row { attempts: row.get(0)?, next_attempt_at: row.get(1)?, sent_at: row.get(2)?, last_error: row.get(3)? }),
        )
        .optional()
        .unwrap()
        .expect("one alert in the queue")
}

fn minutes_from_now(moment: Timestamp) -> i64 {
    (moment.unix_seconds() - Timestamp::now().unix_seconds() + 30).div_euclid(60)
}

fn copy_database(from: &Path, to: &Path) {
    for name in ["stellarr.db", "stellarr.db-wal", "stellarr.db-shm"] {
        if from.join(name).exists() {
            std::fs::copy(from.join(name), to.join(name)).unwrap();
        }
    }
}

// ---- the proof ----

#[tokio::test]
async fn a_sign_in_ends_as_one_message_to_the_address_with_the_password_another_process_stored() {
    let folder = with_a_queued_alert("owner").await;
    let smtp = SmtpServer::start(0, "the panel's password ").await;
    configure(folder.path(), &Saved { port: smtp.port, username: "mailer", mailing: true, alerts: true });
    set_secret(folder.path(), &secrets::SMTP_PASSWORD, "the panel's password ");
    let stored: String = open(folder.path())
        .query_row("SELECT \"ProtectedValue\" FROM \"InstanceSecrets\"", [], |row| row.get(0))
        .unwrap();
    assert!(!stored.contains("password"), "{stored}");

    let worker = start_worker(folder.path(), &[]).await;
    until("the message", || !smtp.seen().mails.is_empty()).await;
    assert!(worker.stop().await);

    let seen = smtp.seen();
    assert_eq!(seen.logins, [("mailer".to_string(), "the panel's password ".to_string())]);
    assert_eq!(seen.mails.len(), 1);
    let mail = &seen.mails[0];
    assert!(mail.contains("To: owner@example.test"), "{mail}");
    assert!(mail.contains("From: Stellarr <noreply@chat.test>"), "{mail}");
    assert!(mail.contains("Subject: Security activity on Stellarr"), "{mail}");
    assert!(mail.contains("Activity: Signed in"), "{mail}");
    let recorded = row(folder.path());
    assert_eq!((recorded.attempts, recorded.last_error.as_str()), (1, ""));
    assert!(recorded.sent_at.is_some());
}

#[tokio::test]
async fn a_database_copied_to_another_data_folder_reads_the_secret_as_not_set() {
    let folder = with_a_queued_alert("owner").await;
    let smtp = SmtpServer::start(0, "the panel's password").await;
    configure(folder.path(), &Saved { port: smtp.port, username: "mailer", mailing: true, alerts: true });
    set_secret(folder.path(), &secrets::SMTP_PASSWORD, "the panel's password");
    // The same database under another master key.
    let elsewhere = TempDir::new();
    copy_database(folder.path(), elsewhere.path());

    let worker = start_worker(elsewhere.path(), &[]).await;
    until("a login", || !smtp.seen().logins.is_empty()).await;
    until("the failed try", || row(elsewhere.path()).attempts == 1).await;
    let log = worker.log();
    assert!(worker.stop().await);

    // The server asked for a password, and this instance had none to give.
    assert_eq!(smtp.seen().logins, [("mailer".to_string(), String::new())]);
    assert!(smtp.seen().mails.is_empty());
    let recorded = row(elsewhere.path());
    assert!(recorded.last_error.contains("535"), "{}", recorded.last_error);
    assert!(log.iter().any(|line| line.contains("mail.smtp-password") && line.contains("counts as unset")), "{log:?}");
    assert!(!log.iter().any(|line| line.contains("the panel's password")));
}

#[tokio::test]
async fn with_mail_or_alerts_off_nothing_is_sent_and_the_alert_waits_an_hour() {
    for (mailing, alerts) in [(false, true), (true, false)] {
        let folder = with_a_queued_alert("owner").await;
        let smtp = SmtpServer::start(0, "").await;
        configure(folder.path(), &Saved { port: smtp.port, username: "", mailing, alerts });

        let worker = start_worker(folder.path(), &[]).await;
        until("the alert is put off", || minutes_from_now_of(folder.path()) >= 59).await;
        assert!(worker.stop().await);

        assert_eq!(smtp.seen().connections, 0);
        let recorded = row(folder.path());
        assert_eq!((recorded.attempts, recorded.sent_at), (0, None));
        assert_eq!(minutes_from_now(recorded.next_attempt_at.unwrap()), 60);
    }
}

fn minutes_from_now_of(folder: &Path) -> i64 {
    row(folder).next_attempt_at.map_or(0, minutes_from_now)
}

#[tokio::test]
async fn a_server_that_is_down_is_retried_after_five_minutes_and_the_alert_goes_once_it_is_back() {
    let folder = with_a_queued_alert("owner").await;
    let port = free_port();
    configure(folder.path(), &Saved { port, username: "", mailing: true, alerts: true });

    let first = start_worker(folder.path(), &[]).await;
    until("the failed try", || row(folder.path()).attempts == 1).await;
    assert!(first.stop().await);
    let failed = row(folder.path());
    assert_eq!(failed.sent_at, None);
    assert!(!failed.last_error.is_empty());
    assert_eq!(minutes_from_now(failed.next_attempt_at.unwrap()), 5);

    // Nothing is tried before the five minutes are up.
    let smtp = SmtpServer::start(port, "").await;
    let early = start_worker(folder.path(), &[]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(early.stop().await);
    assert_eq!(smtp.seen().connections, 0);

    // Five minutes later, as far as the alert is concerned.
    open(folder.path())
        .execute("UPDATE \"SecurityNotifications\" SET \"NextAttemptAt\" = ?1", [Timestamp::now().minus(Duration::from_secs(1))])
        .unwrap();
    let second = start_worker(folder.path(), &[]).await;
    until("the message", || !smtp.seen().mails.is_empty()).await;
    assert!(second.stop().await);

    assert_eq!(smtp.seen().mails.len(), 1);
    let recorded = row(folder.path());
    assert_eq!((recorded.attempts, recorded.last_error.as_str()), (2, ""));
    assert!(recorded.sent_at.is_some());
}

#[tokio::test]
async fn the_log_holds_no_password_no_address_and_no_text_of_the_message() {
    let folder = with_a_queued_alert("zebramark").await;
    let smtp = SmtpServer::start(0, "hunter2-marker").await;
    configure(folder.path(), &Saved { port: smtp.port, username: "mailer", mailing: true, alerts: true });
    set_secret(folder.path(), &secrets::SMTP_PASSWORD, "hunter2-marker");

    let worker = start_worker(folder.path(), &[("Logging__LogLevel__Default", "Trace")]).await;
    until("the message", || !smtp.seen().mails.is_empty()).await;
    let log = worker.log().join("\n").to_lowercase();
    assert!(worker.stop().await);

    // The log is not empty: it holds what the SMTP client says at this level.
    assert!(log.contains("smtp reply"), "{log}");
    assert!(smtp.seen().mails[0].contains("Zebramark"));
    for secret in ["hunter2", "zebramark", "example.test", "this just happened on your account", "noreply@chat.test"] {
        assert!(!log.contains(secret), "the log holds {secret}:\n{log}");
    }
}

// ---- a real mail server: Mailpit ----

mod mailpit {
    use std::net::SocketAddr;

    use http::Method;
    use serde_json::Value;

    use super::*;

    fn ports() -> (u16, SocketAddr) {
        let value = std::env::var("STELLARR_TEST_MAILPIT")
            .expect("STELLARR_TEST_MAILPIT is not set: the SMTP and the HTTP port, comma between");
        let (smtp, http) = value.split_once(',').expect("smtp,http");
        (smtp.parse().unwrap(), format!("127.0.0.1:{http}").parse().unwrap())
    }

    async fn call(api: SocketAddr, method: Method, path: &str) -> Value {
        let answer = Client::anonymous(api).send(Request::new(method, path)).await;
        assert!(answer.status.is_success(), "{path}: {}", answer.text());
        serde_json::from_str(&answer.text()).unwrap_or(Value::Null)
    }

    async fn empty(api: SocketAddr) {
        call(api, Method::DELETE, "/api/v1/messages").await;
    }

    async fn messages(api: SocketAddr) -> Vec<Value> {
        let list = call(api, Method::GET, "/api/v1/messages").await;
        let mut found = Vec::new();
        for summary in list["messages"].as_array().cloned().unwrap_or_default() {
            let id = summary["ID"].as_str().unwrap();
            found.push(call(api, Method::GET, &format!("/api/v1/message/{id}")).await);
        }
        found
    }

    /// The worker is already running when the person signs in, so the alert has
    /// to be found by one of its looks, which come every thirty seconds.
    #[tokio::test]
    #[ignore = "needs Mailpit; see the header of the file"]
    async fn an_alert_reaches_mailpit_within_the_workers_interval_in_both_languages() {
        let (smtp, api) = ports();
        for (language, subject, activity) in [
            ("en", "Security activity on Stellarr", "Signed in"),
            ("pl", "Aktywność zabezpieczeń w Stellarr", "Udane logowanie"),
        ] {
            empty(api).await;
            let folder = with_a_queued_alert("owner").await;
            configure(folder.path(), &Saved { port: smtp, username: "", mailing: true, alerts: true });
            // The queued row of the set-up is not what this test is about.
            open(folder.path()).execute("DELETE FROM \"SecurityNotifications\"", []).unwrap();
            let worker = start_worker(folder.path(), &[("STELLARR_LANGUAGE", language)]).await;
            let signed_in = std::time::Instant::now();
            assert_eq!(worker.login(&worker.client(), "owner", PASSWORD).await.status, 200);

            for _ in 0..1400 {
                if !messages(api).await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let waited = signed_in.elapsed();
            assert!(worker.stop().await);

            let found = messages(api).await;
            assert_eq!(found.len(), 1, "{language}");
            let mail = &found[0];
            assert!(waited < Duration::from_secs(35), "{language}: {waited:?}");
            assert_eq!(mail["Subject"], subject, "{language}");
            assert_eq!(mail["From"]["Address"], "noreply@chat.test");
            assert_eq!(mail["From"]["Name"], "Stellarr");
            assert_eq!(mail["To"][0]["Address"], "owner@example.test");
            let (text, html) = (mail["Text"].as_str().unwrap(), mail["HTML"].as_str().unwrap());
            assert!(text.contains(activity), "{text}");
            assert!(html.contains(&format!("lang=\"{language}\"")), "{html}");
            assert!(html.contains(activity), "{html}");
            println!("{language}: delivered {waited:?} after the sign-in");
        }
    }

    #[tokio::test]
    #[ignore = "needs Mailpit; see the header of the file"]
    async fn with_mail_off_nothing_arrives_and_a_server_that_was_down_gets_the_alert_once() {
        let (smtp, api) = ports();
        empty(api).await;

        // Off: nothing is sent, and the row is put off for an hour.
        let off = with_a_queued_alert("owner").await;
        configure(off.path(), &Saved { port: smtp, username: "", mailing: false, alerts: true });
        let worker = start_worker(off.path(), &[]).await;
        until("the alert is put off", || minutes_from_now_of(off.path()) >= 59).await;
        assert!(worker.stop().await);
        assert!(messages(api).await.is_empty());

        // Down: a port nobody listens on, then the real one, as the operator
        // would correct it, and the five minutes passed.
        let folder = with_a_queued_alert("owner").await;
        configure(folder.path(), &Saved { port: free_port(), username: "", mailing: true, alerts: true });
        let first = start_worker(folder.path(), &[]).await;
        until("the failed try", || row(folder.path()).attempts == 1).await;
        assert!(first.stop().await);
        assert!(messages(api).await.is_empty());
        configure(folder.path(), &Saved { port: smtp, username: "", mailing: true, alerts: true });
        open(folder.path())
            .execute("UPDATE \"SecurityNotifications\" SET \"NextAttemptAt\" = ?1", [Timestamp::now().minus(Duration::from_secs(1))])
            .unwrap();
        let second = start_worker(folder.path(), &[]).await;
        until("the message", || row(folder.path()).sent_at.is_some()).await;
        assert!(second.stop().await);

        assert_eq!(messages(api).await.len(), 1);
    }
}
