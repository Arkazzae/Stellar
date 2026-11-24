//! The custom emoji of a server: the list, adding one, deleting one and the
//! image.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Server};
use crate::uploads::{Form, JPEG, PDF, PNG, WAVE, WEBP, first_server, objects, started};
use crate::object_stores::fake_s3::{ACCESS_KEY, FakeS3, SECRET_KEY};

const GIF: &[u8] = b"GIF89a....";

fn form(code: &str, bytes: &[u8], content_type: &str) -> Request {
    Form::new()
        .text("shortcode", code)
        .file("file", "e.png", content_type, bytes)
        .post("/api/emoji")
}

async fn add(person: &Person, code: &str) -> crate::support::Response {
    person.client.send(form(code, PNG, "image/png")).await
}

/// What an audit entry says: the action, the line, what it was done to (its type, id and name) and the changes.
type AuditRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>);

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn events(server: &Server, name: &str) -> Vec<Value> {
    server.events().into_iter().filter(|event| event["name"] == name).collect()
}

/// A second member, who may not manage anything.
async fn with_member() -> (Server, Person, Person) {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let jo = server.register("joe").await;
    (server, maja, jo)
}

#[tokio::test]
async fn the_owner_adds_and_removes_an_emoji_that_every_member_reads() {
    let (server, maja, jo) = with_member().await;

    let refused = add(&jo, "mushris").await;
    let created = add(&maja, ":mushris:").await;

    assert_eq!(refused.status, 403);
    assert_eq!(created.status, 201, "{}", created.text());
    let emoji: Value = created.json();
    let id = emoji["id"].as_str().unwrap();
    let glyph = emoji["glyph"].as_str().unwrap();
    assert_eq!(emoji["shortcode"], ":mushris:");
    assert_eq!(created.header("location"), Some(format!("/api/emoji/{id}").as_str()));
    assert_eq!(glyph, format!("/api/emoji/{id}/image?serverId={}", first_server(&server.database())));
    assert_eq!(created.text(), format!("{{\"id\":\"{id}\",\"shortcode\":\":mushris:\",\"glyph\":\"{glyph}\"}}"));
    let list = jo.client.get("/api/emoji").await;
    assert_eq!(list.json::<Value>(), json!([emoji]));
    let image = jo.client.get(glyph).await;
    assert_eq!((image.status.as_u16(), image.body.as_ref()), (200, PNG));
    assert_eq!(add(&maja, "mushris").await.status, 409);
    assert_eq!(jo.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await.status, 403);
    let key: String = server
        .database()
        .query_row("SELECT \"ObjectKey\" FROM \"ServerEmoji\"", [], |row| row.get(0))
        .unwrap();
    assert!(key.starts_with("emoji/") && key.ends_with(".png") && !key[6..].contains('-'), "{key}");

    let removed = maja.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await;

    assert_eq!(removed.status, 204);
    assert!(removed.body.is_empty());
    assert_eq!(jo.client.get("/api/emoji").await.json::<Value>(), json!([]));
    assert_eq!(jo.client.get(glyph).await.status, 404);
    // The row's own trigger queued the image, and nothing else.
    let queued: Vec<String> = {
        let db = server.database();
        let mut statement = db.prepare("SELECT \"ObjectKey\" FROM \"ObjectDeletions\"").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(queued, [key]);
    let again = maja.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await;
    assert_eq!(again.status, 404);
}

#[tokio::test]
async fn the_list_is_in_the_order_they_were_added_and_by_name_within_one_moment() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    for code in ["bravo", "alpha", "charlie"] {
        assert_eq!(add(&maja, code).await.status, 201);
    }
    let db = server.database();
    db.execute("UPDATE \"ServerEmoji\" SET \"CreatedAt\" = '2026-01-01 00:00:00' WHERE \"Shortcode\" IN ('bravo', 'alpha')", []).unwrap();
    db.execute("UPDATE \"ServerEmoji\" SET \"CreatedAt\" = '2025-01-01 00:00:00' WHERE \"Shortcode\" = 'charlie'", []).unwrap();

    let list: Vec<Value> = maja.client.get("/api/emoji").await.json();

    let codes: Vec<&str> = list.iter().map(|emoji| emoji["shortcode"].as_str().unwrap()).collect();
    assert_eq!(codes, [":charlie:", ":alpha:", ":bravo:"]);
}

#[tokio::test]
async fn a_name_is_trimmed_unwrapped_and_lower_cased_and_must_then_be_a_code() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let long = "x".repeat(32);

    for (typed, kept) in [
        ("mushris", Some("mushris")),
        (" :Mushris2: ", Some("mushris2")),
        ("::ab::", Some("ab")),
        ("a_1", Some("a_1")),
        (long.as_str(), Some(long.as_str())),
        // The first server's pattern let one line break stand last, and kept it.
        (":ab\n:", Some("ab\n")),
        // Kelvin, which lower-cases to a plain k.
        ("\u{212a}\u{212a}", Some("kk")),
        ("\u{3000}spaced\u{a0}", Some("spaced")),
        ("a", None),
        ("", None),
        ("   ", None),
        ("a-b", None),
        ("a b", None),
        ("ab:cd", None),
        (":ab\n\n:", None),
        (":\nab:", None),
        ("\u{130}ab", None),
        ("ab\u{e9}", None),
        ("a\0b", None),
    ] {
        let response = add(&maja, typed).await;
        match kept {
            Some(code) => {
                assert_eq!(response.status, 201, "{typed:?}: {}", response.text());
                assert_eq!(response.json::<Value>()["shortcode"], format!(":{code}:"), "{typed:?}");
            }
            None => {
                assert_eq!(response.status, 400, "{typed:?}");
                assert_eq!(
                    response.json::<Value>()["errors"]["shortcode"][0],
                    "The name must be 2\u{2013}32 characters: lowercase letters, digits or underscores.",
                    "{typed:?}"
                );
            }
        }
    }
    let too_long = add(&maja, &"x".repeat(33)).await;
    assert_eq!(too_long.status, 400);
    assert_eq!(too_long.header("content-type"), Some("application/problem+json"));
}

#[tokio::test]
async fn a_name_taken_is_taken_in_any_letter_case_and_with_its_colons() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    assert_eq!(add(&maja, "mushris").await.status, 201);

    for typed in ["mushris", "MUSHRIS", ":Mushris:", " mushris "] {
        let response = add(&maja, typed).await;
        assert_eq!(response.status, 409, "{typed}");
        assert!(response.body.is_empty());
    }
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerEmoji\""), 1);
    assert_eq!(objects(&server).len(), 1);
}

#[tokio::test]
async fn the_refusals_come_in_the_order_the_first_server_gave_them() {
    let (server, maja, jo) = with_member().await;
    assert_eq!(add(&maja, "taken").await.status, 201);
    async fn send(person: &Person, code: &str, bytes: &[u8], content_type: &str) -> u16 {
        person.client.send(form(code, bytes, content_type)).await.status.as_u16()
    }

    // Who may comes before the name, the name before the file, the file's
    // type and size before whether the name is taken, and that before what
    // the bytes are.
    assert_eq!(send(&jo, "!", b"", "text/plain").await, 403);
    assert_eq!(send(&maja, "!", b"", "text/plain").await, 400);
    assert_eq!(send(&maja, "taken", b"", "image/png").await, 413);
    assert_eq!(send(&maja, "taken", b"x", "text/plain").await, 415);
    assert_eq!(send(&maja, "taken", &vec![b'x'; 512 * 1024 + 1], "image/png").await, 413);
    assert_eq!(send(&maja, "taken", b"not a png", "image/png").await, 409);
    assert_eq!(send(&maja, "fresh", b"not a png", "image/png").await, 415);
    assert_eq!(objects(&server).len(), 1, "nothing was stored by a refusal");
}

#[tokio::test]
async fn a_file_that_is_refused_says_so_in_the_first_servers_words() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let problem = |response: &crate::support::Response| {
        let body: Value = response.json();
        (response.status.as_u16(), body["title"].as_str().unwrap().to_string(), body["detail"].as_str().unwrap().to_string())
    };

    let empty = maja.client.send(form("empty", b"", "image/png")).await;
    let huge = maja.client.send(form("huge", &vec![0; 512 * 1024 + 1], "image/png")).await;
    let text = maja.client.send(form("text", b"hello", "text/plain")).await;
    let svg = maja.client.send(form("svg", b"<svg/>", "image/svg+xml")).await;
    let lie = maja.client.send(form("lie", b"\x00\x01\x02\x03wrong", "image/png")).await;

    let too_large = (413, "The emoji is too large".to_string(), "The maximum size of an emoji file is 512 KB.".to_string());
    assert_eq!(problem(&empty), too_large);
    assert_eq!(problem(&huge), too_large);
    let unsupported = (415, "Unsupported emoji format".to_string(), "Only JPEG, PNG, GIF and WebP images are allowed.".to_string());
    assert_eq!(problem(&text), unsupported);
    assert_eq!(problem(&svg), unsupported);
    assert_eq!(
        problem(&lie),
        (415, "Invalid emoji format".to_string(), "The content of the file does not match its declared type.".to_string())
    );
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerEmoji\""), 0);
}

#[tokio::test]
async fn every_image_type_is_taken_with_its_own_bytes_and_refused_with_another_types() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let mut kept = Vec::new();

    for (index, (content_type, bytes, extension)) in [
        ("image/png", PNG, ".png"),
        ("image/jpeg", JPEG, ".jpg"),
        ("image/gif", GIF, ".gif"),
        ("image/webp", WEBP, ".webp"),
        ("IMAGE/PNG; charset=x", PNG, ".png"),
    ]
    .into_iter()
    .enumerate()
    {
        let response = maja.client.send(form(&format!("kind{index}"), bytes, content_type)).await;
        assert_eq!(response.status, 201, "{content_type}: {}", response.text());
        let id = response.json::<Value>()["id"].as_str().unwrap().to_string();
        let (key, stored_type): (String, String) = server
            .database()
            .query_row(
                "SELECT \"ObjectKey\", \"ContentType\" FROM \"ServerEmoji\" WHERE \"Id\" = ?1",
                [Id::parse(&id).unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(key.ends_with(extension), "{key}");
        kept.push((id, stored_type));
    }
    assert_eq!(
        kept.iter().map(|(_, content_type)| content_type.as_str()).collect::<Vec<_>>(),
        ["image/png", "image/jpeg", "image/gif", "image/webp", "image/png"]
    );
    for (content_type, bytes) in [("image/png", JPEG), ("image/jpeg", PNG), ("image/gif", WEBP), ("image/webp", GIF)] {
        let response = maja.client.send(form("wrong", bytes, content_type)).await;
        assert_eq!(response.status, 415, "{content_type}");
        assert_eq!(response.json::<Value>()["title"], "Invalid emoji format");
    }
    for (content_type, bytes) in [("application/pdf", PDF), ("audio/wav", WAVE), ("video/mp4", crate::uploads::MP4), ("image/bmp", b"BM....")] {
        let response = maja.client.send(form("other", bytes, content_type)).await;
        assert_eq!(response.status, 415, "{content_type}");
        assert_eq!(response.json::<Value>()["title"], "Unsupported emoji format", "{content_type}");
    }
    let untyped = maja
        .client
        .send(Form::new().text("shortcode", "untyped").part(b"Content-Disposition: form-data; name=\"file\"; filename=\"e.png\"", PNG).post("/api/emoji"))
        .await;
    assert_eq!(untyped.status, 415);
}

#[tokio::test]
async fn a_file_of_exactly_the_limit_is_taken_and_one_byte_more_is_not() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sized = |size: usize| [PNG, &vec![b'x'; size - PNG.len()][..]].concat();

    let at = maja.client.send(form("atlimit", &sized(512 * 1024), "image/png")).await;
    let over = maja.client.send(form("overlimit", &sized(512 * 1024 + 1), "image/png")).await;

    assert_eq!((at.status.as_u16(), over.status.as_u16()), (201, 413));
    let size: i64 = server.database().query_row("SELECT \"SizeBytes\" FROM \"ServerEmoji\"", [], |row| row.get(0)).unwrap();
    assert_eq!(size, 512 * 1024);
    assert_eq!(objects(&server).len(), 1);
}

#[tokio::test]
async fn eight_uploads_of_one_name_at_once_leave_one_emoji_and_one_object() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let at_once = || maja.client.send(form("race", PNG, "image/png"));
    let answers = tokio::join!(at_once(), at_once(), at_once(), at_once(), at_once(), at_once(), at_once(), at_once());
    let answers = [answers.0, answers.1, answers.2, answers.3, answers.4, answers.5, answers.6, answers.7];

    let mut statuses: Vec<u16> = answers.iter().map(|answer| answer.status.as_u16()).collect();
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 409, 409, 409, 409, 409, 409, 409]);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerEmoji\""), 1);
    assert_eq!(objects(&server).len(), 1, "the objects of the uploads that lost were taken back");
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerAuditEntries\" WHERE \"Action\" = 'emoji.create'"), 1);
    assert_eq!(events(&server, "EmojiChanged").len(), 1);
}

#[tokio::test]
async fn a_form_that_cannot_be_read_is_a_bad_request_and_another_kind_of_body_is_not_a_form() {
    let (_server, maja, jo) = with_member().await;

    let no_name = maja.client.send(Form::new().file("file", "e.png", "image/png", PNG).post("/api/emoji")).await;
    let no_file = maja.client.send(Form::new().text("shortcode", "nofile").post("/api/emoji")).await;
    let file_as_text = maja.client.send(Form::new().text("shortcode", "text").text("file", "x").post("/api/emoji")).await;
    let nothing = maja.client.send(Request::new(Method::POST, "/api/emoji")).await;
    let json = maja.client.send(Request::new(Method::POST, "/api/emoji").json(&json!({ "shortcode": "x" }))).await;
    let member_with_no_name = jo.client.send(Form::new().file("file", "e.png", "image/png", PNG).post("/api/emoji")).await;
    let named_twice = maja
        .client
        .send(Form::new().text("shortcode", "first").text("shortcode", "second").file("file", "e.png", "image/png", PNG).post("/api/emoji"))
        .await;
    let any_case = maja
        .client
        .send(Form::new().text("SHORTCODE", "upper").file("FILE", "e.png", "image/png", PNG).post("/api/emoji"))
        .await;

    assert_eq!(
        [no_name.status.as_u16(), no_file.status.as_u16(), file_as_text.status.as_u16(), nothing.status.as_u16(), json.status.as_u16(), member_with_no_name.status.as_u16()],
        [400, 400, 400, 400, 415, 400],
        "the form is read before anybody is asked whether they may"
    );
    // Two values of a field are one, joined by a comma, which is no name.
    assert_eq!(named_twice.status, 400);
    assert_eq!(any_case.status, 201, "{}", any_case.text());
}

#[tokio::test]
async fn a_person_who_is_no_member_is_told_so_before_their_form_is_read() {
    let (_server, maja, _jo) = with_member().await;
    let elsewhere = "00000000-0000-0000-0000-0000000000aa";

    let list = maja.client.send(Request::new(Method::GET, "/api/emoji").header("X-Stellarr-Server", elsewhere)).await;
    let upload = maja
        .client
        .send(Request::new(Method::POST, "/api/emoji").header("X-Stellarr-Server", elsewhere).bytes("text/plain", b"x".to_vec()))
        .await;
    let malformed = maja.client.send(Request::new(Method::GET, "/api/emoji?serverId=x")).await;

    assert_eq!((list.status.as_u16(), upload.status.as_u16(), malformed.status.as_u16()), (403, 403, 400));
    assert!(list.body.is_empty() && upload.body.is_empty());
}

#[tokio::test]
async fn nobody_reads_or_writes_emoji_without_signing_in() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = add(&maja, "known").await.json::<Value>()["id"].as_str().unwrap().to_string();
    let anonymous = server.client_of("web", "2.0.0");

    assert_eq!(anonymous.get("/api/emoji").await.status, 401);
    assert_eq!(anonymous.send(form("anon", PNG, "image/png")).await.status, 401);
    assert_eq!(anonymous.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await.status, 401);
    assert_eq!(anonymous.get(&format!("/api/emoji/{id}/image")).await.status, 401);
}

#[tokio::test]
async fn the_image_is_served_whole_with_the_headers_the_first_server_gave_it() {
    let (server, maja, jo) = with_member().await;
    let created: Value = maja.client.send(form("pic", &[PNG, &[7u8; 100][..]].concat(), "image/png")).await.json();
    let glyph = created["glyph"].as_str().unwrap().to_string();
    let path = created["glyph"].as_str().unwrap().split('?').next().unwrap().to_string();

    let member = jo.client.get(&glyph).await;
    let without_server = jo.client.get(&path).await;
    let ranged = jo.client.send(Request::new(Method::GET, &glyph).header("Range", "bytes=0-3")).await;
    let conditional = jo
        .client
        .send(Request::new(Method::GET, &glyph).header("If-Modified-Since", "Wed, 21 Oct 2037 07:28:00 GMT").header("If-None-Match", "*"))
        .await;
    let elsewhere = jo
        .client
        .send(Request::new(Method::GET, &path).header("X-Stellarr-Server", "00000000-0000-0000-0000-0000000000aa"))
        .await;
    let unknown = jo.client.get(&glyph.replace(&created["id"].as_str().unwrap()[..8], "00000000")).await;
    let not_an_id = jo.client.get("/api/emoji/x/image?serverId=nothing").await;
    let braces = jo.client.get(&format!("/api/emoji/%7B{}%7D/image", created["id"].as_str().unwrap())).await;

    assert_eq!(member.status, 200);
    assert_eq!(member.body.len(), PNG.len() + 100);
    assert_eq!(member.header("content-type"), Some("image/png"));
    assert_eq!(member.header("content-length"), Some("112"));
    assert_eq!(member.header("cache-control"), Some("private, max-age=86400, immutable"));
    for absent in ["etag", "last-modified", "accept-ranges", "content-disposition", "x-content-type-options", "content-range"] {
        assert_eq!(member.header(absent), None, "{absent}");
    }
    assert_eq!(without_server.status, 200, "the first server of the person is the one asked about");
    assert_eq!(ranged.status, 200, "a range is not understood");
    assert_eq!(ranged.body.len(), 112);
    assert_eq!(conditional.status, 200);
    assert_eq!((elsewhere.status.as_u16(), unknown.status.as_u16(), not_an_id.status.as_u16()), (403, 404, 400));
    assert_eq!(braces.status, 404, "an id in braces is no id");
    // The files are gone from the store: the answer is the same as for an unknown emoji.
    std::fs::remove_dir_all(server.folder().join("files").join("emoji")).unwrap();
    assert_eq!(jo.client.get(&glyph).await.status, 404);
}

#[tokio::test]
async fn deleting_an_emoji_that_messages_and_reactions_use_leaves_them_as_they_are() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let created: Value = add(&maja, "wave").await.json();
    let db = server.database();
    let channel = crate::uploads::channel_of(&db, first_server(&db));
    let message = Id::random();
    db.execute(
        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 0, 'hi :wave:', ?4)",
        params![message, channel, Id::parse(&maja.id).unwrap(), Timestamp::now()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"MessageReactions\" (\"MessageId\", \"UserId\", \"Emoji\", \"CreatedAt\") VALUES (?1, ?2, ':wave:', ?3)",
        params![message, Id::parse(&maja.id).unwrap(), Timestamp::now()],
    )
    .unwrap();

    let removed = maja.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{}", created["id"].as_str().unwrap()))).await;

    assert_eq!(removed.status, 204);
    let content: String = db.query_row("SELECT \"Content\" FROM \"Messages\" WHERE \"Id\" = ?1", [message], |row| row.get(0)).unwrap();
    let reaction: String = db.query_row("SELECT \"Emoji\" FROM \"MessageReactions\"", [], |row| row.get(0)).unwrap();
    assert_eq!((content.as_str(), reaction.as_str()), ("hi :wave:", ":wave:"));
}

#[tokio::test]
async fn adding_and_deleting_are_announced_after_they_are_kept_and_written_in_the_audit_log() {
    let (server, maja, jo) = with_member().await;
    let server_id = first_server(&server.database()).to_string();

    let created: Value = add(&maja, "Party").await.json();
    add(&jo, "nope").await;
    add(&maja, "party").await;
    maja.client.send(form("!", PNG, "image/png")).await;
    let id = created["id"].as_str().unwrap().to_string();
    maja.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await;
    maja.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await;

    let told = events(&server, "EmojiChanged");
    assert_eq!(told.len(), 2, "{told:?}");
    for event in &told {
        assert_eq!(event["audience"]["server"], server_id);
        assert_eq!(event["payload"], json!({}));
    }
    let db = server.database();
    let mut statement = db
        .prepare("SELECT \"Action\", \"Summary\", \"TargetType\", \"TargetId\", \"TargetName\", \"ChangesJson\" FROM \"ServerAuditEntries\" WHERE \"Action\" LIKE 'emoji.%' ORDER BY \"CreatedAt\"")
        .unwrap();
    let rows: Vec<AuditRow> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let target = Id::parse(&id).unwrap().to_db_text();
    let actor = "Maja";
    assert_eq!(
        rows,
        [
            ("emoji.create".to_string(), format!("{actor} added the emoji :party:"), Some("emoji".to_string()), Some(target.clone()), Some("party".to_string()), None),
            ("emoji.delete".to_string(), format!("{actor} deleted the emoji :party:"), Some("emoji".to_string()), Some(target), Some("party".to_string()), None),
        ]
    );
}

#[tokio::test]
async fn an_emoji_counts_against_the_quota_and_a_refusal_for_room_leaves_nothing() {
    let server = started(&[("Uploads__MaxStoredBytesPerUser", "30")]).await;
    let maja = server.found("maja").await;

    let first = maja.client.send(form("first", PNG, "image/png")).await;
    let second = maja.client.send(form("second", PNG, "image/png")).await;
    let third = maja.client.send(form("third", PNG, "image/png")).await;

    assert_eq!((first.status.as_u16(), second.status.as_u16(), third.status.as_u16()), (201, 201, 507));
    assert_eq!(third.json::<Value>()["title"], "No room for the attachment");
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerEmoji\""), 2);
    assert_eq!(objects(&server).len(), 2);
}

#[tokio::test]
async fn forms_are_counted_against_the_limit_of_uploads_per_minute() {
    let server = Server::start_with(&[
        ("STELLARR_ENVIRONMENT", "Production"),
        ("Rtc__TurnSharedSecret", "a-secret-for-the-relay-that-is-long-enough"),
        ("Uploads__MinimumFreeDataRootBytes", "0"),
        ("RateLimiting__Uploads__PermitLimit", "2"),
    ])
    .await;
    let maja = server.found("maja").await;

    let first = add(&maja, "one").await;
    let second = add(&maja, "two").await;
    let over = add(&maja, "three").await;
    let list = maja.client.get("/api/emoji").await;

    assert_eq!((first.status.as_u16(), second.status.as_u16(), over.status.as_u16(), list.status.as_u16()), (201, 201, 429, 200));
    assert!(over.body.is_empty());
}

#[tokio::test]
async fn a_file_over_the_limit_is_never_written_to_the_staging_folder() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let response = maja.client.send(form("big", &[PNG, &vec![0u8; 3 * 1024 * 1024][..]].concat(), "image/png")).await;

    assert_eq!(response.status, 413);
    assert_eq!(crate::uploads::staged(&server), 0);
    assert!(objects(&server).is_empty());
}

// The files in a bucket instead of the data volume.

async fn with_bucket(fake: &FakeS3) -> Server {
    started(&[
        ("Storage__Provider", "S3"),
        ("Storage__Endpoint", fake.endpoint.as_str()),
        ("Storage__AccessKey", ACCESS_KEY),
        ("Storage__SecretKey", SECRET_KEY),
        ("Storage__Bucket", "emoji"),
    ])
    .await
}

#[tokio::test]
async fn in_a_bucket_an_emoji_is_stored_served_and_queued_for_removal() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake).await;
    let maja = server.found("maja").await;

    let created = add(&maja, "bucket").await;

    assert_eq!(created.status, 201, "{}", created.text());
    let glyph = created.json::<Value>()["glyph"].as_str().unwrap().to_string();
    let key: String = server.database().query_row("SELECT \"ObjectKey\" FROM \"ServerEmoji\"", [], |row| row.get(0)).unwrap();
    assert_eq!(fake.stored("emoji", &key).unwrap().0, PNG);
    assert!(objects(&server).is_empty(), "nothing is kept on the volume");
    let image = maja.client.get(&glyph).await;
    assert_eq!((image.status.as_u16(), image.body.as_ref(), image.header("cache-control")), (200, PNG, Some("private, max-age=86400, immutable")));
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    assert_eq!(maja.client.send(Request::new(Method::DELETE, &format!("/api/emoji/{id}"))).await.status, 204);
    let queued: String = server.database().query_row("SELECT \"ObjectKey\" FROM \"ObjectDeletions\"", [], |row| row.get(0)).unwrap();
    assert_eq!(queued, key);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerEmoji\""), 0);
}

#[tokio::test]
async fn a_bucket_that_refuses_the_write_leaves_no_emoji() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake).await;
    let maja = server.found("maja").await;
    fake.fail_next("PUT", 403, 20);

    let refused = add(&maja, "nope").await;

    assert_eq!(refused.status, 500);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerEmoji\""), 0);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerAuditEntries\" WHERE \"Action\" = 'emoji.create'"), 0);
    assert!(events(&server, "EmojiChanged").is_empty());
}

#[tokio::test]
async fn an_object_the_bucket_does_not_have_is_not_found() {
    let fake = FakeS3::start().await;
    let server = with_bucket(&fake).await;
    let maja = server.found("maja").await;
    let kept = add(&maja, "kept").await;
    let glyph = kept.json::<Value>()["glyph"].as_str().unwrap().to_string();
    assert_eq!(kept.status, 201);
    fake.fail_next("GET", 404, 1);

    assert_eq!(maja.client.get(&glyph).await.status, 404);
}
