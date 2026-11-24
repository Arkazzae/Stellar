//! Direct conversations through the real binary: opening, posting plain and
//! encrypted messages, reading history, read markers, and what blocks,
//! friendships and the peer's settings change. The encryption rows are
//! arranged in the database here.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, HubClient, Person, Request, Server};

const COOKIE: &str = "stellarr-dev-session";
const QUIET: Duration = Duration::from_millis(400);

struct Trio {
    server: Server,
    ada: Person,
    ben: Person,
    cyd: Person,
}

async fn trio() -> Trio {
    let server = Server::start().await;
    let ada = server.found("ada").await;
    let ben = server.register("ben").await;
    let cyd = server.register("cyd").await;
    Trio { server, ada, ben, cyd }
}

fn id(person: &Person) -> Id {
    Id::parse(&person.id).unwrap()
}

async fn open_with(client: &Client, other: &Person) -> String {
    let response = client.post_json("/api/social/conversations", &json!({ "userId": other.id })).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

async fn send(client: &Client, conversation: &str, body: Value) -> crate::support::Response {
    client.post_json(&format!("/api/social/conversations/{conversation}/messages"), &body).await
}

async fn say(client: &Client, conversation: &str, content: &str) -> Value {
    let response = send(client, conversation, json!({ "content": content })).await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json()
}

async fn status(client: &Client, method: Method, path: &str) -> u16 {
    client.send(Request::new(method, path)).await.status.as_u16()
}

async fn history(client: &Client, conversation: &str, query: &str) -> Value {
    let response = client.get(&format!("/api/social/conversations/{conversation}/messages{query}")).await;
    assert_eq!(response.status, 200, "{query}: {}", response.text());
    response.json()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn cookie(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

async fn befriend(a: &Person, b: &Person) {
    let sent = a.client.post_json("/api/social/requests", &json!({ "username": b.username })).await;
    assert_eq!(sent.json::<Value>()["result"], "sent");
    let state: Value = b.client.get("/api/social").await.json();
    let request = state["requests"][0]["id"].as_str().unwrap();
    assert_eq!(status(&b.client, Method::POST, &format!("/api/social/requests/{request}/accept")).await, 204);
}

fn events_named(server: &Server, name: &str) -> Vec<Value> {
    server.events().into_iter().filter(|e| e["name"] == name).collect()
}

fn direct_off_for(server: &Server, person: &Person) {
    server
        .database()
        .execute(
            "INSERT OR REPLACE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
             VALUES (?1, 1, 1, 1, 0, 0, 0, 1, 1, 0, 1, 1, 0, 0, 50)",
            params![id(person)],
        )
        .unwrap();
}

/// An upload of `owner`'s, as the upload routes leave it.
fn upload(server: &Server, owner: &Person, encrypted: bool) -> Id {
    let attachment = Id::random();
    server
        .database()
        .execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"Width\", \"Height\", \"IsVoiceMessage\", \"IsEncrypted\", \"CreatedAt\")
             VALUES (?1, ?2, ?5, 'cat.png', 'image/png', 12, 3, 4, 0, ?3, ?4)",
            params![attachment, id(owner), encrypted, Timestamp::now(), attachment.to_string()],
        )
        .unwrap();
    attachment
}

// ---- opening ----

#[tokio::test]
async fn a_pair_has_one_conversation_whoever_opens_it_and_however_often() {
    let t = trio().await;
    let first = open_with(&t.ada.client, &t.ben).await;
    assert_eq!(open_with(&t.ada.client, &t.ben).await, first);
    assert_eq!(open_with(&t.ben.client, &t.ada).await, first);
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"DirectConversations\""), 1);
    let (a, b): (String, String) = t
        .server
        .database()
        .query_row("SELECT \"UserAId\", \"UserBId\" FROM \"DirectConversations\"", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert!(a < b);
    // Only the opening that made it announced anything.
    assert_eq!(events_named(&t.server, "SocialChanged").len(), 1);
}

#[tokio::test]
async fn both_people_opening_at_the_same_moment_make_one_conversation() {
    for _ in 0..5 {
        let t = trio().await;
        let (one, two) = tokio::join!(open_with(&t.ada.client, &t.ben), open_with(&t.ben.client, &t.ada));
        assert_eq!(one, two);
        assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"DirectConversations\""), 1);
        assert_eq!(events_named(&t.server, "SocialChanged").len(), 1);
    }
}

#[tokio::test]
async fn a_conversation_is_opened_with_a_person_who_exists_and_has_not_blocked_the_caller() {
    let t = trio().await;
    let own = t.ada.client.post_json("/api/social/conversations", &json!({ "userId": t.ada.id })).await;
    assert_eq!(own.status, 404);
    for body in [json!({ "userId": Id::random() }), json!({}), json!({ "UserId": Id::random() })] {
        assert_eq!(t.ada.client.post_json("/api/social/conversations", &body).await.status, 404, "{body}");
    }
    for body in ["null", "[]", "{\"userId\":null}", "{\"userId\":5}", "{\"userId\":\"nobody\"}", "not json"] {
        let response = t.ada.client.send(Request::new(Method::POST, "/api/social/conversations").body("application/json", body)).await;
        assert_eq!(response.status, 400, "{body}");
        assert!(response.text().is_empty(), "{body}");
    }
    let plain_id = t.ben.id.replace('-', "");
    assert_eq!(t.ada.client.post_json("/api/social/conversations", &json!({ "userId": plain_id })).await.status, 400);
    assert_eq!(
        t.ada.client.send(Request::new(Method::POST, "/api/social/conversations").body("text/plain", "{}")).await.status,
        415
    );

    // An account that stands in for somebody is nobody, and so is a disabled one.
    t.server.database().execute("UPDATE \"Users\" SET \"Origin\" = 'discord' WHERE \"Username\" = 'cyd'", []).unwrap();
    assert_eq!(t.ada.client.post_json("/api/social/conversations", &json!({ "userId": t.cyd.id })).await.status, 404);
    t.server.database().execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'ben'", []).unwrap();
    assert_eq!(t.ada.client.post_json("/api/social/conversations", &json!({ "userId": t.ben.id })).await.status, 404);

    // A block in either direction refuses, for a pair that has a conversation too.
    let t = trio().await;
    status(&t.ben.client, Method::POST, &format!("/api/social/blocks/{}", t.ada.id)).await;
    let response = t.ada.client.post_json("/api/social/conversations", &json!({ "userId": t.ben.id })).await;
    assert_eq!(response.status, 403);
    assert!(response.text().is_empty());
    assert_eq!(t.ben.client.post_json("/api/social/conversations", &json!({ "userId": t.ada.id })).await.status, 403);
}

#[tokio::test]
async fn every_route_needs_somebody_signed_in_and_a_conversation_of_the_callers() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let anonymous = t.server.anonymous();
    for (method, path) in [
        (Method::POST, "/api/social/conversations".to_string()),
        (Method::GET, format!("/api/social/conversations/{conversation}/messages")),
        (Method::POST, format!("/api/social/conversations/{conversation}/messages")),
        (Method::POST, format!("/api/social/conversations/{conversation}/read")),
    ] {
        assert_eq!(status(&anonymous, method.clone(), &path).await, 401, "{method} {path}");
    }
    // Somebody else's conversation answers as one that is not there.
    let missing = Id::random().to_string();
    for who in [&t.cyd.client, &t.ada.client] {
        let target = if std::ptr::eq(who, &t.ada.client) { missing.clone() } else { conversation.clone() };
        let get = who.get(&format!("/api/social/conversations/{target}/messages")).await;
        assert_eq!(get.status, 404);
        assert!(get.text().is_empty());
        assert_eq!(send(who, &target, json!({ "content": "hi" })).await.status, 404);
        assert_eq!(status(who, Method::POST, &format!("/api/social/conversations/{target}/read")).await, 404);
    }
    assert_eq!(t.ada.client.get("/api/social/conversations/not-an-id/messages").await.status, 404);
}

// ---- posting ----

#[tokio::test]
async fn a_message_is_stored_announced_to_both_people_and_nobody_else() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let journal = count(&t.server, "SELECT COUNT(*) FROM \"MessageChanges\"");
    let mut ben = HubClient::connect(t.server.address(), &cookie(&t.ben)).await;
    let mut cyd = HubClient::connect(t.server.address(), &cookie(&t.cyd)).await;
    ben.quiet_for(QUIET).await;
    cyd.quiet_for(QUIET).await;

    let response = send(&t.ada.client, &conversation, json!({ "content": "  hello there  " })).await;
    assert_eq!(response.status, 201);
    let message: Value = response.json();
    let message_id = message["id"].as_str().unwrap().to_string();
    assert_eq!(
        response.header("location"),
        Some(format!("/api/social/conversations/{conversation}/messages/{message_id}").as_str())
    );
    assert_eq!(message["content"], "hello there");
    assert_eq!(message["channelId"], conversation.as_str());
    assert_eq!(message["author"]["username"], "ada");
    assert_eq!(message["author"]["presence"], "online");
    for (key, want) in [("embeds", json!([])), ("attachments", json!([])), ("editedAt", Value::Null), ("pinnedAt", Value::Null), ("replyTo", Value::Null), ("reactions", Value::Null), ("encrypted", Value::Null), ("system", Value::Null)] {
        assert_eq!(message[key], want, "{key}");
    }
    assert!(message["createdAt"].as_str().unwrap().ends_with('Z'));

    assert_eq!(ben.event("DirectMessageCreated").await, message);
    assert_eq!(ben.event("SocialChanged").await, json!({}));
    assert!(ben.quiet_for(QUIET).await.is_empty());
    assert!(cyd.quiet_for(QUIET).await.is_empty());
    let events = events_named(&t.server, "DirectMessageCreated");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["audience"]["users"].as_array().unwrap().len(), 2);

    // The sender has read what they said; the peer has one unread.
    let mine: Value = t.ada.client.get("/api/social").await.json();
    assert_eq!(mine["conversations"][0]["unread"], 0);
    assert_eq!(mine["conversations"][0]["lastMessage"]["content"], "hello there");
    let theirs: Value = t.ben.client.get("/api/social").await.json();
    assert_eq!(theirs["conversations"][0]["unread"], 1);
    // A conversation keeps no journal.
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"MessageChanges\""), journal);
}

#[tokio::test]
async fn message_json_preserves_field_order() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let text = send(&t.ada.client, &conversation, json!({ "content": "x" })).await.text();
    let keys = ["\"id\"", "\"channelId\"", "\"author\"", "\"content\"", "\"createdAt\"", "\"attachments\"", "\"embeds\"", "\"editedAt\"", "\"pinnedAt\"", "\"replyTo\"", "\"reactions\"", "\"encrypted\"", "\"system\""];
    let mut at = 0;
    for key in keys {
        at += text[at..].find(key).unwrap_or_else(|| panic!("{key} in {text}")) + key.len();
    }
}

#[tokio::test]
async fn a_retry_with_the_same_client_id_is_answered_with_the_stored_message() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let client_id = Id::random().to_string();
    let first = send(&t.ada.client, &conversation, json!({ "content": "once", "clientMessageId": client_id })).await;
    assert_eq!(first.status, 201);
    let again = send(&t.ada.client, &conversation, json!({ "content": " once ", "clientMessageId": client_id })).await;
    assert_eq!(again.status, 200);
    assert!(again.header("location").is_none());
    assert_eq!(again.json::<Value>(), first.json::<Value>());
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"DirectMessages\""), 1);
    assert_eq!(events_named(&t.server, "DirectMessageCreated").len(), 1);

    // What is not the same send is a conflict: other text, other uploads,
    // another author, another conversation.
    let upload = upload(&t.server, &t.ada, false);
    for body in [json!({ "content": "twice" }), json!({ "content": "once", "attachmentIds": [upload] })] {
        let mut body = body;
        body["clientMessageId"] = json!(client_id);
        let response = send(&t.ada.client, &conversation, body).await;
        assert_eq!(response.status, 409);
        assert!(response.text().is_empty());
    }
    assert_eq!(send(&t.ben.client, &conversation, json!({ "content": "once", "clientMessageId": client_id })).await.status, 409);
    let other = open_with(&t.ada.client, &t.cyd).await;
    assert_eq!(send(&t.ada.client, &other, json!({ "content": "once", "clientMessageId": client_id })).await.status, 409);
    // A retry is answered before anything that could have changed since.
    status(&t.ben.client, Method::POST, &format!("/api/social/blocks/{}", t.ada.id)).await;
    assert_eq!(send(&t.ada.client, &conversation, json!({ "content": "once", "clientMessageId": client_id })).await.status, 403);
}

#[tokio::test]
async fn eight_posts_with_one_client_id_at_once_store_one_message() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let client_id = Id::random().to_string();
    let c = &t.ada.client;
    let body = || json!({ "content": "same", "clientMessageId": client_id });
    let (a, b, c2, d, e, f, g, h) = tokio::join!(
        send(c, &conversation, body()), send(c, &conversation, body()), send(c, &conversation, body()), send(c, &conversation, body()),
        send(c, &conversation, body()), send(c, &conversation, body()), send(c, &conversation, body()), send(c, &conversation, body())
    );
    let statuses: Vec<u16> = [a, b, c2, d, e, f, g, h].iter().map(|r| r.status.as_u16()).collect();
    assert_eq!(statuses.iter().filter(|s| **s == 201).count(), 1, "{statuses:?}");
    assert_eq!(statuses.iter().filter(|s| **s == 200).count(), 7, "{statuses:?}");
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"DirectMessages\""), 1);
    assert_eq!(events_named(&t.server, "DirectMessageCreated").len(), 1);
}

#[tokio::test]
async fn what_may_be_posted_is_bounded_and_a_body_must_be_an_object_with_exact_names() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let c = &t.ada.client;
    assert_eq!(send(c, &conversation, json!({ "content": "x".repeat(20_000) })).await.status, 201);
    // Counted in UTF-16 units: an astral character is two.
    assert_eq!(send(c, &conversation, json!({ "content": "😀".repeat(10_000) })).await.status, 201);
    for content in ["x".repeat(20_001), "😀".repeat(10_001), "".into(), "  \n ".into()] {
        let response = send(c, &conversation, json!({ "content": content })).await;
        assert_eq!(response.status, 400);
        assert_eq!(response.json::<Value>()["errors"]["content"][0], "A message must contain text or an attachment and be at most 20000 characters long.");
    }
    // Spaces around count for nothing: 20000 with spaces on both ends is fine.
    assert_eq!(send(c, &conversation, json!({ "content": format!(" {} ", "y".repeat(20_000)) })).await.status, 201);

    let zero = send(c, &conversation, json!({ "content": "x", "clientMessageId": "00000000-0000-0000-0000-000000000000" })).await;
    assert_eq!(zero.json::<Value>()["errors"]["clientMessageId"][0], "The message identifier is invalid.");
    for body in ["null", "[]", "{\"content\":5}", "{\"attachmentIds\":[null]}", "{\"clientMessageId\":\"11111111111111111111111111111112\"}"] {
        let response = c.send(Request::new(Method::POST, &format!("/api/social/conversations/{conversation}/messages")).body("application/json", body)).await;
        assert_eq!(response.status, 400, "{body}");
        assert!(response.text().is_empty(), "{body}");
    }
    // A name in another case names nothing, so the message has no content.
    let capitals = c.send(Request::new(Method::POST, &format!("/api/social/conversations/{conversation}/messages")).body("application/json", "{\"CONTENT\":\"x\"}")).await;
    assert_eq!(capitals.status, 400);
    assert!(capitals.json::<Value>()["errors"]["content"].is_array());
    let big = "z".repeat(1_100_000);
    let response = c.send(Request::new(Method::POST, &format!("/api/social/conversations/{conversation}/messages")).body("application/json", &format!("{{\"content\":\"{big}\"}}"))).await;
    assert_eq!(response.status, 413);
}

#[tokio::test]
async fn attachments_are_the_senders_own_and_counted_before_anything_else() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let c = &t.ada.client;
    let mine: Vec<Id> = (0..10).map(|_| upload(&t.server, &t.ada, false)).collect();
    let response = send(c, &conversation, json!({ "attachmentIds": mine, "clientMessageId": Id::random() })).await;
    assert_eq!(response.status, 201, "{}", response.text());
    let shown = response.json::<Value>();
    let mut sorted: Vec<String> = mine.iter().map(Id::to_string).collect();
    sorted.sort();
    let listed: Vec<String> = shown["attachments"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(listed, sorted);
    assert_eq!(shown["attachments"][0]["url"], format!("/assets/{}", sorted[0]));

    let eleven: Vec<Id> = (0..11).map(|_| upload(&t.server, &t.ada, false)).collect();
    let refused = send(c, &conversation, json!({ "attachmentIds": eleven })).await;
    assert_eq!(refused.json::<Value>()["errors"]["attachmentIds"][0], "A message can contain at most 10 attachments.");
    // A repeated id counts as given, and sends once.
    let one = upload(&t.server, &t.ada, false);
    let twice = send(c, &conversation, json!({ "attachmentIds": [one, one] })).await;
    assert_eq!(twice.status, 201);
    assert_eq!(twice.json::<Value>()["attachments"].as_array().unwrap().len(), 1);
    // Somebody else's upload, an upload that is nobody's, and one used already.
    let theirs = upload(&t.server, &t.ben, false);
    for ids in [vec![theirs], vec![Id::random()], vec![one, theirs]] {
        let response = send(c, &conversation, json!({ "attachmentIds": ids })).await;
        assert_eq!(response.status, 400);
        assert_eq!(response.json::<Value>()["errors"]["attachmentIds"][0], "At least one attachment does not exist or belongs to another user.");
    }
    assert_eq!(send(c, &conversation, json!({ "attachmentIds": [one] })).await.status, 201, "an upload may be used again");
    // An encrypted upload goes only into an encrypted message.
    let sealed = upload(&t.server, &t.ada, true);
    let mismatch = send(c, &conversation, json!({ "content": "x", "attachmentIds": [sealed] })).await;
    assert_eq!(mismatch.status, 409);
    assert_eq!(mismatch.json::<Value>()["code"], "attachment-kind-mismatch");
}

#[tokio::test]
async fn blocks_and_the_peers_settings_decide_who_may_write() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    say(&t.ada.client, &conversation, "before").await;

    // Direct messages off: strangers are refused, friends are not.
    direct_off_for(&t.server, &t.ben);
    let refused = send(&t.ada.client, &conversation, json!({ "content": "hi" })).await;
    assert_eq!(refused.status, 403);
    assert!(refused.text().is_empty());
    assert_eq!(send(&t.ben.client, &conversation, json!({ "content": "I may write" })).await.status, 201);
    befriend(&t.ben, &t.ada).await;
    assert_eq!(send(&t.ada.client, &conversation, json!({ "content": "now I may" })).await.status, 201);

    // A block in either direction: post 403, history 404, read still answers.
    for blocker in [&t.ada, &t.ben] {
        let blocked = if std::ptr::eq(blocker, &t.ada) { &t.ben } else { &t.ada };
        status(&blocker.client, Method::POST, &format!("/api/social/blocks/{}", blocked.id)).await;
        for who in [&t.ada, &t.ben] {
            assert_eq!(send(&who.client, &conversation, json!({ "content": "x" })).await.status, 403);
            assert_eq!(who.client.get(&format!("/api/social/conversations/{conversation}/messages")).await.status, 404);
            assert_eq!(status(&who.client, Method::POST, &format!("/api/social/conversations/{conversation}/read")).await, 204);
        }
        status(&blocker.client, Method::DELETE, &format!("/api/social/blocks/{}", blocked.id)).await;
        // Blocking ended the friendship; lifting it does not restore it, and
        // the conversation is where it was.
        assert_eq!(history(&t.ada.client, &conversation, "").await["messages"].as_array().unwrap().len(), 3);
        befriend(&t.ada, &t.ben).await;
    }
}

#[tokio::test]
async fn posting_while_the_peer_blocks_leaves_a_consistent_conversation() {
    for _ in 0..5 {
        let t = trio().await;
        let conversation = open_with(&t.ada.client, &t.ben).await;
        let block = format!("/api/social/blocks/{}", t.ada.id);
        let (posted, blocked) = tokio::join!(
            send(&t.ada.client, &conversation, json!({ "content": "racing" })),
            status(&t.ben.client, Method::POST, &block)
        );
        assert_eq!(blocked, 204);
        assert!([201, 403].contains(&posted.status.as_u16()), "{}", posted.status);
        assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"DirectMessages\""), i64::from(posted.status == 201));
        assert_eq!(send(&t.ada.client, &conversation, json!({ "content": "after" })).await.status, 403);
    }
}

#[tokio::test]
async fn a_message_to_somebody_with_push_on_queues_a_delivery_for_them_only() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let database = t.server.database();
    database
        .execute(
            "INSERT OR REPLACE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
             VALUES (?1, 1, 1, 1, 0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 50)",
            params![id(&t.ben)],
        )
        .unwrap();
    let session: String = database
        .query_row("SELECT \"Id\" FROM \"UserSessions\" WHERE \"UserId\" = ?1", params![id(&t.ben)], |r| r.get(0))
        .unwrap();
    let subscription = Id::random();
    database
        .execute(
            "INSERT INTO \"PushSubscriptions\" (\"Id\", \"UserId\", \"SessionId\", \"Endpoint\", \"P256dh\", \"Auth\", \"CreatedAt\") VALUES (?1, ?2, ?3, 'https://push.example/x', 'k', 'a', ?4)",
            params![subscription, id(&t.ben), session, Timestamp::now()],
        )
        .unwrap();
    let (inbox, indexed) = (count(&t.server, "SELECT COUNT(*) FROM \"UserNotifications\""), count(&t.server, "SELECT COUNT(*) FROM \"MessageIndexEntries\""));
    let message = say(&t.ada.client, &conversation, "ping").await;
    let (kind, channel, key, recipient, source): (String, String, String, String, String) = database
        .query_row(
            "SELECT \"EventKind\", \"Channel\", \"DestinationKey\", \"RecipientId\", \"SourceMessageId\" FROM \"NotificationDeliveries\"",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!((kind.as_str(), channel.as_str()), ("direct-message", "push"));
    assert_eq!(key, subscription.to_string().replace('-', ""));
    assert_eq!(recipient.to_lowercase(), t.ben.id);
    assert_eq!(source.to_lowercase(), message["id"].as_str().unwrap());
    // A retry writes nothing more.
    let client_id = message["id"].as_str().unwrap();
    send(&t.ada.client, &conversation, json!({ "content": "ping", "clientMessageId": client_id })).await;
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"NotificationDeliveries\""), 1);
    // A direct message writes no inbox entry and is never searched or indexed.
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"UserNotifications\""), inbox);
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"MessageIndexEntries\""), indexed);
}

// ---- reading ----

#[tokio::test]
async fn history_is_paged_oldest_first_to_its_end_at_any_size() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let mut said = Vec::new();
    for n in 0..23 {
        let who = if n % 2 == 0 { &t.ada } else { &t.ben };
        said.push(say(&who.client, &conversation, &format!("m{n}")).await["id"].as_str().unwrap().to_string());
    }
    for size in [1, 2, 7, 23, 50, 100] {
        let mut ids: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let query = match &cursor {
                Some(c) => format!("?limit={size}&cursor={c}"),
                None => format!("?limit={size}"),
            };
            let page = history(&t.ben.client, &conversation, &query).await;
            let mut found: Vec<String> = page["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
            found.extend(ids);
            ids = found;
            match page["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
        }
        assert_eq!(ids, said, "size {size}");
    }
    let default = history(&t.ada.client, &conversation, "").await;
    assert_eq!(default["messages"].as_array().unwrap().len(), 23);
    assert!(default["nextCursor"].is_null());
}

#[tokio::test]
async fn a_cursor_keeps_its_snapshot_and_belongs_to_its_conversation() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    for n in 0..4 {
        say(&t.ada.client, &conversation, &format!("m{n}")).await;
    }
    let first = history(&t.ben.client, &conversation, "?limit=2").await;
    let cursor = first["nextCursor"].as_str().unwrap().to_string();
    say(&t.ada.client, &conversation, "late").await;
    let rest = history(&t.ben.client, &conversation, &format!("?limit=10&cursor={cursor}")).await;
    let contents: Vec<&str> = rest["messages"].as_array().unwrap().iter().map(|m| m["content"].as_str().unwrap()).collect();
    assert_eq!(contents, ["m0", "m1"]);

    let other = open_with(&t.ada.client, &t.cyd).await;
    let wrong = t.ada.client.get(&format!("/api/social/conversations/{other}/messages?cursor={cursor}")).await;
    assert_eq!(wrong.status, 400);
    assert_eq!(wrong.json::<Value>()["errors"]["cursor"][0], "The direct message history cursor is invalid.");
    for bad in ["", "abc", "AAAA"] {
        let response = t.ada.client.get(&format!("/api/social/conversations/{conversation}/messages?cursor={bad}")).await;
        assert_eq!(response.status, 400, "{bad:?}");
    }
    for limit in ["0", "101"] {
        let response = t.ada.client.get(&format!("/api/social/conversations/{conversation}/messages?limit={limit}")).await;
        assert_eq!(response.status, 400, "{limit}");
        assert_eq!(response.json::<Value>()["errors"]["limit"][0], "The limit must be between 1 and 100.");
    }
    for limit in ["abc", "", "1e1", "2&limit=3", "-1", "+2", "%202"] {
        let response = t.ada.client.get(&format!("/api/social/conversations/{conversation}/messages?limit={limit}")).await;
        assert_eq!(response.status, 400, "{limit}");
        assert!(response.json::<Value>()["errors"]["limit"].is_array(), "{limit}");
    }
}

#[tokio::test]
async fn marking_a_conversation_read_sets_the_marker_to_now_and_tells_only_the_reader() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    say(&t.ada.client, &conversation, "one").await;
    say(&t.ada.client, &conversation, "two").await;
    let state: Value = t.ben.client.get("/api/social").await.json();
    assert_eq!(state["conversations"][0]["unread"], 2);
    let before = events_named(&t.server, "SocialChanged").len();
    assert_eq!(status(&t.ben.client, Method::POST, &format!("/api/social/conversations/{conversation}/read")).await, 204);
    let state: Value = t.ben.client.get("/api/social").await.json();
    assert_eq!(state["conversations"][0]["unread"], 0);
    let told = events_named(&t.server, "SocialChanged");
    assert_eq!(told.len(), before + 1);
    assert_eq!(told.last().unwrap()["audience"]["users"], json!([t.ben.id]));

    // Reading resets the marker to the current time, even if the stored marker was later.
    t.server
        .database()
        .execute(
            "UPDATE \"DirectConversationReads\" SET \"LastReadAt\" = '2999-01-01 00:00:00' WHERE \"UserId\" = ?1",
            params![id(&t.ben)],
        )
        .unwrap();
    status(&t.ben.client, Method::POST, &format!("/api/social/conversations/{conversation}/read")).await;
    let marker: String = t.server.database().query_row("SELECT \"LastReadAt\" FROM \"DirectConversationReads\" WHERE \"UserId\" = ?1", params![id(&t.ben)], |r| r.get(0)).unwrap();
    assert!(marker.starts_with("20") && marker.as_str() < "2999", "{marker}");
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"DirectConversationReads\""), 2);
}

// ---- encryption ----

struct Sealed {
    epoch: Id,
}

/// Switches encryption on the way its routes leave the rows: both people have
/// an account key, and one epoch is wrapped for both.
fn seal(t: &Trio, conversation: &str) -> Sealed {
    let database = t.server.database();
    let conversation_id = Id::parse(conversation).unwrap();
    let epoch = Id::random();
    for person in [&t.ada, &t.ben] {
        database
            .execute(
                "INSERT OR IGNORE INTO \"AccountKeys\" (\"UserId\", \"KeyId\", \"Algorithm\", \"PublicKey\", \"CreatedAt\") VALUES (?1, ?2, 'x25519', ?3, ?4)",
                params![id(person), Id::random(), vec![7u8; 32], Timestamp::now()],
            )
            .unwrap();
    }
    new_epoch(t, conversation_id, epoch, None);
    database
        .execute(
            "UPDATE \"DirectConversations\" SET \"EncryptionState\" = 'On', \"EncryptionEnabledAt\" = ?2, \"CurrentEpochId\" = ?3 WHERE \"Id\" = ?1",
            params![conversation_id, Timestamp::now(), epoch],
        )
        .unwrap();
    Sealed { epoch }
}

fn new_epoch(t: &Trio, conversation: Id, epoch: Id, previous: Option<Id>) {
    let database = t.server.database();
    let sequence: i64 = database
        .query_row("SELECT COALESCE(MAX(\"Sequence\"), 0) + 1 FROM \"EncryptionEpochs\" WHERE \"ConversationId\" = ?1", params![conversation], |r| r.get(0))
        .unwrap();
    database
        .execute(
            "INSERT INTO \"EncryptionEpochs\" (\"Id\", \"ConversationId\", \"Sequence\", \"PreviousEpochId\", \"CreatedById\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![epoch, conversation, sequence, previous, id(&t.ada), Timestamp::now()],
        )
        .unwrap();
    let keys: Vec<(Id, Id)> = {
        let mut statement = database.prepare("SELECT \"KeyId\", \"UserId\" FROM \"AccountKeys\"").unwrap();
        statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap()
    };
    for (key, user) in keys {
        database
            .execute(
                "INSERT INTO \"EncryptionEnvelopes\" (\"EpochId\", \"RecipientKeyId\", \"RecipientUserId\", \"RecipientKind\", \"EphemeralPublicKey\", \"Iv\", \"Ciphertext\") VALUES (?1, ?2, ?3, 'account', ?4, ?5, ?6)",
                params![epoch, key, user, vec![1u8; 32], vec![2u8; 12], vec![3u8; 48]],
            )
            .unwrap();
    }
}

fn b64(bytes: &[u8]) -> String {
    stellarr_api::encoding::base64(bytes)
}

fn encrypted(epoch: Id, iv: usize, ciphertext: usize) -> Value {
    json!({ "epochId": epoch, "iv": b64(&vec![9u8; iv]), "ciphertext": b64(&vec![5u8; ciphertext]) })
}

#[tokio::test]
async fn while_encryption_is_on_plain_text_is_refused_and_ciphertext_is_stored_as_it_came() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let sealed = seal(&t, &conversation);
    let indexed = count(&t.server, "SELECT COUNT(*) FROM \"MessageIndexEntries\"");

    let plain = send(&t.ada.client, &conversation, json!({ "content": "secret" })).await;
    assert_eq!(plain.status, 409);
    let text = plain.text();
    assert!(text.contains("\"encryption\":{\"state\":\"on\",\"requestedById\":null,\"requestedAt\":null,\"enabledAt\":\""), "{text}");
    assert!(text.contains("\"code\":\"encryption-required\",\"traceId\""), "{text}");
    assert_eq!(plain.json::<Value>()["title"], "This conversation is encrypted. Messages and files have to be encrypted.");

    let client_id = Id::random();
    let ciphertext = vec![0xabu8; 80];
    let response = send(&t.ada.client, &conversation, json!({
        "clientMessageId": client_id,
        "encrypted": { "epochId": sealed.epoch, "iv": b64(&[1u8; 12]), "ciphertext": b64(&ciphertext) }
    })).await;
    assert_eq!(response.status, 201, "{}", response.text());
    let message: Value = response.json();
    assert_eq!(message["content"], "");
    assert_eq!(message["encrypted"], json!({ "epochId": sealed.epoch, "iv": b64(&[1u8; 12]), "ciphertext": b64(&ciphertext) }));
    let (content, iv, stored): (String, Vec<u8>, Vec<u8>) = t
        .server
        .database()
        .query_row("SELECT \"Content\", \"EncryptedIv\", \"EncryptedCiphertext\" FROM \"DirectMessages\"", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!((content.as_str(), iv, stored), ("", vec![1u8; 12], ciphertext));
    // Read back byte for byte, and nothing readable is kept anywhere else.
    let page = history(&t.ben.client, &conversation, "").await;
    assert_eq!(page["messages"][0]["encrypted"], message["encrypted"]);
    assert_eq!(count(&t.server, "SELECT COUNT(*) FROM \"MessageIndexEntries\""), indexed);
    // The retry of an encrypted send is the same send, however it was encrypted.
    let retry = send(&t.ada.client, &conversation, json!({ "clientMessageId": client_id, "encrypted": encrypted(sealed.epoch, 12, 40) })).await;
    assert_eq!(retry.status, 200);
    assert_eq!(retry.json::<Value>(), message);
    let unread: Value = t.ben.client.get("/api/social").await.json();
    assert_eq!(unread["conversations"][0]["unread"], 1);
}

#[tokio::test]
async fn an_encrypted_message_is_bounded_at_every_length() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let sealed = seal(&t, &conversation);
    let post = |body: Value| send(&t.ada.client, &conversation, body);
    let with = |encrypted: Value| json!({ "clientMessageId": Id::random(), "encrypted": encrypted });

    for (iv, ciphertext, want) in [
        (12, 17, 201), (12, 16, 400), (12, 1, 400), (11, 40, 400), (13, 40, 400), (0, 40, 400),
        (12, 262_144, 201), (12, 262_145, 400),
    ] {
        let response = post(with(encrypted(sealed.epoch, iv, ciphertext))).await;
        assert_eq!(response.status, want, "iv {iv} ciphertext {ciphertext}: {}", response.text().chars().take(200).collect::<String>());
        if want == 400 {
            assert_eq!(response.json::<Value>()["errors"]["encrypted"][0], "An encrypted message needs an identifier, a valid ciphertext and no plain text.");
        }
    }
    // No identifier, plain text beside the ciphertext, a missing part.
    let good = encrypted(sealed.epoch, 12, 40);
    for body in [
        json!({ "encrypted": good }),
        json!({ "clientMessageId": Id::random(), "content": "x", "encrypted": good }),
        json!({ "clientMessageId": Id::random(), "encrypted": { "iv": b64(&[1u8; 12]), "ciphertext": b64(&[1u8; 40]) } }),
        json!({ "clientMessageId": Id::random(), "encrypted": { "epochId": sealed.epoch, "ciphertext": b64(&[1u8; 40]) } }),
        json!({ "clientMessageId": Id::random(), "encrypted": { "epochId": sealed.epoch, "iv": b64(&[1u8; 12]) } }),
        json!({ "clientMessageId": Id::random(), "encrypted": {} }),
    ] {
        assert_eq!(post(body.clone()).await.status, 400, "{body}");
    }
    // Spaces and line breaks inside Base64 are ignored, other junk is no body at all.
    let spaced = format!("{{\"clientMessageId\":\"{}\",\"encrypted\":{{\"epochId\":\"{}\",\"iv\":\"AAAA AAAA\\nAAAA\",\"ciphertext\":\"{}\"}}}}", Id::random(), sealed.epoch, b64(&[1u8; 40]));
    assert_eq!(t.ada.client.send(Request::new(Method::POST, &format!("/api/social/conversations/{conversation}/messages")).body("application/json", &spaced)).await.status, 400);
    let junk = format!("{{\"clientMessageId\":\"{}\",\"encrypted\":{{\"epochId\":\"{}\",\"iv\":\"!!\",\"ciphertext\":\"x\"}}}}", Id::random(), sealed.epoch);
    let response = t.ada.client.send(Request::new(Method::POST, &format!("/api/social/conversations/{conversation}/messages")).body("application/json", &junk)).await;
    assert_eq!(response.status, 400);
    assert!(response.text().is_empty());
    // Twenty uploads and no more.
    let twenty: Vec<Id> = (0..20).map(|_| upload(&t.server, &t.ada, true)).collect();
    let mut body = with(encrypted(sealed.epoch, 12, 40));
    body["attachmentIds"] = json!(twenty);
    assert_eq!(post(body.clone()).await.status, 201);
    let mut more = twenty.clone();
    more.push(upload(&t.server, &t.ada, true));
    body["attachmentIds"] = json!(more);
    body["clientMessageId"] = json!(Id::random());
    let refused = post(body).await;
    assert_eq!(refused.status, 400);
    assert_eq!(refused.json::<Value>()["errors"]["attachmentIds"][0], "A message can contain at most 20 attachments.");
    // A plain upload does not go into an encrypted message.
    let plain_upload = upload(&t.server, &t.ada, false);
    let mut body = with(encrypted(sealed.epoch, 12, 40));
    body["attachmentIds"] = json!([plain_upload]);
    assert_eq!(post(body).await.json::<Value>()["code"], "attachment-kind-mismatch");
}

#[tokio::test]
async fn an_epoch_that_is_not_the_conversations_current_one_is_refused_with_what_to_use() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let sealed = seal(&t, &conversation);
    let other = open_with(&t.ada.client, &t.cyd).await;
    let _ = other;
    let with = |epoch: Id| json!({ "clientMessageId": Id::random(), "encrypted": encrypted(epoch, 12, 40) });

    let unknown = send(&t.ada.client, &conversation, with(Id::random())).await;
    assert_eq!(unknown.status, 409);
    let text = unknown.text();
    assert!(text.contains(&format!("\"currentEpochId\":\"{}\",\"code\":\"epoch-not-current\"", sealed.epoch)), "{text}");

    // An old epoch after a rotation.
    let newer = Id::random();
    new_epoch(&t, Id::parse(&conversation).unwrap(), newer, Some(sealed.epoch));
    t.server.database().execute("UPDATE \"DirectConversations\" SET \"CurrentEpochId\" = ?1", params![newer]).unwrap();
    assert_eq!(send(&t.ada.client, &conversation, with(sealed.epoch)).await.json::<Value>()["code"], "epoch-not-current");
    assert_eq!(send(&t.ada.client, &conversation, with(newer)).await.status, 201);

    // The epoch of another conversation.
    let third = open_with(&t.ada.client, &t.cyd).await;
    t.server.database().execute("INSERT OR IGNORE INTO \"AccountKeys\" (\"UserId\", \"KeyId\", \"Algorithm\", \"PublicKey\", \"CreatedAt\") VALUES (?1, ?2, 'x25519', ?3, ?4)", params![id(&t.cyd), Id::random(), vec![7u8; 32], Timestamp::now()]).unwrap();
    let foreign = Id::random();
    new_epoch(&t, Id::parse(&third).unwrap(), foreign, None);
    assert_eq!(send(&t.ada.client, &conversation, with(foreign)).await.json::<Value>()["code"], "epoch-not-current");

    // A conversation whose encryption is off takes nothing encrypted.
    t.server.database().execute("UPDATE \"DirectConversations\" SET \"EncryptionState\" = 'Off', \"CurrentEpochId\" = NULL WHERE \"Id\" = ?1", params![Id::parse(&conversation).unwrap()]).unwrap();
    let off = send(&t.ada.client, &conversation, with(newer)).await;
    assert_eq!(off.status, 409);
    assert!(off.text().contains("\"code\":\"encryption-off\""));
}

#[tokio::test]
async fn an_epoch_that_no_longer_reaches_every_key_must_be_renewed() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let sealed = seal(&t, &conversation);
    // A device was added since: its key is missing from the epoch.
    let session: String = t.server.database().query_row("SELECT \"Id\" FROM \"UserSessions\" WHERE \"UserId\" = ?1", params![id(&t.ben)], |r| r.get(0)).unwrap();
    let device = Id::random();
    t.server
        .database()
        .execute(
            "INSERT INTO \"EncryptionDevices\" (\"Id\", \"UserId\", \"SessionId\", \"Algorithm\", \"PublicKey\", \"Label\", \"CreatedAt\", \"LastSeenAt\", \"RevokedAt\") VALUES (?1, ?2, ?3, 'x25519', ?4, 'Firefox', ?5, ?5, NULL)",
            params![device, id(&t.ben), session, vec![4u8; 32], Timestamp::now()],
        )
        .unwrap();
    let body = json!({ "clientMessageId": Id::random(), "encrypted": encrypted(sealed.epoch, 12, 40) });
    let refused = send(&t.ada.client, &conversation, body.clone()).await;
    assert_eq!(refused.status, 409);
    let text = refused.text();
    assert!(text.contains(&format!("\"missing\":[\"{device}\"],\"extra\":[],\"code\":\"epoch-outdated\"")), "{text}");

    // A key the epoch holds that does not belong in it.
    t.server.database().execute("UPDATE \"EncryptionDevices\" SET \"RevokedAt\" = ?1", params![Timestamp::now()]).unwrap();
    let ghost = Id::random();
    t.server
        .database()
        .execute(
            "INSERT INTO \"EncryptionEnvelopes\" (\"EpochId\", \"RecipientKeyId\", \"RecipientUserId\", \"RecipientKind\", \"EphemeralPublicKey\", \"Iv\", \"Ciphertext\") VALUES (?1, ?2, ?3, 'device', ?4, ?5, ?6)",
            params![sealed.epoch, ghost, id(&t.ben), vec![1u8; 32], vec![2u8; 12], vec![3u8; 48]],
        )
        .unwrap();
    let refused = send(&t.ada.client, &conversation, body).await;
    assert!(refused.text().contains(&format!("\"missing\":[],\"extra\":[\"{ghost}\"]")), "{}", refused.text());
}

#[tokio::test]
async fn timeline_lines_are_listed_in_history_and_do_not_count_as_messages_to_retry() {
    let t = trio().await;
    let conversation = open_with(&t.ada.client, &t.ben).await;
    let line = Id::random();
    t.server
        .database()
        .execute(
            "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\", \"SystemKind\") VALUES (?1, ?2, ?3, '', ?4, 'encryption_enabled')",
            params![line, Id::parse(&conversation).unwrap(), id(&t.ada), Timestamp::now()],
        )
        .unwrap();
    let page = history(&t.ben.client, &conversation, "").await;
    assert_eq!(page["messages"][0]["system"], json!({ "kind": "encryption_enabled", "actorId": t.ada.id }));
    // Reusing a line's id as a client id is a conflict, not a replay.
    assert_eq!(send(&t.ada.client, &conversation, json!({ "content": "", "attachmentIds": [], "clientMessageId": line })).await.status, 400);
    assert_eq!(send(&t.ada.client, &conversation, json!({ "content": "x", "clientMessageId": line })).await.status, 409);
}
