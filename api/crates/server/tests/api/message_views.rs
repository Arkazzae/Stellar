//! Reactions, pins, threads, and searching a channel, through the real binary.
//!
//! Messages are written straight into the database, which is how a test gets
//! as many as it needs: the triggers of the schema fill the journal and the
//! search index as they do for the routes that post.

use rusqlite::{Connection, params};
use serde_json::Value;

use crate::support::{Person, Server};

const THUMB: &str = "%F0%9F%91%8D";

struct World {
    server: Server,
    owner: Person,
    maya: Person,
    server_id: String,
    channel: String,
}

async fn world() -> World {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let (server_id, channel): (String, String) = server
        .database()
        .query_row(
            "SELECT \"ServerId\", \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    // The founding left a welcome message; the tests count their own.
    server.database().execute("DELETE FROM \"Messages\"", []).unwrap();
    World { server, owner, maya, server_id, channel: channel.to_lowercase() }
}

fn put(db: &Connection, channel: &str, author: &str, content: &str, at: &str, reply: Option<&str>) -> String {
    let id = stellarr_api::values::Id::random().to_db_text();
    db.execute(
        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"ReplyToMessageId\")
         VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6)",
        params![id, channel.to_uppercase(), author.to_uppercase(), content, at, reply.map(str::to_uppercase)],
    )
    .unwrap();
    id.to_lowercase()
}

fn at(second: u32) -> String {
    format!("2026-01-01 10:00:{second:02}")
}

async fn react(who: &Person, method: &str, message: &str, emoji: &str) -> crate::support::Response {
    let request = crate::support::Request::new(method.parse().unwrap(), &format!("/api/messages/{message}/reactions/{emoji}"));
    who.client.send(request).await
}

fn events_named(w: &World, name: &str) -> Vec<Value> {
    w.server.events().into_iter().filter(|e| e["name"] == name).collect()
}

#[tokio::test]
async fn a_reaction_is_added_once_and_taken_back() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);

    let first = react(&w.maya, "PUT", &message, THUMB).await;
    assert_eq!(first.status, 200, "{}", first.text());
    let body: Value = first.json();
    assert_eq!(body["count"], 1);
    assert_eq!(body["added"], true);
    assert_eq!(body["emoji"], "\u{1F44D}");
    assert_eq!(body["userId"], w.maya.id);
    let again = react(&w.maya, "PUT", &message, THUMB).await;
    assert_eq!(again.json::<Value>()["count"], 1);
    let other = react(&w.owner, "PUT", &message, THUMB).await;
    assert_eq!(other.json::<Value>()["count"], 2);

    let back = react(&w.maya, "DELETE", &message, THUMB).await;
    let back: Value = back.json();
    assert_eq!((back["count"].clone(), back["added"].clone()), (1.into(), false.into()));
    let nothing = react(&w.maya, "DELETE", &message, THUMB).await;
    assert_eq!(nothing.status, 200);
    // Every call tells the channel how things stand, whether or not it changed them.
    assert_eq!(events_named(&w, "MessageReactionChanged").len(), 5);
    let rows: i64 = w.server.database().query_row("SELECT COUNT(*) FROM \"MessageReactions\"", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn the_same_reaction_eight_times_at_once_is_one_row() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);
    let (a, b, c, d, e, f, g, h) = tokio::join!(
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
        react(&w.maya, "PUT", &message, THUMB),
    );
    for response in [a, b, c, d, e, f, g, h] {
        assert_eq!(response.status, 200);
        assert_eq!(response.json::<Value>()["count"], 1);
    }
    let rows: i64 = w.server.database().query_row("SELECT COUNT(*) FROM \"MessageReactions\"", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn an_emoji_that_is_not_one_word_is_refused_and_odd_ones_are_kept_as_written() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);
    for bad in ["a%20b", "%E3%80%80", "%C2%A0", "%0A", &"x".repeat(65), &"%F0%9F%98%80".repeat(33)] {
        let response = react(&w.maya, "PUT", &message, bad).await;
        assert_eq!(response.status, 400, "{bad}");
        assert!(response.json::<Value>()["errors"]["emoji"][0].as_str().unwrap().contains("64"));
    }
    for (sent, kept) in [
        ("%F0%9F%8F%B3%EF%B8%8F%E2%80%8D%F0%9F%8C%88", "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}"),
        ("%2F", "%2F"),
        ("%FF", "%FF"),
        ("%C3%28", "%C3("),
        ("%20x%20", "x"),
        ("%3Cblob%3E", "<blob>"),
        ("+", "+"),
    ] {
        let response = react(&w.maya, "PUT", &message, sent).await;
        assert_eq!(response.status, 200, "{sent}");
        assert_eq!(response.json::<Value>()["emoji"], kept, "{sent}");
    }
    assert_eq!(react(&w.maya, "PUT", &message, "%00").await.status, 400);
    assert_eq!(react(&w.maya, "PUT", "x", "").await.status, 404);
}

#[tokio::test]
async fn a_reaction_to_a_message_that_is_not_there_is_not_found() {
    let w = world().await;
    let unknown = "00000000-0000-4000-8000-000000000000";
    assert_eq!(react(&w.maya, "PUT", unknown, THUMB).await.status, 404);
    assert_eq!(react(&w.maya, "PUT", "not-an-id", THUMB).await.status, 404);
}

fn deny_member(w: &World, who: &Person, channel: &str, mask: i64) {
    w.server
        .database()
        .execute(
            "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\")
             VALUES (?1, ?2, 0, ?3, ?4, 0, ?5)",
            params![
                stellarr_api::values::Id::random().to_db_text(),
                w.server_id,
                channel.to_uppercase(),
                who.id.to_uppercase(),
                mask
            ],
        )
        .unwrap();
}

#[tokio::test]
async fn adding_needs_the_right_but_taking_back_does_not() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);
    assert_eq!(react(&w.maya, "PUT", &message, THUMB).await.status, 200);
    deny_member(&w, &w.maya, &w.channel, 1 << 11);

    let denied = react(&w.maya, "PUT", &message, "%F0%9F%8E%89").await;
    assert_eq!(denied.status, 403);
    assert_eq!(denied.json::<Value>()["permission"], "addReactions");
    let back = react(&w.maya, "DELETE", &message, THUMB).await;
    assert_eq!(back.status, 200);
    assert_eq!(back.json::<Value>()["count"], 0);
}

#[tokio::test]
async fn a_channel_that_is_not_seen_does_not_exist_for_reactions_or_pins() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);
    deny_member(&w, &w.maya, &w.channel, 1 << 14);
    for method in ["PUT", "DELETE"] {
        assert_eq!(react(&w.maya, method, &message, THUMB).await.status, 404);
    }
    for path in [format!("/api/channels/{}/pins", w.channel), format!("/api/channels/{}/threads", w.channel), format!("/api/channels/{}/messages/index", w.channel), format!("/api/channels/{}/messages/search?query=hello", w.channel), format!("/api/channels/{}/threads/{message}", w.channel)] {
        assert_eq!(w.maya.client.get(&path).await.status, 404, "{path}");
    }
    let pin = crate::support::Request::new("PUT".parse().unwrap(), &format!("/api/messages/{message}/pin"));
    assert_eq!(w.maya.client.send(pin).await.status, 404);
}

#[tokio::test]
async fn a_reaction_is_told_to_the_people_who_see_the_channel() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);
    react(&w.maya, "PUT", &message, THUMB).await;
    let first = &events_named(&w, "MessageReactionChanged")[0];
    assert_eq!(first["audience"]["server"], w.server_id.to_lowercase());

    deny_member(&w, &w.maya, &w.channel, 1 << 14);
    react(&w.owner, "PUT", &message, THUMB).await;
    let second = &events_named(&w, "MessageReactionChanged")[1];
    let users = second["audience"]["followers"]["users"].as_array().unwrap();
    assert!(users.iter().any(|u| *u == w.owner.id));
    assert!(!users.iter().any(|u| *u == w.maya.id));
}

async fn pin(who: &Person, method: &str, message: &str) -> crate::support::Response {
    who.client
        .send(crate::support::Request::new(method.parse().unwrap(), &format!("/api/messages/{message}/pin")))
        .await
}

#[tokio::test]
async fn pinning_is_for_those_who_manage_messages_and_is_told_to_the_channel() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "keep", &at(1), None);
    assert_eq!(pin(&w.maya, "PUT", &message).await.status, 403);

    let pinned = pin(&w.owner, "PUT", &message).await;
    assert_eq!(pinned.status, 200);
    let first_pin = pinned.json::<Value>()["pinnedAt"].clone();
    assert!(first_pin.is_string());
    // A second pin leaves the first where it is.
    let again = pin(&w.owner, "PUT", &message).await;
    assert_eq!(again.json::<Value>()["pinnedAt"], first_pin);

    let list = w.maya.client.get(&format!("/api/channels/{}/pins", w.channel)).await;
    assert_eq!(list.json::<Value>()["messages"][0]["id"], message);
    assert_eq!(events_named(&w, "MessageUpdated").len(), 2);
    let journal: i64 = w.server.database().query_row("SELECT COUNT(*) FROM \"MessageChanges\" WHERE \"MessageId\" = ?1", [message.to_uppercase()], |r| r.get(0)).unwrap();
    // The post, and the one pin: the second pin changed nothing.
    assert_eq!(journal, 2);

    let unpinned = pin(&w.owner, "DELETE", &message).await;
    assert!(unpinned.json::<Value>()["pinnedAt"].is_null());
    assert_eq!(pin(&w.owner, "DELETE", &message).await.status, 200);
    assert_eq!(pin(&w.owner, "PUT", "00000000-0000-4000-8000-000000000000").await.status, 404);
}

#[tokio::test]
async fn a_channel_holds_fifty_pins_at_most() {
    let w = world().await;
    let db = w.server.database();
    let mut ids = Vec::new();
    for n in 0..51 {
        ids.push(put(&db, &w.channel, &w.owner.id, &format!("m{n}"), &at(n % 60), None));
    }
    for id in &ids[..50] {
        assert_eq!(pin(&w.owner, "PUT", id).await.status, 200);
    }
    let full = pin(&w.owner, "PUT", &ids[50]).await;
    assert_eq!(full.status, 400);
    assert!(full.json::<Value>()["errors"]["pins"][0].as_str().unwrap().contains("50"));
    // One already pinned is not a new pin.
    assert_eq!(pin(&w.owner, "PUT", &ids[0]).await.status, 200);
    assert_eq!(pin(&w.owner, "DELETE", &ids[0]).await.status, 200);
    assert_eq!(pin(&w.owner, "PUT", &ids[50]).await.status, 200);
}

#[tokio::test]
async fn the_pins_list_the_newest_pin_first_and_stops_at_fifty() {
    let w = world().await;
    let db = w.server.database();
    for n in 0..53 {
        let id = put(&db, &w.channel, &w.owner.id, &format!("m{n}"), &at(n % 60), None);
        db.execute("UPDATE \"Messages\" SET \"PinnedAt\" = ?2 WHERE \"Id\" = ?1", params![id.to_uppercase(), format!("2026-02-01 10:{:02}:00", n)]).unwrap();
    }
    let list: Value = w.maya.client.get(&format!("/api/channels/{}/pins", w.channel)).await.json();
    let messages = list["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 50);
    assert_eq!(messages[0]["content"], "m52");
    assert_eq!(messages[49]["content"], "m3");
}

#[tokio::test]
async fn a_message_answers_with_its_reactions_flagged_for_the_viewer() {
    let w = world().await;
    let message = put(&w.server.database(), &w.channel, &w.owner.id, "hello", &at(1), None);
    react(&w.maya, "PUT", &message, THUMB).await;
    react(&w.owner, "PUT", &message, "%F0%9F%8E%89").await;
    react(&w.owner, "PUT", &message, THUMB).await;
    pin(&w.owner, "PUT", &message).await;
    let list: Value = w.maya.client.get(&format!("/api/channels/{}/pins", w.channel)).await.json();
    let reactions = &list["messages"][0]["reactions"];
    assert_eq!(reactions[0]["emoji"], "\u{1F44D}");
    assert_eq!(reactions[0]["count"], 2);
    assert_eq!(reactions[0]["mine"], true);
    assert_eq!(reactions[1]["mine"], false);
    // The event is for everybody, so it flags nothing.
    let updated = &events_named(&w, "MessageUpdated")[0];
    assert!(updated["payload"]["reactions"].as_array().unwrap().iter().all(|r| r["mine"] == false));
    // The members of the message, in the order the clients read them.
    let raw = w.maya.client.get(&format!("/api/channels/{}/pins", w.channel)).await.text();
    let names = ["\"id\"", "\"channelId\"", "\"author\"", "\"content\"", "\"createdAt\"", "\"attachments\"", "\"embeds\"", "\"editedAt\"", "\"pinnedAt\"", "\"replyTo\"", "\"reactions\""];
    let at: Vec<usize> = names.iter().map(|n| raw.find(n).unwrap()).collect();
    assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{raw}");
}

#[tokio::test]
async fn threads_are_listed_by_their_last_reply_and_opened_whole() {
    let w = world().await;
    let db = w.server.database();
    let (a, b) = (put(&db, &w.channel, &w.owner.id, "root a", &at(1), None), put(&db, &w.channel, &w.owner.id, "root   b\n with   spaces", &at(2), None));
    put(&db, &w.channel, &w.maya.id, "r1", &at(3), Some(&a));
    put(&db, &w.channel, &w.maya.id, "r2", &at(5), Some(&b));
    put(&db, &w.channel, &w.owner.id, "r3", &at(7), Some(&a));
    let path = format!("/api/channels/{}/threads", w.channel);

    let list: Value = w.maya.client.get(&path).await.json();
    assert_eq!(list[0]["root"]["id"], a);
    assert_eq!((list[0]["replyCount"].clone(), list[1]["replyCount"].clone()), (2.into(), 1.into()));
    assert_eq!(list[1]["root"]["preview"], "root b with spaces");
    assert_eq!(list[0]["channelId"], w.channel);

    let thread: Value = w.maya.client.get(&format!("{path}/{a}")).await.json();
    let contents: Vec<&str> = thread["messages"].as_array().unwrap().iter().map(|m| m["content"].as_str().unwrap()).collect();
    assert_eq!(contents, ["root a", "r1", "r3"]);
    assert_eq!(thread["messages"][1]["replyTo"]["authorName"], "Owner");
    let reply = thread["messages"][1]["id"].as_str().unwrap();
    // A message that nobody answered is a thread of one.
    let alone: Value = w.maya.client.get(&format!("{path}/{reply}")).await.json();
    assert_eq!(alone["messages"].as_array().unwrap().len(), 1);
    assert_eq!(w.maya.client.get(&format!("{path}/00000000-0000-4000-8000-000000000000")).await.status, 404);
}

#[tokio::test]
async fn the_index_pages_newest_first_to_its_end() {
    let w = world().await;
    let db = w.server.database();
    let ids: Vec<String> = (0..7).map(|n| put(&db, &w.channel, &w.owner.id, &format!("message {n} https://example.com/{n}"), &at(n), None)).collect();
    let base = format!("/api/channels/{}/messages/index?limit=3", w.channel);
    let mut seen = Vec::new();
    let mut cursor = String::new();
    loop {
        let page: Value = w.maya.client.get(&format!("{base}{cursor}")).await.json();
        for m in page["messages"].as_array().unwrap() {
            seen.push(m["id"].as_str().unwrap().to_string());
        }
        match page["nextCursor"].as_str() {
            Some(next) => cursor = format!("&cursor={next}"),
            None => break,
        }
    }
    let mut expected = ids.clone();
    expected.reverse();
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn messages_that_arrive_while_paging_do_not_move_the_pages() {
    let w = world().await;
    let db = w.server.database();
    for n in 0..4 {
        put(&db, &w.channel, &w.owner.id, &format!("old {n}"), &at(n), None);
    }
    let base = format!("/api/channels/{}/messages/index?limit=2", w.channel);
    let first: Value = w.maya.client.get(&base).await.json();
    put(&db, &w.channel, &w.owner.id, "new", "2999-01-01 00:00:00", None);
    let second: Value = w.maya.client.get(&format!("{base}&cursor={}", first["nextCursor"].as_str().unwrap())).await.json();
    assert_eq!(second["messages"][0]["content"], "old 1");
    assert!(second["nextCursor"].is_null());
}

#[tokio::test]
async fn the_index_can_be_narrowed_to_media_links_or_a_text() {
    let w = world().await;
    let db = w.server.database();
    put(&db, &w.channel, &w.owner.id, "plain", &at(1), None);
    let link = put(&db, &w.channel, &w.owner.id, "see https://example.com/x", &at(2), None);
    let media = put(&db, &w.channel, &w.owner.id, "with a file", &at(3), None);
    let attachment = stellarr_api::values::Id::random().to_db_text();
    db.execute(
        "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"IsVoiceMessage\", \"IsEncrypted\", \"CreatedAt\")
         VALUES (?1, ?2, 'k', 'zdj\u{119}cie.png', 'image/png', 10, 0, 0, ?3)",
        params![attachment, w.owner.id.to_uppercase(), at(3)],
    )
    .unwrap();
    db.execute("INSERT INTO \"MessageAttachments\" (\"MessageId\", \"AttachmentId\") VALUES (?1, ?2)", params![media.to_uppercase(), attachment]).unwrap();
    let ids = |value: Value| -> Vec<String> { value["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect() };
    let base = format!("/api/channels/{}/messages/index", w.channel);
    assert_eq!(ids(w.maya.client.get(&format!("{base}?kind=Media")).await.json()), std::slice::from_ref(&media));
    assert_eq!(ids(w.maya.client.get(&format!("{base}?kind=links")).await.json()), std::slice::from_ref(&link));
    assert_eq!(ids(w.maya.client.get(&format!("{base}?query=zdjecie")).await.json()), std::slice::from_ref(&media));
    let attachments = w.maya.client.get(&format!("{base}?kind=media")).await.json::<Value>()["messages"][0]["attachments"].clone();
    assert_eq!(attachments[0]["url"], format!("/assets/{}", attachment.to_lowercase()));
}

#[tokio::test]
async fn the_index_refuses_what_it_cannot_read() {
    let w = world().await;
    let base = format!("/api/channels/{}/messages/index", w.channel);
    let status = |path: String| {
        let client = &w.maya.client;
        async move { client.get(&path).await }
    };
    for (query, field) in [("limit=0", "limit"), ("limit=101", "limit"), ("kind=videos", "kind"), ("query=a", "query"), ("cursor=abc", "cursor"), ("cursor=", "cursor")] {
        let response = status(format!("{base}?{query}")).await;
        assert_eq!(response.status, 400, "{query}");
        assert!(response.json::<Value>()["errors"][field].is_array(), "{query}");
    }
    // A limit that is no number is refused before anything else is looked at.
    for query in ["limit=abc", "limit=", "limit=5&limit=6"] {
        let response = status(format!("{base}?{query}")).await;
        assert_eq!((response.status.as_u16(), response.body.len()), (400, 0), "{query}");
    }
    assert_eq!(status(format!("{base}?limit=%205")).await.status, 200);
    assert_eq!(status(format!("{base}?limit=%2B5&KIND=ALL")).await.status, 200);
}

#[tokio::test]
async fn a_cursor_belongs_to_one_channel_kind_and_text() {
    let w = world().await;
    let db = w.server.database();
    for n in 0..6 {
        put(&db, &w.channel, &w.owner.id, &format!("lampa {n}"), &at(n), None);
    }
    let base = format!("/api/channels/{}/messages/index?limit=2", w.channel);
    let page: Value = w.maya.client.get(&base).await.json();
    let cursor = page["nextCursor"].as_str().unwrap();
    assert_eq!(w.maya.client.get(&format!("{base}&cursor={cursor}")).await.status, 200);
    assert_eq!(w.maya.client.get(&format!("{base}&kind=media&cursor={cursor}")).await.status, 400);
    assert_eq!(w.maya.client.get(&format!("{base}&query=lampa&cursor={cursor}")).await.status, 400);
    // One that is padded the way a stricter reader would have it still reads.
    assert_eq!(w.maya.client.get(&format!("{base}&cursor={cursor}%3D%3D")).await.status, 200);
}

#[tokio::test]
async fn a_search_finds_prefixes_diacritics_and_the_polish_l() {
    let w = world().await;
    let db = w.server.database();
    let lamp = put(&db, &w.channel, &w.owner.id, "Nowa lampa na biurku", &at(1), None);
    let lodz = put(&db, &w.channel, &w.owner.id, "Spotkanie w \u{141}odzi", &at(2), None);
    let ciastko = put(&db, &w.channel, &w.owner.id, "za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}", &at(3), None);
    let find = |q: &str| {
        let path = format!("/api/channels/{}/messages/search?query={q}", w.channel);
        let client = &w.maya.client;
        async move {
            let value: Value = client.get(&path).await.json();
            value.as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        }
    };
    assert_eq!(find("lamp").await, std::slice::from_ref(&lamp));
    assert_eq!(find("%C5%81AMP").await, std::slice::from_ref(&lamp));
    assert_eq!(find("lodzi").await, std::slice::from_ref(&lodz));
    assert_eq!(find("%C5%81odzi").await, std::slice::from_ref(&lodz));
    assert_eq!(find("zazolc%20gesla").await.len(), 1);
    assert_eq!(find("za%C5%BC%C3%B3%C5%82%C4%87%20jazn").await.len(), 1);
    assert_eq!(find("owner").await.len(), 3);
    let _ = ciastko;
}

#[tokio::test]
async fn nothing_typed_in_a_search_is_taken_for_syntax() {
    let w = world().await;
    let db = w.server.database();
    put(&db, &w.channel, &w.owner.id, "cat dog NOT bird OR fish", &at(1), None);
    for q in ["%22", "*", "%22%2A%3A%28%29-", "cat%22%20OR%20%22", "NEAR%28cat%20dog%29", "cat%20AND", "-cat", "col%3Acat", "%5Ecat", "%7Bcat%7D", "cat%27%3B%20DROP%20TABLE%20Messages%3B--", "%00cat", "%F0%9F%98%80%20cat", "a%22b", "NOT", "OR%20OR", "cat%20-%20dog"] {
        let response = w.maya.client.get(&format!("/api/channels/{}/messages/search?query={q}", w.channel)).await;
        assert!(response.status == 200 || response.status == 400, "{q}: {}", response.status);
        let index = w.maya.client.get(&format!("/api/channels/{}/messages/index?query={q}", w.channel)).await;
        assert!(index.status == 200 || index.status == 400, "{q}: {}", index.status);
    }
    let count = |q: &str| {
        let path = format!("/api/channels/{}/messages/search?query={q}", w.channel);
        let client = &w.maya.client;
        async move { client.get(&path).await.json::<Value>().as_array().unwrap().len() }
    };
    assert_eq!(count("cat%20NOT").await, 1);
    assert_eq!(count("cat%20OR%20zebra").await, 0);
    assert_eq!(count("%22cat%22").await, 1);
}

#[tokio::test]
async fn a_search_needs_two_to_two_hundred_characters() {
    let w = world().await;
    let path = |q: &str| format!("/api/channels/{}/messages/search{q}", w.channel);
    for q in ["", "?query=", "?query=a", "?query=%20a%20", &format!("?query={}", "a".repeat(201))] {
        let response = w.maya.client.get(&path(q)).await;
        assert_eq!(response.status, 400, "{q}");
        assert!(response.json::<Value>()["errors"]["query"][0].as_str().unwrap().contains("200"));
    }
    assert_eq!(w.maya.client.get(&path(&format!("?query={}", "a".repeat(200)))).await.status, 200);
    // A name given twice has its values joined with a comma.
    assert_eq!(w.maya.client.get(&path("?query=a&query=b")).await.status, 200);
}

#[tokio::test]
async fn every_route_wants_a_session_and_a_membership() {
    let w = world().await;
    let anonymous = w.server.anonymous();
    for path in [format!("/api/channels/{}/pins", w.channel), format!("/api/channels/{}/messages/index", w.channel)] {
        assert_eq!(anonymous.get(&path).await.status, 401);
    }
    let outsider = w.server.register("outsider").await;
    w.server.database().execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [outsider.id.to_uppercase()]).unwrap();
    assert_eq!(outsider.client.get(&format!("/api/channels/{}/pins", w.channel)).await.status, 403);
}
