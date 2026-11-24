//! The servers an instance holds: making, finding and joining them, their
//! invites, and what the people who run one can do: the audit log, the
//! notification defaults, handing a server over and deleting it.

use std::time::Duration;

use http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, HubClient, Person, Request, Response, Server};

const SERVER_HEADER: &str = "X-Stellarr-Server";
const VIEW_CHANNEL: i64 = 1 << 14;
const VIEW_AUDIT_LOG: i64 = 1 << 7;
const MANAGE_SERVER: i64 = 1;
const COOKIE: &str = "stellarr-dev-session";

// ---------------------------------------------------------------------------
// What the tests share.

/// A request on behalf of a person, about the server named (the first they
/// joined when none is).
async fn send(
    client: &Client,
    method: Method,
    path: &str,
    server: Option<&str>,
    body: Option<&Value>,
) -> Response {
    let mut request = Request::new(method, path);
    if let Some(server) = server {
        request = request.header(SERVER_HEADER, server);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    client.send(request).await
}

async fn get(client: &Client, path: &str, server: Option<&str>) -> Response {
    send(client, Method::GET, path, server, None).await
}

async fn post(client: &Client, path: &str, server: Option<&str>, body: Option<&Value>) -> Response {
    send(client, Method::POST, path, server, body).await
}

async fn delete(client: &Client, path: &str, server: Option<&str>) -> Response {
    send(client, Method::DELETE, path, server, None).await
}

async fn patch(client: &Client, path: &str, server: Option<&str>, body: &Value) -> Response {
    send(client, Method::PATCH, path, server, Some(body)).await
}

/// Makes a server of the person's own and returns its id.
async fn create_server(client: &Client, body: Value) -> String {
    let response = post(client, "/api/servers", None, Some(&body)).await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

async fn named_server(client: &Client, name: &str) -> String {
    create_server(client, json!({ "name": name })).await
}

/// The column of a row of the server's database, by the id as clients write
/// it.
fn one<T: rusqlite::types::FromSql>(server: &Server, sql: &str, id: &str) -> T {
    server
        .database()
        .query_row(sql, [id.to_uppercase()], |row| row.get(0))
        .unwrap()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn count_of(server: &Server, sql: &str, id: &str) -> i64 {
    one(server, sql, id)
}

fn text_id() -> String {
    Id::random().to_db_text()
}

fn now_text() -> String {
    Timestamp::now().to_db_text()
}

/// A role of the server with the permissions given, held by the person.
fn grant(server: &Server, server_id: &str, person: &Person, permissions: i64) -> String {
    let connection = server.database();
    let role = text_id();
    // A role somebody was given outranks the base role, which ranks last.
    connection
        .execute(
            "UPDATE \"ServerRoles\" SET \"Position\" = \"Position\" + 1 WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0",
            [server_id.to_uppercase()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\")
             VALUES (?1, ?2, ?3, '#62ad78', ?4, 1, 0, 0)",
            params![role, server_id.to_uppercase(), format!("Role {role}"), permissions],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"ServerMemberRoles\" (\"ServerId\", \"UserId\", \"RoleId\") VALUES (?1, ?2, ?3)",
            params![server_id.to_uppercase(), person.id.to_uppercase(), role],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE \"ServerMembers\" SET \"RoleId\" = ?1 WHERE \"ServerId\" = ?2 AND \"UserId\" = ?3 AND \"OwnerSlot\" IS NULL",
            params![role, server_id.to_uppercase(), person.id.to_uppercase()],
        )
        .unwrap();
    role
}

fn ban(server: &Server, server_id: &str, person: &Person, by: &Person) {
    server
        .database()
        .execute(
            "INSERT INTO \"ServerBans\" (\"ServerId\", \"UserId\", \"BannedById\", \"Reason\", \"BannedAt\") VALUES (?1, ?2, ?3, 'test', ?4)",
            params![server_id.to_uppercase(), person.id.to_uppercase(), by.id.to_uppercase(), now_text()],
        )
        .unwrap();
}

/// A server of the person's own with `others` joined to it by an invite, and
/// its id.
async fn crowded(_server: &Server, owner: &Person, others: &[&Person]) -> String {
    let id = named_server(&owner.client, "Crew").await;
    let invite = make_invite(&owner.client, &id, json!({})).await;
    for other in others {
        let joined = post(&other.client, &format!("/api/invites/{}/join", invite.1), None, None).await;
        assert_eq!(joined.status, 200, "{}", joined.text());
    }
    id
}

/// An invite to the server: its id and its code.
async fn make_invite(client: &Client, server_id: &str, body: Value) -> (String, String) {
    let response = post(client, "/api/admin/invites", Some(server_id), Some(&body)).await;
    assert_eq!(response.status, 201, "{}", response.text());
    let invite = response.json::<Value>();
    let url = invite["url"].as_str().unwrap();
    (invite["id"].as_str().unwrap().to_string(), url.rsplit('/').next().unwrap().to_string())
}

fn channel_of(server: &Server, server_id: &str, name: &str) -> String {
    let id: String = server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Name\" = ?2",
            params![server_id.to_uppercase(), name],
            |row| row.get(0),
        )
        .unwrap();
    id.to_lowercase()
}

/// A channel nobody but those who hold a role of their own sees: the base
/// role is denied view in it.
fn hide_channel(server: &Server, server_id: &str, channel: &str) {
    let connection = server.database();
    let base: String = connection
        .query_row(
            "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0",
            [server_id.to_uppercase()],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\")
             VALUES (?1, ?2, 0, ?3, ?4, 0, ?5)",
            params![text_id(), server_id.to_uppercase(), channel.to_uppercase(), base, VIEW_CHANNEL],
        )
        .unwrap();
}

fn member_count(connection: &Connection, server_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT count(*) FROM \"ServerMembers\" WHERE \"ServerId\" = ?1",
            [server_id.to_uppercase()],
            |row| row.get(0),
        )
        .unwrap()
}

/// The arguments of the next event of that name, past the presence changes a
/// connection hears as it starts.
async fn until(hub: &mut HubClient, name: &str) -> Value {
    loop {
        let record = hub.record().await;
        if record["target"] == name {
            return record["arguments"][0].clone();
        }
    }
}

fn cookie_of(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

/// The names of the members of the JSON object `text`, in the order it
/// writes them, with the text of each value. `Value` sorts its keys, so the
/// order a client sees can only be read from the text.
fn members(text: &str) -> Vec<(String, &str)> {
    let bytes = text.as_bytes();
    let mut at = text.find('{').expect("an object") + 1;
    let mut found = Vec::new();
    // The end of the value that starts at `from`: past its nested brackets
    // and strings, at the comma or the closing brace after it.
    let value_end = |from: usize| -> usize {
        let mut depth = 0;
        let mut in_string = false;
        let mut index = from;
        while index < bytes.len() {
            let byte = bytes[index];
            if in_string {
                if byte == b'\\' {
                    index += 1;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else {
                match byte {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' if depth == 0 => return index,
                    b'}' | b']' => depth -= 1,
                    b',' if depth == 0 => return index,
                    _ => {}
                }
            }
            index += 1;
        }
        index
    };
    loop {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= bytes.len() || bytes[at] == b'}' {
            return found;
        }
        let start = at + 1;
        let end = start + text[start..].find('"').expect("a closing quote");
        let name = text[start..end].to_string();
        let colon = end + text[end..].find(':').expect("a colon") + 1;
        let stop = value_end(colon);
        found.push((name, text[colon..stop].trim()));
        at = stop + 1;
    }
}

/// The names of the members of the object, in order.
fn order(text: &str) -> Vec<String> {
    members(text).into_iter().map(|(name, _)| name).collect()
}

/// The text of one member of the object.
fn member<'a>(text: &'a str, name: &str) -> &'a str {
    members(text).into_iter().find(|(have, _)| have == name).expect("a member").1
}

/// The objects of an array of objects, as text, in order.
fn objects_of(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut depth = 0;
    let mut in_string = false;
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\\' {
                index += 1;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' => {
                    if depth == 0 {
                        start = index;
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        found.push(&text[start..=index]);
                    }
                }
                _ => {}
            }
        }
        index += 1;
    }
    found
}

fn keys(value: &Value) -> Vec<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

fn events_named(server: &Server, name: &str) -> Vec<Value> {
    server.events().into_iter().filter(|event| event["name"] == name).collect()
}

// ---------------------------------------------------------------------------
// Making a server.

#[tokio::test]
async fn creating_a_server_seeds_what_the_request_names_and_makes_the_creator_its_owner() {
    let server = Server::start().await;
    let founder = server.found("owner").await;
    let maya = server.register("maya").await;

    let response = post(
        &maya.client,
        "/api/servers",
        None,
        Some(&json!({
            "name": "Załoga",
            "description": "Wieczorne granie",
            "textChannelName": "Ogłoszenia",
            "voiceChannelName": "salon",
            "mentionsOnly": false,
            "voiceJoinSound": false,
        })),
    )
    .await;

    assert_eq!(response.status, 201, "{}", response.text());
    let created = response.json::<Value>();
    assert_eq!(order(&response.text()), ["id", "name", "description", "iconUrl", "createdAt", "isPublic"]);
    assert_eq!(response.header("location"), Some(format!("/api/servers/{}", created["id"].as_str().unwrap()).as_str()));
    assert_eq!(created["iconUrl"], Value::Null);
    assert_eq!(created["isPublic"], false);
    assert!(created["createdAt"].as_str().unwrap().ends_with('Z'));
    let id = created["id"].as_str().unwrap();

    let connection = server.database();
    let mut statement = connection
        .prepare("SELECT \"Name\", \"Kind\" FROM \"Channels\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\"")
        .unwrap();
    let channels: Vec<(String, i64)> = statement
        .query_map([id.to_uppercase()], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(channels, [("ogłoszenia".to_string(), 0), ("salon".to_string(), 1)]);
    let settings: (bool, bool, bool) = connection
        .query_row(
            "SELECT \"MentionsOnly\", \"AllMessages\", \"VoiceJoinSound\" FROM \"ServerAdminSettings\" WHERE \"ServerId\" = ?1",
            [id.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(settings, (false, true, false));
    let (owner_slot, is_owner): (Option<i64>, bool) = connection
        .query_row(
            "SELECT m.\"OwnerSlot\", r.\"IsOwner\" FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" WHERE m.\"ServerId\" = ?1",
            [id.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((owner_slot, is_owner), (Some(1), true));
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerRoles\" WHERE \"ServerId\" = ?1", id), 2);
    let held = count_of(&server, "SELECT count(*) FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1", id);
    assert_eq!(held, 1);
    // The first server is somebody else's, and its owner is not in the new one.
    assert_eq!(member_count(&connection, id), 1);
    assert_ne!(founder.id, maya.id);
}

#[tokio::test]
async fn a_new_server_greets_its_owner_in_its_first_channel_and_says_so_to_its_audience() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let before = server.events().len();

    let id = named_server(&maya.client, "Crew").await;

    let welcome: (String, bool) = server
        .database()
        .query_row(
            "SELECT m.\"Content\", m.\"IsSystem\" FROM \"Messages\" m JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\" WHERE c.\"ServerId\" = ?1",
            [id.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(welcome, ("**Maya** has joined the server. Say hello!".to_string(), true));
    let events = &server.events()[before..];
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "MessageCreated");
    assert_eq!(events[0]["audience"]["server"], id);
}

#[tokio::test]
async fn an_empty_voice_name_makes_a_server_without_a_voice_channel() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;

    let id = create_server(&maya.client, json!({ "name": "Text only", "voiceChannelName": "" })).await;

    let channels: Vec<(String, i64)> = {
        let connection = server.database();
        let mut statement = connection
            .prepare("SELECT \"Name\", \"Kind\" FROM \"Channels\" WHERE \"ServerId\" = ?1")
            .unwrap();
        statement
            .query_map([id.to_uppercase()], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    // The omitted text name still falls back to the usual one.
    assert_eq!(channels, [("general".to_string(), 0)]);
}

#[tokio::test]
async fn a_server_is_refused_a_bad_name_a_long_description_and_starter_channels_that_clash() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let refused = |body: Value| {
        let client = &maya.client;
        async move {
            let response = post(client, "/api/servers", None, Some(&body)).await;
            assert_eq!(response.status, 400, "{}", response.text());
            assert_eq!(response.header("content-type"), Some("application/problem+json"));
            let errors = response.json::<Value>()["errors"].clone();
            keys(&errors).iter().map(|key| key.to_string()).collect::<Vec<_>>()
        }
    };

    assert_eq!(refused(json!({ "name": "   " })).await, ["server"]);
    assert_eq!(refused(json!({ "name": "x", "description": "d".repeat(281) })).await, ["server"]);
    assert_eq!(refused(json!({ "name": "x".repeat(81) })).await, ["server"]);
    assert_eq!(refused(json!({ "name": "x", "textChannelName": "two words" })).await, ["channels"]);
    assert_eq!(
        refused(json!({ "name": "x", "textChannelName": "shared", "voiceChannelName": "shared" })).await,
        ["channels"]
    );
    assert_eq!(count(&server, "SELECT count(*) FROM \"Servers\""), 1);
}

#[tokio::test]
async fn a_body_that_is_not_the_right_json_makes_no_server() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;

    for raw in ["", "{", "null", "[]", "{\"name\": 5}", "{\"name\": \"x\", \"isPublic\": null}"] {
        let response = maya.client.send(Request::new(Method::POST, "/api/servers").body("application/json", raw)).await;
        assert_eq!(response.status, 400, "{raw}: {}", response.text());
    }
    let other = maya.client.send(Request::new(Method::POST, "/api/servers").body("text/plain", "x")).await;
    assert_eq!(other.status, 415);
    assert_eq!(count(&server, "SELECT count(*) FROM \"Servers\""), 1);
}

#[tokio::test]
async fn the_server_routes_need_a_signed_in_person() {
    let server = Server::start().await;
    server.found("owner").await;
    let anonymous = server.client();

    for (method, path) in [
        (Method::GET, "/api/servers"),
        (Method::GET, "/api/servers/discover"),
        (Method::POST, "/api/servers"),
        (Method::POST, "/api/servers/6f9a2f60-0000-4000-8000-000000000000/join"),
        (Method::POST, "/api/invites/abc/join"),
        (Method::PATCH, "/api/admin/server"),
        (Method::GET, "/api/admin/invites"),
        (Method::POST, "/api/admin/invites"),
        (Method::DELETE, "/api/admin/invites/6f9a2f60-0000-4000-8000-000000000000"),
        (Method::POST, "/api/admin/invites/6f9a2f60-0000-4000-8000-000000000000/members/6f9a2f60-0000-4000-8000-000000000001"),
        (Method::GET, "/api/admin/audit"),
        (Method::GET, "/api/admin/notifications"),
        (Method::PATCH, "/api/admin/notifications"),
        (Method::POST, "/api/admin/server/transfer"),
        (Method::DELETE, "/api/admin/server"),
    ] {
        let response = anonymous.send(Request::new(method.clone(), path)).await;
        assert_eq!(response.status, 401, "{method} {path}");
        assert!(response.body.is_empty(), "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// Finding and joining servers.

#[tokio::test]
async fn the_server_list_holds_only_the_servers_of_the_person_asking_in_order_of_joining() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let created = named_server(&maya.client, "Crew").await;

    let mine_response = get(&maya.client, "/api/servers", None).await;
    let mine = mine_response.json::<Value>();
    let theirs = get(&owner.client, "/api/servers", None).await.json::<Value>();

    let ids = |list: &Value| list.as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&mine).len(), 2);
    assert_eq!(ids(&mine)[1], created);
    assert!(!ids(&theirs).contains(&created));
    assert_eq!(order(objects_of(&mine_response.text())[0]), ["id", "name", "description", "iconUrl", "createdAt", "isPublic"]);
}

#[tokio::test]
async fn discovery_offers_public_servers_until_the_person_joins_or_is_banned() {
    let server = Server::start().await;
    server.found("owner").await;
    let ada = server.register("ada").await;
    let ilya = server.register("ilya").await;
    let nils = server.register("nils").await;
    let open = create_server(&ada.client, json!({ "name": "Open", "isPublic": true })).await;
    let closed = create_server(&ada.client, json!({ "name": "Closed", "isPublic": false })).await;
    ban(&server, &open, &nils, &ada);
    async fn discovered(person: &Person) -> Vec<String> {
        let list = get(&person.client, "/api/servers/discover", None).await.json::<Value>();
        list.as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
    }

    assert!(discovered(&ilya).await.contains(&open));
    assert!(!discovered(&ilya).await.contains(&closed));
    assert!(!discovered(&ada).await.contains(&open));
    assert!(!discovered(&nils).await.contains(&open));

    assert_eq!(post(&ilya.client, &format!("/api/servers/{closed}/join"), None, None).await.status, 404);
    let banned = post(&nils.client, &format!("/api/servers/{open}/join"), None, None).await;
    assert_eq!(banned.status, 403);
    assert!(banned.body.is_empty());
    let joined = post(&ilya.client, &format!("/api/servers/{open}/join"), None, None).await;
    assert_eq!(joined.status, 200);
    assert_eq!(joined.json::<Value>()["id"], open);
    assert_eq!(post(&ilya.client, &format!("/api/servers/{open}/join"), None, None).await.status, 200);
    assert!(!discovered(&ilya).await.contains(&open));

    let (owned, role_owner): (Option<i64>, bool) = server
        .database()
        .query_row(
            "SELECT m.\"OwnerSlot\", r.\"IsOwner\" FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2",
            params![open.to_uppercase(), ilya.id.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((owned, role_owner), (None, false));
    let greetings = count_of(
        &server,
        "SELECT count(*) FROM \"Messages\" m JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\" WHERE m.\"IsSystem\" = 1 AND c.\"ServerId\" = ?1",
        &open,
    );
    // The owner's, and the one new member's: a second join adds none.
    assert_eq!(greetings, 2);
    let held = count_of(&server, "SELECT count(*) FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1", &open);
    assert_eq!(held, 1, "the base role is held without a row");
}

#[tokio::test]
async fn a_server_without_a_role_for_plain_members_cannot_be_joined() {
    let server = Server::start().await;
    server.found("owner").await;
    let ada = server.register("ada").await;
    let ilya = server.register("ilya").await;
    let open = create_server(&ada.client, json!({ "name": "Open", "isPublic": true })).await;
    server
        .database()
        .execute("DELETE FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsOwner\" = 0", [open.to_uppercase()])
        .unwrap();

    let response = post(&ilya.client, &format!("/api/servers/{open}/join"), None, None).await;

    assert_eq!(response.status, 503);
    assert_eq!(response.header("content-type"), Some("application/problem+json"));
    assert_eq!(response.json::<Value>()["title"], "The server has no role for new members.");
    assert_eq!(member_count(&server.database(), &open), 1);
}

#[tokio::test]
async fn a_path_that_is_not_an_id_is_a_route_that_does_not_exist() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;

    assert_eq!(post(&maya.client, "/api/servers/not-an-id/join", None, None).await.status, 404);
    assert_eq!(post(&maya.client, "/api/servers/00000000-0000-0000-0000-000000000000/join", None, None).await.status, 404);
}

// ---------------------------------------------------------------------------
// Invites.

#[tokio::test]
async fn a_channel_invite_names_its_channel_and_the_audit_log_says_so() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "voiceChannelName": "meeting" })).await;
    let channel = channel_of(&server, &id, "meeting");

    let response = post(&maya.client, "/api/admin/invites", Some(&id), Some(&json!({ "channelId": channel }))).await;

    assert_eq!(response.status, 201, "{}", response.text());
    let invite = response.json::<Value>();
    assert_eq!(order(&response.text()), ["id", "url", "createdByName", "uses", "maxUses", "expiresAt", "channelId", "channelName"]);
    assert_eq!(invite["channelId"], channel);
    assert_eq!(invite["channelName"], "meeting");
    assert_eq!(invite["uses"], 0);
    assert_eq!(invite["maxUses"], 25);
    assert_eq!(invite["createdByName"], "Maya");
    assert!(invite["expiresAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(response.header("location"), Some(format!("/api/admin/invites/{}", invite["id"].as_str().unwrap()).as_str()));
    let stored: Option<String> = one(&server, "SELECT \"ChannelId\" FROM \"ServerInvites\" WHERE \"Id\" = ?1", invite["id"].as_str().unwrap());
    assert_eq!(stored.unwrap().to_lowercase(), channel);
    let summaries: Vec<String> = {
        let connection = server.database();
        let mut statement = connection
            .prepare("SELECT \"Summary\" FROM \"ServerAuditEntries\" WHERE \"ServerId\" = ?1")
            .unwrap();
        statement.query_map([id.to_uppercase()], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    };
    assert!(summaries.contains(&"Maya created an invite to channel #meeting".to_string()), "{summaries:?}");
}

#[tokio::test]
async fn an_invite_is_made_without_a_body_or_with_an_empty_or_null_one_and_is_server_wide() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;

    let bodiless = post(&maya.client, "/api/admin/invites", Some(&id), None).await;
    let empty = post(&maya.client, "/api/admin/invites", Some(&id), Some(&json!({}))).await;
    let null = maya
        .client
        .send(Request::new(Method::POST, "/api/admin/invites").header(SERVER_HEADER, &id).body("application/json", "null"))
        .await;

    for response in [&bodiless, &empty, &null] {
        assert_eq!(response.status, 201, "{}", response.text());
        let invite = response.json::<Value>();
        assert_eq!(invite["channelId"], Value::Null);
        assert_eq!(invite["channelName"], Value::Null);
    }
    let broken = maya
        .client
        .send(Request::new(Method::POST, "/api/admin/invites").header(SERVER_HEADER, &id).body("application/json", "{"))
        .await;
    assert_eq!(broken.status, 400);
    let wrong = maya
        .client
        .send(Request::new(Method::POST, "/api/admin/invites").header(SERVER_HEADER, &id).body("application/json", "{\"channelId\":\"x\"}"))
        .await;
    assert_eq!(wrong.status, 400);
    let typed = maya
        .client
        .send(Request::new(Method::POST, "/api/admin/invites").header(SERVER_HEADER, &id).body("text/plain", "{}"))
        .await;
    assert_eq!(typed.status, 415);
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerInvites\" WHERE \"ServerId\" = ?1", &id), 3);
}

#[tokio::test]
async fn an_invite_refuses_a_channel_of_another_server_and_one_the_caller_does_not_see() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let here = create_server(&maya.client, json!({ "name": "Here", "textChannelName": "here" })).await;
    let there = create_server(&maya.client, json!({ "name": "There", "textChannelName": "there" })).await;
    let foreign = channel_of(&server, &there, "there");
    let secret = channel_of(&server, &here, "here");
    let joined = make_invite(&maya.client, &here, json!({})).await;
    post(&ada.client, &format!("/api/invites/{}/join", joined.1), None, None).await;
    grant(&server, &here, &ada, 1 << 13);
    hide_channel(&server, &here, &secret);

    let other_server = post(&maya.client, "/api/admin/invites", Some(&here), Some(&json!({ "channelId": foreign }))).await;
    let hidden = post(&ada.client, "/api/admin/invites", Some(&here), Some(&json!({ "channelId": secret }))).await;

    for response in [other_server, hidden] {
        assert_eq!(response.status, 400, "{}", response.text());
        let errors = response.json::<Value>()["errors"].clone();
        assert_eq!(errors["channelId"][0], "Unknown channel.");
    }
}

#[tokio::test]
async fn making_an_invite_is_refused_without_the_permission_naming_it() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;

    let refused = post(&ada.client, "/api/admin/invites", Some(&id), None).await;

    assert_eq!(refused.status, 403);
    assert_eq!(refused.header("content-type"), Some("application/problem+json"));
    let problem = refused.json::<Value>();
    assert_eq!(problem["permission"], "createInvite");
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerInvites\" WHERE \"ServerId\" = ?1", &id), 1);
}

#[tokio::test]
async fn invites_are_listed_until_revoked_and_only_for_those_who_manage_the_server() {
    let server = Server::start_with(&[("Operations__PublicUrl", "https://chat.test/")]).await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;

    let refused = get(&ada.client, "/api/admin/invites", Some(&id)).await;
    assert_eq!(refused.status, 403);
    assert!(refused.body.is_empty());
    let invite = make_invite(&maya.client, &id, json!({})).await;

    let listing = get(&maya.client, "/api/admin/invites", Some(&id)).await;
    assert_eq!(order(objects_of(&listing.text())[0]), ["id", "url", "createdByName", "uses", "maxUses", "expiresAt", "channelId", "channelName"]);
    let listed = listing.json::<Value>();
    let found = listed.as_array().unwrap().iter().find(|x| x["id"] == invite.0).unwrap();
    // The instance's public address, without its trailing slash.
    assert_eq!(found["url"], format!("https://chat.test/invite/{}", invite.1));
    // Read back from the database, the moment says nothing of its zone.
    assert!(!found["expiresAt"].as_str().unwrap().ends_with('Z'));

    let unknown = delete(&maya.client, &format!("/api/admin/invites/{}", Id::random()), Some(&id)).await;
    assert_eq!(unknown.status, 404);
    assert!(unknown.body.is_empty());
    assert_eq!(delete(&ada.client, &format!("/api/admin/invites/{}", invite.0), Some(&id)).await.status, 403);
    assert_eq!(delete(&maya.client, &format!("/api/admin/invites/{}", invite.0), Some(&id)).await.status, 204);
    let listed = get(&maya.client, "/api/admin/invites", Some(&id)).await.json::<Value>();
    assert!(!listed.as_array().unwrap().iter().any(|x| x["id"] == invite.0));
}

#[tokio::test]
async fn an_invite_link_falls_back_to_the_address_the_request_came_to() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;

    let response = post(&maya.client, "/api/admin/invites", Some(&id), None).await;

    let url = response.json::<Value>()["url"].as_str().unwrap().to_string();
    assert!(url.starts_with(&format!("http://{}/invite/", server.address())), "{url}");
}

#[tokio::test]
async fn an_invite_that_is_used_up_or_has_expired_is_not_listed_and_one_that_never_expires_is() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    let spent = make_invite(&maya.client, &id, json!({})).await;
    let expired = make_invite(&maya.client, &id, json!({})).await;
    let endless = make_invite(&maya.client, &id, json!({})).await;
    let connection = server.database();
    connection.execute("UPDATE \"ServerInvites\" SET \"Uses\" = \"MaxUses\" WHERE \"Id\" = ?1", [spent.0.to_uppercase()]).unwrap();
    connection
        .execute(
            "UPDATE \"ServerInvites\" SET \"ExpiresAt\" = ?2 WHERE \"Id\" = ?1",
            params![expired.0.to_uppercase(), Timestamp::now().minus(Duration::from_secs(60)).to_db_text()],
        )
        .unwrap();
    connection
        .execute("UPDATE \"ServerInvites\" SET \"ExpiresAt\" = NULL, \"MaxUses\" = NULL WHERE \"Id\" = ?1", [endless.0.to_uppercase()])
        .unwrap();

    let listed = get(&maya.client, "/api/admin/invites", Some(&id)).await.json::<Value>();

    let ids: Vec<&str> = listed.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [endless.0.as_str()]);
    let only = &listed[0];
    assert_eq!((&only["maxUses"], &only["expiresAt"]), (&Value::Null, &Value::Null));
}

#[tokio::test]
async fn joining_through_an_invite_counts_each_new_member_once_and_refuses_the_banned_expired_and_used_up() {
    let server = Server::start().await;
    server.found("owner").await;
    let ada = server.register("ada").await;
    let ilya = server.register("ilya").await;
    let nils = server.register("nils").await;
    let id = named_server(&ada.client, "Crew").await;
    let counted = make_invite(&ada.client, &id, json!({})).await;
    let expired = make_invite(&ada.client, &id, json!({})).await;
    let spent = make_invite(&ada.client, &id, json!({})).await;
    ban(&server, &id, &nils, &ada);
    let connection = server.database();
    connection
        .execute(
            "UPDATE \"ServerInvites\" SET \"ExpiresAt\" = ?2 WHERE \"Id\" = ?1",
            params![expired.0.to_uppercase(), Timestamp::now().minus(Duration::from_secs(60)).to_db_text()],
        )
        .unwrap();
    connection.execute("UPDATE \"ServerInvites\" SET \"Uses\" = \"MaxUses\" WHERE \"Id\" = ?1", [spent.0.to_uppercase()]).unwrap();

    async fn join(code: String, person: &Person) -> Response {
        post(&person.client, &format!("/api/invites/{code}/join"), None, None).await
    }
    let first = join(counted.1.clone(), &ilya).await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert_eq!(join(counted.1.clone(), &ilya).await.status, 200);
    assert_eq!(join(expired.1.clone(), &ilya).await.status, 404);
    assert_eq!(join(spent.1.clone(), &ilya).await.status, 404);
    assert_eq!(join("no-such-code".into(), &ilya).await.status, 404);
    let banned = join(counted.1.clone(), &nils).await;
    assert_eq!(banned.status, 403);
    assert!(banned.body.is_empty());

    let uses: i64 = one(&server, "SELECT \"Uses\" FROM \"ServerInvites\" WHERE \"Id\" = ?1", &counted.0);
    assert_eq!(uses, 1);
    assert_eq!(member_count(&connection, &id), 2);
    let answer = first.json::<Value>();
    assert_eq!(order(&first.text()), ["id", "name", "description", "iconUrl", "createdAt", "isPublic", "channelId", "channelName"]);
    assert_eq!(answer["channelId"], Value::Null);
}

#[tokio::test]
async fn an_invite_code_is_found_whatever_its_letter_case_and_the_spaces_around_it() {
    let server = Server::start().await;
    server.found("owner").await;
    let ada = server.register("ada").await;
    let ilya = server.register("ilya").await;
    let id = named_server(&ada.client, "Crew").await;
    let invite = make_invite(&ada.client, &id, json!({})).await;

    let response = post(&ilya.client, &format!("/api/invites/%20{}%20/join", invite.1.to_uppercase()), None, None).await;

    assert_eq!(response.status, 200, "{}", response.text());
}

#[tokio::test]
async fn an_invite_with_one_use_left_lets_one_of_eight_simultaneous_joiners_in() {
    let server = Server::start().await;
    server.found("owner").await;
    let ada = server.register("ada").await;
    let id = named_server(&ada.client, "Crew").await;
    let invite = make_invite(&ada.client, &id, json!({})).await;
    server
        .database()
        .execute("UPDATE \"ServerInvites\" SET \"MaxUses\" = 1 WHERE \"Id\" = ?1", [invite.0.to_uppercase()])
        .unwrap();
    let mut people = Vec::new();
    for index in 0..8 {
        people.push(server.register(&format!("guest{index}")).await);
    }

    let code = invite.1.clone();
    let answers = futures_join(people.iter().map(|person| {
        let client = &person.client;
        let path = format!("/api/invites/{code}/join");
        async move { post(client, &path, None, None).await.status.as_u16() }
    }))
    .await;

    assert_eq!(answers.iter().filter(|status| **status == 200).count(), 1, "{answers:?}");
    assert_eq!(answers.iter().filter(|status| **status == 404).count(), 7, "{answers:?}");
    let uses: i64 = one(&server, "SELECT \"Uses\" FROM \"ServerInvites\" WHERE \"Id\" = ?1", &invite.0);
    assert_eq!(uses, 1);
    assert_eq!(member_count(&server.database(), &id), 2);
}

/// Runs the futures at once and gathers what they say, in order.
async fn futures_join<F: std::future::Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<std::pin::Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(move |context| {
        let mut all = true;
        for (index, future) in pending.iter_mut().enumerate() {
            if done[index].is_none() {
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(value) => done[index] = Some(value),
                    std::task::Poll::Pending => all = false,
                }
            }
        }
        if all {
            std::task::Poll::Ready(done.iter_mut().map(|slot| slot.take().unwrap()).collect())
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

#[tokio::test]
async fn one_person_joining_twice_at_once_is_a_member_once_and_counts_one_use() {
    let server = Server::start().await;
    server.found("owner").await;
    let ada = server.register("ada").await;
    let ilya = server.register("ilya").await;
    let id = named_server(&ada.client, "Crew").await;
    let invite = make_invite(&ada.client, &id, json!({})).await;

    let path = format!("/api/invites/{}/join", invite.1);
    let answers = futures_join((0..4).map(|_| {
        let client = &ilya.client;
        let path = path.clone();
        async move { post(client, &path, None, None).await.status.as_u16() }
    }))
    .await;

    assert!(answers.iter().all(|status| *status == 200), "{answers:?}");
    let uses: i64 = one(&server, "SELECT \"Uses\" FROM \"ServerInvites\" WHERE \"Id\" = ?1", &invite.0);
    assert_eq!(uses, 1);
    assert_eq!(member_count(&server.database(), &id), 2);
}

#[tokio::test]
async fn joining_through_a_channel_invite_names_the_channel_only_when_the_newcomer_will_see_it() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let jo = server.register("joe").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "textChannelName": "open", "voiceChannelName": "meeting" })).await;
    let open = channel_of(&server, &id, "open");
    let meeting = channel_of(&server, &id, "meeting");
    let to_open = make_invite(&maya.client, &id, json!({ "channelId": open })).await;
    let to_meeting = make_invite(&maya.client, &id, json!({ "channelId": meeting })).await;
    hide_channel(&server, &id, &meeting);

    let seen = post(&ada.client, &format!("/api/invites/{}/join", to_open.1), None, None).await.json::<Value>();
    let unseen = post(&jo.client, &format!("/api/invites/{}/join", to_meeting.1), None, None).await.json::<Value>();

    assert_eq!((&seen["channelId"], &seen["channelName"]), (&json!(open), &json!("open")));
    // The newcomer lands in the server without being told of the channel.
    assert_eq!((&unseen["channelId"], &unseen["channelName"]), (&Value::Null, &Value::Null));
    let still_stored: Option<String> = one(&server, "SELECT \"ChannelId\" FROM \"ServerInvites\" WHERE \"Id\" = ?1", &to_meeting.0);
    assert!(still_stored.is_some());
}

#[tokio::test]
async fn an_invite_to_a_hidden_channel_is_listed_to_a_manager_who_does_not_see_it_without_the_channel() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "voiceChannelName": "meeting" })).await;
    let meeting = channel_of(&server, &id, "meeting");
    let invite = make_invite(&maya.client, &id, json!({ "channelId": meeting })).await;
    let joiner = make_invite(&maya.client, &id, json!({})).await;
    post(&ada.client, &format!("/api/invites/{}/join", joiner.1), None, None).await;
    grant(&server, &id, &ada, MANAGE_SERVER);
    hide_channel(&server, &id, &meeting);

    let listed = get(&ada.client, "/api/admin/invites", Some(&id)).await.json::<Value>();

    let found = listed.as_array().unwrap().iter().find(|x| x["id"] == invite.0).unwrap();
    assert_eq!((&found["channelId"], &found["channelName"]), (&Value::Null, &Value::Null));
    let maya_sees = get(&maya.client, "/api/admin/invites", Some(&id)).await.json::<Value>();
    let found = maya_sees.as_array().unwrap().iter().find(|x| x["id"] == invite.0).unwrap();
    assert_eq!(found["channelName"], "meeting");
}

// ---------------------------------------------------------------------------
// The page an invite link opens.

#[tokio::test]
async fn an_invite_shows_anybody_where_it_leads_and_who_sent_it_and_nothing_more() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = create_server(&maya.client, json!({ "name": "Orbit", "description": "A test server", "textChannelName": "hall" })).await;
    let hall = channel_of(&server, &id, "hall");
    let to_channel = make_invite(&maya.client, &id, json!({ "channelId": hall })).await;
    let wide = make_invite(&maya.client, &id, json!({})).await;
    server
        .database()
        .execute("UPDATE \"ServerInvites\" SET \"ExpiresAt\" = NULL WHERE \"Id\" = ?1", [wide.0.to_uppercase()])
        .unwrap();
    server
        .database()
        .execute("UPDATE \"Servers\" SET \"IconUrl\" = '/assets/icon' WHERE \"Id\" = ?1", [id.to_uppercase()])
        .unwrap();
    server
        .database()
        .execute("UPDATE \"Users\" SET \"AvatarUrl\" = '/assets/avatar' WHERE \"Id\" = ?1", [maya.id.to_uppercase()])
        .unwrap();
    let anonymous = server.client();

    let response = get(&anonymous, &format!("/api/invites/{}", to_channel.1), None).await;

    assert_eq!(response.status, 200, "{}", response.text());
    assert_eq!(response.header("cache-control"), Some("no-store"));
    let preview = response.json::<Value>();
    let text = response.text();
    assert_eq!(order(&text), ["server", "channel", "inviter", "expiresAt"]);
    assert_eq!(order(member(&text, "server")), ["id", "name", "description", "iconUrl"]);
    assert_eq!(order(member(&text, "channel")), ["id", "name"]);
    assert_eq!(order(member(&text, "inviter")), ["displayName", "avatarColor", "avatarUrl"]);
    assert_eq!(preview["server"]["id"], id);
    assert_eq!(preview["server"]["name"], "Orbit");
    assert_eq!(preview["server"]["description"], "A test server");
    // Stored images are served only to a session, so anybody else gets none.
    assert_eq!(preview["server"]["iconUrl"], Value::Null);
    assert_eq!(preview["channel"]["name"], "hall");
    assert_eq!(preview["inviter"]["displayName"], "Maya");
    assert_eq!(preview["inviter"]["avatarUrl"], Value::Null);
    assert!(preview["expiresAt"].as_str().unwrap().ends_with('Z'));

    let signed_in = get(&maya.client, &format!("/api/invites/{}", to_channel.1), None).await.json::<Value>();
    assert_eq!(signed_in["server"]["iconUrl"], "/assets/icon");
    assert_eq!(signed_in["inviter"]["avatarUrl"], "/assets/avatar");

    let server_wide = get(&anonymous, &format!("/api/invites/{}", wide.1), None).await.json::<Value>();
    assert_eq!(server_wide["channel"], Value::Null);
    assert_eq!(server_wide["expiresAt"], Value::Null);
}

#[tokio::test]
async fn the_preview_does_not_name_a_channel_the_newcomer_would_not_see() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "voiceChannelName": "meeting" })).await;
    let meeting = channel_of(&server, &id, "meeting");
    let invite = make_invite(&maya.client, &id, json!({ "channelId": meeting })).await;
    hide_channel(&server, &id, &meeting);

    let anonymous = get(&server.client(), &format!("/api/invites/{}", invite.1), None).await.json::<Value>();
    let stranger = get(&ada.client, &format!("/api/invites/{}", invite.1), None).await.json::<Value>();
    let owner = get(&maya.client, &format!("/api/invites/{}", invite.1), None).await.json::<Value>();

    assert_eq!(anonymous["channel"], Value::Null);
    assert_eq!(stranger["channel"], Value::Null);
    assert_eq!(owner["channel"]["name"], "meeting");
}

#[tokio::test]
async fn every_invite_that_cannot_be_used_gets_the_same_not_found() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    let revoked = make_invite(&maya.client, &id, json!({})).await;
    assert_eq!(delete(&maya.client, &format!("/api/admin/invites/{}", revoked.0), Some(&id)).await.status, 204);
    let expired = make_invite(&maya.client, &id, json!({})).await;
    let spent = make_invite(&maya.client, &id, json!({})).await;
    let gone_server = named_server(&maya.client, "Brief").await;
    let to_gone = make_invite(&maya.client, &gone_server, json!({})).await;
    let connection = server.database();
    connection
        .execute(
            "UPDATE \"ServerInvites\" SET \"ExpiresAt\" = ?2 WHERE \"Id\" = ?1",
            params![expired.0.to_uppercase(), Timestamp::now().minus(Duration::from_secs(60)).to_db_text()],
        )
        .unwrap();
    connection.execute("UPDATE \"ServerInvites\" SET \"Uses\" = \"MaxUses\" WHERE \"Id\" = ?1", [spent.0.to_uppercase()]).unwrap();
    assert_eq!(delete(&maya.client, "/api/admin/server", Some(&gone_server)).await.status, 204);
    let anonymous = server.client();

    let mut answers = Vec::new();
    for code in ["0123456789abcdef", &revoked.1, &expired.1, &spent.1, &to_gone.1] {
        let response = get(&anonymous, &format!("/api/invites/{code}"), None).await;
        let mut problem = response.json::<Value>();
        problem.as_object_mut().unwrap().remove("traceId");
        answers.push((
            response.status.as_u16(),
            response.header("content-type").map(str::to_string),
            response.header("cache-control").map(str::to_string),
            problem.to_string(),
        ));
    }

    answers.dedup();
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(answers[0].0, 404);
    assert_eq!(answers[0].1.as_deref(), Some("application/problem+json"));
    assert_eq!(answers[0].2.as_deref(), Some("no-store"));
    assert!(answers[0].3.contains("The invite has expired, reached its use limit or is invalid."), "{}", answers[0].3);
}

#[tokio::test]
async fn looking_at_invites_is_limited_so_that_codes_cannot_be_walked() {
    let server = Server::start_with(&[
        ("STELLARR_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret"),
        ("RateLimiting__InvitePreview__PermitLimit", "3"),
    ])
    .await;
    let anonymous = server.client();

    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(get(&anonymous, "/api/invites/0123456789abcdef", None).await.status.as_u16());
    }

    assert_eq!(statuses, [404, 404, 404, 429, 429]);
}

// ---------------------------------------------------------------------------
// Sending an invite to a person, and what the hub is told.

#[tokio::test]
async fn sending_an_invite_is_refused_for_members_the_banned_and_people_who_do_not_exist() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let nils = server.register("nils").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    let invite = make_invite(&maya.client, &id, json!({})).await;
    ban(&server, &id, &nils, &maya);
    let send_to = |invite: &str, user: &str| {
        let client = &maya.client;
        let path = format!("/api/admin/invites/{invite}/members/{user}");
        let id = id.clone();
        async move { post(client, &path, Some(&id), None).await }
    };

    let member = send_to(&invite.0, &ada.id).await;
    assert_eq!(member.status, 409);
    assert_eq!(member.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(member.text(), "{\"error\":\"already_member\"}");
    let banned = send_to(&invite.0, &nils.id).await;
    assert_eq!((banned.status.as_u16(), banned.body.len()), (403, 0));
    assert_eq!(send_to(&invite.0, &Id::random().to_string()).await.status, 404);
    assert_eq!(send_to(&Id::random().to_string(), &nils.id).await.status, 404);
    // A placeholder that stands in for an account on another service is not a person.
    let placeholder = text_id();
    server
        .database()
        .execute(
            "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\", \"Origin\")
             VALUES (?1, 'ghost', 'GHOST', 'Ghost', 0, 0, 0, '#777', ?2, 'discord')",
            params![placeholder, now_text()],
        )
        .unwrap();
    assert_eq!(send_to(&invite.0, &placeholder.to_lowercase()).await.status, 404);
    let spent = make_invite(&maya.client, &id, json!({})).await;
    server.database().execute("UPDATE \"ServerInvites\" SET \"Uses\" = \"MaxUses\" WHERE \"Id\" = ?1", [spent.0.to_uppercase()]).unwrap();
    let ilya = server.register("ilya").await;
    assert_eq!(send_to(&spent.0, &ilya.id).await.status, 404);
    assert_eq!(count(&server, "SELECT count(*) FROM \"UserNotifications\""), 0);
}

#[tokio::test]
async fn a_sent_invite_waits_in_the_inbox_once_until_it_is_answered_or_revoked() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let nils = server.register("nils").await;
    let id = named_server(&maya.client, "Crew").await;
    let invite = make_invite(&maya.client, &id, json!({})).await;
    let path = format!("/api/admin/invites/{}/members/{}", invite.0, nils.id);
    let before = server.events().len();

    let first = post(&maya.client, &path, Some(&id), None).await;
    let again = post(&maya.client, &path, Some(&id), None).await;

    assert_eq!((first.status.as_u16(), again.status.as_u16()), (204, 204));
    assert!(first.body.is_empty());
    let row: (String, String, String, String, String, String, Option<String>, Option<String>) = server
        .database()
        .query_row(
            "SELECT \"Source\", \"SourcePackageId\", \"SourcePackageName\", \"SourceModuleId\", \"SourceModuleName\", \"Body\", \"TargetInviteCode\", \"ReadAt\" FROM \"UserNotifications\" WHERE \"UserId\" = ?1",
            [nils.id.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        )
        .unwrap();
    assert_eq!(row.0, "system");
    assert_eq!(row.1, "stellarr.server-invite");
    assert_eq!(row.2, "Crew");
    assert_eq!(row.3, "server-invite");
    assert_eq!(row.4, "Maya");
    assert_eq!(row.5, "Maya invited you to Crew.");
    assert_eq!(row.6.as_deref(), Some(invite.1.as_str()));
    assert_eq!(row.7, None);
    let events = &server.events()[before..];
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["name"], "NotificationInboxChanged");
    assert_eq!(events[0]["audience"]["users"], json!([nils.id]));
    assert_eq!(events[0]["payload"]["global"], true);
    assert!(events[0]["payload"]["sequence"].is_number());

    let answered = post(&nils.client, &format!("/api/invites/{}/join", invite.1), None, None).await;
    assert_eq!(answered.status, 200);
    assert_eq!(count(&server, "SELECT count(*) FROM \"UserNotifications\""), 0);
    let told = events_named(&server, "NotificationInboxChanged");
    let last = told.last().unwrap();
    assert_eq!(last["payload"]["serverId"], id);
}

#[tokio::test]
async fn revoking_an_invite_takes_it_out_of_the_inboxes_it_was_sent_to_and_tells_each_person_once() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let nils = server.register("nils").await;
    let ilya = server.register("ilya").await;
    let id = named_server(&maya.client, "Crew").await;
    let invite = make_invite(&maya.client, &id, json!({})).await;
    for person in [&nils, &ilya] {
        let path = format!("/api/admin/invites/{}/members/{}", invite.0, person.id);
        assert_eq!(post(&maya.client, &path, Some(&id), None).await.status, 204);
    }
    let before = server.events().len();

    let revoked = delete(&maya.client, &format!("/api/admin/invites/{}", invite.0), Some(&id)).await;

    assert_eq!(revoked.status, 204);
    assert_eq!(count(&server, "SELECT count(*) FROM \"UserNotifications\""), 0);
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerInvites\" WHERE \"ServerId\" = ?1", &id), 0);
    let events = &server.events()[before..];
    let mut told: Vec<String> = events
        .iter()
        .map(|event| {
            assert_eq!(event["name"], "NotificationInboxChanged");
            event["audience"]["users"][0].as_str().unwrap().to_string()
        })
        .collect();
    told.sort();
    let mut expected = vec![nils.id.clone(), ilya.id.clone()];
    expected.sort();
    assert_eq!(told, expected);
}

#[tokio::test]
async fn a_person_is_told_through_the_hub_that_an_invitation_arrived() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let nils = server.register("nils").await;
    let id = named_server(&maya.client, "Crew").await;
    let invite = make_invite(&maya.client, &id, json!({})).await;
    let mut listener = HubClient::connect(server.address(), &cookie_of(&nils)).await;

    let path = format!("/api/admin/invites/{}/members/{}", invite.0, nils.id);
    assert_eq!(post(&maya.client, &path, Some(&id), None).await.status, 204);

    let heard = until(&mut listener, "NotificationInboxChanged").await;
    assert_eq!(heard["serverId"], id);
    assert_eq!(heard["global"], true);
    assert!(heard["sequence"].is_number());
    assert_eq!(delete(&maya.client, &format!("/api/admin/invites/{}", invite.0), Some(&id)).await.status, 204);
    let revoked = until(&mut listener, "NotificationInboxChanged").await;
    assert!(revoked.get("sequence").is_none());
    assert_eq!(revoked["global"], true);
}

// ---------------------------------------------------------------------------
// The profile.

#[tokio::test]
async fn the_profile_changes_name_description_and_visibility_and_each_change_is_audited_and_announced() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    let before = server.events().len();

    let response = patch(
        &maya.client,
        "/api/admin/server",
        Some(&id),
        &json!({ "name": "  Crew 2  ", "description": " Evenings ", "isPublic": true }),
    )
    .await;

    assert_eq!(response.status, 200, "{}", response.text());
    let dto = response.json::<Value>();
    assert_eq!(dto["name"], "Crew 2");
    assert_eq!(dto["description"], "Evenings");
    assert_eq!(dto["isPublic"], true);
    let events = &server.events()[before..];
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "ServerUpdated");
    assert_eq!(events[0]["audience"]["server"], id);
    assert_eq!(events[0]["payload"], dto);
    let connection = server.database();
    let mut statement = connection
        .prepare("SELECT \"Action\", \"Summary\", \"TargetType\", \"TargetName\", \"ChangesJson\" FROM \"ServerAuditEntries\" WHERE \"ServerId\" = ?1 ORDER BY \"CreatedAt\"")
        .unwrap();
    let lines: Vec<(String, String, String, String, String)> = statement
        .query_map([id.to_uppercase()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        lines,
        [
            (
                "server.update".to_string(),
                "Maya updated the server profile".to_string(),
                "server".to_string(),
                "Crew 2".to_string(),
                "[{\"field\":\"name\",\"from\":\"Crew\",\"to\":\"Crew 2\"},{\"field\":\"description\",\"from\":\"\",\"to\":\"Evenings\"}]".to_string()
            ),
            (
                "server.visibility".to_string(),
                "Maya made the server public".to_string(),
                "server".to_string(),
                "Crew 2".to_string(),
                "[{\"field\":\"visibility\",\"from\":\"invite-only\",\"to\":\"public\"}]".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn a_profile_change_that_changes_nothing_writes_no_audit_line_but_is_still_announced() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    let before = server.events().len();

    let response = patch(&maya.client, "/api/admin/server", Some(&id), &json!({})).await;

    assert_eq!(response.status, 200);
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerAuditEntries\" WHERE \"ServerId\" = ?1", &id), 0);
    assert_eq!(server.events()[before..].len(), 1);
}

#[tokio::test]
async fn a_profile_is_refused_to_those_who_may_not_manage_the_server_and_for_bad_values() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;

    let denied = patch(&ada.client, "/api/admin/server", Some(&id), &json!({ "name": "Mine" })).await;
    assert_eq!((denied.status.as_u16(), denied.body.len()), (403, 0));
    for (body, field) in [
        (json!({ "name": "" }), "name"),
        (json!({ "name": "   " }), "name"),
        (json!({ "name": "x".repeat(81) }), "name"),
        (json!({ "description": "d".repeat(281) }), "description"),
    ] {
        let refused = patch(&maya.client, "/api/admin/server", Some(&id), &body).await;
        assert_eq!(refused.status, 400, "{body}");
        let errors = refused.json::<Value>()["errors"].clone();
        assert_eq!(keys(&errors), [field]);
    }
    let outsider = server.register("ilya").await;
    let not_member = patch(&outsider.client, "/api/admin/server", Some(&id), &json!({ "name": "Mine" })).await;
    assert_eq!((not_member.status.as_u16(), not_member.body.len()), (403, 0));
    let broken = patch(&maya.client, "/api/admin/server", Some("not-an-id"), &json!({})).await;
    assert_eq!((broken.status.as_u16(), broken.body.len()), (400, 0));
    assert_eq!(one::<String>(&server, "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1", &id), "Crew");
}

// ---------------------------------------------------------------------------
// The audit log.

#[tokio::test]
async fn the_audit_log_is_refused_without_view_audit_log_and_does_not_open_the_rest() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let jo = server.register("joe").await;
    let id = crowded(&server, &maya, &[&ada, &jo]).await;
    grant(&server, &id, &jo, VIEW_AUDIT_LOG);

    let denied = get(&ada.client, "/api/admin/audit", Some(&id)).await;
    assert_eq!((denied.status.as_u16(), denied.body.len()), (403, 0));
    for reader in [&maya, &jo] {
        let entries = get(&reader.client, "/api/admin/audit", Some(&id)).await.json::<Value>();
        assert!(entries.as_array().unwrap().iter().any(|x| x["summary"] == "Maya created an invite"));
    }
    assert_eq!(get(&jo.client, "/api/admin/invites", Some(&id)).await.status, 403);
}

#[tokio::test]
async fn the_audit_log_says_who_did_what_to_what_with_every_member_of_a_change_present() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "voiceChannelName": "meeting" })).await;
    let meeting = channel_of(&server, &id, "meeting");
    let invite = make_invite(&maya.client, &id, json!({ "channelId": meeting })).await;
    delete(&maya.client, &format!("/api/admin/invites/{}", invite.0), Some(&id)).await;

    let listing = get(&maya.client, "/api/admin/audit", Some(&id)).await;

    assert_eq!(listing.status, 200);
    let newest = objects_of(&listing.text())[1].to_string();
    let entries = listing.json::<Value>();
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    // Newest first.
    assert_eq!(entries[0]["action"], "invite.revoke");
    let created = &entries[1];
    assert_eq!(order(&newest), ["id", "summary", "at", "actor", "action", "target", "changes"]);
    assert_eq!(created["summary"], "Maya created an invite to channel #meeting");
    assert!(!created["at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(created["actor"]["id"], maya.id);
    assert_eq!(created["actor"]["presence"], "offline");
    assert_eq!(order(member(&newest, "target")), ["type", "id", "name"]);
    assert_eq!(created["target"]["type"], "invite");
    assert_eq!(created["target"]["id"], invite.0);
    assert_eq!(created["target"]["name"], invite.1);
    assert_eq!(
        created["changes"],
        json!([{ "field": "channel", "from": null, "to": "meeting", "added": null, "removed": null, "fromColor": null, "toColor": null }])
    );
    assert_eq!(order(objects_of(member(&newest, "changes"))[0]), ["field", "from", "to", "added", "removed", "fromColor", "toColor"]);
    // A line with no fields has none at all, not an empty list.
    assert_eq!(entries[0]["changes"], Value::Null);
}

#[tokio::test]
async fn an_entry_from_before_the_structured_fields_has_only_its_sentence() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    server
        .database()
        .execute(
            "INSERT INTO \"ServerAuditEntries\" (\"Id\", \"ServerId\", \"ActorId\", \"Summary\", \"CreatedAt\") VALUES (?1, ?2, ?3, 'Maya did something', ?4)",
            params![text_id(), id.to_uppercase(), maya.id.to_uppercase(), now_text()],
        )
        .unwrap();
    server
        .database()
        .execute(
            "INSERT INTO \"ServerAuditEntries\" (\"Id\", \"ServerId\", \"ActorId\", \"Summary\", \"CreatedAt\", \"Action\", \"TargetType\", \"ChangesJson\") VALUES (?1, ?2, ?3, 'Damaged', ?4, 'x', 'role', '[{\"field\":5}]')",
            params![text_id(), id.to_uppercase(), maya.id.to_uppercase(), Timestamp::now().minus(Duration::from_secs(5)).to_db_text()],
        )
        .unwrap();

    let entries = get(&maya.client, "/api/admin/audit", Some(&id)).await.json::<Value>();

    assert_eq!(entries[0]["summary"], "Maya did something");
    assert_eq!(entries[0]["action"], Value::Null);
    assert_eq!(entries[0]["target"], Value::Null);
    assert_eq!(entries[0]["changes"], Value::Null);
    // A line whose fields cannot be read is read as having none.
    assert_eq!(entries[1]["summary"], "Damaged");
    assert_eq!(entries[1]["changes"], json!([]));
}

#[tokio::test]
async fn the_audit_log_comes_in_pages_from_the_newest_back() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    let start = Timestamp::parse("2026-01-01 12:00:00").unwrap();
    {
        let connection = server.database();
        for index in 0..60u64 {
            connection
                .execute(
                    "INSERT INTO \"ServerAuditEntries\" (\"Id\", \"ServerId\", \"ActorId\", \"Summary\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        text_id(),
                        id.to_uppercase(),
                        maya.id.to_uppercase(),
                        format!("Maya did {index}"),
                        start.plus(Duration::from_secs(index * 60)).to_db_text()
                    ],
                )
                .unwrap();
        }
    }

    let first = get(&maya.client, "/api/admin/audit", Some(&id)).await.json::<Value>();
    assert_eq!(first.as_array().unwrap().len(), 50);
    assert_eq!(first[0]["summary"], "Maya did 59");
    let oldest = first[49]["at"].as_str().unwrap().to_string();
    assert_eq!(oldest, "2026-01-01T12:10:00");

    for before in ["2026-01-01T12:10:00Z", "2026-01-01T12:10:00", "2026-01-01%2012:10:00", "2026-01-01T13:10:00%2B01:00"] {
        let rest = get(&maya.client, &format!("/api/admin/audit?before={before}"), Some(&id)).await;
        assert_eq!(rest.status, 200, "{before}: {}", rest.text());
        let rest = rest.json::<Value>();
        assert_eq!(rest.as_array().unwrap().len(), 10, "{before}");
        assert_eq!(rest[0]["summary"], "Maya did 9", "{before}");
    }
    let empty = get(&maya.client, "/api/admin/audit?before=", Some(&id)).await;
    assert_eq!((empty.status.as_u16(), empty.body.len()), (400, 0));
    let informal = get(&maya.client, "/api/admin/audit?before=Jan%201%202026", Some(&id)).await;
    assert_eq!(informal.status, 200);
    assert_eq!(informal.json::<Value>(), json!([]));
    let garbage = get(&maya.client, "/api/admin/audit?before=yesterday", Some(&id)).await;
    assert_eq!((garbage.status.as_u16(), garbage.body.len()), (400, 0));
}

// ---------------------------------------------------------------------------
// Notification defaults.

#[tokio::test]
async fn notification_settings_round_trip_and_refuse_channels_of_other_servers() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    let other = named_server(&maya.client, "Other").await;
    let general = channel_of(&server, &id, "general");
    let foreign = channel_of(&server, &other, "general");

    assert_eq!(get(&ada.client, "/api/admin/notifications", Some(&id)).await.status, 403);
    let initial = get(&maya.client, "/api/admin/notifications", Some(&id)).await;
    assert_eq!(initial.status, 200);
    assert_eq!(order(&initial.text()), ["allMessages", "mentionsOnly", "voiceJoinSound", "mutedChannelIds", "serverMuted"]);
    let initial = initial.json::<Value>();
    assert_eq!(
        initial,
        json!({ "allMessages": false, "mentionsOnly": true, "voiceJoinSound": true, "mutedChannelIds": [], "serverMuted": false })
    );

    let updated = patch(
        &maya.client,
        "/api/admin/notifications",
        Some(&id),
        &json!({ "allMessages": true, "mentionsOnly": false, "serverMuted": true, "mutedChannelIds": [general, general] }),
    )
    .await;
    assert_eq!(updated.status, 200, "{}", updated.text());
    assert_eq!(
        updated.json::<Value>(),
        json!({ "allMessages": true, "mentionsOnly": false, "voiceJoinSound": true, "mutedChannelIds": [general], "serverMuted": true })
    );
    let stored: String = one(&server, "SELECT \"MutedChannelIdsJson\" FROM \"ServerAdminSettings\" WHERE \"ServerId\" = ?1", &id);
    assert_eq!(stored, format!("[\"{general}\"]"));

    let foreign_muted = patch(&maya.client, "/api/admin/notifications", Some(&id), &json!({ "mutedChannelIds": [foreign] })).await;
    assert_eq!(foreign_muted.status, 400);
    assert_eq!(keys(&foreign_muted.json::<Value>()["errors"]), ["mutedChannelIds"]);
    let read = get(&maya.client, "/api/admin/notifications", Some(&id)).await.json::<Value>();
    assert_eq!(read["mutedChannelIds"], json!([general]));
    // A list that is left out is left as it was, and an empty one clears it.
    patch(&maya.client, "/api/admin/notifications", Some(&id), &json!({ "voiceJoinSound": false })).await;
    let read = get(&maya.client, "/api/admin/notifications", Some(&id)).await.json::<Value>();
    assert_eq!((&read["mutedChannelIds"], &read["voiceJoinSound"]), (&json!([general]), &json!(false)));
    patch(&maya.client, "/api/admin/notifications", Some(&id), &json!({ "mutedChannelIds": [] })).await;
    let read = get(&maya.client, "/api/admin/notifications", Some(&id)).await.json::<Value>();
    assert_eq!(read["mutedChannelIds"], json!([]));
}

#[tokio::test]
async fn a_server_that_has_no_notification_defaults_yet_gets_them_the_first_time_somebody_looks() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;
    server.database().execute("DELETE FROM \"ServerAdminSettings\" WHERE \"ServerId\" = ?1", [id.to_uppercase()]).unwrap();

    let read = get(&maya.client, "/api/admin/notifications", Some(&id)).await;

    assert_eq!(read.status, 200);
    assert_eq!(read.json::<Value>()["mentionsOnly"], true);
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerAdminSettings\" WHERE \"ServerId\" = ?1", &id), 1);
}

#[tokio::test]
async fn muted_channels_a_manager_does_not_see_stay_muted_and_unnamed() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "textChannelName": "open", "voiceChannelName": "meeting" })).await;
    let open = channel_of(&server, &id, "open");
    let meeting = channel_of(&server, &id, "meeting");
    let joiner = make_invite(&maya.client, &id, json!({})).await;
    post(&ada.client, &format!("/api/invites/{}/join", joiner.1), None, None).await;
    grant(&server, &id, &ada, MANAGE_SERVER);
    hide_channel(&server, &id, &meeting);
    assert_eq!(
        patch(&maya.client, "/api/admin/notifications", Some(&id), &json!({ "mutedChannelIds": [meeting] })).await.status,
        200
    );

    let read = get(&ada.client, "/api/admin/notifications", Some(&id)).await.json::<Value>();
    assert_eq!(read["mutedChannelIds"], json!([]));
    let named = patch(&ada.client, "/api/admin/notifications", Some(&id), &json!({ "mutedChannelIds": [meeting] })).await;
    assert_eq!(named.status, 400);
    let changed = patch(&ada.client, "/api/admin/notifications", Some(&id), &json!({ "mutedChannelIds": [open] })).await;
    assert_eq!(changed.status, 200);
    assert_eq!(changed.json::<Value>()["mutedChannelIds"], json!([open]));
    // What she could not see is still muted, after hers.
    let stored: String = one(&server, "SELECT \"MutedChannelIdsJson\" FROM \"ServerAdminSettings\" WHERE \"ServerId\" = ?1", &id);
    assert_eq!(stored, format!("[\"{open}\",\"{meeting}\"]"));
    let owner_sees = get(&maya.client, "/api/admin/notifications", Some(&id)).await.json::<Value>();
    assert_eq!(owner_sees["mutedChannelIds"], json!([open, meeting]));
}

// ---------------------------------------------------------------------------
// Handing a server over.

#[tokio::test]
async fn a_transfer_makes_the_target_the_only_owner_and_demotes_the_caller() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    patch(&maya.client, "/api/admin/server", Some(&id), &json!({})).await;
    let before = server.events().len();

    let response = post(&maya.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;

    assert_eq!((response.status.as_u16(), response.body.len()), (204, 0));
    let connection = server.database();
    let owners: Vec<(String, Option<i64>, bool)> = {
        let mut statement = connection
            .prepare("SELECT m.\"UserId\", m.\"OwnerSlot\", r.\"IsOwner\" FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" WHERE m.\"ServerId\" = ?1 ORDER BY m.\"JoinedAt\"")
            .unwrap();
        statement
            .query_map([id.to_uppercase()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        owners,
        [(maya.id.to_uppercase(), None, false), (ada.id.to_uppercase(), Some(1), true)]
    );
    let held = count_of(&server, "SELECT count(*) FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1", &id);
    assert_eq!(held, 1, "only the new owner holds a role besides the base role");
    let line: (String, String, String, String) = connection
        .query_row(
            "SELECT \"Action\", \"Summary\", \"TargetType\", \"TargetName\" FROM \"ServerAuditEntries\" WHERE \"ServerId\" = ?1 AND \"Action\" = 'server.transfer'",
            [id.to_uppercase()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(line, ("server.transfer".into(), "Maya handed the server over to Ada".into(), "member".into(), "Ada".into()));
    // Nobody saw a channel differently, so nobody is told.
    assert!(server.events()[before..].is_empty());
    assert_eq!(delete(&maya.client, "/api/admin/server", Some(&id)).await.status, 403);
    assert_eq!(delete(&ada.client, "/api/admin/server", Some(&id)).await.status, 204);
}

#[tokio::test]
async fn a_transfer_keeps_the_roles_both_people_held_and_clears_the_successors_overrides() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    let maya_role = grant(&server, &id, &maya, MANAGE_SERVER);
    let ada_role = grant(&server, &id, &ada, VIEW_AUDIT_LOG);
    let general = channel_of(&server, &id, "general");
    server
        .database()
        .execute(
            "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 512)",
            params![text_id(), id.to_uppercase(), general.to_uppercase(), ada.id.to_uppercase()],
        )
        .unwrap();

    let response = post(&maya.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;

    assert_eq!(response.status, 204);
    let connection = server.database();
    let held = |person: &Person| -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT \"RoleId\" FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2 ORDER BY \"RoleId\"")
            .unwrap();
        statement
            .query_map(params![id.to_uppercase(), person.id.to_uppercase()], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    // The one who handed the server on keeps the role they also held, and
    // ranks by it; the new owner keeps theirs next to the owner role.
    assert_eq!(held(&maya), std::slice::from_ref(&maya_role));
    let mut ada_holds = held(&ada);
    ada_holds.sort();
    assert_eq!(ada_holds.len(), 2);
    assert!(ada_holds.contains(&ada_role));
    let top: String = connection
        .query_row(
            "SELECT \"RoleId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
            params![id.to_uppercase(), maya.id.to_uppercase()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(top, maya_role);
    assert_eq!(count(&server, "SELECT count(*) FROM \"MemberPermissionOverrides\""), 0);
}

#[tokio::test]
async fn a_transfer_is_the_owners_alone_and_refuses_oneself_nobody_and_a_person_who_is_not_a_member() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let nils = server.register("nils").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    let owner_slot = || {
        server
            .database()
            .query_row::<String, _, _>(
                "SELECT \"UserId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"OwnerSlot\" = 1",
                [id.to_uppercase()],
                |row| row.get(0),
            )
            .unwrap()
    };

    let not_owner = post(&ada.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;
    assert_eq!((not_owner.status.as_u16(), not_owner.body.len()), (403, 0));
    for (label, body) in [
        ("oneself", json!({ "userId": maya.id })),
        ("a stranger", json!({ "userId": nils.id })),
        ("nobody", json!({})),
        ("the empty id", json!({ "userId": "00000000-0000-0000-0000-000000000000" })),
        ("an unknown id", json!({ "userId": Id::random() })),
    ] {
        let refused = post(&maya.client, "/api/admin/server/transfer", Some(&id), Some(&body)).await;
        assert_eq!(refused.status, 400, "{label}: {}", refused.text());
        let problem = refused.json::<Value>();
        assert_eq!(keys(&problem["errors"]), ["userId"], "{label}");
        assert_eq!(problem["errors"]["userId"][0], "Pick another member of the server.", "{label}");
    }
    for (label, raw) in [("not an id", "{\"userId\":\"x\"}"), ("null", "{\"userId\":null}"), ("no body", ""), ("not JSON", "{")] {
        let refused = maya
            .client
            .send(Request::new(Method::POST, "/api/admin/server/transfer").header(SERVER_HEADER, &id).body("application/json", raw))
            .await;
        assert_eq!((refused.status.as_u16(), refused.body.len()), (400, 0), "{label}");
    }
    assert_eq!(owner_slot(), maya.id.to_uppercase());
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerAuditEntries\" WHERE \"Action\" = 'server.transfer' AND \"ServerId\" = ?1", &id), 0);
}

#[tokio::test]
async fn simultaneous_transfers_leave_exactly_one_owner() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let jo = server.register("joe").await;
    let id = crowded(&server, &maya, &[&ada, &jo]).await;
    let first = server.sign_in(&maya).await;
    let second = server.sign_in(&maya).await;

    let (one_answer, two_answer) = {
        let to_ada = json!({ "userId": ada.id });
        let to_jo = json!({ "userId": jo.id });
        let a = post(&first, "/api/admin/server/transfer", Some(&id), Some(&to_ada));
        let b = post(&second, "/api/admin/server/transfer", Some(&id), Some(&to_jo));
        tokio::join!(a, b)
    };

    let statuses = [one_answer.status.as_u16(), two_answer.status.as_u16()];
    assert!(statuses.contains(&204), "{statuses:?}");
    assert!(statuses.iter().all(|status| *status < 500), "{statuses:?}");
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1 AND \"ServerId\" = ?1", &id), 1);
    assert_eq!(
        count_of(
            &server,
            "SELECT count(*) FROM \"ServerMembers\" m JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" WHERE r.\"IsOwner\" = 1 AND m.\"ServerId\" = ?1",
            &id
        ),
        1
    );
}

#[tokio::test]
async fn a_transfer_tells_the_people_who_now_see_something_else_and_nobody_else() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let jo = server.register("joe").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "voiceChannelName": "meeting" })).await;
    let invite = make_invite(&maya.client, &id, json!({})).await;
    for person in [&ada, &jo] {
        post(&person.client, &format!("/api/invites/{}/join", invite.1), None, None).await;
    }
    let meeting = channel_of(&server, &id, "meeting");
    hide_channel(&server, &id, &meeting);
    let before = server.events().len();

    let response = post(&maya.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;

    assert_eq!(response.status, 204);
    let events = &server.events()[before..];
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["name"], "ChannelAccessChanged");
    assert_eq!(events[0]["payload"], json!({ "serverId": id }));
    // The new owner sees the channel now; the former owner no longer does.
    let mut told: Vec<String> = events[0]["audience"]["followers"]["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|user| user.as_str().unwrap().to_string())
        .collect();
    told.sort();
    let mut expected = vec![ada.id.clone(), maya.id.clone()];
    expected.sort();
    assert_eq!(told, expected);
    assert_eq!(events[0]["audience"]["followers"]["server"], id);
}

// ---------------------------------------------------------------------------
// Deleting a server.

#[tokio::test]
async fn only_the_owner_may_delete_the_server_and_it_goes_with_everything_it_holds() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let nils = server.register("nils").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    let invite = make_invite(&maya.client, &id, json!({})).await;
    post(&maya.client, &format!("/api/admin/invites/{}/members/{}", invite.0, nils.id), Some(&id), None).await;
    let before = server.events().len();

    let refused = delete(&ada.client, "/api/admin/server", Some(&id)).await;
    assert_eq!((refused.status.as_u16(), refused.body.len()), (403, 0));
    let deleted = delete(&maya.client, "/api/admin/server", Some(&id)).await;
    assert_eq!((deleted.status.as_u16(), deleted.body.len()), (204, 0));

    for table in ["Servers", "ServerRoles", "ServerAdminSettings", "ServerInvites", "ServerAuditEntries", "UserNotifications", "ServerMembers", "ServerMemberRoles"] {
        let column = if table == "Servers" { "Id" } else { "ServerId" };
        let left = count_of(&server, &format!("SELECT count(*) FROM \"{table}\" WHERE \"{column}\" = ?1"), &id);
        assert_eq!(left, 0, "{table}");
    }
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"Channels\" WHERE \"ServerId\" = ?1", &id), 0);
    assert_eq!(count(&server, "SELECT count(*) FROM \"Users\" WHERE \"Username\" = 'ada'"), 1);
    assert!(server.events()[before..].is_empty());
    assert_eq!(delete(&maya.client, "/api/admin/server", Some(&id)).await.status, 403, "she is no longer a member");
}

#[tokio::test]
async fn the_instance_server_cannot_be_deleted_so_nobody_else_takes_the_instance_over() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    named_server(&maya.client, "Hers").await;

    let refused = delete(&owner.client, "/api/admin/server", None).await;

    assert_eq!(refused.status, 409);
    assert_eq!(refused.header("content-type"), Some("application/problem+json"));
    assert_eq!(refused.json::<Value>()["title"], "This is the instance's server, so it cannot be deleted.");
    assert_eq!(count(&server, "SELECT count(*) FROM \"Servers\""), 2);
    // An owner who owns another server may delete that one.
    let hers = get(&maya.client, "/api/servers", None).await.json::<Value>();
    let hers = hers.as_array().unwrap().iter().find(|s| s["name"] == "Hers").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(delete(&maya.client, "/api/admin/server", Some(&hers)).await.status, 204);
}

#[tokio::test]
async fn deleting_a_server_leaves_nothing_written_in_its_channels_and_queues_its_emoji_files() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let kept = named_server(&maya.client, "Kept").await;
    let doomed = create_server(&maya.client, json!({ "name": "Doomed", "textChannelName": "talk" })).await;
    let seed = |server_id: &str, token: &str, key: &str| {
        let connection = server.database();
        let channel: String = connection
            .query_row(
                "SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = 0",
                [server_id.to_uppercase()],
                |row| row.get(0),
            )
            .unwrap();
        let message = text_id();
        connection
            .execute(
                "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 0, ?4, ?5)",
                params![message, channel, maya.id.to_uppercase(), format!("hello {token}"), now_text()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"MessageReactions\" (\"MessageId\", \"UserId\", \"Emoji\", \"CreatedAt\") VALUES (?1, ?2, '+', ?3)",
                params![message, maya.id.to_uppercase(), now_text()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"ServerEmoji\" (\"Id\", \"ServerId\", \"Shortcode\", \"ObjectKey\", \"ContentType\", \"SizeBytes\", \"UploadedById\", \"CreatedAt\") VALUES (?1, ?2, 'cat', ?3, 'image/png', 1, ?4, ?5)",
                params![text_id(), server_id.to_uppercase(), key, maya.id.to_uppercase(), now_text()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"UserNotifications\" (\"Id\", \"ServerId\", \"UserId\", \"Source\", \"SourcePackageId\", \"SourcePackageName\", \"SourceModuleId\", \"SourceModuleName\", \"Title\", \"Body\", \"Tone\", \"TargetChannelId\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, 'x', 'x', 'x', 'x', 'x', 't', ?4, 'info', ?5, ?6)",
                params![text_id(), server_id.to_uppercase(), maya.id.to_uppercase(), token, channel, now_text()],
            )
            .unwrap();
    };
    seed(&kept, "keptword", "emoji/kept.png");
    seed(&doomed, "doomedword", "emoji/doomed.png");
    let counts = || -> Vec<i64> {
        [
            "SELECT count(*) FROM \"Messages\" WHERE \"Content\" LIKE '%word'",
            "SELECT count(*) FROM \"MessageReactions\"",
            "SELECT count(*) FROM \"MessageIndexEntries\"",
            "SELECT count(*) FROM \"MessageChanges\"",
            "SELECT count(*) FROM \"ChannelPostTimes\"",
            "SELECT count(*) FROM \"ServerEmoji\"",
            "SELECT count(*) FROM \"UserNotifications\"",
            "SELECT count(*) FROM \"Channels\"",
        ]
        .iter()
        .map(|sql| count(&server, sql))
        .collect()
    };
    let both = counts();

    assert_eq!(delete(&maya.client, "/api/admin/server", Some(&doomed)).await.status, 204);

    let after = counts();
    // Everything the doomed server held is gone and the kept one's is as it was.
    assert_eq!(after[0], 1);
    assert_eq!(after[1], both[1] - 1);
    assert!(after[2] < both[2]);
    assert!(after[3] < both[3]);
    assert_eq!(after[5], 1);
    assert_eq!(after[6], both[6] - 1);
    assert_eq!(after[7], both[7] - 2);
    let search: i64 = server
        .database()
        .query_row("SELECT count(*) FROM \"MessageSearch\" WHERE \"MessageSearch\" MATCH 'doomedword'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(search, 0);
    let kept_search: i64 = server
        .database()
        .query_row("SELECT count(*) FROM \"MessageSearch\" WHERE \"MessageSearch\" MATCH 'keptword'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kept_search, 1);
    let queued: Vec<String> = {
        let connection = server.database();
        let mut statement = connection.prepare("SELECT \"ObjectKey\" FROM \"ObjectDeletions\"").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    };
    assert_eq!(queued, ["emoji/doomed.png"]);
}

#[tokio::test]
async fn deleting_an_unknown_server_is_not_a_member_s_business() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;

    let response = delete(&maya.client, "/api/admin/server", Some(&Id::random().to_string())).await;

    assert_eq!((response.status.as_u16(), response.body.len()), (403, 0));
}

#[tokio::test]
async fn a_newcomer_who_comes_through_a_channel_invite_is_greeted_in_that_channel() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let kim = server.register("kim").await;
    let id = create_server(&maya.client, json!({ "name": "Crew", "textChannelName": "hall", "voiceChannelName": "" })).await;
    let staff = {
        let response = post(&maya.client, "/api/admin/channels", Some(&id), Some(&json!({ "name": "staff", "kind": "text" }))).await;
        // The channels routes belong to another group; the channel is made in the database when they are not there yet.
        if response.status == 201 {
            response.json::<Value>()["id"].as_str().unwrap().to_string()
        } else {
            let channel = text_id();
            server
                .database()
                .execute(
                    "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'staff', 0, 2, '', 0, 0, 0, NULL)",
                    params![channel, id.to_uppercase()],
                )
                .unwrap();
            channel.to_lowercase()
        }
    };
    hide_channel(&server, &id, &staff);
    let to_staff = make_invite(&maya.client, &id, json!({ "channelId": staff })).await;
    let plain = make_invite(&maya.client, &id, json!({})).await;

    assert_eq!(post(&ada.client, &format!("/api/invites/{}/join", to_staff.1), None, None).await.status, 200);
    assert_eq!(post(&kim.client, &format!("/api/invites/{}/join", plain.1), None, None).await.status, 200);

    let greeted = |person: &Person| -> String {
        let channel: String = server
            .database()
            .query_row(
                "SELECT c.\"Name\" FROM \"Messages\" m JOIN \"Channels\" c ON c.\"Id\" = m.\"ChannelId\" WHERE m.\"IsSystem\" = 1 AND m.\"AuthorId\" = ?1 AND c.\"ServerId\" = ?2",
                params![person.id.to_uppercase(), id.to_uppercase()],
                |row| row.get(0),
            )
            .unwrap();
        channel
    };
    // The invite names the channel the person is greeted in, hidden or not;
    // anybody else is greeted in the first channel they see.
    assert_eq!(greeted(&ada), "staff");
    assert_eq!(greeted(&kim), "hall");
}

#[tokio::test]
async fn a_list_is_not_an_object_for_any_route_that_reads_one() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;

    for (method, path) in [
        (Method::PATCH, "/api/admin/server"),
        (Method::PATCH, "/api/admin/notifications"),
        (Method::POST, "/api/admin/server/transfer"),
        (Method::POST, "/api/admin/invites"),
    ] {
        let response = maya
            .client
            .send(Request::new(method.clone(), path).header(SERVER_HEADER, &id).body("application/json", "[]"))
            .await;
        assert_eq!((response.status.as_u16(), response.body.len()), (400, 0), "{method} {path}");
    }
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"ServerAuditEntries\" WHERE \"ServerId\" = ?1", &id), 1);
}

// ---------------------------------------------------------------------------
// The second step.

/// Gives the person a confirmed second step, and their session the moment it
/// passed it (none when it never did).
fn second_step(server: &Server, person: &Person, passed_seconds_ago: Option<u64>) {
    let connection = server.database();
    connection
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\") VALUES (?1, ?2, 'totp', 'Phone', ?3, ?3)",
            params![text_id(), person.id.to_uppercase(), now_text()],
        )
        .unwrap();
    let passed = passed_seconds_ago.map(|seconds| Timestamp::now().minus(Duration::from_secs(seconds)).to_db_text());
    connection
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = ?2 WHERE \"UserId\" = ?1",
            params![person.id.to_uppercase(), passed],
        )
        .unwrap();
}

#[tokio::test]
async fn handing_over_and_deleting_need_a_second_step_passed_lately_when_the_account_has_one() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    second_step(&server, &maya, None);
    let writes = || count(&server, "SELECT count(*) FROM \"ServerAuditEntries\"");
    let before = writes();

    let handover = post(&maya.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;
    let removal = delete(&maya.client, "/api/admin/server", Some(&id)).await;

    for response in [handover, removal] {
        assert_eq!(response.status, 403);
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        assert_eq!(response.json::<Value>()["code"], "fresh-mfa-required");
    }
    assert_eq!(writes(), before);
    assert_eq!(count_of(&server, "SELECT count(*) FROM \"Servers\" WHERE \"Id\" = ?1", &id), 1);
    let owner_slot: String = server
        .database()
        .query_row("SELECT \"UserId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"OwnerSlot\" = 1", [id.to_uppercase()], |row| row.get(0))
        .unwrap();
    assert_eq!(owner_slot, maya.id.to_uppercase());

    // A session that passed it a minute ago may.
    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = ?2 WHERE \"UserId\" = ?1",
            params![maya.id.to_uppercase(), Timestamp::now().minus(Duration::from_secs(60)).to_db_text()],
        )
        .unwrap();
    let handover = post(&maya.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;
    assert_eq!(handover.status, 204, "{}", handover.text());
}

#[tokio::test]
async fn a_stale_session_is_told_of_the_second_step_before_any_other_refusal_of_these_routes() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let ada = server.register("ada").await;
    let id = crowded(&server, &maya, &[&ada]).await;
    second_step(&server, &ada, Some(3600));

    // Not the owner, and not fresh: the second step is what is said.
    let handover = post(&ada.client, "/api/admin/server/transfer", Some(&id), Some(&json!({ "userId": ada.id }))).await;
    let removal = delete(&ada.client, "/api/admin/server", Some(&id)).await;

    for response in [handover, removal] {
        assert_eq!(response.status, 403);
        assert_eq!(response.json::<Value>()["code"], "fresh-mfa-required");
    }
}

#[tokio::test]
async fn an_account_without_a_second_step_passes_the_guard_of_these_routes() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let id = named_server(&maya.client, "Crew").await;

    assert_eq!(delete(&maya.client, "/api/admin/server", Some(&id)).await.status, 204);
}
