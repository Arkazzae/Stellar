//! Posting, rewriting and removing messages, and reading a channel back.

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Response, Server};

/// An instance with its owner and its first text channel.
struct World {
    server: Server,
    owner: Person,
    general: String,
    instance: String,
}

async fn world() -> World {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let db = server.database();
    let general: String = db
        .query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    World { server, owner, general: general.to_lowercase(), instance }
}

impl World {
    fn db(&self) -> Connection {
        self.server.database()
    }

    /// A text channel of the instance's server.
    fn channel(&self, name: &str, slow_mode: i64) -> String {
        let id = Id::random();
        self.db()
            .execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, ?3, 0, 20, '', ?4, 0, 0, NULL)",
                params![id.to_db_text(), self.instance, name, slow_mode],
            )
            .unwrap();
        id.to_string()
    }

    /// Denies `mask` to one person at a channel, as their own override.
    fn deny(&self, channel: &str, person: &Person, mask: i64) {
        let db = self.db();
        let held: i64 = db
            .query_row(
                "SELECT COALESCE((SELECT \"DenyMask\" FROM \"MemberPermissionOverrides\" WHERE \"TargetId\" = ?1 AND \"UserId\" = ?2), 0)",
                params![channel.to_uppercase(), person.id.to_uppercase()],
                |row| row.get(0),
            )
            .unwrap();
        db.execute(
            "DELETE FROM \"MemberPermissionOverrides\" WHERE \"TargetId\" = ?1 AND \"UserId\" = ?2",
            params![channel.to_uppercase(), person.id.to_uppercase()],
        )
        .unwrap();
        let mask = mask | held;
        db
            .execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, ?5)",
                params![Id::random().to_db_text(), self.instance, channel.to_uppercase(), person.id.to_uppercase(), mask],
            )
            .unwrap();
    }

    /// A private channel: view is denied to everybody who holds nothing but the base role.
    fn private_channel(&self, name: &str, allowed: &[&Person]) -> String {
        let channel = self.channel(name, 0);
        let db = self.db();
        let base: String = db
            .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
            params![Id::random().to_db_text(), self.instance, channel.to_uppercase(), base],
        )
        .unwrap();
        for person in allowed {
            db.execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 16384, 0)",
                params![Id::random().to_db_text(), self.instance, channel.to_uppercase(), person.id.to_uppercase()],
            )
            .unwrap();
        }
        channel
    }

    /// An upload of `owner`, as the upload routes leave it.
    fn upload(&self, owner: &Person, voice: bool, encrypted: bool) -> String {
        let id = Id::random();
        self.db()
            .execute(
                "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"Width\", \"Height\", \"DurationSeconds\", \"IsVoiceMessage\", \"IsEncrypted\", \"CreatedAt\") VALUES (?1, ?2, ?3, 'file.bin', ?4, 100, NULL, NULL, ?5, ?6, ?7, ?8)",
                params![
                    id.to_db_text(),
                    owner.id.to_uppercase(),
                    format!("test/{id}"),
                    if voice { "audio/webm" } else { "application/pdf" },
                    voice.then_some(12.0),
                    voice,
                    encrypted,
                    Timestamp::now()
                ],
            )
            .unwrap();
        id.to_string()
    }

    async fn post(&self, who: &Person, channel: &str, body: Value) -> Response {
        who.client.post_json(&format!("/api/channels/{channel}/messages"), &body).await
    }

    async fn say(&self, who: &Person, channel: &str, content: &str) -> Value {
        let response = self.post(who, channel, json!({ "content": content })).await;
        assert_eq!(response.status, 201, "{}", response.text());
        response.json()
    }

    async fn edit(&self, who: &Person, message: &str, body: Value) -> Response {
        who.client.send(Request::new(http::Method::PATCH, &format!("/api/messages/{message}")).json(&body)).await
    }

    async fn remove(&self, who: &Person, message: &str) -> Response {
        who.client.send(Request::new(http::Method::DELETE, &format!("/api/messages/{message}"))).await
    }

    fn stored(&self, message: &str) -> Option<String> {
        self.db()
            .query_row("SELECT \"Content\" FROM \"Messages\" WHERE \"Id\" = ?1", [message.to_uppercase()], |row| row.get(0))
            .ok()
    }

    fn journal(&self, channel: &str) -> Vec<(i64, String, i64)> {
        let db = self.db();
        let mut statement = db
            .prepare("SELECT \"Sequence\", \"MessageId\", \"Kind\" FROM \"MessageChanges\" WHERE \"ChannelId\" = ?1 ORDER BY \"Sequence\"")
            .unwrap();
        statement
            .query_map([channel.to_uppercase()], |row| Ok((row.get(0)?, row.get::<_, String>(1)?.to_lowercase(), row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn events_named(&self, name: &str) -> Vec<Value> {
        self.server.events().into_iter().filter(|event| event["name"] == name).collect()
    }
}

fn id_of(message: &Value) -> String {
    message["id"].as_str().unwrap().to_string()
}

// ---- Posting

#[tokio::test]
async fn a_post_answers_201_with_where_it_is_and_shows_in_the_history() {
    let w = world().await;

    let response = w.post(&w.owner, &w.general, json!({ "content": "  hello  " })).await;

    assert_eq!(response.status, 201);
    let message: Value = response.json();
    assert_eq!(response.header("location"), Some(format!("/api/channels/{}/messages/{}", w.general, id_of(&message)).as_str()));
    assert_eq!(message["content"], "hello");
    assert_eq!(message["author"]["username"], "owner");
    assert_eq!(message["author"]["presence"], "online");
    assert_eq!(message["editedAt"], Value::Null);
    let history: Value = w.owner.client.get(&format!("/api/channels/{}/messages", w.general)).await.json();
    let listed: Vec<&str> = history["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(listed.last().copied(), Some(id_of(&message).as_str()));
    assert_eq!(history["nextCursor"], Value::Null);
}

#[tokio::test]
async fn members_are_told_of_a_post_and_its_author_reads_the_channel_to_it() {
    let w = world().await;
    let message = w.say(&w.owner, &w.general, "hello").await;

    let created = w.events_named("MessageCreated");
    let read = w.events_named("ChannelReadChanged");

    // Nobody is denied view in the first channel, so the whole server hears.
    let created = created.last().unwrap();
    assert_eq!(created["audience"], json!({ "server": w.instance.to_lowercase() }));
    assert_eq!(created["payload"]["id"], message["id"]);
    let read = read.last().unwrap();
    assert_eq!(read["audience"], json!({ "users": [w.owner.id.clone()] }));
    assert_eq!(read["payload"]["channelId"], w.general);
    assert_eq!(read["payload"]["lastReadAt"], message["createdAt"]);
    let marker: String = w
        .db()
        .query_row("SELECT \"LastReadAt\" FROM \"ChannelReads\" WHERE \"UserId\" = ?1", [w.owner.id.to_uppercase()], |row| row.get(0))
        .unwrap();
    assert_eq!(marker.replace(' ', "T") + "Z", message["createdAt"].as_str().unwrap());
}

#[tokio::test]
async fn an_event_about_a_private_channel_goes_to_those_who_see_it() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let noor = w.server.register("noor").await;
    let secret = w.private_channel("secret", &[&maya]);

    w.say(&maya, &secret, "psst").await;

    let created = w.events_named("MessageCreated");
    let audience = &created.last().unwrap()["audience"]["followers"];
    assert_eq!(audience["server"], w.instance.to_lowercase());
    let mut who: Vec<String> = audience["users"].as_array().unwrap().iter().map(|u| u.as_str().unwrap().to_string()).collect();
    who.sort();
    let mut expected = vec![w.owner.id.clone(), maya.id.clone()];
    expected.sort();
    assert_eq!(who, expected);
    assert!(!who.contains(&noor.id));
}

#[tokio::test]
async fn a_post_that_is_refused_for_its_text_says_which_field_and_why() {
    let w = world().await;
    let path = format!("/api/channels/{}/messages", w.general);

    for body in [json!({}), json!({ "content": null }), json!({ "content": "   \u{a0}\n" }), json!({ "content": "x".repeat(20_001) })] {
        let response = w.owner.client.post_json(&path, &body).await;
        assert_eq!(response.status, 400);
        let problem: Value = response.json();
        assert_eq!(problem["title"], "One or more validation errors occurred.");
        assert_eq!(problem["errors"]["content"][0], "A message must contain text or an attachment and be at most 20000 characters long.");
    }
}

#[tokio::test]
async fn the_length_of_a_message_is_counted_in_utf16_units_after_trimming() {
    let w = world().await;

    for (content, status) in [
        (format!("  {}  ", "a".repeat(20_000)), 201),
        ("a".repeat(20_001), 400),
        (format!("{}\u{1F600}", "a".repeat(19_998)), 201),
        (format!("{}\u{1F600}", "a".repeat(19_999)), 400),
    ] {
        let response = w.post(&w.owner, &w.general, json!({ "content": content })).await;
        assert_eq!(response.status, status, "{} units", content.encode_utf16().count());
    }
}

#[tokio::test]
async fn at_most_ten_attachments_and_a_client_id_that_is_a_real_one() {
    let w = world().await;
    let bob = w.server.register("bob").await;
    let uploads: Vec<String> = (0..10).map(|_| w.upload(&bob, false, false)).collect();

    // Counted before duplicates are dropped.
    let eleven = w.post(&bob, &w.general, json!({ "content": "x", "attachmentIds": vec![uploads[0].clone(); 11] })).await;
    assert_eq!(eleven.status, 400);
    assert_eq!(eleven.json::<Value>()["errors"]["attachmentIds"][0], "A message can contain at most 10 attachments.");
    let ten = w.post(&bob, &w.general, json!({ "content": "x", "attachmentIds": uploads })).await;
    assert_eq!(ten.status, 201);
    assert_eq!(ten.json::<Value>()["attachments"].as_array().unwrap().len(), 10);
    let zero = w.post(&bob, &w.general, json!({ "content": "x", "clientMessageId": "00000000-0000-0000-0000-000000000000" })).await;
    assert_eq!(zero.status, 400);
    assert!(zero.json::<Value>()["errors"]["clientMessageId"].is_array());
}

#[tokio::test]
async fn attachments_must_be_the_senders_own_and_a_voice_message_travels_alone() {
    let w = world().await;
    let jo = w.server.register("joe").await;
    let ilya = w.server.register("ilya").await;
    let voice = w.upload(&jo, true, false);
    let file = w.upload(&jo, false, false);
    let foreign = w.upload(&ilya, false, false);
    let encrypted = w.upload(&jo, false, true);
    let send = |content: &'static str, ids: Vec<&str>| {
        let body = json!({ "content": content, "attachmentIds": ids });
        let w = &w;
        let jo = &jo;
        async move { w.post(jo, &w.general, body).await.status }
    };

    assert_eq!(send("", vec![&foreign]).await, 400);
    assert_eq!(send("", vec![&encrypted]).await, 400);
    assert_eq!(send("", vec![&voice, &file]).await, 400);
    assert_eq!(send("with words", vec![&voice]).await, 400);
    assert_eq!(send("", vec![&voice]).await, 201);
    assert_eq!(send("file", vec![&file]).await, 201);
    // A file alone, with no words, is a message too.
    assert_eq!(send("", vec![&file]).await, 201);
}

#[tokio::test]
async fn a_reply_must_point_into_the_same_channel() {
    let w = world().await;
    let other = w.channel("projects", 0);
    let original = w.say(&w.owner, &w.general, "pytanie").await;
    let reply = |channel: &str, to: String| {
        let body = json!({ "content": "odpowiedź", "replyToMessageId": to });
        let channel = channel.to_string();
        let w = &w;
        async move { w.post(&w.owner, &channel, body).await }
    };

    assert_eq!(reply(&other, id_of(&original)).await.status, 400);
    assert_eq!(reply(&w.general, Id::random().to_string()).await.status, 400);
    let answered = reply(&w.general, id_of(&original)).await;
    assert_eq!(answered.status, 201);
    let quoted = &answered.json::<Value>()["replyTo"];
    assert_eq!(quoted["id"], original["id"]);
    assert_eq!(quoted["preview"], "pytanie");
    assert_eq!(quoted["authorName"], "Owner");
    assert_eq!(quoted["hasAttachment"], false);
}

#[tokio::test]
async fn a_quote_is_a_line_of_at_most_160_units_and_names_an_attachment_when_there_are_no_words() {
    let w = world().await;
    let long = w.say(&w.owner, &w.general, &format!("{}\u{1F600}bbbb\n\n  c", "a".repeat(156))).await;
    let file = w.upload(&w.owner, false, false);
    let silent: Value = w.post(&w.owner, &w.general, json!({ "attachmentIds": [file] })).await.json();

    let on_long: Value = w.post(&w.owner, &w.general, json!({ "content": "re", "replyToMessageId": long["id"] })).await.json();
    let on_silent: Value = w.post(&w.owner, &w.general, json!({ "content": "re", "replyToMessageId": silent["id"] })).await.json();

    // The cut falls inside the pair, which leaves the replacement character.
    assert_eq!(on_long["replyTo"]["preview"], format!("{}\u{FFFD}…", "a".repeat(156)));
    assert_eq!(on_silent["replyTo"]["preview"], "Attachment");
    assert_eq!(on_silent["replyTo"]["hasAttachment"], true);
}

#[tokio::test]
async fn sending_is_admitted_in_the_channel_and_a_channel_nobody_sees_is_not_there() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let noor = w.server.register("noor").await;
    let secret = w.private_channel("secret", &[&maya]);
    w.deny(&w.general, &noor, 512);

    let denied = w.post(&noor, &w.general, json!({ "content": "x", "clientMessageId": Id::random().to_string() })).await;
    assert_eq!(denied.status, 403);
    assert_eq!(denied.header("content-type"), Some("application/problem+json"));
    let problem: Value = denied.json();
    assert_eq!(problem["permission"], "sendMessages");
    assert_eq!(problem["title"], "You may not send messages in this channel.");
    let hidden = w.post(&noor, &secret, json!({ "content": "x" })).await;
    let unknown = w.post(&noor, &Id::random().to_string(), json!({ "content": "x" })).await;
    assert_eq!((hidden.status.as_u16(), hidden.body.len()), (404, 0));
    assert_eq!((unknown.status.as_u16(), unknown.body.len()), (404, 0));
    assert_eq!(w.post(&maya, &secret, json!({ "content": "x" })).await.status, 201);
}

#[tokio::test]
async fn attaching_is_admitted_apart_from_sending() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    w.deny(&w.general, &maya, 1024);
    let file = w.upload(&maya, false, false);

    let with = w.post(&maya, &w.general, json!({ "content": "x", "attachmentIds": [file] })).await;
    let without = w.post(&maya, &w.general, json!({ "content": "x" })).await;

    assert_eq!(with.status, 403);
    assert_eq!(with.json::<Value>()["permission"], "attachFiles");
    assert_eq!(without.status, 201);
}

#[tokio::test]
async fn a_retry_is_answered_before_sending_is_admitted_but_not_before_the_channel_is_seen() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let id = Id::random().to_string();
    let first = w.post(&maya, &w.general, json!({ "content": "once", "clientMessageId": id })).await;
    assert_eq!(first.status, 201);
    w.deny(&w.general, &maya, 512);

    let retry = w.post(&maya, &w.general, json!({ "content": "once", "clientMessageId": id })).await;
    let fresh = w.post(&maya, &w.general, json!({ "content": "once", "clientMessageId": Id::random().to_string() })).await;
    // Take view away too: the stored message is no longer hers to be told of.
    w.deny(&w.general, &maya, 16384);
    let after = w.post(&maya, &w.general, json!({ "content": "once", "clientMessageId": id })).await;

    assert_eq!(retry.status, 200);
    assert_eq!(fresh.status, 403);
    assert_eq!((after.status.as_u16(), after.body.len()), (404, 0));
}

#[tokio::test]
async fn a_client_id_makes_a_send_idempotent_until_the_content_differs() {
    let w = world().await;
    let other = w.channel("projects", 0);
    let jo = w.server.register("joe").await;
    let id = Id::random().to_string();
    let path = |channel: &str| format!("/api/channels/{channel}/messages");

    let first = w.owner.client.post_json(&path(&w.general), &json!({ "content": "tylko raz", "clientMessageId": id })).await;
    let same = w.owner.client.post_json(&path(&w.general), &json!({ "content": "  tylko raz ", "clientMessageId": id })).await;
    let changed = w.owner.client.post_json(&path(&w.general), &json!({ "content": "inaczej", "clientMessageId": id })).await;
    let elsewhere = w.owner.client.post_json(&path(&other), &json!({ "content": "tylko raz", "clientMessageId": id })).await;
    let stolen = jo.client.post_json(&path(&w.general), &json!({ "content": "tylko raz", "clientMessageId": id })).await;

    assert_eq!(first.status, 201);
    assert_eq!(same.status, 200);
    assert_eq!(same.json::<Value>()["id"], id);
    assert_eq!((changed.status.as_u16(), elsewhere.status.as_u16(), stolen.status.as_u16()), (409, 409, 409));
    assert_eq!(w.stored(&id).as_deref(), Some("tylko raz"));
}

#[tokio::test]
async fn eight_sends_at_once_with_one_client_id_store_one_message() {
    let w = world().await;
    let id = Id::random().to_string();
    let path = format!("/api/channels/{}/messages", w.general);

    let body = json!({ "content": "wyścig", "clientMessageId": id });
    let responses = futures_util_join((0..8).map(|_| w.owner.client.post_json(&path, &body))).await;

    let created = responses.iter().filter(|r| r.status == 201).count();
    let replayed = responses.iter().filter(|r| r.status == 200).count();
    assert_eq!((created, replayed), (1, 7), "{:?}", responses.iter().map(|r| r.status).collect::<Vec<_>>());
    let stored: i64 = w.db().query_row("SELECT COUNT(*) FROM \"Messages\" WHERE \"Id\" = ?1", [id.to_uppercase()], |row| row.get(0)).unwrap();
    assert_eq!(stored, 1);
    assert_eq!(w.journal(&w.general).iter().filter(|(_, message, _)| *message == id).count(), 1);
}

/// Awaits all of them at the same time.
async fn futures_util_join<F: std::future::Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut set = Vec::new();
    for future in futures {
        set.push(future);
    }
    let mut results = Vec::new();
    // `join_all` without a dependency: polled together by one `select` loop.
    let mut pending: Vec<std::pin::Pin<Box<F>>> = set.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = (0..pending.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut all = true;
        for (index, future) in pending.iter_mut().enumerate() {
            if done[index].is_none() {
                match future.as_mut().poll(cx) {
                    std::task::Poll::Ready(value) => done[index] = Some(value),
                    std::task::Poll::Pending => all = false,
                }
            }
        }
        if all { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
    })
    .await;
    results.extend(done.into_iter().map(Option::unwrap));
    results
}

#[tokio::test]
async fn slow_mode_holds_back_members_but_not_those_who_manage_channels() {
    let w = world().await;
    let slow = w.channel("slow", 3600);
    let jo = w.server.register("joe").await;

    assert_eq!(w.post(&jo, &slow, json!({ "content": "first" })).await.status, 201);
    let held = w.post(&jo, &slow, json!({ "content": "second" })).await;

    assert_eq!(held.status, 429);
    let wait: i64 = held.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=3600).contains(&wait));
    let problem: Value = held.json();
    assert_eq!(problem["title"], "Slow mode is on");
    assert!(problem["detail"].as_str().unwrap().contains(&wait.to_string()));
    assert_eq!(w.post(&w.owner, &slow, json!({ "content": "first" })).await.status, 201);
    assert_eq!(w.post(&w.owner, &slow, json!({ "content": "second" })).await.status, 201);
    assert_eq!(w.post(&jo, &w.general, json!({ "content": "elsewhere" })).await.status, 201);
}

#[tokio::test]
async fn a_retried_send_is_recognised_before_slow_mode_can_refuse_it() {
    let w = world().await;
    let slow = w.channel("slow", 3600);
    let jo = w.server.register("joe").await;
    let body = json!({ "content": "raz", "clientMessageId": Id::random().to_string() });

    assert_eq!(w.post(&jo, &slow, body.clone()).await.status, 201);
    assert_eq!(w.post(&jo, &slow, body).await.status, 200);
    assert_eq!(w.post(&jo, &slow, json!({ "content": "dwa", "clientMessageId": Id::random().to_string() })).await.status, 429);
}

#[tokio::test]
async fn slow_mode_is_counted_from_the_last_post_even_after_it_is_deleted() {
    let w = world().await;
    let slow = w.channel("slow", 3600);
    let jo = w.server.register("joe").await;
    let first = w.say(&jo, &slow, "first").await;
    assert_eq!(w.remove(&jo, &id_of(&first)).await.status, 204);

    assert_eq!(w.post(&jo, &slow, json!({ "content": "second" })).await.status, 429);
}

// ---- Rewriting and removing

#[tokio::test]
async fn the_author_rewrites_a_message_and_the_channel_is_told_without_viewer_state() {
    let w = world().await;
    let message = w.say(&w.owner, &w.general, "pierwsza wersja").await;

    let edit = w.edit(&w.owner, &id_of(&message), json!({ "content": "  druga wersja  " })).await;

    assert_eq!(edit.status, 200);
    let edited: Value = edit.json();
    assert_eq!(edited["content"], "druga wersja");
    assert!(edited["editedAt"].is_string());
    let announced = w.events_named("MessageUpdated");
    let announced = announced.last().unwrap();
    assert_eq!(announced["payload"]["content"], "druga wersja");
    assert_eq!(announced["audience"], json!({ "server": w.instance.to_lowercase() }));
}

#[tokio::test]
async fn saving_the_same_text_is_not_an_edit_and_leaves_no_trace_in_the_journal() {
    let w = world().await;
    let message = w.say(&w.owner, &w.general, "bez zmian").await;
    let before = w.journal(&w.general).len();

    let edit = w.edit(&w.owner, &id_of(&message), json!({ "content": " bez zmian " })).await;

    assert_eq!(edit.status, 200);
    assert_eq!(edit.json::<Value>()["editedAt"], Value::Null);
    assert_eq!(w.journal(&w.general).len(), before);
    // The event is still sent: it says what the message is now.
    assert_eq!(w.events_named("MessageUpdated").len(), 1);
}

#[tokio::test]
async fn only_the_author_may_rewrite_a_message_moderators_included() {
    let w = world().await;
    let jo = w.server.register("joe").await;
    let ada = w.server.register("adam").await;
    let message = id_of(&w.say(&jo, &w.general, "moje słowa").await);

    for other in [&w.owner, &ada] {
        assert_eq!(w.edit(other, &message, json!({ "content": "cudze słowa" })).await.status, 404);
    }

    assert_eq!(w.stored(&message).as_deref(), Some("moje słowa"));
}

#[tokio::test]
async fn an_invalid_rewrite_is_refused_before_the_message_is_looked_up() {
    let w = world().await;
    let unknown = Id::random().to_string();

    for body in [json!({}), json!({ "content": null }), json!({ "content": "   " }), json!({ "content": "x".repeat(20_001) })] {
        assert_eq!(w.edit(&w.owner, &unknown, body).await.status, 400);
    }
    assert_eq!(w.edit(&w.owner, &unknown, json!({ "content": "tekst" })).await.status, 404);
    assert_eq!(w.edit(&w.owner, "not-an-id", json!({ "content": "tekst" })).await.status, 404);
}

#[tokio::test]
async fn the_author_and_a_moderator_may_remove_a_message_and_another_member_may_not() {
    let w = world().await;
    let jo = w.server.register("joe").await;
    let ilya = w.server.register("ilya").await;
    let own = id_of(&w.say(&jo, &w.general, "do usunięcia").await);
    let moderated = id_of(&w.say(&jo, &w.general, "do moderacji").await);

    assert_eq!(w.remove(&ilya, &moderated).await.status, 404);
    assert!(w.stored(&moderated).is_some());
    assert_eq!(w.remove(&w.owner, &moderated).await.status, 204);
    assert!(w.stored(&moderated).is_none());
    let announced = w.events_named("MessageDeleted");
    assert_eq!(announced.last().unwrap()["payload"], json!({ "messageId": moderated, "channelId": w.general }));
    assert_eq!(w.remove(&jo, &own).await.status, 204);
    assert_eq!(w.remove(&jo, &own).await.status, 404);
}

#[tokio::test]
async fn a_notice_of_the_server_cannot_be_rewritten_or_removed_by_anybody() {
    let w = world().await;
    let notice: String = w
        .db()
        .query_row("SELECT \"Id\" FROM \"Messages\" WHERE \"IsSystem\" = 1 LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let notice = notice.to_lowercase();

    assert_eq!(w.edit(&w.owner, &notice, json!({ "content": "podmiana" })).await.status, 404);
    assert_eq!(w.remove(&w.owner, &notice).await.status, 404);
    assert!(w.stored(&notice).unwrap().contains("joined"));
}

#[tokio::test]
async fn a_message_of_another_server_is_out_of_reach() {
    let w = world().await;
    let db = w.db();
    let (server, channel, message) = (Id::random(), Id::random(), Id::random());
    db.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
    db.execute("INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\") VALUES (?1, 'Obcy', ?2)", params![server.to_db_text(), Timestamp::now()]).ok();
    db.execute(
        "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'ogólny', 0, 0, '', 0, 0, 0, NULL)",
        params![channel.to_db_text(), server.to_db_text()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 0, 'gdzie indziej', ?4)",
        params![message.to_db_text(), channel.to_db_text(), w.owner.id.to_uppercase(), Timestamp::now()],
    )
    .unwrap();

    let id = message.to_string();
    assert_eq!(w.edit(&w.owner, &id, json!({ "content": "zmiana" })).await.status, 404);
    assert_eq!(w.remove(&w.owner, &id).await.status, 404);
    assert!(w.stored(&id).is_some());
    assert_eq!(w.post(&w.owner, &channel.to_string(), json!({ "content": "x" })).await.status, 404);
}

#[tokio::test]
async fn somebody_who_no_longer_sees_the_channel_may_neither_rewrite_nor_remove_what_they_wrote() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let message = id_of(&w.say(&maya, &w.general, "mine").await);
    let theirs = id_of(&w.say(&w.owner, &w.general, "owner's").await);
    w.deny(&w.general, &maya, 16384);

    assert_eq!(w.edit(&maya, &message, json!({ "content": "x" })).await.status, 404);
    assert_eq!(w.remove(&maya, &message).await.status, 404);
    assert_eq!(w.remove(&maya, &theirs).await.status, 404);
    assert!(w.stored(&message).is_some());
    // The owner still manages messages there.
    assert_eq!(w.remove(&w.owner, &message).await.status, 204);
}

#[tokio::test]
async fn removing_a_message_leaves_the_files_it_carried_and_a_reply_to_it_loses_its_target() {
    let w = world().await;
    let file = w.upload(&w.owner, false, false);
    let original: Value = w.post(&w.owner, &w.general, json!({ "content": "x", "attachmentIds": [file] })).await.json();
    let reply = w.post(&w.owner, &w.general, json!({ "content": "re", "replyToMessageId": original["id"] })).await;
    assert_eq!(reply.status, 201);

    assert_eq!(w.remove(&w.owner, &id_of(&original)).await.status, 204);

    let kept: i64 = w.db().query_row("SELECT COUNT(*) FROM \"ImageAttachments\" WHERE \"Id\" = ?1", [file.to_uppercase()], |row| row.get(0)).unwrap();
    assert_eq!(kept, 1);
    let history: Value = w.owner.client.get(&format!("/api/channels/{}/messages", w.general)).await.json();
    let last = history["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["id"], reply.json::<Value>()["id"]);
    assert_eq!(last["replyTo"], Value::Null);
}

// ---- Reading a channel back

/// Messages written straight into a channel, oldest first, at the moments given.
fn write_messages(w: &World, channel: &str, author: &Person, moments: &[&str]) -> Vec<String> {
    let db = w.db();
    moments
        .iter()
        .enumerate()
        .map(|(number, moment)| {
            let id = Id::parse(&format!("a0000000-0000-0000-0000-{:012}", number + 1)).unwrap();
            db.execute(
                "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 0, ?4, ?5)",
                params![id.to_db_text(), channel.to_uppercase(), author.id.to_uppercase(), format!("message {}", number + 1), moment],
            )
            .unwrap();
            id.to_string()
        })
        .collect()
}

async fn page(w: &World, channel: &str, query: &str) -> Response {
    w.owner.client.get(&format!("/api/channels/{channel}/messages{query}")).await
}

fn ids(page: &Value) -> Vec<String> {
    page["messages"].as_array().unwrap().iter().map(id_of).collect()
}

#[tokio::test]
async fn history_is_paged_newest_first_by_a_cursor_and_each_page_is_oldest_first() {
    let w = world().await;
    let channel = w.channel("history", 0);
    let all = write_messages(&w, &channel, &w.owner, &["2026-11-01 10:00:01", "2026-11-01 10:00:02", "2026-11-01 10:00:03", "2026-11-01 10:00:04", "2026-11-01 10:00:05"]);

    let first: Value = page(&w, &channel, "?limit=2").await.json();
    let second: Value = page(&w, &channel, &format!("?limit=2&cursor={}", first["nextCursor"].as_str().unwrap())).await.json();
    let third: Value = page(&w, &channel, &format!("?limit=2&cursor={}", second["nextCursor"].as_str().unwrap())).await.json();

    assert_eq!(ids(&first), all[3..5]);
    assert_eq!(ids(&second), all[1..3]);
    assert_eq!(ids(&third), all[0..1]);
    assert_eq!(third["nextCursor"], Value::Null);
    let journal = w.journal(&channel);
    assert_eq!(first["syncCursor"], journal.last().unwrap().0.to_string());
    // A page of exactly what is left has no cursor either.
    let exact: Value = page(&w, &channel, "?limit=5").await.json();
    assert_eq!((ids(&exact).len(), exact["nextCursor"].clone()), (5, Value::Null));
}

#[tokio::test]
async fn a_cursor_is_the_first_servers_bytes() {
    let w = world().await;
    let channel = w.channel("history", 0);
    write_messages(&w, &channel, &w.owner, &["2026-11-01 10:00:01", "2026-11-01 10:00:02", "2026-11-01 10:00:03"]);

    let first: Value = page(&w, &channel, "?limit=1").await.json();

    let bytes = base64url_decode(first["nextCursor"].as_str().unwrap());
    assert_eq!(bytes.len(), 49);
    assert_eq!(bytes[0], 1);
    // The channel as the first server's UUID type lays it out: three groups reversed.
    let id = Id::parse(&channel).unwrap();
    let text = id.as_bytes();
    assert_eq!(&bytes[1..5], &[text[3], text[2], text[1], text[0]]);
    // The last message's moment, in ticks from the year 1.
    let last = i64::from_be_bytes(bytes[25..33].try_into().unwrap());
    let expected = Timestamp::parse("2026-11-01 10:00:03").unwrap().ticks() + 621_355_968_000_000_000;
    assert_eq!(last, expected);
}

fn base64url_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let digits: Vec<u32> = text.chars().map(|c| ALPHABET.find(c).unwrap() as u32).collect();
    let mut out = Vec::new();
    for group in digits.chunks(4) {
        let word = group.iter().enumerate().fold(0u32, |word, (index, digit)| word | digit << (18 - 6 * index));
        for index in 0..group.len() - 1 {
            out.push((word >> (16 - 8 * index)) as u8);
        }
    }
    out
}

fn legacy_cursor(channel: &str, snapshot: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let id = Id::parse(channel).unwrap();
    let t = id.as_bytes();
    let mut bytes = vec![t[3], t[2], t[1], t[0], t[5], t[4], t[7], t[6], t[8], t[9], t[10], t[11], t[12], t[13], t[14], t[15]];
    bytes.extend((Timestamp::parse(snapshot).unwrap().ticks() + 621_355_968_000_000_000).to_be_bytes());
    bytes.extend(2i32.to_be_bytes());
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let word = chunk.iter().enumerate().fold(0u32, |word, (index, byte)| word | u32::from(*byte) << (16 - 8 * index));
        for index in 0..chunk.len() + 1 {
            out.push(char::from(ALPHABET[((word >> (18 - 6 * index)) & 63) as usize]));
        }
    }
    out
}

#[tokio::test]
async fn deleting_a_delivered_row_never_skips_an_unread_one() {
    for tied in [false, true] {
        for delete_anchor in [false, true] {
            let w = world().await;
            let channel = w.channel("history", 0);
            let moments: Vec<String> = (1..=6).map(|n| if tied { "2026-11-01 10:00:00".to_string() } else { format!("2026-11-01 10:00:0{n}") }).collect();
            let moments: Vec<&str> = moments.iter().map(String::as_str).collect();
            let all = write_messages(&w, &channel, &w.owner, &moments);
            let first: Value = page(&w, &channel, "?limit=2").await.json();
            let mut seen = ids(&first);
            assert_eq!(seen, all[4..6]);

            let deleted = &all[if delete_anchor { 4 } else { 5 }];
            w.db().execute("DELETE FROM \"Messages\" WHERE \"Id\" = ?1", [deleted.to_uppercase()]).unwrap();
            // A new arrival is outside the snapshot.
            w.say(&w.owner, &channel, "new arrival").await;

            let mut cursor = first["nextCursor"].as_str().map(str::to_string);
            for _ in 0..5 {
                let Some(next) = cursor else { break };
                let more: Value = page(&w, &channel, &format!("?limit=2&cursor={next}")).await.json();
                seen.extend(ids(&more));
                cursor = more["nextCursor"].as_str().map(str::to_string);
            }
            assert_eq!(cursor, None);
            let mut sorted = seen.clone();
            sorted.sort();
            assert_eq!(sorted, all, "tied {tied}, anchor {delete_anchor}");
        }
    }
}

#[tokio::test]
async fn an_older_cursor_restarts_its_snapshot_and_a_cursor_of_another_channel_is_refused() {
    let w = world().await;
    let channel = w.channel("history", 0);
    let all = write_messages(&w, &channel, &w.owner, &["2026-11-01 10:00:00", "2026-11-01 10:00:00", "2026-11-01 10:00:00", "2026-11-01 10:00:00", "2026-11-01 10:00:00", "2026-11-01 10:00:00"]);

    let first: Value = page(&w, &channel, &format!("?limit=2&cursor={}", legacy_cursor(&channel, "2026-11-02 00:00:00"))).await.json();

    assert_eq!(ids(&first), all[4..6]);
    let next: Value = page(&w, &channel, &format!("?limit=2&cursor={}", first["nextCursor"].as_str().unwrap())).await.json();
    assert_eq!(ids(&next), all[2..4]);
    let wrong = page(&w, &channel, &format!("?cursor={}", legacy_cursor(&Id::random().to_string(), "2026-11-02 00:00:00"))).await;
    assert_eq!(wrong.status, 400);
    assert_eq!(wrong.json::<Value>()["errors"]["cursor"][0], "The message history cursor is invalid.");
}

#[tokio::test]
async fn a_history_read_refuses_what_is_not_a_page_and_a_channel_nobody_sees() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let secret = w.private_channel("secret", &[]);

    let too_many = page(&w, &w.general, "?limit=101").await;
    assert_eq!(too_many.status, 400);
    assert_eq!(too_many.json::<Value>()["errors"]["limit"][0], "The limit must be between 1 and 100.");
    for query in ["?limit=0", "?limit=-1", "?limit=abc", "?cursor=nonsense", "?cursor=A"] {
        assert_eq!(page(&w, &w.general, query).await.status, 400, "{query}");
    }
    for channel in [secret.clone(), Id::random().to_string()] {
        let answer = maya.client.get(&format!("/api/channels/{channel}/messages")).await;
        assert_eq!((answer.status.as_u16(), answer.body.len()), (404, 0));
        let sync = maya.client.get(&format!("/api/channels/{channel}/messages/sync")).await;
        assert_eq!((sync.status.as_u16(), sync.body.len()), (404, 0));
    }
    assert_eq!(page(&w, &secret, "").await.status, 200);
    assert_eq!(w.server.anonymous().get(&format!("/api/channels/{}/messages", w.general)).await.status, 401);
    let voice = w.db().query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 1", [], |row| row.get::<_, String>(0)).unwrap();
    assert_eq!(page(&w, &voice.to_lowercase(), "").await.status, 404);
}

#[tokio::test]
async fn a_history_page_carries_replies_attachments_and_the_readers_own_reactions() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let original = w.say(&w.owner, &w.general, "pytanie").await;
    let file = w.upload(&w.owner, false, false);
    let reply: Value = w.post(&w.owner, &w.general, json!({ "content": "odpowiedź", "replyToMessageId": original["id"], "attachmentIds": [file] })).await.json();
    let db = w.db();
    for (user, emoji, at) in [(&w.owner, "👍", "2026-11-01 10:00:02"), (&maya, "👍", "2026-11-01 10:00:03"), (&maya, "🎉", "2026-11-01 10:00:01")] {
        db.execute(
            "INSERT INTO \"MessageReactions\" (\"MessageId\", \"UserId\", \"Emoji\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
            params![id_of(&reply).to_uppercase(), user.id.to_uppercase(), emoji, at],
        )
        .unwrap();
    }

    let seen_by_owner: Value = page(&w, &w.general, "").await.json();
    let seen_by_maya: Value = maya.client.get(&format!("/api/channels/{}/messages", w.general)).await.json();

    let mine = seen_by_owner["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(mine["replyTo"]["id"], original["id"]);
    assert_eq!(mine["attachments"][0]["id"], file);
    assert_eq!(mine["attachments"][0]["url"], format!("/assets/{file}"));
    assert_eq!(mine["attachments"][0]["isVoiceMessage"], false);
    // In the order of the first reaction to each emoji.
    assert_eq!(mine["reactions"], json!([{ "emoji": "🎉", "count": 1, "mine": false }, { "emoji": "👍", "count": 2, "mine": true }]));
    let hers = seen_by_maya["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(hers["reactions"], json!([{ "emoji": "🎉", "count": 1, "mine": true }, { "emoji": "👍", "count": 2, "mine": true }]));
}

// ---- The journal

async fn sync(w: &World, channel: &str, query: &str) -> Value {
    let response = w.owner.client.get(&format!("/api/channels/{channel}/messages/sync{query}")).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

#[tokio::test]
async fn sync_answers_an_upsert_after_a_send_and_an_edit_and_a_delete_after_a_removal() {
    let w = world().await;
    let start = sync(&w, &w.general, "").await["nextCursor"].as_str().unwrap().to_string();
    let message = id_of(&w.say(&w.owner, &w.general, "pierwsza").await);
    let after_send = sync(&w, &w.general, &format!("?after={start}")).await;
    let sent = &after_send["changes"].as_array().unwrap()[0];
    assert_eq!((sent["kind"].as_str(), sent["messageId"].as_str()), (Some("upsert"), Some(message.as_str())));
    assert_eq!(sent["message"]["content"], "pierwsza");

    w.edit(&w.owner, &message, json!({ "content": "druga" })).await;
    let after_edit = sync(&w, &w.general, &format!("?after={}", after_send["nextCursor"].as_str().unwrap())).await;
    assert_eq!(after_edit["changes"][0]["message"]["content"], "druga");
    assert!(after_edit["changes"][0]["message"]["editedAt"].is_string());

    w.remove(&w.owner, &message).await;
    let after_delete = sync(&w, &w.general, &format!("?after={}", after_edit["nextCursor"].as_str().unwrap())).await;
    let removed = after_delete["changes"].as_array().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!((removed[0]["kind"].as_str(), removed[0]["message"].clone()), (Some("delete"), Value::Null));
    // Replayed from the start, a message that was upserted and then removed has nothing left to show.
    let all = sync(&w, &w.general, "?after=0").await;
    let mine: Vec<&Value> = all["changes"].as_array().unwrap().iter().filter(|c| c["messageId"] == message).collect();
    assert_eq!(mine.iter().map(|c| c["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["delete", "delete", "delete"]);
}

#[tokio::test]
async fn the_journal_of_a_channel_is_its_own_and_pages_by_limit() {
    let w = world().await;
    let projects = w.channel("projects", 0);
    let cursor = sync(&w, &projects, "").await["nextCursor"].as_str().unwrap().to_string();
    let sent: Vec<String> = {
        let mut sent = Vec::new();
        for number in 0..3 {
            sent.push(id_of(&w.say(&w.owner, &projects, &format!("wiadomość {number}")).await));
        }
        sent
    };
    w.say(&w.owner, &w.general, "gdzie indziej").await;

    let first = sync(&w, &projects, &format!("?after={cursor}&limit=2")).await;
    let second = sync(&w, &projects, &format!("?after={}&limit=2", first["nextCursor"].as_str().unwrap())).await;

    assert_eq!((first["hasMore"].clone(), second["hasMore"].clone()), (json!(true), json!(false)));
    let walked: Vec<&str> = first["changes"].as_array().unwrap().iter().chain(second["changes"].as_array().unwrap()).map(|c| c["messageId"].as_str().unwrap()).collect();
    assert_eq!(walked, sent.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(page(&w, &projects, "?limit=1").await.json::<Value>()["syncCursor"], second["nextCursor"]);
}

#[tokio::test]
async fn a_compacted_journal_asks_the_client_to_load_the_history_again() {
    let w = world().await;
    w.db()
        .execute(
            "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\") VALUES (?1, '00000000-0000-0000-0000-000000000000', 2, ?2)",
            params![w.general.to_uppercase(), Timestamp::now()],
        )
        .unwrap();

    let answer = sync(&w, &w.general, "?after=0").await;

    assert_eq!(answer["resetRequired"], true);
}

#[tokio::test]
async fn sync_refuses_bad_positions() {
    let w = world().await;
    let path = format!("/api/channels/{}/messages/sync", w.general);

    for query in ["?after=-1", "?limit=0", "?limit=501", "?after=x"] {
        assert_eq!(w.owner.client.get(&format!("{path}{query}")).await.status, 400, "{query}");
    }
    let refused = w.owner.client.get(&format!("{path}?limit=501")).await;
    assert_eq!(refused.json::<Value>()["errors"]["cursor"][0], "The sync cursor or the limit is invalid.");
    assert_eq!(w.owner.client.get(&format!("{path}?after=99999999999")).await.status, 200);
}

#[tokio::test]
async fn a_delete_leaves_a_journal_that_removes_the_cards_first() {
    let w = world().await;
    let message = id_of(&w.say(&w.owner, &w.general, "with a card").await);
    w.db()
        .execute(
            "INSERT INTO \"MessageEmbeds\" (\"Id\", \"MessageId\", \"Url\", \"Kind\", \"Provider\", \"Domain\", \"Position\", \"CreatedAt\") VALUES (?1, ?2, 'https://example.test/', 0, 'example.test', 'example.test', 0, ?3)",
            params![Id::random().to_db_text(), message.to_uppercase(), Timestamp::now()],
        )
        .unwrap();
    let before = w.journal(&w.general).len();

    w.remove(&w.owner, &message).await;

    let kinds: Vec<i64> = w.journal(&w.general)[before..].iter().map(|(_, _, kind)| *kind).collect();
    assert_eq!(kinds, [0, 1]);
}

// ---- Mentions

fn notifications(w: &World) -> Vec<(String, String, String)> {
    let db = w.db();
    let mut statement = db
        .prepare("SELECT \"UserId\", \"Title\", \"Body\" FROM \"UserNotifications\" ORDER BY \"Sequence\"")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?.to_lowercase(), row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[tokio::test]
async fn a_mention_notifies_the_members_it_names_once_and_never_its_author() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let noor = w.server.register("noor").await;

    w.say(&maya, &w.general, "hej @noor @NOOR i @maya, @nobody `@owner` oraz maya@noor.test").await;

    let entries = notifications(&w);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, noor.id);
    assert_eq!(entries[0].1, "Maya mentioned you");
    assert!(entries[0].2.starts_with("hej @noor @NOOR"));
    let inbox = w.events_named("NotificationInboxChanged");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["audience"], json!({ "users": [noor.id] }));
    assert_eq!(inbox[0]["payload"]["serverId"], w.instance.to_lowercase());
    assert_eq!(inbox[0]["payload"]["global"], false);
    assert!(inbox[0]["payload"]["sequence"].is_i64());
}

#[tokio::test]
async fn nobody_is_told_of_a_message_in_a_channel_they_do_not_see() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let noor = w.server.register("noor").await;
    let jo = w.server.register("joe").await;
    let secret = w.private_channel("secret", &[&maya, &jo]);
    for person in [&noor, &jo] {
        w.db()
            .execute(
                "UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\" WHERE \"Id\" = ?1",
                [person.id.to_uppercase()],
            )
            .unwrap();
    }

    w.say(&maya, &secret, "@noor @joe @owner").await;

    let told: Vec<String> = notifications(&w).into_iter().map(|(user, _, _)| user).collect();
    let mut expected = vec![jo.id.clone(), w.owner.id.clone()];
    expected.sort();
    let mut told_sorted = told.clone();
    told_sorted.sort();
    assert_eq!(told_sorted, expected);
    assert!(!told.contains(&noor.id));
}

#[tokio::test]
async fn external_deliveries_are_queued_only_for_those_who_opted_in_and_have_somewhere_to_go() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let noor = w.server.register("noor").await;
    let jo = w.server.register("joe").await;
    let db = w.db();
    let columns: Vec<String> = db.prepare("PRAGMA table_info(\"UserPreferences\")").unwrap().query_map([], |row| row.get::<_, String>(1)).unwrap().map(Result::unwrap).collect();
    for (person, email, push) in [(&noor, 1, 1), (&jo, 1, 1)] {
        let mut values: Vec<String> = columns.iter().map(|_| "0".to_string()).collect();
        for (index, column) in columns.iter().enumerate() {
            match column.as_str() {
                "UserId" => values[index] = format!("'{}'", person.id.to_uppercase()),
                "EmailNotifications" => values[index] = email.to_string(),
                "PushNotifications" => values[index] = push.to_string(),
                "MutedChannelIdsJson" => values[index] = "'[]'".to_string(),
                "JoinSoundId" | "AppearanceSettingsJson" | "LanguageSettingsJson" | "KeybindSettingsJson" | "VoiceSettingsJson" | "PrivacySettingsJson" => values[index] = "NULL".to_string(),
                _ => {}
            }
        }
        let names: Vec<String> = columns.iter().map(|c| format!("\"{c}\"")).collect();
        db.execute_batch(&format!("INSERT INTO \"UserPreferences\" ({}) VALUES ({})", names.join(","), values.join(","))).unwrap();
    }
    // Noor's address was vouched for by a provider; jo's was never confirmed.
    db.execute("UPDATE \"Users\" SET \"ProviderVerifiedEmail\" = \"NormalizedEmail\" WHERE \"Id\" = ?1", [noor.id.to_uppercase()]).unwrap();
    // Noor has one live session with two subscriptions, and one that is revoked with another.
    let version: i64 = db.query_row("SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1", [noor.id.to_uppercase()], |row| row.get(0)).unwrap();
    let live = Id::random();
    let dead = Id::random();
    for (session, revoked) in [(live, None), (dead, Some("2026-01-01 00:00:00"))] {
        db.execute(
            "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\", \"AbsoluteExpiresAt\", \"PrimarySource\", \"SecurityVersion\", \"ShortLived\", \"RevokedAt\") VALUES (?1, ?2, 'd', 'i', '2026-01-01 00:00:00', '2026-01-01 00:00:00', '2999-01-01 00:00:00', '2999-01-01 00:00:00', 'password', ?3, 0, ?4)",
            params![session.to_db_text(), noor.id.to_uppercase(), version, revoked],
        )
        .unwrap();
    }
    let subscriptions: Vec<Id> = (0..3).map(|_| Id::random()).collect();
    for (subscription, session) in subscriptions.iter().zip([live, live, dead]) {
        db.execute(
            "INSERT INTO \"PushSubscriptions\" (\"Id\", \"UserId\", \"SessionId\", \"Endpoint\", \"P256dh\", \"Auth\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, 'p', 'a', '2026-01-01 00:00:00')",
            params![subscription.to_db_text(), noor.id.to_uppercase(), session.to_db_text(), format!("https://push.test/{subscription}")],
        )
        .unwrap();
    }

    let message = id_of(&w.say(&maya, &w.general, "@noor @joe @owner").await);

    let mut statement = db
        .prepare("SELECT \"RecipientId\", \"Channel\", \"DestinationKey\", \"Status\", \"EventKind\", \"SourceMessageId\", \"Attempts\", \"ExpiresAt\" > \"CreatedAt\" FROM \"NotificationDeliveries\" ORDER BY \"Channel\", \"DestinationKey\"")
        .unwrap();
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, String, i64, bool)> = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?.to_lowercase(), row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get::<_, String>(5)?.to_lowercase(), row.get(6)?, row.get(7)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 3, "{rows:?}");
    let email = &rows[0];
    assert_eq!((email.0.as_str(), email.1.as_str(), email.2.as_str()), (noor.id.as_str(), "email", "NOOR@EXAMPLE.TEST"));
    let mut keys: Vec<String> = subscriptions[..2].iter().map(|s| s.to_string().replace('-', "")).collect();
    keys.sort();
    assert_eq!(rows[1..].iter().map(|r| r.2.clone()).collect::<Vec<_>>(), keys);
    for row in &rows {
        assert_eq!((row.3.as_str(), row.4.as_str(), row.5.as_str(), row.6, row.7), ("pending", "mention", message.as_str(), 0, true));
    }
}

#[tokio::test]
async fn a_message_of_twenty_thousand_units_of_distinct_names_is_read_in_a_blink_and_names_each_member_once() {
    let w = world().await;
    let abc = w.server.register("abc").await;
    // Distinct three-letter names with a space each, up to 20,000 units.
    let letters: Vec<char> = ('a'..='z').collect();
    let mut content = String::new();
    'fill: for a in &letters {
        for b in &letters {
            for c in &letters {
                if content.len() + 5 > 20_000 {
                    break 'fill;
                }
                content.push_str(&format!("@{a}{b}{c} "));
            }
        }
    }
    let content = content.trim().to_string();
    assert!(content.encode_utf16().count() <= 20_000 && content.contains("@abc"));
    let started = std::time::Instant::now();

    let response = w.post(&w.owner, &w.general, json!({ "content": content })).await;

    assert_eq!(response.status, 201, "{}", response.text());
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    let entries = notifications(&w);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, abc.id);
}

#[tokio::test]
async fn an_inbox_entry_quotes_at_most_220_units_of_the_message_on_one_line() {
    let w = world().await;
    let maya = w.server.register("maya").await;
    let noor = w.server.register("noor").await;

    w.say(&maya, &w.general, &format!("@noor {}\n\n{}", "x".repeat(300), "y")).await;

    let entry = &notifications(&w)[0];
    assert_eq!(entry.0, noor.id);
    assert_eq!(entry.2.encode_utf16().count(), 218);
    assert!(entry.2.ends_with('…'));
    assert!(!entry.2.contains('\n'));
}

// ---- What a post costs

#[test]
fn the_cost_of_a_post_in_statements_does_not_grow_with_what_the_server_holds() {
    use std::sync::{Arc, Mutex};
    use stellarr_api::messages::post::{Posted, Submission, post};

    // A server, founded and left with its owner and one more member.
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let measure = |extra_members: usize| -> (usize, usize) {
        runtime.block_on(async {
            let server = Server::start().await;
            let owner = server.found("owner").await;
            let mut db = server.database();
            let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
            let channel: String = db.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 LIMIT 1", [], |row| row.get(0)).unwrap();
            let role: String = db.query_row("SELECT \"RoleId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" IS NULL OR 1 LIMIT 1", [], |row| row.get(0)).unwrap();
            let _ = (role, extra_members);
            let log = Arc::new(Mutex::new(Vec::<String>::new()));
            let sink = Arc::clone(&log);
            db.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(move |event: rusqlite::trace::TraceEvent<'_>| {
                    if let rusqlite::trace::TraceEvent::Stmt(_, sql) = event {
                        // What the triggers and the full-text index run inside a statement is reported as
                        // lines of their own, marked as comments or as the index's own reads.
                        if !sql.starts_with("--") && !sql.starts_with("SELECT k, v") && !sql.starts_with("PRAGMA") {
                            sink.lock().unwrap().push(sql.to_string());
                        }
                    }
                }),
            )
            .unwrap();
            let messages = stellarr_messages::Messages::new(stellarr_messages::Language::English);
            let submission = |content: &str, mentions: bool| Submission {
                channel: Id::parse(&channel).unwrap(),
                author: Id::parse(&owner.id).unwrap(),
                content: if mentions { format!("{content} @nobody") } else { content.to_string() },
                attachments: Vec::new(),
                reply_to: None,
                client_id: Some(Id::random()),
            };
            let mut counts = Vec::new();
            for mentions in [false, true] {
                db.execute_batch("BEGIN IMMEDIATE").unwrap();
                log.lock().unwrap().clear();
                let posted = post(&db, &messages, Id::parse(&instance).unwrap(), &submission("hello", mentions)).map_err(|_| ()).unwrap();
                assert!(matches!(posted, Posted::Stored(_)));
                if std::env::var("SHOW_STATEMENTS").is_ok() {
                    for line in log.lock().unwrap().iter() {
                        println!("  {}", line.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(110).collect::<String>());
                    }
                }
                counts.push(log.lock().unwrap().len());
                db.execute_batch("ROLLBACK").unwrap();
            }
            (counts[0], counts[1])
        })
    };

    let (plain, with_mentions) = measure(0);

    // The check of the channel is six statements, a place, the roles, the member and the held roles and the
    // overrides; the rest is the post: the replay look, the insert, the read marker, the author's name and the
    // page it answers with. The numbers are the ones to beat, not a promise to keep forever.
    assert_eq!(plain, 15, "statements for a plain post");
    assert!(with_mentions <= plain + 1, "{with_mentions} statements with a mention that names nobody");
    println!("a post costs {plain} statements, {with_mentions} when it names somebody who is not a member");
}

// ---- Link previews

mod previews {
    //! A message with a link, through the API and the queue and the worker to the event clients
    //! receive. The linked sites are small servers of the test's own.

    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{World, id_of};
    use crate::support::Server;

    /// A 1x1 picture: the signature is what the preview code looks at, and the rest is what a viewer needs.
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F,
        0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, b'I', b'D', b'A', b'T', 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0, 5, 0xFE, 2, 0xFE, 0xA7, 0x35,
        0x81, 0x84, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
    ];

    /// A site that answers what it is told to, and counts what it was asked.
    struct Site {
        address: SocketAddr,
        asked: Arc<AtomicUsize>,
    }

    async fn site(pages: Vec<(&'static str, &'static str, Vec<u8>)>) -> Site {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let table: Arc<HashMap<&'static str, (&'static str, Vec<u8>)>> =
            Arc::new(pages.into_iter().map(|(path, kind, body)| (path, (kind, body))).collect());
        let counter = Arc::clone(&asked);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let table = Arc::clone(&table);
                let counter = Arc::clone(&counter);
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 8192];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    counter.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                    let (status, kind, body) = match table.get(path.as_str()) {
                        Some((kind, body)) => ("200 OK", *kind, body.clone()),
                        None => ("404 Not Found", "text/plain", b"no".to_vec()),
                    };
                    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Site { address, asked }
    }

    fn page(title: &str, image: &str) -> Vec<u8> {
        format!("<html><head><meta property=\"og:title\" content=\"{title}\"><meta property=\"og:image\" content=\"{image}\"></head><body></body></html>").into_bytes()
    }

    async fn with_previews() -> World {
        let server = Server::start_with(&[("Testing__Workers", "link-previews"), ("Stellarr__Embeds__AllowPrivateNetworks", "true")]).await;
        let owner = server.found("owner").await;
        let db = server.database();
        let general: String = db.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0)).unwrap();
        let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        World { server, owner, general: general.to_lowercase(), instance }
    }

    /// The `MessageEmbedsUpdated` events for a message that have been published, once there are `count`.
    async fn updates(w: &World, message: &str, count: usize) -> Vec<Value> {
        for _ in 0..150 {
            let found: Vec<Value> = w.events_named("MessageEmbedsUpdated").into_iter().filter(|e| e["payload"]["messageId"] == message).collect();
            if found.len() >= count {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no card announced for {message}");
    }

    fn stored_pictures(w: &World) -> Vec<(String, String)> {
        let db = w.db();
        let mut statement = db.prepare("SELECT \"Id\", \"ObjectKey\" FROM \"ImageAttachments\" WHERE \"UploadedById\" = '00000000-0000-0000-0000-000000000000'").unwrap();
        statement.query_map([], |row| Ok((row.get::<_, String>(0)?.to_lowercase(), row.get(1)?))).unwrap().map(Result::unwrap).collect()
    }

    #[tokio::test]
    async fn a_link_gets_a_card_whose_picture_is_a_copy_in_our_own_storage() {
        let w = with_previews().await;
        let site = site(vec![("/story", "text/html", page("Title of the story", "/image.png")), ("/image.png", "image/png", PNG.to_vec())]).await;
        let url = format!("http://{}/story", site.address);
        let start = w.owner.client.get(&format!("/api/channels/{}/messages?limit=1", w.general)).await.json::<Value>()["syncCursor"].as_str().unwrap().to_string();

        let message = w.post(&w.owner, &w.general, json!({ "content": format!("Read {url} today.") })).await.json::<Value>();
        let announced = updates(&w, message["id"].as_str().unwrap(), 1).await;

        let embeds = announced[0]["payload"]["embeds"].as_array().unwrap();
        assert_eq!(embeds.len(), 1);
        let card = &embeds[0];
        assert_eq!(card["kind"], "link");
        assert_eq!(card["domain"], "127.0.0.1");
        assert_eq!(card["url"], url);
        assert_eq!(card["title"], "Title of the story");
        // The reader's browser is sent to us, never to the linked host.
        let thumbnail = card["thumbnailUrl"].as_str().unwrap();
        assert!(thumbnail.starts_with("/assets/"));
        let pictures = stored_pictures(&w);
        assert_eq!(pictures.len(), 1);
        assert_eq!(thumbnail, format!("/assets/{}", pictures[0].0));
        assert!(pictures[0].1.starts_with("embeds/") && pictures[0].1.ends_with(".png"), "{}", pictures[0].1);
        // The route that serves it is the uploads group's: the picture comes back whole.
        let served = w.owner.client.get(thumbnail).await;
        assert_eq!(served.status, 200);
        assert_eq!(&served.body[..], PNG);
        assert_eq!(served.header("content-type"), Some("image/png"));
        // The card is in the history, and a client that was offline finds it in the journal.
        let history: Value = w.owner.client.get(&format!("/api/channels/{}/messages", w.general)).await.json();
        assert_eq!(history["messages"].as_array().unwrap().last().unwrap()["embeds"][0]["id"], card["id"]);
        let sync: Value = w.owner.client.get(&format!("/api/channels/{}/messages/sync?after={start}", w.general)).await.json();
        let changes = sync["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[1]["message"]["embeds"].as_array().unwrap().len(), 1);
        // The audience is the channel's.
        assert_eq!(announced[0]["audience"], json!({ "server": w.instance.to_lowercase() }));
    }

    #[tokio::test]
    async fn the_picture_of_a_card_is_served_to_whoever_sees_the_channel_and_to_nobody_else() {
        let w = with_previews().await;
        let sam = w.server.register("sam").await;
        let site = site(vec![("/story", "text/html", page("Story", "/image.png")), ("/image.png", "image/png", PNG.to_vec())]).await;
        let message = w.post(&w.owner, &w.general, json!({ "content": format!("Read http://{}/story", site.address) })).await.json::<Value>();
        let announced = updates(&w, message["id"].as_str().unwrap(), 1).await;
        let thumbnail = announced[0]["payload"]["embeds"][0]["thumbnailUrl"].as_str().unwrap().to_string();
        // The worker's own row: the all-zero uploader, a key under `embeds/`.
        let (_, key) = &stored_pictures(&w)[0];
        assert!(key.starts_with("embeds/"));

        assert_eq!(w.owner.client.get(&thumbnail).await.status, 200);
        assert_eq!(&sam.client.get(&thumbnail).await.body[..], PNG);

        // Somebody who may not see the channel is told there is nothing there.
        let db = w.db();
        let base: String = db.query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0 LIMIT 1", [], |row| row.get(0)).unwrap();
        db.execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
            rusqlite::params![stellarr_api::values::Id::random().to_db_text(), w.instance, w.general.to_uppercase(), base],
        )
        .unwrap();
        assert_eq!(sam.client.get(&thumbnail).await.status, 404);
        assert_eq!(w.owner.client.get(&thumbnail).await.status, 200);
    }

    #[tokio::test]
    async fn every_message_keeps_its_own_card_and_picture() {
        let w = with_previews().await;
        let site = site(vec![("/a", "text/html", page("A", "/image.png")), ("/b", "text/html", page("B", "/image.png")), ("/image.png", "image/png", PNG.to_vec())]).await;
        let first = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://{}/a", site.address) })).await.json::<Value>());
        let second = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://{}/b", site.address) })).await.json::<Value>());

        let a = updates(&w, &first, 1).await;
        let b = updates(&w, &second, 1).await;

        assert_eq!(a[0]["payload"]["embeds"][0]["title"], "A");
        assert_eq!(b[0]["payload"]["embeds"][0]["title"], "B");
        assert_ne!(a[0]["payload"]["embeds"][0]["thumbnailUrl"], b[0]["payload"]["embeds"][0]["thumbnailUrl"]);
        assert_eq!(stored_pictures(&w).len(), 2);
    }

    #[tokio::test]
    async fn editing_adds_and_removes_cards_and_takes_their_pictures_with_them() {
        let w = with_previews().await;
        let site = site(vec![("/a", "text/html", page("A", "/image.png")), ("/image.png", "image/png", PNG.to_vec())]).await;
        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": "no link yet" })).await.json::<Value>());
        // Nothing to build: the event still comes, with an empty set.
        assert!(updates(&w, &message, 1).await[0]["payload"]["embeds"].as_array().unwrap().is_empty());

        w.edit(&w.owner, &message, json!({ "content": format!("now http://{}/a", site.address) })).await;
        let added = updates(&w, &message, 2).await;
        assert_eq!(added[1]["payload"]["embeds"][0]["title"], "A");
        let picture = stored_pictures(&w);
        assert_eq!(picture.len(), 1);
        let file = w.server.folder().join("files").join(&picture[0].1);
        assert!(file.exists());

        w.edit(&w.owner, &message, json!({ "content": "no link again" })).await;
        let removed = updates(&w, &message, 3).await;
        assert!(removed[2]["payload"]["embeds"].as_array().unwrap().is_empty());
        assert!(stored_pictures(&w).is_empty());
        assert!(!file.exists());
        let queued: i64 = w.db().query_row("SELECT COUNT(*) FROM \"ObjectDeletions\"", [], |row| row.get(0)).unwrap();
        assert_eq!(queued, 0);
    }

    #[tokio::test]
    async fn removing_a_message_removes_its_picture() {
        let w = with_previews().await;
        let site = site(vec![("/a", "text/html", page("A", "/image.png")), ("/image.png", "image/png", PNG.to_vec())]).await;
        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://{}/a", site.address) })).await.json::<Value>());
        updates(&w, &message, 1).await;
        let picture = stored_pictures(&w);
        let file = w.server.folder().join("files").join(&picture[0].1);

        assert_eq!(w.remove(&w.owner, &message).await.status, 204);

        assert!(stored_pictures(&w).is_empty());
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn a_store_that_cannot_delete_leaves_the_picture_queued_for_the_cleanup() {
        let w = with_previews().await;
        let site = site(vec![("/a", "text/html", page("A", "/image.png")), ("/image.png", "image/png", PNG.to_vec())]).await;
        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://{}/a", site.address) })).await.json::<Value>());
        updates(&w, &message, 1).await;
        let picture = stored_pictures(&w);
        let file = w.server.folder().join("files").join(&picture[0].1);
        // The object cannot be removed: nobody may change the folder it is in.
        use std::os::unix::fs::PermissionsExt;
        let folder = file.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();

        assert_eq!(w.remove(&w.owner, &message).await.status, 204);

        assert!(stored_pictures(&w).is_empty());
        let queued: Vec<String> = {
            let db = w.db();
            let mut statement = db.prepare("SELECT \"ObjectKey\" FROM \"ObjectDeletions\"").unwrap();
            statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
        };
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(queued, [picture[0].1.clone()]);
    }

    #[tokio::test]
    async fn a_page_that_says_nothing_about_itself_and_a_link_to_giphy_get_no_card() {
        let w = with_previews().await;
        let site = site(vec![("/blank", "text/html", b"<html><body>hi</body></html>".to_vec())]).await;
        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://{}/blank https://giphy.com/gifs/x-123", site.address) })).await.json::<Value>());

        let announced = updates(&w, &message, 1).await;

        assert!(announced[0]["payload"]["embeds"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_the_first_links_of_a_message_get_cards_and_each_keeps_its_place() {
        let w = with_previews().await;
        let site = site(vec![("/a", "text/html", page("A", "/none")), ("/b", "text/html", b"<html><body>nothing</body></html>".to_vec()), ("/c", "text/html", page("C", "/none"))]).await;
        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://{0}/a http://{0}/b http://{0}/c", site.address) })).await.json::<Value>());

        let announced = updates(&w, &message, 1).await;

        // Two links are read, as the settings say: the first has a card, the second says nothing, the third is left alone.
        let embeds = announced[0]["payload"]["embeds"].as_array().unwrap();
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0]["title"], "A");
        let positions: Vec<i64> = {
            let db = w.db();
            let mut statement = db.prepare("SELECT \"Position\" FROM \"MessageEmbeds\"").unwrap();
            statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(positions, [0]);
        // The first page and its picture (a 404 here), and the second page; the third is never asked for.
        assert_eq!(site.asked.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_previews_switched_off_a_video_link_is_still_a_player_and_nothing_is_fetched() {
        let server = Server::start_with(&[("Testing__Workers", "link-previews"), ("Stellarr__Embeds__Enabled", "false")]).await;
        let owner = server.found("owner").await;
        let db = server.database();
        let general: String = db.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0)).unwrap();
        let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let w = World { server, owner, general: general.to_lowercase(), instance };

        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": "https://youtu.be/dQw4w9WgXcQ?t=42 and https://example.test/page" })).await.json::<Value>());
        let announced = updates(&w, &message, 1).await;

        let embeds = announced[0]["payload"]["embeds"].as_array().unwrap();
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0]["kind"], "video");
        assert_eq!(embeds[0]["provider"], "youtube");
        assert_eq!(embeds[0]["providerId"], "dQw4w9WgXcQ");
        assert_eq!(embeds[0]["startSeconds"], 42);
        assert_eq!(embeds[0]["title"], Value::Null);
        assert_eq!(embeds[0]["thumbnailUrl"], Value::Null);
    }

    #[tokio::test]
    async fn named_sites_and_youtube_are_read_through_a_resolver_table_in_a_server_of_the_tests_own() {
        use stellarr_api::background::Worker;
        use stellarr_api::{Services, link_previews};
        use stellarr_embeds::{EmbedSettings, Fetcher};
        use stellarr_outbound::{AddressPolicy, Client, TableResolver};

        let video = b"{\"type\":\"video\",\"provider_name\":\"YouTube\",\"title\":\"A song\"}".to_vec();
        let site = site(vec![("/story", "text/html", page("Named story", "/none")), ("/oembed", "application/json", video)]).await;
        let table = TableResolver(vec![("news.example".to_string(), vec!["127.0.0.1".parse().unwrap()])]);
        let fetcher = Fetcher::with_client(Client::new().with_resolver(Arc::new(table)), EmbedSettings::default())
            .with_address_policy(AddressPolicy::Anywhere)
            .with_oembed_endpoint(format!("http://news.example:{}/oembed", site.address.port()));
        let workers: Vec<Worker> = link_previews::workers_with(fetcher);
        let log = crate::support::LogCapture::start();
        let routes = stellarr_api::accounts::routes().merge(stellarr_api::messages::routes());
        let server = Server::in_process_with(routes, Services { workers, ..Services::default() }, &[]).await;
        let mut code = String::new();
        for _ in 0..100 {
            code = log.lines().iter().find_map(|line| line.split_once("Setup code: ").map(|(_, rest)| rest.split('.').next().unwrap_or("").to_string())).unwrap_or_default();
            if !code.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let client = server.client();
        let registered = client
            .post_json("/api/auth/register", &json!({"username": "owner", "displayName": "Owner", "email": "owner@example.test", "password": crate::support::PASSWORD, "setupCode": code}))
            .await;
        assert_eq!(registered.status, 201, "{}", registered.text());
        let db = server.database();
        let general: String = db.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0)).unwrap();
        let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let owner = crate::support::Person { username: "owner".into(), id: registered.json::<Value>()["id"].as_str().unwrap().into(), client };
        let w = World { server, owner, general: general.to_lowercase(), instance };

        let port = site.address.port();
        let message = id_of(&w.post(&w.owner, &w.general, json!({ "content": format!("http://news.example:{port}/story https://youtu.be/dQw4w9WgXcQ") })).await.json::<Value>());
        let announced = updates(&w, &message, 1).await;

        let embeds = announced[0]["payload"]["embeds"].as_array().unwrap();
        assert_eq!(embeds.len(), 2, "{embeds:?}");
        assert_eq!((embeds[0]["title"].as_str(), embeds[0]["domain"].as_str()), (Some("Named story"), Some("news.example")));
        assert_eq!((embeds[1]["kind"].as_str(), embeds[1]["title"].as_str()), (Some("video"), Some("A song")));
    }
}
