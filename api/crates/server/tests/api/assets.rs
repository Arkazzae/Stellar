//! Downloading stored files: who may read what, and how the bytes are sent.

use http::Method;
use rusqlite::{Connection, params};
use serde_json::Value;
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Server};
use crate::uploads::{
    Form, JPEG, MP4, PDF, WAVE, WEBP, channel_of, first_server, id_of, image, post_message, started, upload,
};

const BODY: &[u8] = b"\x89PNG\r\n\x1a\nabcdefghijklmnopqrstuvwxyz0123456789";

async fn long_image(person: &Person) -> String {
    id_of(&upload(person, Form::new().file("file", "long.png", "image/png", BODY)).await)
}

#[tokio::test]
async fn the_uploader_reads_the_same_bytes_back_with_the_headers_of_an_inline_asset() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = long_image(&maja).await;

    let response = maja.client.get(&format!("/assets/{id}")).await;

    assert_eq!(response.status, 200);
    assert_eq!(&response.body[..], BODY);
    assert_eq!(response.header("content-type"), Some("image/png"));
    assert_eq!(response.header("content-length"), Some(BODY.len().to_string().as_str()));
    assert_eq!(response.header("accept-ranges"), Some("bytes"));
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    // Media is played in the page, and nothing about it is cached or dated.
    for absent in ["content-disposition", "last-modified", "etag", "content-range"] {
        assert_eq!(response.header(absent), None, "{absent}");
    }
}

#[tokio::test]
async fn assets_need_a_signed_in_person_and_only_get_is_served() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = long_image(&maja).await;

    for path in [format!("/assets/{id}"), format!("/assets/{id}/poster"), format!("/assets/{id}/preview")] {
        assert_eq!(server.client().get(&path).await.status, 401, "{path}");
    }
    let head = maja.client.send(Request::new(Method::HEAD, &format!("/assets/{id}"))).await;
    assert_eq!(head.status, 405);
}

#[tokio::test]
async fn an_address_that_is_no_id_or_names_nothing_is_not_found() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for path in [
        "/assets/nope",
        "/assets/00000000-0000-0000-0000-000000000000",
        "/assets/00000000-0000-0000-0000-000000000000/poster",
        "/assets/00000000-0000-0000-0000-000000000000/preview",
    ] {
        let response = maja.client.get(path).await;
        assert_eq!((response.status.as_u16(), response.body.is_empty()), (404, true), "{path}");
    }
}

#[tokio::test]
async fn a_member_who_shares_no_reference_to_the_attachment_gets_404() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let id = long_image(&maja).await;

    let theirs = sam.client.get(&format!("/assets/{id}")).await;
    let nothing = sam.client.get("/assets/00000000-0000-0000-0000-000000000000").await;

    assert_eq!(theirs.status, 404);
    assert_eq!(theirs.body, nothing.body);
}

#[tokio::test]
async fn an_attachment_is_opened_to_the_members_of_the_server_of_the_message_that_carries_it() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let id = long_image(&maja).await;
    let poster = id_of(
        &upload(
            &maja,
            Form::new().file("file", "c.mp4", "video/mp4", MP4).file("poster", "p.jpg", "image/jpeg", JPEG),
        )
        .await,
    );
    let preview = id_of(&upload(&maja, image("a.png").file("preview", "p", "image/webp", WEBP)).await);
    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 404);
    let db = server.database();
    let channel = channel_of(&db, first_server(&db));
    for attachment in [&id, &poster, &preview] {
        post_message(&db, channel, &maja.id, attachment);
    }

    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 200);
    assert_eq!(sam.client.get(&format!("/assets/{poster}/poster")).await.status, 200);
    assert_eq!(sam.client.get(&format!("/assets/{preview}/preview")).await.status, 200);
}

#[tokio::test]
async fn a_channel_that_somebody_is_denied_view_of_does_not_show_them_its_attachments() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let id = long_image(&maja).await;
    let db = server.database();
    let home = first_server(&db);
    let channel = channel_of(&db, home);
    post_message(&db, channel, &maja.id, &id);
    let base: Id = db
        .query_row(
            "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0 LIMIT 1",
            [home],
            |row| row.get(0),
        )
        .unwrap();
    db.execute(
        "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
        params![Id::random(), home, channel, base],
    )
    .unwrap();

    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 404);
    assert_eq!(maja.client.get(&format!("/assets/{id}")).await.status, 200);
}

#[tokio::test]
async fn a_file_posted_in_two_channels_is_read_by_whoever_sees_either() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let id = long_image(&maja).await;
    let db = server.database();
    let home = first_server(&db);
    let first = channel_of(&db, home);
    let second = Id::random();
    db.execute(
        "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\")
         VALUES (?1, ?2, 'second', 0, 99, '', 0, 0, 0)",
        params![second, home],
    )
    .unwrap();
    post_message(&db, first, &maja.id, &id);
    post_message(&db, second, &maja.id, &id);
    let base: Id = db
        .query_row(
            "SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0 LIMIT 1",
            [home],
            |row| row.get(0),
        )
        .unwrap();
    db.execute(
        "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
        params![Id::random(), home, first, base],
    )
    .unwrap();

    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 200);
}

#[tokio::test]
async fn a_range_is_answered_with_206_and_only_the_requested_bytes() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = long_image(&maja).await;
    let path = format!("/assets/{id}");
    let total = BODY.len();
    let ranged = |value: &'static str| {
        let client = &maja.client;
        let path = path.clone();
        async move { client.send(Request::new(Method::GET, &path).header("Range", value)).await }
    };

    let middle = ranged("bytes=2-5").await;
    let open = ranged("bytes=10-").await;
    let past_the_end = ranged("bytes=0-99999").await;
    let beyond = ranged("bytes=99-").await;
    let at_the_end = ranged("bytes=40-").await;

    assert_eq!(middle.status, 206);
    assert_eq!(&middle.body[..], &BODY[2..6]);
    assert_eq!(middle.header("content-range"), Some(format!("bytes 2-5/{total}").as_str()));
    assert_eq!(middle.header("content-length"), Some("4"));
    assert_eq!(middle.header("accept-ranges"), Some("bytes"));
    assert_eq!(open.status, 206);
    assert_eq!(&open.body[..], &BODY[10..]);
    assert_eq!(open.header("content-range"), Some(format!("bytes 10-{}/{total}", total - 1).as_str()));
    assert_eq!(past_the_end.status, 206);
    assert_eq!(&past_the_end.body[..], BODY);
    assert_eq!(past_the_end.header("content-range"), Some(format!("bytes 0-{}/{total}", total - 1).as_str()));
    assert_eq!((beyond.status.as_u16(), beyond.body.is_empty()), (416, true));
    assert_eq!(at_the_end.status, if total > 40 { 206 } else { 416 });
}

#[tokio::test]
async fn a_range_that_is_not_served_is_answered_with_the_whole_file() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = long_image(&maja).await;

    for value in ["bytes=-5", "bytes=5-2", "bytes=a-b", "items=0-1", "", "bytes=", "bytes=1"] {
        let response = maja
            .client
            .send(Request::new(Method::GET, &format!("/assets/{id}")).header("Range", value))
            .await;

        assert_eq!(response.status, 200, "{value:?}");
        assert_eq!(&response.body[..], BODY, "{value:?}");
        assert_eq!(response.header("content-range"), None);
    }
    // Several ranges: the first is served.
    let several = maja
        .client
        .send(Request::new(Method::GET, &format!("/assets/{id}")).header("Range", "bytes=0-1, 5-6"))
        .await;
    assert_eq!((several.status.as_u16(), several.body.len()), (206, 2));
}

#[tokio::test]
async fn conditions_of_a_request_are_not_looked_at() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = long_image(&maja).await;

    for (name, value) in [
        ("If-None-Match", "\"x\""),
        ("If-Modified-Since", "Fri, 20 Nov 2090 10:11:12 GMT"),
        ("If-Unmodified-Since", "Fri, 20 Nov 2000 10:11:12 GMT"),
        ("If-Match", "\"x\""),
    ] {
        let response = maja
            .client
            .send(Request::new(Method::GET, &format!("/assets/{id}")).header(name, value))
            .await;

        assert_eq!(response.status, 200, "{name}");
        assert_eq!(&response.body[..], BODY);
    }
}

#[tokio::test]
async fn a_generic_file_is_offered_as_a_download_under_its_verified_name() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let pdf = id_of(&upload(&maja, Form::new().file("file", "Ünï q x.exe", "application/pdf", PDF)).await);
    let audio = id_of(&upload(&maja, Form::new().file("file", "s.wav", "audio/wav", WAVE)).await);
    let encrypted = id_of(
        &maja
            .client
            .send(Form::new().file("file", "x", "text/html", b"\x01\x02").post("/api/uploads/encrypted"))
            .await,
    );

    let pdf = maja.client.get(&format!("/assets/{pdf}")).await;
    let audio = maja.client.get(&format!("/assets/{audio}")).await;
    let encrypted = maja.client.get(&format!("/assets/{encrypted}")).await;

    assert_eq!(pdf.header("content-type"), Some("application/pdf"));
    assert_eq!(
        pdf.header("content-disposition"),
        Some("attachment; filename*=UTF-8''%C3%9Cn%C3%AF%20q%20x.pdf")
    );
    assert_eq!(audio.header("content-disposition"), None);
    assert_eq!(encrypted.header("content-type"), Some("application/octet-stream"));
    assert_eq!(encrypted.header("content-disposition"), Some("attachment; filename*=UTF-8''encrypted.bin"));
}

#[tokio::test]
async fn a_name_is_escaped_for_everything_that_is_not_unreserved() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = id_of(&upload(&maja, Form::new().file("file", "a_b-c.d~e f(1)!'*.pdf", "application/pdf", PDF)).await);

    let response = maja.client.get(&format!("/assets/{id}")).await;

    // What is not a letter, a digit, a space, a hyphen, an underscore or a
    // point is dropped from the name before it is stored.
    assert_eq!(response.header("content-disposition"), Some("attachment; filename*=UTF-8''a_b-c.de%20f1.pdf"));
}

#[tokio::test]
async fn a_poster_and_a_preview_are_served_whole_with_their_type_and_size_only() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let video = id_of(
        &upload(&maja, Form::new().file("file", "c.mp4", "video/mp4", MP4).file("poster", "p.jpg", "image/jpeg", JPEG)).await,
    );
    let plain = id_of(&upload(&maja, image("a.png")).await);

    let poster = maja
        .client
        .send(Request::new(Method::GET, &format!("/assets/{video}/poster")).header("Range", "bytes=0-1"))
        .await;

    assert_eq!(poster.status, 200);
    assert_eq!(&poster.body[..], JPEG);
    assert_eq!(poster.header("content-type"), Some("image/jpeg"));
    assert_eq!(poster.header("content-length"), Some(JPEG.len().to_string().as_str()));
    for absent in ["accept-ranges", "x-content-type-options", "content-disposition"] {
        assert_eq!(poster.header(absent), None, "{absent}");
    }
    assert_eq!(maja.client.get(&format!("/assets/{plain}/poster")).await.status, 404);
    assert_eq!(maja.client.get(&format!("/assets/{plain}/preview")).await.status, 404);
}

#[tokio::test]
async fn an_object_that_is_gone_answers_404_and_a_range_the_store_cannot_satisfy_416() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let id = long_image(&maja).await;
    let empty = id_of(&upload(&maja, Form::new().file("file", "e.txt", "text/plain", b"x")).await);
    let key: String = server
        .database()
        .query_row("SELECT \"ObjectKey\" FROM \"ImageAttachments\" WHERE \"Id\" = ?1", [Id::parse(&id).unwrap()], |row| row.get(0))
        .unwrap();
    std::fs::remove_file(server.folder().join("files").join(&key)).unwrap();

    let gone = maja.client.get(&format!("/assets/{id}")).await;
    let past = maja
        .client
        .send(Request::new(Method::GET, &format!("/assets/{empty}")).header("Range", "bytes=1-"))
        .await;

    assert_eq!(gone.status, 404);
    assert_eq!(past.status, 416);
}

fn row(db: &Connection, sql: &str, values: &[&dyn rusqlite::ToSql]) {
    db.execute(sql, values).unwrap();
}

fn attachment_of(server: &Server, id: &str) -> Id {
    let _ = server;
    Id::parse(id).unwrap()
}

#[tokio::test]
async fn only_the_uploader_reads_an_attachment_that_nothing_uses() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let id = long_image(&maja).await;

    assert_eq!(maja.client.get(&format!("/assets/{id}")).await.status, 200);
    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 404);
}

#[tokio::test]
async fn a_direct_message_attachment_is_for_the_two_people_in_the_conversation() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let lee = server.register("lee").await;
    let id = long_image(&maja).await;
    let db = server.database();
    let (conversation, message) = (Id::random(), Id::random());
    let (a, b) = (Id::parse(&maja.id).unwrap(), Id::parse(&sam.id).unwrap());
    row(
        &db,
        "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\") VALUES (?1, ?2, ?3, ?4, 'off')",
        params![conversation, a, b, Timestamp::now()],
    );
    row(
        &db,
        "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 'hi', ?4)",
        params![message, conversation, a, Timestamp::now()],
    );
    row(
        &db,
        "INSERT INTO \"DirectMessageAttachments\" (\"DirectMessageId\", \"AttachmentId\") VALUES (?1, ?2)",
        params![message, attachment_of(&server, &id)],
    );

    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 200);
    assert_eq!(lee.client.get(&format!("/assets/{id}")).await.status, 404);
}

#[tokio::test]
async fn avatars_and_banners_are_for_everybody_and_a_wallpaper_for_its_owner_alone() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let avatar = maja.client.send(image("a.png").post("/api/me/images/avatar")).await;
    let banner = maja.client.send(image("b.png").post("/api/me/images/banner")).await;
    let wallpaper = maja.client.send(image("w.png").post("/api/me/images/wallpaper")).await;

    for (answer, readable) in [(avatar, 200), (banner, 200), (wallpaper, 404)] {
        let address = answer.header("location").unwrap().to_string();

        assert_eq!(sam.client.get(&address).await.status, readable, "{address}");
        assert_eq!(maja.client.get(&address).await.status, 200, "{address}");
    }
}

#[tokio::test]
async fn a_server_icon_is_for_its_members_unless_the_server_is_public() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let db = server.database();
    let home = first_server(&db);
    let icon = maja.client.send(image("i.png").post("/api/admin/server/icon")).await;
    let address = icon.json::<Value>()["iconUrl"].as_str().unwrap().to_string();
    assert_eq!(sam.client.get(&address).await.status, 200);

    // Moving the member out leaves only the public rule.
    row(&db, "DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", params![Id::parse(&sam.id).unwrap()]);
    assert_eq!(sam.client.get(&address).await.status, 404);
    row(&db, "UPDATE \"Servers\" SET \"IsPublic\" = 1 WHERE \"Id\" = ?1", params![home]);
    assert_eq!(sam.client.get(&address).await.status, 200);
}

#[tokio::test]
async fn a_soundboard_clip_is_for_the_members_of_its_server_and_a_join_sound_travels_with_its_owner() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let lee = server.register("lee").await;
    let id = id_of(&upload(&maja, Form::new().file("file", "s.wav", "audio/wav", WAVE)).await);
    let db = server.database();
    let home = first_server(&db);
    let sound = Id::random();
    row(
        &db,
        "INSERT INTO \"SoundboardSounds\" (\"Id\", \"ServerId\", \"AttachmentId\", \"CreatedById\", \"Name\", \"Glyph\", \"TrimStartSeconds\", \"TrimEndSeconds\", \"Position\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, 'ding', 'x', 0, 1, 0, ?5)",
        params![sound, home, attachment_of(&server, &id), Id::parse(&maja.id).unwrap(), Timestamp::now()],
    );
    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 200);
    // lee leaves the server and loses the clip, until a join sound of somebody
    // who shares a server with them names it.
    row(&db, "DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", params![Id::parse(&lee.id).unwrap()]);
    assert_eq!(lee.client.get(&format!("/assets/{id}")).await.status, 404);
    row(
        &db,
        "INSERT INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"MutedChannelIdsJson\", \"VadSensitivity\", \"JoinSoundId\")
         VALUES (?1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, '[]', 60, ?2)",
        params![Id::parse(&sam.id).unwrap(), sound],
    );
    // lee shares nothing with sam any more, until they talk.
    assert_eq!(lee.client.get(&format!("/assets/{id}")).await.status, 404);
    row(
        &db,
        "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\") VALUES (?1, ?2, ?3, ?4, 'off')",
        params![Id::random(), Id::parse(&lee.id).unwrap(), Id::parse(&sam.id).unwrap(), Timestamp::now()],
    );
    assert_eq!(lee.client.get(&format!("/assets/{id}")).await.status, 200);
}

#[tokio::test]
async fn the_picture_of_a_link_card_is_for_whoever_sees_the_channel_of_its_message() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let sam = server.register("sam").await;
    let id = long_image(&maja).await;
    let db = server.database();
    let channel = channel_of(&db, first_server(&db));
    let message = post_message(&db, channel, &maja.id, &long_image(&maja).await);
    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 404);
    row(
        &db,
        "INSERT INTO \"MessageEmbeds\" (\"Id\", \"MessageId\", \"Url\", \"Kind\", \"Provider\", \"Domain\", \"ThumbnailAttachmentId\", \"Position\", \"CreatedAt\") VALUES (?1, ?2, 'https://example.test', 0, 'example.test', 'example.test', ?3, 0, ?4)",
        params![Id::random(), message, attachment_of(&server, &id), Timestamp::now()],
    );

    assert_eq!(sam.client.get(&format!("/assets/{id}")).await.status, 200);
}
