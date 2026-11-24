//! People for a test to be: founding an instance, registering somebody,
//! signing in, a second person.
//!
//! ```ignore
//! let server = Server::start().await;
//! let owner = server.found("owner").await;          // founds the instance, signed in
//! let maya = server.register("maya").await;         // joins by an invite, signed in
//! let again = server.sign_in(&maya).await;          // a second sign-in, a client of its own
//! ```

use std::time::Duration;

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::client::{Client, Response};
use super::server::Server;

/// The password every person of the tests has.
pub const PASSWORD: &str = "Passw0rd!2026";

/// An account, and a client that is signed in as it.
pub struct Person {
    pub username: String,
    pub id: String,
    pub client: Client,
}

impl Server {
    /// The code the server logged at start for founding the instance.
    pub async fn setup_code(&self) -> String {
        for _ in 0..100 {
            let found = self.log().iter().rev().find_map(|line| {
                line.split_once("Setup code: ")
                    .map(|(_, rest)| rest.split('.').next().unwrap_or("").to_string())
            });
            if let Some(code) = found {
                return code;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the server logged no setup code");
    }

    /// Founds the instance with an owner called `username`, and returns them
    /// signed in. The binary only: the code is read from its log.
    pub async fn found(&self, username: &str) -> Person {
        let code = self.setup_code().await;
        let client = self.client();
        let response = client
            .post_json(
                "/api/auth/register",
                &json!({
                    "username": username,
                    "displayName": display_name(username),
                    "email": format!("{username}@example.test"),
                    "password": PASSWORD,
                    "setupCode": code,
                }),
            )
            .await;
        assert_eq!(response.status, 201, "{}", response.text());
        Person {
            username: username.to_string(),
            id: response.json::<Value>()["id"].as_str().unwrap().to_string(),
            client,
        }
    }

    /// An invite to the instance's server, written into the database as the
    /// owner would make one, and its code.
    pub fn invite(&self) -> String {
        let connection = self.database();
        let (server, owner): (String, String) = connection
            .query_row(
                "SELECT \"ServerId\", \"UserId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1 LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the instance has an owner");
        let code = format!(
            "invite{}",
            stellarr_api::values::Id::random()
                .to_string()
                .replace('-', "")
        )[..16]
            .to_string();
        connection
            .execute(
                "INSERT INTO \"ServerInvites\" (\"Id\", \"ServerId\", \"CreatedById\", \"ChannelId\", \"Code\", \"Uses\", \"MaxUses\", \"ExpiresAt\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, NULL, ?4, 0, NULL, NULL, ?5)",
                params![stellarr_api::values::Id::random().to_db_text(), server, owner, code, stellarr_api::values::Timestamp::now().to_db_text()],
            )
            .expect("the invite is written");
        code
    }

    /// Registers somebody called `username` with an invite, and returns them
    /// signed in.
    pub async fn register(&self, username: &str) -> Person {
        let client = self.client();
        let response = client
            .post_json(
                "/api/auth/register",
                &json!({
                    "username": username,
                    "displayName": display_name(username),
                    "email": format!("{username}@example.test"),
                    "password": PASSWORD,
                    "inviteCode": self.invite(),
                }),
            )
            .await;
        assert_eq!(response.status, 201, "{}", response.text());
        Person {
            username: username.to_string(),
            id: response.json::<Value>()["id"].as_str().unwrap().to_string(),
            client,
        }
    }

    /// Signs `person` in again, with a client of its own.
    pub async fn sign_in(&self, person: &Person) -> Client {
        let client = self.client();
        let response = self.login(&client, &person.username, PASSWORD).await;
        assert_eq!(response.status, 200, "{}", response.text());
        client
    }

    pub async fn login(&self, client: &Client, username: &str, password: &str) -> Response {
        client
            .post_json(
                "/api/auth/login",
                &json!({ "username": username, "password": password }),
            )
            .await
    }

    /// A connection of the test's own to the server's database.
    pub fn database(&self) -> Connection {
        let connection = Connection::open(self.database_path()).expect("the database opens");
        connection.busy_timeout(Duration::from_secs(10)).unwrap();
        connection
    }
}

fn display_name(username: &str) -> String {
    let mut name = username.to_string();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    name
}
