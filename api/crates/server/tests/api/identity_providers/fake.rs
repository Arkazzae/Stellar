//! A provider of our own, over TLS, that the server under test is pointed at.
//!
//! It publishes a discovery document and a key set, redeems any code for an
//! ID token about whoever the script says is signing in, and answers the
//! profile request. Every part can be made to misbehave: a token signed with
//! another key or not at all, claims changed or taken out, an answer that is
//! slow, wrong or moved.
//!
//! The server reaches it by `https://localhost:<port>`, trusting the
//! certificate authority of [`super::certs`] through `SSL_CERT_FILE`.

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::certs;
use crate::support::TempDir;

pub const CLIENT_ID: &str = "stellarr-tests";
pub const CLIENT_SECRET: &str = "test-client-secret";
pub const CODE: &str = "the-code";

fn b64(bytes: &[u8]) -> String {
    use base64_like::encode;
    encode(bytes)
}

/// Base64url without padding, which is all the tokens need.
mod base64_like {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
                << (8 * (3 - chunk.len()));
            for i in 0..=chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }
}

/// How the ID token is signed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Signing {
    /// With the key the provider publishes.
    Published,
    /// With a key it does not publish, under the published key's id.
    Stranger,
    /// With a key it does not publish, under an id nobody has.
    StrangerUnderOwnId,
    /// Not at all, with `alg` set to `none`.
    Unsigned,
}

/// What a path answers instead of its usual answer.
#[derive(Clone, Default)]
pub struct Instead {
    pub status: Option<u16>,
    pub body: Option<String>,
    pub content_type: Option<String>,
    pub location: Option<String>,
    pub delay: Duration,
}

/// What a test can change about the provider while it runs.
pub struct Script {
    pub subject: String,
    pub nonce: String,
    pub amr: Option<Vec<String>>,
    pub auth_time: Option<i64>,
    /// Claims added to the ID token; `null` takes one out.
    pub claims: Map<String, Value>,
    /// The profile the userinfo endpoint answers with; none means the provider
    /// has no such endpoint.
    pub userinfo: Option<Map<String, Value>>,
    pub signing: Signing,
    pub alg: String,
    /// What the discovery document says the issuer is, when not its own.
    pub issuer_in_discovery: Option<String>,
    pub instead: HashMap<String, Instead>,
    /// Every request the provider got: method, path, the form it carried.
    pub seen: Vec<(String, String, String)>,
    /// Whether the token response leaves the access token out.
    pub no_access_token: bool,
    published: Arc<EcdsaKeyPair>,
}

pub struct FakeProvider {
    address: SocketAddr,
    pub script: Arc<Mutex<Script>>,
    ca: std::path::PathBuf,
    _folder: TempDir,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn key_pair() -> Arc<EcdsaKeyPair> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    Arc::new(EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap())
}

impl FakeProvider {
    pub async fn start() -> FakeProvider {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let folder = TempDir::new();
        let ca = folder.path().join("ca-bundle.crt");
        std::fs::File::create(&ca)
            .unwrap()
            .write_all(certs::CA.as_bytes())
            .unwrap();
        let script = Arc::new(Mutex::new(Script {
            subject: format!("subject-{}", stellarr_api::values::Id::random().to_string().replace('-', "")),
            nonce: String::new(),
            amr: None,
            auth_time: None,
            claims: Map::new(),
            userinfo: None,
            signing: Signing::Published,
            alg: "ES256".into(),
            issuer_in_discovery: None,
            instead: HashMap::new(),
            seen: Vec::new(),
            no_access_token: false,
            published: key_pair(),
        }));
        let acceptor = TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(certs::CERTIFICATE.as_bytes()).unwrap()],
                PrivateKeyDer::from_pem_slice(certs::KEY.as_bytes()).unwrap(),
            )
            .unwrap(),
        ));
        let issuer = format!("https://localhost:{}", address.port());
        let shared = script.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (acceptor, script, issuer) = (acceptor.clone(), shared.clone(), issuer.clone());
                tokio::spawn(async move {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        serve(stream, script, issuer).await;
                    }
                });
            }
        });
        FakeProvider {
            address,
            script,
            ca,
            _folder: folder,
            task,
        }
    }

    pub fn issuer(&self) -> String {
        format!("https://localhost:{}", self.address.port())
    }

    /// The variables that make a server trust this provider's certificate.
    pub fn trust(&self) -> (String, String) {
        ("SSL_CERT_FILE".to_string(), self.ca.display().to_string())
    }

    /// The settings of the original provider, with the display name `name`.
    pub fn settings(&self, name: &str) -> Vec<(String, String)> {
        vec![
            self.trust(),
            ("Oidc__Enabled".into(), "true".into()),
            ("Oidc__Authority".into(), self.issuer()),
            ("Oidc__ClientId".into(), CLIENT_ID.into()),
            ("Oidc__ClientSecret".into(), CLIENT_SECRET.into()),
            ("Oidc__DisplayName".into(), name.into()),
        ]
    }

    /// The settings of a provider named `id`.
    pub fn named_settings(&self, id: &str, name: &str) -> Vec<(String, String)> {
        let prefix = format!("Oidc__Providers__{id}__");
        vec![
            self.trust(),
            (format!("{prefix}Authority"), self.issuer()),
            (format!("{prefix}ClientId"), CLIENT_ID.into()),
            (format!("{prefix}ClientSecret"), CLIENT_SECRET.into()),
            (format!("{prefix}DisplayName"), name.into()),
        ]
    }

    pub fn set_nonce(&self, nonce: &str) {
        self.script.lock().unwrap().nonce = nonce.to_string();
    }

    pub fn subject(&self) -> String {
        self.script.lock().unwrap().subject.clone()
    }

    pub fn requests(&self) -> Vec<(String, String, String)> {
        self.script.lock().unwrap().seen.clone()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn serve(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    script: Arc<Mutex<Script>>,
    issuer: String,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
    let form = String::from_utf8_lossy(&buffer[head_end..head_end + length]).into_owned();
    let mut first = head.lines().next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let path = target.split('?').next().unwrap_or("").to_string();
    let authorization = head
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        .map(|line| line.split_once(':').unwrap().1.trim().to_string())
        .unwrap_or_default();

    let (instead, answer) = {
        let mut script = script.lock().unwrap();
        script.seen.push((
            method.to_string(),
            path.clone(),
            if form.is_empty() { authorization.clone() } else { form.clone() },
        ));
        (script.instead.get(&path).cloned(), answer(&script, method, &path, &form, &authorization, &issuer))
    };
    let (mut status, mut body, mut content_type, mut location) = (answer.0, answer.1, "application/json".to_string(), None);
    if let Some(instead) = instead {
        if !instead.delay.is_zero() {
            tokio::time::sleep(instead.delay).await;
        }
        status = instead.status.unwrap_or(status);
        body = instead.body.unwrap_or(body);
        content_type = instead.content_type.unwrap_or(content_type);
        location = instead.location;
    }
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(location) = location {
        head.push_str(&format!("Location: {location}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn answer(
    script: &Script,
    method: &str,
    path: &str,
    form: &str,
    authorization: &str,
    issuer: &str,
) -> (u16, String) {
    match (method, path) {
        ("GET", "/.well-known/openid-configuration") => {
            let mut document = json!({
                "issuer": script.issuer_in_discovery.clone().unwrap_or_else(|| issuer.to_string()),
                "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"),
                "jwks_uri": format!("{issuer}/jwks"),
            });
            if script.userinfo.is_some() {
                document["userinfo_endpoint"] = json!(format!("{issuer}/userinfo"));
            }
            (200, document.to_string())
        }
        ("GET", "/jwks") => {
            let point = script.published.public_key().as_ref();
            let key = json!({
                "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig", "kid": "test-key",
                "x": b64(&point[1..33]), "y": b64(&point[33..65]),
            });
            (200, json!({ "keys": [key] }).to_string())
        }
        ("POST", "/token") => {
            if !form.contains(&format!("client_secret={CLIENT_SECRET}"))
                || !form.contains(&format!("code={CODE}"))
            {
                return (400, json!({ "error": "invalid_grant" }).to_string());
            }
            let mut answer = json!({
                "id_token": id_token(script, issuer),
                "token_type": "Bearer",
                "expires_in": 300,
            });
            if !script.no_access_token {
                answer["access_token"] = json!("access-token");
            }
            (200, answer.to_string())
        }
        ("GET", "/userinfo") => match &script.userinfo {
            Some(profile) if authorization == "Bearer access-token" => {
                let mut answer = profile.clone();
                answer.insert("sub".into(), json!(script.subject));
                (200, Value::Object(answer).to_string())
            }
            _ => (401, String::new()),
        },
        _ => (404, String::new()),
    }
}

fn id_token(script: &Script, issuer: &str) -> String {
    let mut claims = Map::new();
    claims.insert("iss".into(), json!(issuer));
    claims.insert("aud".into(), json!(CLIENT_ID));
    claims.insert("sub".into(), json!(script.subject));
    claims.insert("nonce".into(), json!(script.nonce));
    claims.insert("iat".into(), json!(now()));
    claims.insert("exp".into(), json!(now() + 300));
    if let Some(amr) = &script.amr {
        claims.insert("amr".into(), json!(amr));
    }
    if let Some(at) = script.auth_time {
        claims.insert("auth_time".into(), json!(at));
    }
    for (name, value) in &script.claims {
        if value.is_null() {
            claims.remove(name);
        } else {
            claims.insert(name.clone(), value.clone());
        }
    }
    let kid = if script.signing == Signing::StrangerUnderOwnId { "another-key" } else { "test-key" };
    let alg = if script.signing == Signing::Unsigned { "none" } else { script.alg.as_str() };
    let header = b64(json!({ "alg": alg, "typ": "JWT", "kid": kid }).to_string().as_bytes());
    let payload = b64(Value::Object(claims).to_string().as_bytes());
    let signed = format!("{header}.{payload}");
    let signature = match script.signing {
        Signing::Unsigned => String::new(),
        Signing::Published => b64(script.published.sign(&SystemRandom::new(), signed.as_bytes()).unwrap().as_ref()),
        Signing::Stranger | Signing::StrangerUnderOwnId => {
            b64(key_pair().sign(&SystemRandom::new(), signed.as_bytes()).unwrap().as_ref())
        }
    };
    format!("{signed}.{signature}")
}
