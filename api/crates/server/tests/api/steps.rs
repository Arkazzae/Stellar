//! What the tests of the second step stand on: an instance that asks everybody
//! for one, accounts to sign in as, a mail server that keeps what it is given,
//! and the answers of an authenticator app.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::auth::totp;
use stellarr_api::values::{Id, Timestamp};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::support::{Client, PASSWORD, Response, Server};

/// The address passkeys are bound to in these tests.
pub const PUBLIC_URL: &str = "https://chat.test";

/// A mail server that keeps every message it is given.
pub struct Smtp {
    pub port: u16,
    mails: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for Smtp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Smtp {
    pub async fn start() -> Smtp {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mails = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&mails);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mails = Arc::clone(&shared);
                tokio::spawn(async move { converse(stream, &mails).await });
            }
        });
        Smtp { port, mails, task }
    }

    /// The messages that have arrived, oldest first.
    pub fn mails(&self) -> Vec<String> {
        self.mails.lock().unwrap().clone()
    }

    /// Waits until `count` messages have arrived, and gives them.
    pub async fn until(&self, count: usize) -> Vec<String> {
        for _ in 0..400 {
            if self.mails().len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        self.mails()
    }
}

async fn converse(stream: tokio::net::TcpStream, mails: &Mutex<Vec<String>>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let _ = write.write_all(b"220 fake.test ESMTP\r\n").await;
    while let Ok(Some(line)) = lines.next_line().await {
        let verb = line.split(' ').next().unwrap_or("").to_ascii_uppercase();
        let answer = match verb.as_str() {
            "EHLO" => "250-fake.test\r\n250 8BITMIME".to_string(),
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
                mails.lock().unwrap().push(data);
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

/// The six digits a mail carries on a line of their own.
pub fn code_in(mail: &str) -> String {
    mail.lines()
        .map(str::trim)
        .find(|line| line.len() == 6 && line.bytes().all(|byte| byte.is_ascii_digit()))
        .unwrap_or_else(|| panic!("no code in {mail}"))
        .to_string()
}

/// The token of a reset link, as the mail spells it.
pub fn token_in(mail: &str) -> String {
    let unwrapped = mail.replace("=\n", "").replace("=3D", "=");
    let after = unwrapped.split("token=").nth(1).expect("a link with a token");
    after
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// An instance with an owner, accounts that can sign in with their password,
/// and a mail server.
pub struct Instance {
    pub server: Server,
    pub smtp: Smtp,
}

impl Instance {
    /// An instance whose public address is [`PUBLIC_URL`], with the accounts
    /// named, whose addresses are `<name>@example.test`. Nothing asks for a
    /// second step until [`Instance::require`] says so.
    pub async fn with(accounts: &[&str], extra: &[(&str, &str)]) -> Instance {
        let smtp = Smtp::start().await;
        let mut variables = vec![("Operations__PublicUrl", PUBLIC_URL)];
        variables.extend_from_slice(extra);
        let server = Server::start_with(&variables).await;
        server.found("owner").await;
        for name in accounts {
            server.register(name).await;
        }
        let instance = Instance { server, smtp };
        instance.settings("MfaRequirement = 'optional'");
        instance
    }

    /// Changes the settings of the instance's server, as the operator's panel
    /// does.
    pub fn settings(&self, assignments: &str) {
        let connection = self.server.database();
        let server: Id = connection
            .query_row(
                "SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO \"OperationsSettings\"
                    (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\",
                     \"MailSmtpPort\", \"MailSmtpUsername\", \"MailSmtpSecurity\", \"MailResendEnabled\", \"InviteMailEnabled\",
                     \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
                     \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\")
                 VALUES (?1, '', 'UTC', 1, 'noreply@chat.test', '127.0.0.1', ?2, '', 'none', 0, 1, 1, 0, 14, 30, 1, 1, ?3)",
                params![server, self.smtp.port, Timestamp::now()],
            )
            .unwrap();
        connection
            .execute(&format!("UPDATE \"OperationsSettings\" SET {assignments}"), [])
            .unwrap();
    }

    /// Everybody must have a second step.
    pub fn require(&self) {
        self.settings("\"MfaRequirement\" = 'everyone'");
    }

    /// The first request of a sign-in with a password, from a browser of its
    /// own.
    pub async fn start(&self, name: &str) -> (Client, Value) {
        let client = self.server.client();
        let response = self.server.login(&client, name, PASSWORD).await;
        assert_eq!(response.status, 200, "{}", response.text());
        let body = response.json::<Value>();
        (client, body)
    }

    /// Sets an authenticator app up for `name`, remembering the browser when
    /// asked, and returns the browser (signed in), the secret, the method and
    /// the recovery codes.
    pub async fn enrolled(&self, name: &str, remember: bool) -> Enrolled {
        let (client, challenge) = self.start(name).await;
        let id = challenge["challengeId"].as_str().unwrap().to_string();
        let started = client
            .post_json(
                "/api/auth/mfa/enroll/totp/start",
                &json!({"challengeId": id, "name": "Phone"}),
            )
            .await;
        assert_eq!(started.status, 200, "{}", started.text());
        let started = started.json::<Value>();
        let secret = started["secret"].as_str().unwrap().to_string();
        let method = started["methodId"].as_str().unwrap().to_string();
        let confirmed = client
            .post_json(
                "/api/auth/mfa/enroll/totp/confirm",
                &json!({"challengeId": id, "methodId": method, "code": code(&secret, 0),
                        "name": "Phone", "rememberDevice": remember}),
            )
            .await;
        assert_eq!(confirmed.status, 200, "{}", confirmed.text());
        let codes = confirmed.json::<Value>()["recoveryCodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap().to_string())
            .collect();
        Enrolled {
            client,
            secret,
            method,
            codes,
        }
    }

    /// The security events of an account, oldest first, as `kind detail`.
    pub fn events(&self, name: &str) -> Vec<String> {
        let connection = self.server.database();
        let mut statement = connection
            .prepare(
                "SELECT e.\"Kind\", e.\"Detail\" FROM \"SecurityEvents\" e JOIN \"Users\" u ON u.\"Id\" = e.\"UserId\"
                 WHERE u.\"Username\" = ?1 ORDER BY e.\"CreatedAt\", e.rowid",
            )
            .unwrap();
        statement
            .query_map([name], |row| {
                Ok(format!("{} {}", row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn database(&self) -> Connection {
        self.server.database()
    }
}

/// An account that has an authenticator app, and a browser signed in with it.
pub struct Enrolled {
    pub client: Client,
    pub secret: String,
    pub method: String,
    pub codes: Vec<String>,
}

/// The code the app shows `steps` periods from now.
pub fn code(secret: &str, steps: u64) -> String {
    let at = Timestamp::now().plus(Duration::from_secs(30 * steps));
    totp::code_at(secret, at).expect("a code")
}

/// The `Set-Cookie` lines of an answer, in order.
pub fn cookies(response: &Response) -> Vec<String> {
    response
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_string())
        .collect()
}

/// The names of the cookies an answer sets, in order.
pub fn cookie_names(response: &Response) -> Vec<String> {
    cookies(response)
        .iter()
        .map(|line| line.split('=').next().unwrap().to_string())
        .collect()
}

/// The title of a problem.
pub fn title(response: &Response) -> String {
    response.json::<Value>()["title"].as_str().unwrap_or("").to_string()
}
