//! An instance to arrange rows in, and the servers that stand in for the
//! providers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_lc_rs::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, ECDH_P256, UnparsedPublicKey};
use aws_lc_rs::hkdf::{self, HKDF_SHA256};
use rusqlite::params;
use stellarr_outbound::AddressPolicy;
use stellarr_webpush::{GeneratedKeys, base64url, generate_keys};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::database::Database;
use crate::delivery::sender::{Sender, Wiring};
use crate::events::Unconnected;
use crate::permissions::ownership;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::state::AppState;
use crate::testing::TempDir;
use crate::values::{Id, Timestamp};

pub const PUBLIC_URL: &str = "https://chat.test";
/// The name the fake push service is reached by: the certificate is for it.
pub const PUSH_HOST: &str = "push.example.test";

pub fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).expect("a moment")
}

pub fn minutes(count: u64) -> Duration {
    Duration::from_secs(count * 60)
}

/// An instance with the roles, the channel and the people the notifications
/// are about.
pub struct World {
    pub _folder: TempDir,
    pub state: AppState,
    #[allow(dead_code)]
    pub vapid: GeneratedKeys,
    pub server: Id,
    pub channel: Id,
    pub maja: Id,
    pub ada: Id,
    base_role: Id,
}

/// The keys a browser subscribes with.
pub struct Browser {
    pub id: Id,
    pub session: Id,
    pub endpoint: String,
    pub private: agreement::PrivateKey,
    pub p256dh: Vec<u8>,
    pub auth: [u8; 16],
}

impl World {
    pub async fn new(extra: &[(&str, &str)]) -> World {
        let folder = TempDir::new();
        let vapid = generate_keys().expect("a key pair");
        let mut variables: Vec<(String, String)> = vec![
            ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
            (
                "STELLARR_DATA_ROOT".into(),
                folder.path().display().to_string(),
            ),
            ("Operations__PublicUrl".into(), PUBLIC_URL.into()),
            ("STELLARR_PUSH_ENABLED".into(), "true".into()),
            ("STELLARR_PUSH_SUBJECT".into(), "mailto:push@example.test".into()),
            ("STELLARR_PUSH_PUBLIC_KEY".into(), vapid.public_key.clone()),
            ("STELLARR_PUSH_PRIVATE_KEY".into(), vapid.private_key.clone()),
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
        let (server, channel, base_role) = (Id::random(), Id::random(), Id::random());
        let (maja, ada) = (Id::random(), Id::random());
        state
            .database()
            .write(move |tx| -> rusqlite::Result<()> {
                tx.execute(
                    "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                     VALUES (?1, 'Orbit', '', 0, ?2, 0, 0)",
                    params![server, Timestamp::now()],
                )?;
                tx.execute(
                    "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\")
                     VALUES (?1, ?2, 'Member', '#777777', ?3, 2, 1, 0)",
                    params![base_role, server, crate::permissions::privileges::BASE_ROLE_DEFAULTS],
                )?;
                tx.execute(
                    "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
                     VALUES (?1, ?2, 'general', 0, 0, '', 0, 0, 0, NULL)",
                    params![channel, server],
                )?;
                for (id, name) in [(maja, "maja"), (ada, "ada")] {
                    insert_user(tx, id, name)?;
                    ownership::admit(tx, server, id, base_role, true, None, Timestamp::now())?;
                }
                Ok(())
            })
            .await
            .expect("an instance");
        World {
            _folder: folder,
            state,
            vapid,
            server,
            channel,
            maja,
            ada,
            base_role,
        }
    }

    /// A member of the server.
    pub async fn member(&self, name: &str) -> Id {
        let (id, server, role) = (Id::random(), self.server, self.base_role);
        let name = name.to_string();
        self.write(move |tx| {
            insert_user(tx, id, &name)?;
            ownership::admit(tx, server, id, role, true, None, Timestamp::now())
        })
        .await;
        id
    }

    pub async fn write<T: Send + 'static>(
        &self,
        work: impl FnOnce(&rusqlite::Transaction) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        self.state.database().write(work).await.expect("a write")
    }

    pub async fn read<T: Send + 'static>(
        &self,
        work: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        self.state.database().read(work).await.expect("a read")
    }

    /// Chooses what a person wants to be told by.
    pub async fn prefer(&self, user: Id, email: bool, push: bool) {
        self.write(move |tx| {
            tx.execute(
                "INSERT OR REPLACE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\",
                     \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\",
                     \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\",
                     \"MutedChannelIdsJson\", \"VadSensitivity\")
                 VALUES (?1, 1, 1, 1, ?2, ?3, 1, 1, 1, 1, 1, 1, 0, 0, '[]', 60)",
                params![user, email, push],
            )?;
            Ok(())
        })
        .await;
    }

    /// Gives a person an address they have confirmed by mail.
    pub async fn verify_email(&self, user: Id, address: &str) {
        let address = address.to_string();
        self.write(move |tx| {
            tx.execute(
                "UPDATE \"Users\" SET \"Email\" = ?2, \"NormalizedEmail\" = ?3 WHERE \"Id\" = ?1",
                params![user, address, address.to_uppercase()],
            )?;
            tx.execute(
                "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"EmailAddress\")
                 VALUES (?1, ?2, 'email', 'E-mail', ?3, ?3, ?4)",
                params![Id::random(), user, Timestamp::now(), address.to_uppercase()],
            )?;
            Ok(())
        })
        .await;
    }

    /// A sign-in of the person that has not ended.
    pub async fn session(&self, user: Id) -> Id {
        let id = Id::random();
        self.write(move |tx| {
            let now = Timestamp::now();
            let version: i64 = tx.query_row(
                "SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
                     \"AbsoluteExpiresAt\", \"PrimarySource\", \"SecurityVersion\", \"ShortLived\")
                 VALUES (?1, ?2, 'test', '127.0.0.1', ?3, ?3, ?4, ?4, 'password', ?5, 0)",
                params![id, user, now, now.plus(Duration::from_secs(86_400 * 365)), version],
            )?;
            Ok(())
        })
        .await;
        id
    }

    /// A browser that subscribed from a sign-in of the person, with keys of
    /// its own that the test can decrypt with.
    pub async fn browser(&self, user: Id, path: &str) -> Browser {
        let session = self.session(user).await;
        self.browser_in(user, session, path).await
    }

    pub async fn browser_in(&self, user: Id, session: Id, path: &str) -> Browser {
        let private = agreement::PrivateKey::generate(&ECDH_P256).unwrap();
        let p256dh = private.compute_public_key().unwrap().as_ref().to_vec();
        let mut auth = [0u8; 16];
        aws_lc_rs::rand::fill(&mut auth).unwrap();
        let id = Id::random();
        let endpoint = format!("https://{PUSH_HOST}/{path}");
        let (stored_endpoint, key, secret) = (
            endpoint.clone(),
            base64url::encode(&p256dh),
            base64url::encode(&auth),
        );
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO \"PushSubscriptions\" (\"Id\", \"UserId\", \"SessionId\", \"Endpoint\", \"P256dh\", \"Auth\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, user, session, stored_endpoint, key, secret, Timestamp::now()],
            )?;
            Ok(())
        })
        .await;
        Browser {
            id,
            session,
            endpoint,
            private,
            p256dh,
            auth,
        }
    }

    /// A message in the channel and the owed notices it asked for.
    pub async fn mention(&self, author: Id, content: &str, recipients: &[Id]) -> Id {
        let (id, channel) = (Id::random(), self.channel);
        let content = content.to_string();
        let recipients = recipients.to_vec();
        self.write(move |tx| {
            let now = Timestamp::now();
            tx.execute(
                "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, 0, ?4, ?5)",
                params![id, channel, author, content, now],
            )?;
            crate::delivery::outbox::enqueue_mentions(tx, id, recipients, now)
        })
        .await;
        id
    }

    /// A conversation of two people, a message of the first, and the notice
    /// the second is owed.
    pub async fn direct_message(&self, author: Id, peer: Id) -> (Id, Id) {
        let (conversation, message) = (Id::random(), Id::random());
        self.write(move |tx| {
            let now = Timestamp::now();
            let (a, b) = if author < peer { (author, peer) } else { (peer, author) };
            tx.execute(
                "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\")
                 VALUES (?1, ?2, ?3, ?4, 'none')",
                params![conversation, a, b, now],
            )?;
            tx.execute(
                "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, 'sekret', ?4)",
                params![message, conversation, author, now],
            )?;
            crate::delivery::outbox::enqueue_direct_message(tx, message, peer, now)
        })
        .await;
        (conversation, message)
    }

    pub async fn rows(&self, source: Id) -> Vec<Row> {
        self.read(move |db| {
            let mut statement = db.prepare(
                "SELECT \"Id\", \"RecipientId\", \"EventKind\", \"Channel\", \"DestinationKey\", \"PushSubscriptionId\", \"CreatedAt\",
                        \"ExpiresAt\", \"Attempts\", \"NextAttemptAt\", \"LastAttemptAt\", \"SentAt\", \"Status\", \"LeaseId\",
                        \"LeaseUntil\", \"LastError\"
                 FROM \"NotificationDeliveries\" WHERE \"SourceMessageId\" = ?1 ORDER BY \"Channel\", \"DestinationKey\"",
            )?;
            statement
                .query_map([source], |row| {
                    Ok(Row {
                        id: row.get(0)?,
                        recipient: row.get(1)?,
                        kind: row.get(2)?,
                        channel: row.get(3)?,
                        key: row.get(4)?,
                        subscription: row.get(5)?,
                        created_at: row.get(6)?,
                        expires_at: row.get(7)?,
                        attempts: row.get(8)?,
                        next_attempt_at: row.get(9)?,
                        last_attempt_at: row.get(10)?,
                        sent_at: row.get(11)?,
                        status: row.get(12)?,
                        lease: row.get(13)?,
                        lease_until: row.get(14)?,
                        last_error: row.get(15)?,
                    })
                })?
                .collect()
        })
        .await
    }

    pub async fn subscriptions(&self, user: Id) -> i64 {
        self.read(move |db| {
            db.query_row(
                "SELECT COUNT(*) FROM \"PushSubscriptions\" WHERE \"UserId\" = ?1",
                [user],
                |row| row.get(0),
            )
        })
        .await
    }
}

fn insert_user(tx: &rusqlite::Connection, id: Id, name: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\",
             \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?2, 'hash', 0, 0, 0, '#777777', ?4)",
        params![id, name, name.to_uppercase(), Timestamp::now()],
    )?;
    Ok(())
}

/// A row of the outbox as the database holds it.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: Id,
    pub recipient: Id,
    pub kind: String,
    pub channel: String,
    pub key: String,
    pub subscription: Option<Id>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub attempts: i64,
    pub next_attempt_at: Timestamp,
    pub last_attempt_at: Option<Timestamp>,
    pub sent_at: Option<Timestamp>,
    pub status: String,
    pub lease: Option<Id>,
    pub lease_until: Option<Timestamp>,
    pub last_error: String,
}

// ---- a stand-in for a push service ----

/// What the fake push service does with a request.
#[derive(Clone)]
pub enum Behaviour {
    /// Answers with this status.
    Status(u16),
    /// Takes this many milliseconds to answer 201.
    Slow(u64),
    /// Answers a request for this path with the first status, any other
    /// with the second.
    Split(&'static str, u16, u16),
    /// Answers with this status and a header.
    StatusWith(u16, &'static str, &'static str),
    /// Answers 201 with a body of this many bytes.
    BigBody(usize),
    /// Accepts the request and never answers.
    Silent,
    /// Starts an answer and closes the connection before it is whole.
    CloseMidHead,
    /// Answers 201 and closes the connection before the body is whole.
    CloseMidBody,
    /// Answers 307 to this address.
    Redirect(String),
}

/// What one request to the fake looked like.
#[derive(Clone)]
pub struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub struct FakePush {
    pub port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    task: JoinHandle<()>,
}

impl Drop for FakePush {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakePush {
    pub async fn start(behaviour: Behaviour) -> FakePush {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let acceptor = tokio_rustls::TlsAcceptor::from(certificates::server());
        let recorded = seen.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (acceptor, behaviour, recorded) =
                    (acceptor.clone(), behaviour.clone(), recorded.clone());
                tokio::spawn(async move {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        let _ = serve(stream, behaviour, recorded).await;
                    }
                });
            }
        });
        FakePush { port, seen, task }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn serve<S: AsyncReadExt + AsyncWriteExt + Unpin>(
    mut stream: S,
    behaviour: Behaviour,
    recorded: Arc<Mutex<Vec<Seen>>>,
) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let head_end = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
    let mut lines = head.lines();
    let path = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("")
        .to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    recorded.lock().unwrap().push(Seen {
        path,
        headers,
        body: buffer[head_end..].to_vec(),
    });
    match behaviour {
        Behaviour::Status(status) => {
            let text = format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(text.as_bytes()).await?;
        }
        Behaviour::Slow(milliseconds) => {
            tokio::time::sleep(Duration::from_millis(milliseconds)).await;
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await?;
        }
        Behaviour::Split(path, special, otherwise) => {
            let status = if recorded.lock().unwrap().last().is_some_and(|seen| seen.path == path) {
                special
            } else {
                otherwise
            };
            let text = format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(text.as_bytes()).await?;
        }
        Behaviour::StatusWith(status, name, value) => {
            let text = format!(
                "HTTP/1.1 {status} X\r\n{name}: {value}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(text.as_bytes()).await?;
        }
        Behaviour::BigBody(size) => {
            let head = format!("HTTP/1.1 201 Created\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n");
            stream.write_all(head.as_bytes()).await?;
            let chunk = vec![b'x'; 64 * 1024];
            let mut left = size;
            while left > 0 {
                let n = left.min(chunk.len());
                if stream.write_all(&chunk[..n]).await.is_err() {
                    break;
                }
                left -= n;
            }
        }
        Behaviour::Silent => {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        Behaviour::CloseMidHead => {
            stream.write_all(b"HTTP/1.1 201 Cre").await?;
        }
        Behaviour::CloseMidBody => {
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nabc")
                .await?;
        }
        Behaviour::Redirect(to) => {
            let text = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(text.as_bytes()).await?;
        }
    }
    let _ = stream.shutdown().await;
    Ok(())
}

/// A sender whose push service is the fake on the loopback: the only address
/// it may reach, by the name the certificate is for.
pub fn wired_to(world: &World, fake: &FakePush) -> Arc<Sender> {
    Arc::new(Sender::wired(
        &world.state.settings().push,
        Wiring {
            roots: Some(certificates::roots()),
            resolver: Some(Arc::new(stellarr_outbound::TableResolver(vec![(
                PUSH_HOST.to_string(),
                vec!["127.0.0.1".parse().unwrap()],
            )]))),
            policy: AddressPolicy::LoopbackOnly,
            port: Some(fake.port),
        },
    ))
}

pub mod certificates {
    use std::path::Path;

    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    use super::*;

    fn read(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/delivery/tests/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    pub fn roots() -> rustls::RootCertStore {
        stellarr_outbound::tls::roots_from_pem(&read("ca.crt.txt")).unwrap()
    }

    pub fn server() -> Arc<rustls::ServerConfig> {
        let chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(&read("server.crt.txt"))
                .collect::<Result<_, _>>()
                .unwrap();
        let key = PrivateKeyDer::from_pem_slice(&read("server.pk8.txt")).unwrap();
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

// ---- reading what was sent ----

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn derive(salt: &[u8], secret: &[u8], info: &[u8], size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size];
    hkdf::Salt::new(HKDF_SHA256, salt)
        .extract(secret)
        .expand(&[info], Len(size))
        .unwrap()
        .fill(&mut out)
        .unwrap();
    out
}

/// What the browser reads: the body of a push request, opened with the
/// browser's own keys (RFC 8291). Returns the record size the header
/// declares and the plaintext.
pub fn open(browser: &Browser, body: &[u8]) -> (u32, Vec<u8>) {
    let salt = &body[..16];
    let record_size = u32::from_be_bytes(body[16..20].try_into().unwrap());
    let key_length = body[20] as usize;
    let sender = &body[21..21 + key_length];
    let mut record = body[21 + key_length..].to_vec();
    let shared = agreement::agree(
        &browser.private,
        UnparsedPublicKey::new(&ECDH_P256, sender),
        (),
        |secret| Ok(secret.to_vec()),
    )
    .unwrap();
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&browser.p256dh);
    info.extend_from_slice(sender);
    let ikm = derive(&browser.auth, &shared, &info, 32);
    let key = derive(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce: [u8; 12] = derive(salt, &ikm, b"Content-Encoding: nonce\0", 12)
        .try_into()
        .unwrap();
    let opening = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &key).unwrap());
    let plain = opening
        .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut record)
        .unwrap()
        .to_vec();
    let end = plain.iter().rposition(|&byte| byte != 0).unwrap();
    assert_eq!(plain[end], 2, "the last record ends with its delimiter");
    (record_size, plain[..end].to_vec())
}

// ---- a stand-in for a mail server ----

/// What the fake mail server does with a message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Takes,
    /// Refuses the recipient for good (550).
    Refuses,
    /// Asks for a later try (451).
    Busy,
}

pub struct FakeSmtp {
    pub port: u16,
    mails: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for FakeSmtp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeSmtp {
    pub async fn start(mood: Mood) -> FakeSmtp {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mails: Arc<Mutex<Vec<String>>> = Arc::default();
        let kept = mails.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let kept = kept.clone();
                tokio::spawn(async move {
                    let _ = talk(stream, mood, kept).await;
                });
            }
        });
        FakeSmtp { port, mails, task }
    }

    pub fn mails(&self) -> Vec<String> {
        self.mails.lock().unwrap().clone()
    }
}

async fn talk(
    mut stream: tokio::net::TcpStream,
    mood: Mood,
    kept: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let (read, mut write) = stream.split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"220 fake ESMTP\r\n").await?;
    while let Some(line) = lines.next_line().await? {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            write.write_all(b"250 fake\r\n").await?;
        } else if upper.starts_with("RCPT") && mood != Mood::Takes {
            let reply: &[u8] = if mood == Mood::Refuses {
                b"550 no such mailbox\r\n"
            } else {
                b"451 try later\r\n"
            };
            write.write_all(reply).await?;
        } else if upper.starts_with("DATA") {
            write.write_all(b"354 go on\r\n").await?;
            let mut data = String::new();
            while let Some(line) = lines.next_line().await? {
                if line == "." {
                    break;
                }
                data.push_str(&line);
                data.push('\n');
            }
            kept.lock().unwrap().push(data);
            write.write_all(b"250 queued\r\n").await?;
        } else if upper.starts_with("QUIT") {
            write.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else {
            write.write_all(b"250 ok\r\n").await?;
        }
    }
    Ok(())
}

impl World {
    /// Saves the operator's mail settings: mail on, from this address, over
    /// plain SMTP to this port.
    pub async fn mail_to(&self, port: u16) {
        let server = self.server;
        self.write(move |tx| {
            tx.execute(
                "INSERT OR REPLACE INTO \"OperationsSettings\"
                    (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\",
                     \"MailSmtpPort\", \"MailSmtpUsername\", \"MailSmtpSecurity\", \"MailResendEnabled\", \"InviteMailEnabled\",
                     \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
                     \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\")
                 VALUES (?1, '', 'UTC', 1, 'noreply@chat.test', '127.0.0.1', ?2, '', 'none', NULL, 1, 1, 0, 14, 30, 1, 1, ?3)",
                params![server, port, Timestamp::now()],
            )?;
            Ok(())
        })
        .await;
    }
}
