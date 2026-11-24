//! A placeholder, the account that stands in for a Discord author, is nobody:
//! every route that finds a person or acts on one answers it as an account
//! that is not there, and only the routes that show what it wrote show it,
//! marked as what it is.
//!
//! The parts of the first server's walk that name routes this build does not
//! have yet (the instance's control panel, channel-level member routes and
//! permission overrides, password recovery) are left to the groups that bring
//! them; the walk at the end fails when a route naming a person appears that
//! is not walked here.

use std::collections::BTreeSet;
use std::path::Path;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::database::Database;
use stellarr_api::discord_import::{ImportWriter, message_id};
use stellarr_api::settings::DatabaseSettings;
use stellarr_api::values::Id;

use crate::support::{Client, PASSWORD, Person, Request, Server};

use crate::discord_import::discord;
use discord::Rig;
use discord::guild::{Ch, FakeGuild, ID};

const AUTHOR: &str = "1";

/// Every route with a person's id in its path, which the walk below covers.
const ROUTES_NAMING_A_PERSON: [&str; 9] = [
    "/api/admin/bans/{userId}",
    "/api/admin/invites/{invite_id}/members/{user_id}",
    "/api/admin/members/{userId}",
    "/api/admin/members/{userId}/ban",
    "/api/admin/members/{userId}/role",
    "/api/admin/members/{userId}/roles",
    "/api/social/blocks/{userId}",
    "/api/social/blocks/{userId}",
    "/api/social/friends/{userId}",
];

struct World {
    server: Server,
    ada: Person,
    nils: Person,
    ilya: Person,
    placeholder: String,
    text: String,
    voice: String,
    section: String,
}

async fn world() -> World {
    let server = Server::start().await;
    let ada = server.found("ada").await;
    let nils = server.register("nils").await;
    let ilya = server.register("ilya").await;
    let guild = FakeGuild::new("Krew");
    guild
        .category("900", "Talk", 0)
        .channel("100", "chat", Ch { parent: Some("900"), ..Ch::default() })
        .member(AUTHOR, "jo", None, Some("Ghost"));
    guild.message("1001", "100", AUTHOR, "a ghostly question", 1, Default::default());
    guild.message(
        "1002",
        "100",
        AUTHOR,
        "a ghostly answer",
        2,
        discord::guild::Msg { reply_to: Some("1001"), ..Default::default() },
    );
    let rig = Rig::new(guild.handler()).await;
    let bot = rig.bot();
    let snapshot = bot.read_server(ID).await.unwrap();
    let server_id: String = server
        .database()
        .query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0))
        .unwrap();
    let writer = ImportWriter::new(
        Database::open(&DatabaseSettings {
            path: server.database_path(),
            busy_timeout: std::time::Duration::from_secs(30),
        })
        .unwrap(),
    );
    let server_id = Id::parse(&server_id).unwrap();
    let import = writer
        .start(server_id, Id::parse(&ada.id).unwrap(), &snapshot)
        .await
        .unwrap();
    writer.write_structure(&import, &snapshot).await.unwrap();
    let page = bot.messages("100", None).unwrap().collect_all().await.unwrap();
    writer.write_messages(&import, &snapshot, "100", &page, None).await.unwrap();
    writer.complete_channel(&import, "100").await.unwrap();

    let db = server.database();
    let placeholder: String = db
        .query_row("SELECT \"UserId\" FROM \"DiscordImportAccounts\" WHERE \"DiscordId\" = ?1", [AUTHOR], |row| row.get(0))
        .unwrap();
    // Give the placeholder a password and an address, so that refusing it proves
    // more than a missing hash or a missing address: the password is everybody's.
    db.execute(
        "UPDATE \"Users\" SET \"PasswordHash\" = (SELECT \"PasswordHash\" FROM \"Users\" WHERE \"Id\" = ?2),
                \"Email\" = 'ghost@example.com', \"NormalizedEmail\" = 'GHOST@EXAMPLE.COM' WHERE \"Id\" = ?1",
        params![placeholder, ada.id.to_uppercase()],
    )
    .unwrap();
    let text: String = db
        .query_row("SELECT \"ChannelId\" FROM \"DiscordImportChannels\" WHERE \"DiscordId\" = '100'", [], |row| row.get(0))
        .unwrap();
    let voice: String = db
        .query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 1 LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let section: String = db
        .query_row("SELECT \"SectionId\" FROM \"DiscordImportSections\" WHERE \"DiscordId\" = '900'", [], |row| row.get(0))
        .unwrap();
    World {
        server,
        ada,
        nils,
        ilya,
        placeholder: placeholder.to_lowercase(),
        text: text.to_lowercase(),
        voice: voice.to_lowercase(),
        section: section.to_lowercase(),
    }
}

async fn send(client: &Client, method: Method, path: &str, body: Option<Value>) -> u16 {
    let mut request = Request::new(method, path);
    if let Some(body) = body {
        request = request.json(&body);
    }
    client.send(request).await.status.as_u16()
}

#[tokio::test]
async fn a_placeholder_is_absent_or_refused_everywhere_a_person_is_found_or_acted_on() {
    let w = world().await;
    let id = w.placeholder.clone();
    let mut walked: BTreeSet<&str> = BTreeSet::new();
    let anonymous = w.server.client();

    // Signing in, with the account's right password; every other way in needs
    // something only a signed-in account or the CLI's recovery can make.
    for username in ["discord:1", "DISCORD:1"] {
        let response = anonymous
            .post_json("/api/auth/login", &json!({ "username": username, "password": PASSWORD }))
            .await;
        assert_eq!(response.status, 401, "{username}: {}", response.text());
    }

    // Nobody can take the shape of such a name by signing up; the same request
    // with an ordinary name gets past the name.
    let register = |username: &'static str| {
        let (anonymous, invite) = (&anonymous, w.server.invite());
        async move {
            anonymous
                .post_json(
                    "/api/auth/register",
                    &json!({
                        "username": username,
                        "displayName": "Taker",
                        "email": format!("{}@example.com", username.replace(':', "-")),
                        "password": "Taker!2026-long-enough",
                        "inviteCode": invite,
                    }),
                )
                .await
        }
    };
    let refused = register("discord:42").await;
    assert_eq!(refused.status, 400, "{}", refused.text());
    assert!(refused.json::<Value>()["errors"].get("username").is_some());
    let ordinary = register("taker42").await;
    assert_eq!(ordinary.status, 201, "{}", ordinary.text());

    // Direct messages, friends and blocks: as an account that does not exist,
    // while ilya, a person, is reached by each.
    assert_eq!(send(&w.nils.client, Method::POST, "/api/social/conversations", Some(json!({ "userId": id }))).await, 404);
    assert_eq!(send(&w.nils.client, Method::POST, "/api/social/conversations", Some(json!({ "userId": w.ilya.id }))).await, 200);
    for (username, expected) in [("discord:1", "unknown-user"), ("ilya", "sent")] {
        let response = w.nils.client.post_json("/api/social/requests", &json!({ "username": username })).await;
        assert_eq!(response.json::<Value>()["result"], expected, "{username}");
    }
    walked.insert("/api/social/blocks/{userId}");
    walked.insert("/api/social/friends/{userId}");
    let block = |who: &str| format!("/api/social/blocks/{who}");
    assert_eq!(send(&w.nils.client, Method::POST, &block(&id), None).await, 404);
    assert_eq!(send(&w.nils.client, Method::POST, &block(&w.ilya.id), None).await, 204);
    assert_eq!(send(&w.nils.client, Method::DELETE, &block(&id), None).await, 404);
    assert_eq!(send(&w.nils.client, Method::DELETE, &block(&w.ilya.id), None).await, 204);
    assert_eq!(send(&w.nils.client, Method::DELETE, &format!("/api/social/friends/{id}"), None).await, 404);
    let social = w.nils.client.get("/api/social").await.text();
    assert!(!social.contains(&id), "{social}");

    // An invite sent to it, the moderation of a member, and the successor.
    let invite = w.ada.client.post_json("/api/admin/invites", &json!({ "channelId": null })).await;
    assert!(invite.status.is_success(), "{}", invite.text());
    let invite_id = invite.json::<Value>()["id"].as_str().unwrap().to_string();
    walked.insert("/api/admin/invites/{invite_id}/members/{user_id}");
    assert_eq!(
        send(&w.ada.client, Method::POST, &format!("/api/admin/invites/{invite_id}/members/{id}"), None).await,
        404
    );
    assert_eq!(
        send(&w.ada.client, Method::POST, &format!("/api/admin/invites/{invite_id}/members/{}", w.ilya.id), None).await,
        // ilya is a member already, which only a person can be told.
        409
    );
    let base_role: String = w
        .server
        .database()
        .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0 LIMIT 1", [], |row| row.get(0))
        .unwrap();
    for (method, route, path, body) in [
        (Method::PATCH, "/api/admin/members/{userId}/role", format!("/api/admin/members/{id}/role"), Some(json!({ "roleId": base_role.to_lowercase() }))),
        (Method::PUT, "/api/admin/members/{userId}/roles", format!("/api/admin/members/{id}/roles"), Some(json!({ "roleIds": [] }))),
        (Method::POST, "/api/admin/members/{userId}/ban", format!("/api/admin/members/{id}/ban"), Some(json!({ "reason": "imported" }))),
        (Method::DELETE, "/api/admin/members/{userId}", format!("/api/admin/members/{id}"), None),
        (Method::DELETE, "/api/admin/bans/{userId}", format!("/api/admin/bans/{id}"), None),
    ] {
        walked.insert(route);
        assert_eq!(send(&w.ada.client, method.clone(), &path, body).await, 404, "{method} {path}");
    }
    // The successor must be a member: this one is refused as an invalid choice.
    assert_eq!(
        send(&w.ada.client, Method::POST, "/api/admin/server/transfer", Some(json!({ "userId": id }))).await,
        400
    );

    // Member lists: the placeholder is no member of anything.
    for path in ["/api/admin/members", "/api/admin/bans", "/api/servers"] {
        let listed = w.ada.client.get(path).await;
        assert!(listed.status.is_success(), "{path}: {}", listed.text());
        assert!(!listed.text().contains(&id), "{path}");
    }
    let members: i64 = w
        .server
        .database()
        .query_row("SELECT COUNT(*) FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [id.to_uppercase()], |row| row.get(0))
        .unwrap();
    assert_eq!(members, 0);

    // Mentions: a placeholder's name cannot be written as one, and nothing reaches it.
    let sent = w
        .ada
        .client
        .post_json(&format!("/api/channels/{}/messages", w.text), &json!({ "content": "@discord:1 and @Ghost, hello" }))
        .await;
    assert!(sent.status.is_success(), "{}", sent.text());
    let reached: i64 = w
        .server
        .database()
        .query_row("SELECT COUNT(*) FROM \"UserNotifications\" WHERE \"UserId\" = ?1", [id.to_uppercase()], |row| row.get(0))
        .unwrap();
    assert_eq!(reached, 0);

    // What it wrote is shown, and says whose it is.
    let root = message_id(Id::parse(&server_id(&w)).unwrap(), "1001");
    let pinned = w
        .ada
        .client
        .send(Request::new(Method::PUT, &format!("/api/messages/{}/pin", message_id(Id::parse(&server_id(&w)).unwrap(), "1002"))))
        .await;
    assert!(pinned.status.is_success(), "{}", pinned.text());
    type Select<'a> = &'a dyn Fn(&Value) -> Vec<Value>;
    let views: [(String, Select); 6] = [
        (format!("/api/channels/{}/messages", w.text), &|v| v["messages"].as_array().cloned().unwrap_or_default()),
        (format!("/api/channels/{}/messages/sync?after=0", w.text), &|v| {
            v["changes"].as_array().unwrap().iter().filter(|c| c["message"].is_object()).map(|c| c["message"].clone()).collect()
        }),
        (format!("/api/channels/{}/messages/search?query=ghostly", w.text), &|v| v.as_array().cloned().unwrap_or_default()),
        (format!("/api/channels/{}/messages/index?query=ghostly", w.text), &|v| v["messages"].as_array().cloned().unwrap_or_default()),
        (format!("/api/channels/{}/pins", w.text), &|v| v["messages"].as_array().cloned().unwrap_or_default()),
        (format!("/api/channels/{}/threads/{root}", w.text), &|v| v["messages"].as_array().cloned().unwrap_or_default()),
    ];
    for (path, select) in views {
        let response = w.ada.client.get(&path).await;
        assert_eq!(response.status, 200, "{path}: {}", response.text());
        let authors: Vec<Value> = select(&response.json())
            .iter()
            .map(|m| m["author"].clone())
            .filter(|a| a["id"] == id)
            .collect();
        assert!(!authors.is_empty(), "{path}");
        assert!(authors.iter().all(|a| a["origin"] == "discord"), "{path}: {authors:?}");
    }
    let threads: Value = w.ada.client.get(&format!("/api/channels/{}/threads", w.text)).await.json();
    assert_eq!(threads.as_array().unwrap().len(), 1);
    assert_eq!(threads[0]["root"]["authorId"], id);

    // A person reads as a person.
    let me: Value = w.ada.client.get("/api/session").await.json();
    assert!(me["user"]["origin"].is_null(), "{me}");
    let _ = (&w.voice, &w.section);

    // Every route that names a person in its path was walked.
    let routes = routes_naming_a_person();
    let expected: BTreeSet<String> = ROUTES_NAMING_A_PERSON.iter().map(|r| (*r).to_string()).collect();
    assert_eq!(routes, expected, "a route that names a person is not walked here");
    for route in ROUTES_NAMING_A_PERSON {
        assert!(walked.contains(route), "{route} was not walked");
    }
}

fn server_id(w: &World) -> String {
    w.server
        .database()
        .query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0))
        .unwrap()
}

/// The paths in the routes' source that name a person.
fn routes_naming_a_person() -> BTreeSet<String> {
    fn walk(folder: &Path, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("discord_import/mod.rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for literal in text.split('"') {
                    if literal.starts_with("/api/")
                        && (literal.contains("{userId") || literal.contains("{user_id"))
                        && !literal.contains(char::is_whitespace)
                    {
                        found.insert(literal.to_string());
                    }
                }
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
    found
}
