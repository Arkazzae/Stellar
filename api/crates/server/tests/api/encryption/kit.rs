//! What the encryption tests do the way a browser would, with bytes of no
//! meaning in place of keys and ciphertext: the server never opens either.

use aws_lc_rs::digest::{SHA256, digest};
use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::encoding::base64;
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, HubClient, Person, Request, Response, Server};

pub const ACCOUNT: &str = "/api/me/encryption";
pub const COOKIE: &str = "stellarr-dev-session";

/// `length` bytes that differ from the bytes of another `seed`.
pub fn bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed).wrapping_add(1))
        .collect()
}

pub fn text(bytes: &[u8]) -> String {
    base64(bytes)
}

/// The PIN-protected backup a browser makes, whose verifier is the digest of
/// `auth_key`.
pub fn backup(auth_key: &[u8]) -> Value {
    json!({
        "salt": text(&bytes(16, 1)),
        "iterations": 600_000,
        "verifier": text(digest(&SHA256, auth_key).as_ref()),
        "iv": text(&bytes(12, 2)),
        "ciphertext": text(&bytes(64, 3)),
    })
}

/// The account key's id and the PIN proof its backup was made with.
pub struct Account {
    pub key_id: String,
    pub auth_key: Vec<u8>,
}

pub async fn send(client: &Client, method: Method, path: &str, body: Option<&Value>) -> Response {
    let request = Request::new(method, path);
    client
        .send(match body {
            Some(body) => request.json(body),
            None => request,
        })
        .await
}

pub async fn post(client: &Client, path: &str, body: &Value) -> Response {
    send(client, Method::POST, path, Some(body)).await
}

pub async fn put(client: &Client, path: &str, body: &Value) -> Response {
    send(client, Method::PUT, path, Some(body)).await
}

pub async fn delete(client: &Client, path: &str) -> Response {
    send(client, Method::DELETE, path, None).await
}

pub async fn action(client: &Client, conversation: &str, what: &str) -> Response {
    let path = format!("/api/social/conversations/{conversation}/encryption/{what}");
    send(client, Method::POST, &path, None).await
}

/// The status and the problem code of a refusal.
pub fn refusal(response: &Response) -> (u16, Option<String>) {
    let code = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|body| body.get("code").and_then(Value::as_str).map(str::to_string));
    (response.status.as_u16(), code)
}

pub fn set_up_body(auth_key: &[u8]) -> Value {
    json!({ "algorithm": "x25519", "publicKey": text(&bytes(32, 4)), "backup": backup(auth_key) })
}

pub async fn set_up(client: &Client) -> Account {
    let auth_key = bytes(32, 5);
    let response = post(client, &format!("{ACCOUNT}/account-key"), &set_up_body(&auth_key)).await;
    assert_eq!(response.status, 201, "{}", response.text());
    Account {
        key_id: response.json::<Value>()["accountKey"]["keyId"].as_str().unwrap().to_string(),
        auth_key,
    }
}

pub async fn register_device_with(client: &Client, key: &[u8], label: &str) -> Response {
    post(
        client,
        &format!("{ACCOUNT}/devices"),
        &json!({ "algorithm": "x25519", "publicKey": text(key), "label": label }),
    )
    .await
}

/// Registers this browser's device with a key of its own and says its id.
pub async fn register_device(client: &Client) -> String {
    let response = register_device_with(client, &bytes(32, 6), "Firefox, Linux").await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

pub async fn read(client: &Client, path: &str) -> Value {
    let response = client.get(path).await;
    assert_eq!(response.status, 200, "{path}: {}", response.text());
    response.json()
}

/// Everyone the tests need: three people on one instance.
pub struct Crowd {
    pub server: Server,
    pub maja: Person,
    pub ada: Person,
    pub ilya: Person,
}

impl Crowd {
    pub async fn new() -> Crowd {
        Crowd::on(Server::start().await).await
    }

    pub async fn on(server: Server) -> Crowd {
        let maja = server.found("maja").await;
        let ada = server.register("ada").await;
        let ilya = server.register("ilya").await;
        Crowd { server, maja, ada, ilya }
    }

    /// Everyone signed in with an account key and this browser's device.
    pub async fn ready() -> Crowd {
        let crowd = Crowd::new().await;
        for person in [&crowd.maja, &crowd.ada, &crowd.ilya] {
            set_up(&person.client).await;
            register_device(&person.client).await;
        }
        crowd
    }

    pub fn name(&self, id: &str) -> &str {
        [&self.maja, &self.ada, &self.ilya]
            .into_iter()
            .find(|person| person.id == id)
            .map_or("?", |person| person.username.as_str())
    }

    /// A conversation of two people, as the route that opens one would leave it.
    pub fn conversation(&self, first: &Person, second: &Person) -> String {
        let (a, b) = {
            let (x, y) = (Id::parse(&first.id).unwrap(), Id::parse(&second.id).unwrap());
            if x < y { (x, y) } else { (y, x) }
        };
        let id = Id::random();
        self.server
            .database()
            .execute(
                "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\")
                 VALUES (?1, ?2, ?3, ?4, 'Off')",
                params![id, a, b, Timestamp::now()],
            )
            .expect("a conversation is written");
        id.to_string()
    }

    /// One person asks, the other confirms.
    pub async fn enable(&self, requester: &Person, confirmer: &Person, conversation: &str) {
        assert_eq!(action(&requester.client, conversation, "request").await.status, 200);
        assert_eq!(action(&confirmer.client, conversation, "confirm").await.status, 200);
    }

    pub async fn hub(&self, person: &Person) -> HubClient {
        let cookie = format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"));
        HubClient::connect(self.server.address(), &cookie).await
    }

    /// The timeline lines of a conversation, oldest first, as `kind:who`.
    pub fn timeline(&self, conversation: &str) -> Vec<String> {
        let connection = self.server.database();
        let mut statement = connection
            .prepare(
                "SELECT \"SystemKind\", \"AuthorId\" FROM \"DirectMessages\"
                 WHERE \"ConversationId\" = ?1 AND \"SystemKind\" IS NOT NULL ORDER BY \"CreatedAt\", rowid",
            )
            .unwrap();
        statement
            .query_map([Id::parse(conversation).unwrap()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Id>(1)?))
            })
            .unwrap()
            .map(|row| {
                let (kind, who) = row.unwrap();
                format!("{kind}:{}", self.name(&who.to_string()))
            })
            .collect()
    }

    /// The ids of every key the conversation's next epoch has to reach.
    pub async fn recipients(&self, client: &Client, conversation: &str) -> Vec<String> {
        let keys = read(client, &format!("/api/social/conversations/{conversation}/encryption/keys")).await;
        keys["recipients"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key["keyId"].as_str().unwrap().to_string())
            .collect()
    }

    pub async fn keys(&self, client: &Client, conversation: &str) -> Value {
        read(client, &format!("/api/social/conversations/{conversation}/encryption/keys")).await
    }
}

pub fn epoch_body(epoch: &str, previous: Option<&str>, key_ids: &[String]) -> Value {
    json!({
        "epochId": epoch,
        "previousEpochId": previous,
        "envelopes": key_ids.iter().map(|key| json!({
            "recipientKeyId": key,
            "ephemeralPublicKey": text(&bytes(32, 7)),
            "iv": text(&bytes(12, 8)),
            "ciphertext": text(&bytes(48, 9)),
        })).collect::<Vec<_>>(),
    })
}

pub async fn post_epoch(client: &Client, conversation: &str, body: &Value) -> Response {
    post(client, &format!("/api/social/conversations/{conversation}/encryption/epochs"), body).await
}

/// Starts a new epoch for exactly the keys the conversation has now, as a
/// browser would, and says its id.
pub async fn rotate(crowd: &Crowd, client: &Client, conversation: &str) -> String {
    let keys = crowd.keys(client, conversation).await;
    let previous = keys["encryption"]["currentEpochId"].as_str().map(str::to_string);
    let ids = crowd.recipients(client, conversation).await;
    let epoch = Id::random().to_string();
    let response = post_epoch(client, conversation, &epoch_body(&epoch, previous.as_deref(), &ids)).await;
    assert_eq!(response.status, 201, "{}", response.text());
    epoch
}
