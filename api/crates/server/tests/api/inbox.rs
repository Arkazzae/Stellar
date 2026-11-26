//! A person's inbox through the real binary: paging, reading, dismissing, what
//! an entry looks like for each notification kind, and what each
//! change announces. What writes entries (invitations, mentions) is arranged
//! in the database here.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, HubClient, Person, Request, Server};

const COOKIE: &str = "stellarr-dev-session";
const QUIET: Duration = Duration::from_millis(400);

struct Inbox {
    server: Server,
    ada: Person,
    ben: Person,
    home: Id,
    channel: Id,
}

async fn inbox() -> Inbox {
    let server = Server::start().await;
    let ada = server.found("ada").await;
    let ben = server.register("ben").await;
    let (home, channel): (Id, Id) = server
        .database()
        .query_row(
            "SELECT \"Id\", (SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = \"Servers\".\"Id\" AND \"Kind\" = 0 LIMIT 1) FROM \"Servers\" LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    Inbox { server, ada, ben, home, channel }
}

struct Row<'a> {
    server: Id,
    user: &'a Person,
    title: &'a str,
    at: &'a str,
    channel: Option<Id>,
    invite: Option<&'a str>,
    source: &'a str,
}

impl Inbox {
    fn put(&self, row: Row<'_>) -> Id {
        let id = Id::random();
        let database = self.server.database();
        database.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        database
            .execute(
                "INSERT INTO \"UserNotifications\" (\"Id\", \"ServerId\", \"UserId\", \"Source\", \"SourcePackageId\", \"SourcePackageName\", \"SourceModuleId\", \"SourceModuleName\", \"Title\", \"Body\", \"Tone\", \"TargetChannelId\", \"TargetInviteCode\", \"CreatedAt\", \"ReadAt\")
                 VALUES (?1, ?2, ?3, ?4, 'pkg', 'Package', 'mod', 'Module', ?5, ?6, 'info', ?7, ?8, ?9, NULL)",
                params![id, row.server, Id::parse(&row.user.id).unwrap(), row.source, row.title, format!("Body of {}", row.title), row.channel, row.invite, Timestamp::parse(row.at).unwrap()],
            )
            .unwrap();
        id
    }

    fn plain(&self, user: &Person, title: &str) -> Id {
        self.put(Row { server: self.home, user, title, at: "2026-03-01 10:00:00.5", channel: None, invite: None, source: "sdk" })
    }

    fn invite(&self, server: Id, code: &str, uses: i64, max: Option<i64>, expires: Option<&str>) {
        let database = self.server.database();
        database.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        database
            .execute(
                "INSERT INTO \"ServerInvites\" (\"Id\", \"ServerId\", \"CreatedById\", \"ChannelId\", \"Code\", \"Uses\", \"MaxUses\", \"ExpiresAt\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
                params![Id::random(), server, Id::parse(&self.ada.id).unwrap(), code, uses, max, expires.map(|e| Timestamp::parse(e).unwrap()), Timestamp::now()],
            )
            .unwrap();
    }
}

async fn list(client: &Client, query: &str) -> Value {
    let response = client.get(&format!("/api/notifications{query}")).await;
    assert_eq!(response.status, 200, "{query}: {}", response.text());
    response.json()
}

async fn status(client: &Client, method: Method, path: &str) -> u16 {
    client.send(Request::new(method, path)).await.status.as_u16()
}

fn ids(page: &Value) -> Vec<String> {
    page["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
}

fn inbox_events(server: &Server) -> Vec<Value> {
    server.events().into_iter().filter(|e| e["name"] == "NotificationInboxChanged").collect()
}

#[tokio::test]
async fn the_inbox_is_paged_newest_first_and_private_to_its_owner() {
    let i = inbox().await;
    let mut mine = Vec::new();
    for n in 0..250 {
        mine.push(i.plain(&i.ada, &format!("n{n}")));
        if n % 5 == 0 {
            i.plain(&i.ben, "private to ben");
        }
    }
    let page = list(&i.ada.client, "").await;
    assert_eq!(page["items"].as_array().unwrap().len(), 30);
    assert_eq!(page["unreadCount"], 250);
    assert_eq!(page["items"][0]["id"], mine[249].to_string());
    assert_eq!(page["items"][0]["title"], "n249");

    // Walk to the end by the cursor: every entry once, none of ben's.
    let mut seen = ids(&page);
    let mut cursor = page["nextBeforeSequence"].as_i64();
    let mut pages = 1;
    while let Some(before) = cursor {
        let next = list(&i.ada.client, &format!("?limit=100&beforeSequence={before}")).await;
        seen.extend(ids(&next));
        cursor = next["nextBeforeSequence"].as_i64();
        pages += 1;
        assert!(next["lastSequence"].as_i64().unwrap() >= before);
    }
    assert_eq!(pages, 1 + 3);
    let want: Vec<String> = mine.iter().rev().map(|id| id.to_string()).collect();
    assert_eq!(seen, want);

    // A page that ends exactly at the end has no cursor.
    let exact = list(&i.ada.client, "?limit=100").await;
    assert!(exact["nextBeforeSequence"].is_i64());
    let all = list(&i.ada.client, &format!("?limit=100&beforeSequence={}", exact["nextBeforeSequence"])).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 100);
    let tail = list(&i.ada.client, &format!("?limit=50&beforeSequence={}", all["nextBeforeSequence"])).await;
    assert_eq!(tail["items"].as_array().unwrap().len(), 50);
    assert!(tail["nextBeforeSequence"].is_null());
    let bens = list(&i.ben.client, "?limit=100").await;
    assert_eq!(bens["unreadCount"], 50);
    assert!(ids(&bens).iter().all(|id| !seen.contains(id)));
    assert_eq!(list(&i.ben.client, "?beforeSequence=1").await["items"], json!([]));
}

#[tokio::test]
async fn app_badge_combines_private_messages_and_mentions_for_this_viewer() {
    let i = inbox().await;
    let path = "/api/me/app-badge";
    i.plain(&i.ada, "ordinary notification");
    let mention = i.put(Row { server: i.home, user: &i.ada, title: "mentioned", at: "2026-03-01 10:00:00",
        channel: Some(i.channel), invite: None, source: "system" });
    i.server.database().execute(
        r#"UPDATE "UserNotifications" SET "SourcePackageId" = 'stellarr.mention' WHERE "Id" = ?1"#,
        [mention],
    ).unwrap();
    let conversation = i.ben.client.post_json("/api/social/conversations", &json!({"userId": i.ada.id})).await.json::<Value>();
    let conversation = conversation["id"].as_str().unwrap();
    for content in ["first", "second"] {
        let response = i.ben.client.post_json(&format!("/api/social/conversations/{conversation}/messages"), &json!({ "content": content })).await;
        assert_eq!(response.status, 201);
    }
    let response = i.ada.client.get(path).await;
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.json::<Value>(), json!({"unread": 3}));
    assert_eq!(i.ben.client.get(path).await.json::<Value>(), json!({"unread": 0}));
    assert_eq!(status(&i.ada.client, Method::POST, &format!("/api/social/conversations/{conversation}/read")).await, 204);
    assert_eq!(i.ada.client.get(path).await.json::<Value>(), json!({"unread": 1}));
    assert_eq!(status(&i.ada.client, Method::PUT, &format!("/api/channels/{}/read", i.channel)).await, 200);
    assert_eq!(i.ada.client.get(path).await.json::<Value>(), json!({"unread": 0}));
    assert_eq!(status(&i.ada.client, Method::PUT, &format!("/api/notifications/{mention}/read")).await, 200);
    assert_eq!(i.ada.client.get(path).await.json::<Value>(), json!({"unread": 0}));
}

#[tokio::test]
async fn an_empty_inbox_has_nothing_in_it() {
    let i = inbox().await;
    assert_eq!(list(&i.ada.client, "").await, json!({ "items": [], "nextBeforeSequence": null, "unreadCount": 0, "lastSequence": 0 }));
}

#[tokio::test]
async fn an_inbox_entry_preserves_json_field_order() {
    let i = inbox().await;
    i.put(Row { server: i.home, user: &i.ada, title: "Hello", at: "2026-03-01 10:00:00.5", channel: Some(i.channel), invite: None, source: "sdk" });
    let response = i.ada.client.get("/api/notifications").await;
    let text = response.text();
    let keys = ["\"id\"", "\"sequence\"", "\"title\"", "\"body\"", "\"tone\"", "\"source\":{\"type\"", "\"packageId\"", "\"packageName\"", "\"moduleId\"", "\"moduleName\"", "\"target\":{\"type\":\"channel\"", "\"channelId\"", "\"inviteCode\":null", "\"serverId\":null", "\"serverName\":null", "\"invitedByName\":null", "\"createdAt\"", "\"readAt\":null"];
    let mut at = 0;
    for key in keys {
        let found = text[at..].find(key).unwrap_or_else(|| panic!("{key} in order in {text}"));
        at += found + key.len();
    }
    assert!(text.starts_with("{\"items\":[{"), "{text}");
    assert!(text.contains("\"createdAt\":\"2026-03-01T10:00:00.5Z\""), "{text}");
    assert!(text.ends_with("\"nextBeforeSequence\":null,\"unreadCount\":1,\"lastSequence\":1}"), "{text}");
}

#[tokio::test]
async fn an_invitation_shows_on_every_server_while_its_invite_can_be_used() {
    let i = inbox().await;
    let elsewhere = Id::random();
    let now_ish = "2026-03-01 10:00:00";
    let mut rows = Vec::new();
    for (code, uses, max, expires) in [
        ("usable", 0, Some(5), Some("2999-01-01 00:00:00")),
        ("unlimited", 3, None, None),
        ("expired", 0, None, Some("2020-01-01 00:00:00")),
        ("used-up", 5, Some(5), None),
    ] {
        i.invite(elsewhere, code, uses, max, expires);
        rows.push((code, i.put(Row { server: elsewhere, user: &i.ada, title: code, at: now_ish, channel: None, invite: Some(code), source: "system" })));
    }
    // No invite row at all, a row that is not a notice of the system, and an
    // entry of another server that is no invitation.
    i.put(Row { server: elsewhere, user: &i.ada, title: "gone", at: now_ish, channel: None, invite: Some("deleted"), source: "system" });
    i.invite(elsewhere, "not-system", 0, None, None);
    i.put(Row { server: elsewhere, user: &i.ada, title: "not-system", at: now_ish, channel: None, invite: Some("not-system"), source: "sdk" });
    i.put(Row { server: elsewhere, user: &i.ada, title: "other", at: now_ish, channel: None, invite: None, source: "system" });

    let page = list(&i.ada.client, "").await;
    let mut titles: Vec<&str> = page["items"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    titles.sort();
    assert_eq!(titles, ["unlimited", "usable"]);
    assert_eq!(page["unreadCount"], 2);
    let usable = page["items"].as_array().unwrap().iter().find(|e| e["title"] == "usable").unwrap();
    assert_eq!(usable["target"], json!({ "type": "server-invite", "channelId": null, "inviteCode": "usable", "serverId": elsewhere.to_string(), "serverName": "Package", "invitedByName": "Module" }));

    // One that can no longer be used stays dismissable, and readable.
    let expired = rows.iter().find(|r| r.0 == "expired").unwrap().1;
    assert_eq!(status(&i.ada.client, Method::PUT, &format!("/api/notifications/{expired}/read")).await, 200);
    assert_eq!(status(&i.ada.client, Method::DELETE, &format!("/api/notifications/{expired}")).await, 204);
    // An entry of another server that is no invitation does not exist here.
    let other: Id = i.server.database().query_row("SELECT \"Id\" FROM \"UserNotifications\" WHERE \"Title\" = 'other'", [], |r| r.get(0)).unwrap();
    assert_eq!(status(&i.ada.client, Method::DELETE, &format!("/api/notifications/{other}")).await, 404);
}

#[tokio::test]
async fn reading_deleting_and_reading_everything_are_repeatable_and_private() {
    let i = inbox().await;
    let first = i.plain(&i.ada, "first");
    let second = i.plain(&i.ada, "second");
    let bens = i.plain(&i.ben, "bens");

    let read = i.ada.client.send(Request::new(Method::PUT, &format!("/api/notifications/{second}/read"))).await;
    assert_eq!(read.status, 200);
    let body = read.json::<Value>();
    assert_eq!(body["id"], second.to_string());
    // A moment the request set and one read back are spelled the same.
    assert!(body["readAt"].as_str().unwrap().ends_with('Z'), "{body}");
    assert_eq!(body["createdAt"], "2026-03-01T10:00:00.5Z");
    let again = i.ada.client.send(Request::new(Method::PUT, &format!("/api/notifications/{second}/read"))).await.json::<Value>();
    assert_eq!(again["readAt"], body["readAt"]);
    assert_eq!(list(&i.ada.client, "").await["unreadCount"], 1);

    let all = i.ada.client.send(Request::new(Method::POST, "/api/notifications/read-all")).await;
    assert_eq!(all.text(), "{\"changed\":1}");
    assert_eq!(i.ada.client.send(Request::new(Method::POST, "/api/notifications/read-all")).await.text(), "{\"changed\":0}");
    assert_eq!(list(&i.ada.client, "").await["unreadCount"], 0);
    assert_eq!(list(&i.ben.client, "").await["unreadCount"], 1);

    for path in [format!("/api/notifications/{bens}/read"), format!("/api/notifications/{}/read", Id::random())] {
        assert_eq!(status(&i.ada.client, Method::PUT, &path).await, 404, "{path}");
    }
    assert_eq!(status(&i.ada.client, Method::DELETE, &format!("/api/notifications/{bens}")).await, 404);
    assert_eq!(status(&i.ada.client, Method::DELETE, &format!("/api/notifications/{first}")).await, 204);
    assert_eq!(status(&i.ada.client, Method::DELETE, &format!("/api/notifications/{first}")).await, 404);
    assert_eq!(ids(&list(&i.ada.client, "").await), [second.to_string()]);
    assert_eq!(list(&i.ben.client, "").await["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_unread_count_is_the_whole_inbox_of_the_server_not_the_page() {
    let i = inbox().await;
    let other = Id::random();
    for n in 0..5 {
        i.plain(&i.ada, &format!("n{n}"));
    }
    // An entry of another server does not count while nothing invites to it.
    i.put(Row { server: other, user: &i.ada, title: "elsewhere", at: "2026-03-01 10:00:00", channel: None, invite: None, source: "sdk" });
    let page = list(&i.ada.client, "?limit=2").await;
    assert_eq!(page["unreadCount"], 5);
    assert_eq!(page["lastSequence"], 5);
    assert_eq!(i.ada.client.send(Request::new(Method::POST, "/api/notifications/read-all")).await.text(), "{\"changed\":5}");
}

#[tokio::test]
async fn the_query_is_read_by_exact_names_and_plain_numbers() {
    let i = inbox().await;
    for n in 0..4 {
        i.plain(&i.ada, &format!("n{n}"));
    }
    for (query, count) in [("?limit=2", 2), ("?unknown=1&limit=1", 1), ("?beforeSequence=3", 2)] {
        assert_eq!(list(&i.ada.client, query).await["items"].as_array().unwrap().len(), count, "{query}");
    }
    for query in ["?limit=", "?limit", "?beforeSequence=", "?limit=abc", "?limit=2&limit=3", "?limit=1e1", "?limit=2.0", "?limit=99999999999", "?beforeSequence=9999999999999999999999", "?limit=1_0", "?limit=%C2%B22", "?limit=+2", "?limit=%202", "?limit=-1", "?beforeSequence=-4"] {
        let response = i.ada.client.get(&format!("/api/notifications{query}")).await;
        assert_eq!(response.status, 400, "{query}");
        assert_eq!(response.header("content-type"), Some("application/problem+json"), "{query}");
        assert_eq!(response.header("cache-control"), Some("no-store"), "{query}");
    }
    for query in ["?limit=0", "?limit=101", "?beforeSequence=0"] {
        let response = i.ada.client.get(&format!("/api/notifications{query}")).await;
        assert_eq!(response.status, 400, "{query}");
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        assert_eq!(
            response.json::<Value>()["errors"]["page"][0],
            "limit must be between 1 and 100 and beforeSequence must be positive."
        );
    }
}

#[tokio::test]
async fn which_refusal_comes_first_and_which_answers_forbid_caching() {
    let i = inbox().await;
    i.plain(&i.ada, "one");
    let stranger = Id::random();
    for path in ["/api/notifications", "/api/notifications?limit=0"] {
        let response = i.ada.client.send(Request::new(Method::GET, path).header("X-Stellarr-Server", &stranger.to_string())).await;
        assert_eq!(response.status, 403, "{path}");
        assert!(response.header("cache-control").is_none(), "a refusal of the server comes before the headers");
    }
    // A query that does not bind is refused after the server is looked at.
    let response = i.ada.client.send(Request::new(Method::GET, "/api/notifications?limit=x").header("X-Stellarr-Server", &stranger.to_string())).await;
    assert_eq!(response.status, 403);
    let response = i.ada.client.send(Request::new(Method::GET, "/api/notifications").header("X-Stellarr-Server", "not-an-id")).await;
    assert_eq!(response.status, 400);
    assert_eq!(i.server.anonymous().get("/api/notifications").await.status, 401);

    // Everything after it says no cache may keep the answer.
    for (method, path, status) in [
        (Method::GET, "/api/notifications".to_string(), 200),
        (Method::GET, "/api/notifications?limit=0".to_string(), 400),
        (Method::PUT, format!("/api/notifications/{}/read", Id::random()), 404),
        (Method::DELETE, format!("/api/notifications/{}", Id::random()), 404),
        (Method::POST, "/api/notifications/read-all".to_string(), 200),
    ] {
        let response = i.ada.client.send(Request::new(method.clone(), &path)).await;
        assert_eq!(response.status, status, "{method} {path}");
        assert_eq!(response.header("cache-control"), Some("no-store"), "{path}");
        assert_eq!(response.header("pragma"), Some("no-cache"), "{path}");
        assert_eq!(response.header("x-content-type-options"), Some("nosniff"), "{path}");
    }
}

#[tokio::test]
async fn the_server_may_be_named_by_header_or_query_in_the_id_spelling_only() {
    let i = inbox().await;
    i.plain(&i.ada, "one");
    for query in [format!("?serverId={}", i.home), format!("?serverId={}", i.home.to_string().to_uppercase())] {
        assert_eq!(list(&i.ada.client, &query).await["items"].as_array().unwrap().len(), 1, "{query}");
    }
    let response = i.ada.client.send(Request::new(Method::GET, "/api/notifications").header("x-stellarr-server", &format!("{{{}}}", i.home))).await;
    assert_eq!(response.status, 400);
}

#[tokio::test]
async fn every_route_of_the_inbox_needs_somebody_signed_in() {
    let server = Server::start().await;
    let anonymous = server.anonymous();
    for (method, path) in [
        (Method::GET, "/api/notifications"),
        (Method::PUT, "/api/notifications/3d82da44-8aa5-4bdf-9ee1-2902df99880c/read"),
        (Method::POST, "/api/notifications/read-all"),
        (Method::DELETE, "/api/notifications/3d82da44-8aa5-4bdf-9ee1-2902df99880c"),
    ] {
        assert_eq!(status(&anonymous, method.clone(), path).await, 401, "{method} {path}");
    }
}

fn cookie(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

#[tokio::test]
async fn each_change_tells_the_owners_own_connections_what_changed() {
    let i = inbox().await;
    let elsewhere = Id::random();
    let plain = i.plain(&i.ada, "plain");
    i.invite(elsewhere, "code", 0, None, None);
    let invitation = i.put(Row { server: elsewhere, user: &i.ada, title: "inv", at: "2026-03-01 10:00:00", channel: None, invite: Some("code"), source: "system" });
    let sequence = |id: Id| -> i64 { i.server.database().query_row("SELECT \"Sequence\" FROM \"UserNotifications\" WHERE \"Id\" = ?1", [id], |r| r.get(0)).unwrap() };

    let mut owner = HubClient::connect(i.server.address(), &cookie(&i.ada)).await;
    let mut bystander = HubClient::connect(i.server.address(), &cookie(&i.ben)).await;
    owner.quiet_for(QUIET).await;
    bystander.quiet_for(QUIET).await;

    i.ada.client.send(Request::new(Method::PUT, &format!("/api/notifications/{plain}/read"))).await;
    assert_eq!(owner.event("NotificationInboxChanged").await, json!({ "serverId": i.home.to_string(), "sequence": sequence(plain), "global": false }));
    // Reading what is read says nothing.
    i.ada.client.send(Request::new(Method::PUT, &format!("/api/notifications/{plain}/read"))).await;
    i.ada.client.send(Request::new(Method::DELETE, &format!("/api/notifications/{invitation}"))).await;
    assert_eq!(owner.event("NotificationInboxChanged").await, json!({ "serverId": elsewhere.to_string(), "sequence": 2, "global": true }));
    i.plain(&i.ada, "again");
    i.ada.client.send(Request::new(Method::POST, "/api/notifications/read-all")).await;
    assert_eq!(owner.event("NotificationInboxChanged").await, json!({ "serverId": i.home.to_string(), "sequence": null, "global": true }));
    // Nothing changed, nothing said.
    i.ada.client.send(Request::new(Method::POST, "/api/notifications/read-all")).await;
    i.ada.client.send(Request::new(Method::DELETE, &format!("/api/notifications/{plain}"))).await;
    assert_eq!(owner.event("NotificationInboxChanged").await["sequence"], 1);
    assert!(owner.quiet_for(QUIET).await.is_empty());
    assert!(bystander.quiet_for(QUIET).await.is_empty());
    assert_eq!(inbox_events(&i.server).len(), 4);
}
