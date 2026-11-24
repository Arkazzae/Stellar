//! The soundboard of a server: the catalogue, its order, its two switches, and
//! who may change them or play from them.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Id;

use crate::support::{Person, Request, Response, Server};
use crate::uploads::{Form, PNG, WAVE, first_server, objects, started};

const SOUNDS: &str = "/api/admin/soundboard/sounds";

/// An audio file the person uploaded, which the browser measured at `seconds`.
async fn wave(person: &Person, seconds: f64) -> String {
    let response = person
        .client
        .send(
            Form::new()
                .file("file", "s.wav", "audio/wav", WAVE)
                .text("durationSeconds", &seconds.to_string())
                .post("/api/uploads/images"),
        )
        .await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

fn body(attachment: &str, name: &str, glyph: &str, start: f64, end: f64) -> Value {
    json!({ "attachmentId": attachment, "name": name, "glyph": glyph, "trimStartSeconds": start, "trimEndSeconds": end })
}

async fn put(person: &Person, method: Method, path: &str, body: Value) -> Response {
    person.client.send(Request::new(method, path).json(&body)).await
}

/// Makes a sound of an audio file of 20 seconds and returns its id.
async fn sound(person: &Person, name: &str) -> String {
    let clip = wave(person, 20.0).await;
    let response = put(person, Method::POST, SOUNDS, body(&clip, name, "g", 0.0, 2.0)).await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn events(server: &Server, name: &str) -> Vec<Value> {
    server.events().into_iter().filter(|event| event["name"] == name).collect()
}

/// A role with these permissions, held by the person.
fn give(server: &Server, person: &Person, permissions: i64) {
    let db = server.database();
    let first = first_server(&db);
    let role = Id::random();
    db.execute(
        "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, 5, 0, 0)",
        params![role, first, format!("Staff {role}"), permissions],
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"ServerMemberRoles\" (\"ServerId\", \"UserId\", \"RoleId\") VALUES (?1, ?2, ?3)",
        params![first, Id::parse(&person.id).unwrap(), role],
    )
    .unwrap();
}

struct Board {
    server: Server,
    owner: Person,
    member: Person,
}

async fn board() -> Board {
    let server = started(&[]).await;
    let owner = server.found("maja").await;
    let member = server.register("ilya").await;
    Board { server, owner, member }
}

#[tokio::test]
async fn the_owner_makes_trims_changes_and_removes_a_sound() {
    let Board { server, owner, member } = board().await;
    let clip = wave(&owner, 8.25).await;

    let created = put(&owner, Method::POST, SOUNDS, body(&clip, "  Start rakiety ", "\u{1f680}", 1.25, 5.5)).await;

    assert_eq!(created.status, 201, "{}", created.text());
    let sound: Value = created.json();
    let id = sound["id"].as_str().unwrap();
    assert_eq!(created.header("location"), Some(format!("/api/admin/soundboard/sounds/{id}").as_str()));
    assert_eq!(
        created.text(),
        format!("{{\"id\":\"{id}\",\"name\":\"Start rakiety\",\"glyph\":\"\u{1f680}\",\"url\":\"/assets/{clip}\",\"durationSeconds\":8.25,\"trimStartSeconds\":1.25,\"trimEndSeconds\":5.5}}")
    );
    let snapshot: Value = member.client.get("/api/soundboard").await.json();
    assert_eq!(snapshot["sounds"], json!([sound]));
    assert_eq!(snapshot["settings"], json!({ "everyoneCanPlay": true, "rateLimited": true }));
    assert_eq!((snapshot["canManage"].clone(), snapshot["canPlay"].clone()), (json!(false), json!(true)));
    let updated = put(&owner, Method::PATCH, &format!("{SOUNDS}/{id}"), json!({ "name": "Rakieta", "trimStartSeconds": 2.0, "trimEndSeconds": 4.0 })).await;
    assert_eq!(
        updated.text(),
        format!("{{\"id\":\"{id}\",\"name\":\"Rakieta\",\"glyph\":\"\u{1f680}\",\"url\":\"/assets/{clip}\",\"durationSeconds\":8.25,\"trimStartSeconds\":2,\"trimEndSeconds\":4}}")
    );
    let settings = put(&owner, Method::PATCH, "/api/admin/soundboard/settings", json!({ "everyoneCanPlay": false, "rateLimited": false })).await;
    assert_eq!(settings.text(), "{\"everyoneCanPlay\":false,\"rateLimited\":false}");

    let removed = owner.client.send(Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))).await;

    assert_eq!(removed.status, 204);
    assert_eq!(member.client.get("/api/soundboard").await.json::<Value>()["sounds"], json!([]));
    // The clip is the sound's own and goes with it, queued for the sweep.
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 0);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ObjectDeletions\""), 1);
    assert_eq!(objects(&server).len(), 1, "the object stays until the sweep takes it");
}

#[tokio::test]
async fn the_snapshot_lists_sounds_by_position_then_by_when_they_were_made() {
    let Board { server, owner, member } = board().await;
    sound(&owner, "first").await;
    sound(&owner, "second").await;
    sound(&owner, "third").await;
    let db = server.database();
    db.execute("UPDATE \"SoundboardSounds\" SET \"Position\" = 0, \"CreatedAt\" = '2026-02-01 00:00:00' WHERE \"Name\" = 'second'", []).unwrap();
    db.execute("UPDATE \"SoundboardSounds\" SET \"Position\" = 0, \"CreatedAt\" = '2026-01-01 00:00:00' WHERE \"Name\" = 'third'", []).unwrap();

    let names: Vec<String> = member.client.get("/api/soundboard").await.json::<Value>()["sounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sound| sound["name"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(names, ["third", "second", "first"]);
}

#[tokio::test]
async fn a_trim_that_starts_at_minus_zero_is_written_as_zero() {
    let Board { server: _server, owner, .. } = board().await;
    let clip = wave(&owner, 8.25).await;

    let created = put(&owner, Method::POST, SOUNDS, body(&clip, "Zero", "g", -0.0, 5.5)).await;

    assert_eq!(created.status, 201, "{}", created.text());
    assert!(created.text().contains("\"trimStartSeconds\":0,"), "{}", created.text());
}

#[tokio::test]
async fn a_clip_whose_length_was_never_measured_is_as_long_as_its_trim_ends() {
    let Board { server, owner, .. } = board().await;
    let id = sound(&owner, "bare").await;
    server.database().execute("UPDATE \"ImageAttachments\" SET \"DurationSeconds\" = NULL", []).unwrap();
    server.database().execute("UPDATE \"SoundboardSounds\" SET \"TrimEndSeconds\" = 3.5", []).unwrap();

    let snapshot: Value = owner.client.get("/api/soundboard").await.json();
    let shorter = put(&owner, Method::PATCH, &format!("{SOUNDS}/{id}"), json!({ "trimEndSeconds": 3 })).await;
    let longer = put(&owner, Method::PATCH, &format!("{SOUNDS}/{id}"), json!({ "trimEndSeconds": 3.2 })).await;
    let within_the_tolerance = put(&owner, Method::PATCH, &format!("{SOUNDS}/{id}"), json!({ "trimEndSeconds": 3.01 })).await;

    assert_eq!(snapshot["sounds"][0]["durationSeconds"], json!(3.5));
    assert_eq!(shorter.json::<Value>()["durationSeconds"], json!(3), "its own end, as it now is");
    assert_eq!(longer.status, 400, "a clip of no known length is no longer than its trim says");
    assert_eq!(within_the_tolerance.status, 200);
}

#[tokio::test]
async fn only_somebody_who_manages_the_server_changes_the_soundboard() {
    let Board { server, owner, member } = board().await;
    let id = sound(&owner, "gong").await;
    let moderator = server.register("ada").await;
    give(&server, &moderator, 32 | 16); // moderate members and manage messages
    let manager = server.register("kim").await;
    give(&server, &manager, 1); // manage server

    for (who, person) in [("a member", &member), ("a moderator", &moderator)] {
        let own = wave(person, 3.0).await;
        for (label, request) in [
            ("create", Request::new(Method::POST, SOUNDS).json(&body(&own, "x", "g", 0.0, 1.0))),
            ("update", Request::new(Method::PATCH, &format!("{SOUNDS}/{id}")).json(&json!({ "name": "taken over" }))),
            ("update unknown", Request::new(Method::PATCH, &format!("{SOUNDS}/{}", Id::random())).json(&json!({}))),
            ("delete", Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))),
            ("delete unknown", Request::new(Method::DELETE, &format!("{SOUNDS}/{}", Id::random()))),
            ("order", Request::new(Method::PUT, "/api/admin/soundboard/order").json(&json!({ "soundIds": [id] }))),
            ("settings", Request::new(Method::PATCH, "/api/admin/soundboard/settings").json(&json!({ "everyoneCanPlay": false }))),
        ] {
            let response = person.client.send(request).await;
            assert_eq!(response.status, 403, "{who}: {label}");
            assert!(response.body.is_empty());
        }
        let snapshot: Value = person.client.get("/api/soundboard").await.json();
        assert_eq!(snapshot["canManage"], json!(false), "{who}");
        assert_eq!(snapshot["sounds"][0]["name"], "gong");
    }
    // Somebody who is no member of the server asked about is refused first.
    let elsewhere = ("X-Stellarr-Server", "00000000-0000-0000-0000-0000000000aa");
    for (label, request) in [
        ("read", Request::new(Method::GET, "/api/soundboard")),
        ("create", Request::new(Method::POST, SOUNDS).json(&body("x", "x", "x", 0.0, 1.0))),
        ("update", Request::new(Method::PATCH, &format!("{SOUNDS}/{id}")).json(&json!({}))),
        ("delete", Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))),
        ("order", Request::new(Method::PUT, "/api/admin/soundboard/order").json(&json!({}))),
        ("settings", Request::new(Method::PATCH, "/api/admin/soundboard/settings").json(&json!({}))),
    ] {
        let response = owner.client.send(request.header(elsewhere.0, elsewhere.1)).await;
        assert_eq!(response.status, 403, "{label}");
    }
    let anonymous = server.client_of("web", "2.0.0");
    assert_eq!(anonymous.get("/api/soundboard").await.status, 401);
    assert_eq!(anonymous.send(Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))).await.status, 401);
    // Manage server is the one permission that counts.
    let own = wave(&manager, 3.0).await;
    let by_manager = put(&manager, Method::POST, SOUNDS, body(&own, "kims", "k", 0.0, 1.0)).await;
    assert_eq!(by_manager.status, 201, "{}", by_manager.text());
    assert_eq!(manager.client.get("/api/soundboard").await.json::<Value>()["canManage"], json!(true));
    assert_eq!(owner.client.get("/api/soundboard").await.json::<Value>()["canManage"], json!(true));
    // Whoever may is told when a sound is not there; a deleted one is deleted.
    assert_eq!(put(&owner, Method::PATCH, &format!("{SOUNDS}/{}", Id::random()), json!({})).await.status, 404);
    let gone = owner.client.send(Request::new(Method::DELETE, &format!("{SOUNDS}/{}", Id::random()))).await;
    assert_eq!(gone.status, 204);
}

async fn can_play(person: &Person) -> Value {
    person.client.get("/api/soundboard").await.json::<Value>()["canPlay"].clone()
}

#[tokio::test]
async fn staff_play_when_the_server_keeps_the_soundboard_to_them_and_everybody_otherwise() {
    let Board { server, owner, member } = board().await;
    sound(&owner, "gong").await;
    let moderator = server.register("ada").await;
    give(&server, &moderator, 32);
    let messages = server.register("mia").await;
    give(&server, &messages, 16);
    let reader = server.register("rey").await;
    // Reading the audit log makes somebody staff of a kind, but not of this.
    give(&server, &reader, 128);

    for person in [&owner, &member, &moderator, &messages, &reader] {
        assert_eq!(can_play(person).await, json!(true));
    }
    put(&owner, Method::PATCH, "/api/admin/soundboard/settings", json!({ "everyoneCanPlay": false })).await;

    assert_eq!(can_play(&owner).await, json!(true));
    assert_eq!(can_play(&moderator).await, json!(true));
    assert_eq!(can_play(&messages).await, json!(true));
    assert_eq!(can_play(&member).await, json!(false));
    assert_eq!(can_play(&reader).await, json!(false));
}

#[tokio::test]
async fn a_name_a_symbol_and_a_trim_are_accepted_only_inside_their_bounds() {
    let Board { server: _server, owner, .. } = board().await;
    let sound_error = "The name must be 1\u{2013}32 characters, the symbol 1\u{2013}16 characters and the clip 0.15\u{2013}15 s.";

    for (label, name, glyph, start, end, ok) in [
        ("shortest name", "n".to_string(), "g".to_string(), 0.0, 1.0, true),
        ("longest name", "n".repeat(32), "g".to_string(), 0.0, 1.0, true),
        ("name too long", "n".repeat(33), "g".to_string(), 0.0, 1.0, false),
        ("name trimmed to nothing", "  \u{3000} ".to_string(), "g".to_string(), 0.0, 1.0, false),
        ("name trimmed to fit", format!(" {} ", "n".repeat(32)), "g".to_string(), 0.0, 1.0, true),
        ("name of sixteen emoji", "\u{1f680}".repeat(16), "g".to_string(), 0.0, 1.0, true),
        ("name of seventeen emoji", "\u{1f680}".repeat(17), "g".to_string(), 0.0, 1.0, false),
        ("longest symbol", "n".to_string(), "g".repeat(16), 0.0, 1.0, true),
        ("symbol too long", "n".to_string(), "g".repeat(17), 0.0, 1.0, false),
        ("symbol of eight emoji", "n".to_string(), "\u{1f680}".repeat(8), 0.0, 1.0, true),
        ("symbol of nine emoji", "n".to_string(), "\u{1f680}".repeat(9), 0.0, 1.0, false),
        ("no symbol", "n".to_string(), " ".to_string(), 0.0, 1.0, false),
        ("shortest clip", "n".into(), "g".into(), 0.0, 0.15, true),
        ("clip too short", "n".into(), "g".into(), 0.0, 0.149, false),
        ("longest clip", "n".into(), "g".into(), 0.0, 15.01, true),
        ("clip too long", "n".into(), "g".into(), 0.0, 15.011, false),
        ("start before the beginning", "n".into(), "g".into(), -0.001, 1.0, false),
        ("end at the end of the clip", "n".into(), "g".into(), 10.0, 20.0, true),
        ("end a tenth of a second after it", "n".into(), "g".into(), 10.0, 20.01, true),
        ("end past the tolerance", "n".into(), "g".into(), 10.0, 20.011, false),
        ("end before start", "n".into(), "g".into(), 2.0, 1.0, false),
        ("end beyond anything", "n".into(), "g".into(), 0.0, 1e300, false),
    ] {
        let clip = wave(&owner, 20.0).await;
        let response = put(&owner, Method::POST, SOUNDS, body(&clip, &name, &glyph, start, end)).await;
        if ok {
            assert_eq!(response.status, 201, "{label}: {}", response.text());
        } else {
            assert_eq!(response.status, 400, "{label}");
            assert_eq!(response.json::<Value>()["errors"]["sound"][0], sound_error, "{label}");
        }
    }
}

#[tokio::test]
async fn a_trim_is_kept_to_the_thousandth_and_a_name_trimmed() {
    let Board { server, owner, .. } = board().await;
    let clip = wave(&owner, 20.0).await;

    let created = put(&owner, Method::POST, SOUNDS, body(&clip, "  pad  ", " g ", 0.0004, 1.2345)).await;

    let sound: Value = created.json();
    assert_eq!((sound["name"].clone(), sound["glyph"].clone()), (json!("pad"), json!("g")));
    assert_eq!((sound["trimStartSeconds"].clone(), sound["trimEndSeconds"].clone()), (json!(0), json!(1.234)));
    let (name, start): (String, f64) = server
        .database()
        .query_row("SELECT \"Name\", \"TrimStartSeconds\" FROM \"SoundboardSounds\"", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!((name.as_str(), start), ("pad", 0.0));
}

#[tokio::test]
async fn only_an_audio_file_of_the_persons_own_with_a_known_length_that_no_sound_uses_is_a_clip() {
    let Board { server, owner, member } = board().await;
    let attachment_error = "Pick an uploaded audio file with a known duration.";
    let image = owner.client.send(Form::new().file("file", "a.png", "image/png", PNG).post("/api/uploads/images")).await;
    let image = image.json::<Value>()["id"].as_str().unwrap().to_string();
    let unmeasured = owner.client.send(Form::new().file("file", "n.wav", "audio/wav", WAVE).post("/api/uploads/images")).await;
    let unmeasured = unmeasured.json::<Value>()["id"].as_str().unwrap().to_string();
    let loud = owner
        .client
        .send(Form::new().file("file", "n.wav", "AUDIO/WAV", WAVE).text("durationSeconds", "5").post("/api/uploads/images"))
        .await;
    let loud = loud.json::<Value>()["id"].as_str().unwrap().to_string();
    let theirs = wave(&member, 5.0).await;
    let free = wave(&owner, 5.0).await;
    let unknown = Id::random().to_string();
    let nil = "00000000-0000-0000-0000-000000000000";

    for (label, clip) in [("an image", &image), ("a clip with no length", &unmeasured), ("another person's clip", &theirs), ("no such file", &unknown), ("nothing", &nil.to_string())] {
        let response = put(&owner, Method::POST, SOUNDS, body(clip, "n", "g", 0.0, 1.0)).await;
        assert_eq!(response.status, 400, "{label}");
        assert_eq!(response.json::<Value>()["errors"]["attachmentId"][0], attachment_error, "{label}");
    }
    let missing = put(&owner, Method::POST, SOUNDS, json!({ "name": "n", "glyph": "g", "trimEndSeconds": 1 })).await;
    assert_eq!(missing.json::<Value>()["errors"]["attachmentId"][0], attachment_error);
    assert_eq!(put(&owner, Method::POST, SOUNDS, body(&loud, "n", "g", 0.0, 1.0)).await.status, 201, "a type in capitals is audio");
    assert_eq!(put(&owner, Method::POST, SOUNDS, body(&free, "n", "g", 0.0, 1.0)).await.status, 201);
    let again = put(&owner, Method::POST, SOUNDS, body(&free, "other", "g", 0.0, 1.0)).await;
    assert_eq!(again.status, 409);
    assert!(again.body.is_empty());
    // The clip is checked before the sound is, so a bad sound on a taken clip is a conflict.
    let both = put(&owner, Method::POST, SOUNDS, body(&free, "", "g", 0.0, 1.0)).await;
    assert_eq!(both.status, 409);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"SoundboardSounds\""), 2);
}

#[tokio::test]
async fn eight_sounds_made_of_one_clip_at_once_leave_one() {
    let Board { server, owner, .. } = board().await;
    let clip = wave(&owner, 5.0).await;

    let at_once = || put(&owner, Method::POST, SOUNDS, body(&clip, "same", "g", 0.0, 1.0));
    let answers = tokio::join!(at_once(), at_once(), at_once(), at_once(), at_once(), at_once(), at_once(), at_once());
    let answers = [answers.0, answers.1, answers.2, answers.3, answers.4, answers.5, answers.6, answers.7];

    let mut statuses: Vec<u16> = answers.iter().map(|answer| answer.status.as_u16()).collect();
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 409, 409, 409, 409, 409, 409, 409]);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"SoundboardSounds\""), 1);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ServerAuditEntries\" WHERE \"Action\" = 'sound.create'"), 1);
    assert_eq!(events(&server, "SoundboardChanged").len(), 1);
}

#[tokio::test]
async fn numbers_come_as_numbers_and_nothing_else() {
    let Board { server: _server, owner, .. } = board().await;
    let attempt = |trim_end: Value| {
        let owner = &owner;
        async move {
            let clip = wave(owner, 20.0).await;
            let response = put(owner, Method::POST, SOUNDS, json!({ "attachmentId": clip, "name": "n", "glyph": "g", "trimStartSeconds": 0, "trimEndSeconds": trim_end })).await;
            let status = response.status.as_u16();
            (status, (status == 201).then(|| response.json::<Value>()["trimEndSeconds"].clone()))
        }
    };

    for ok in [json!(2), json!(2.5), json!(0.5e1), json!(2e0)] {
        assert_eq!(attempt(ok.clone()).await.0, 201, "{ok}");
    }
    assert_eq!(attempt(json!(1.5)).await, (201, Some(json!(1.5))));
    for bad in [json!("2"), json!("+2"), json!(".5e1"), json!("1.5"), json!(" 2"), json!("2 "), json!("0x2"), json!("2,5"), json!(""), json!(true), json!([]), json!(null), json!("two")] {
        assert_eq!(attempt(bad.clone()).await.0, 400, "{bad}");
    }
    for words in ["NaN", "Infinity", "-Infinity", "1e999"] {
        let (status, _) = attempt(json!(words)).await;
        assert_eq!(status, 400, "{words}");
    }
}

#[tokio::test]
async fn a_change_names_what_it_changes_and_the_rest_stays() {
    let Board { server, owner, .. } = board().await;
    let id = sound(&owner, "Gong").await;
    let path = format!("{SOUNDS}/{id}");

    let nothing = put(&owner, Method::PATCH, &path, json!({})).await;
    let renamed = put(&owner, Method::PATCH, &path, json!({ "name": " Gong II ", "glyph": "\u{2764}" })).await;
    let trimmed = put(&owner, Method::PATCH, &path, json!({ "trimStartSeconds": 1.005, "trimEndSeconds": 8.255 })).await;
    let same = put(&owner, Method::PATCH, &path, json!({ "name": "Gong II", "trimEndSeconds": 8.255 })).await;
    let nulls = put(&owner, Method::PATCH, &path, json!({ "name": null, "glyph": null, "trimStartSeconds": null, "trimEndSeconds": null })).await;
    let blank = put(&owner, Method::PATCH, &path, json!({ "name": " " })).await;
    let past_the_clip = put(&owner, Method::PATCH, &path, json!({ "trimEndSeconds": 20.011 })).await;

    assert_eq!(nothing.status, 200);
    assert_eq!(renamed.json::<Value>()["name"], "Gong II");
    assert_eq!(trimmed.json::<Value>()["trimStartSeconds"], json!(1.005));
    assert_eq!(same.status, 200);
    assert_eq!(nulls.json::<Value>()["glyph"], "\u{2764}");
    assert_eq!((blank.status.as_u16(), past_the_clip.status.as_u16()), (400, 400));
    let db = server.database();
    let mut statement = db
        .prepare("SELECT \"Summary\", \"TargetName\", \"ChangesJson\" FROM \"ServerAuditEntries\" WHERE \"Action\" = 'sound.update' ORDER BY \"CreatedAt\"")
        .unwrap();
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    // Nothing is written for a change that changes nothing, and a trim is
    // worded to two places the way the first server did (1.005 rounds up).
    assert_eq!(
        rows,
        [
            (
                "Maja updated the sound Gong II".to_string(),
                "Gong II".to_string(),
                "[{\"field\":\"name\",\"from\":\"Gong\",\"to\":\"Gong II\"},{\"field\":\"glyph\",\"from\":\"g\",\"to\":\"\u{2764}\"}]".to_string()
            ),
            (
                "Maja updated the sound Gong II".to_string(),
                "Gong II".to_string(),
                "[{\"field\":\"trim\",\"from\":\"0\u{2013}2\",\"to\":\"1.01\u{2013}8.26\"}]".to_string()
            ),
        ]
    );
    assert_eq!(events(&server, "SoundboardChanged").len(), 1 + 5, "one for the sound made and one for each change that was kept, even one that changed nothing");
}

#[tokio::test]
async fn the_order_names_every_sound_once_and_only_this_servers() {
    let Board { server, owner, member } = board().await;
    let one = sound(&owner, "one").await;
    let two = sound(&owner, "two").await;
    let three = sound(&owner, "three").await;
    let strange = Id::random().to_string();
    let order = |ids: Value| put(&owner, Method::PUT, "/api/admin/soundboard/order", ids);
    let names = || async {
        member.client.get("/api/soundboard").await.json::<Value>()["sounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sound| sound["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    for (label, ids) in [
        ("an incomplete list", json!({ "soundIds": [one, two] })),
        ("a repeat", json!({ "soundIds": [one, one, two, three] })),
        ("a sound of nobody", json!({ "soundIds": [one, two, three, strange] })),
        ("a stranger for a sound", json!({ "soundIds": [one, two, strange] })),
        ("nothing", json!({})),
        ("null", json!({ "soundIds": null })),
        ("an empty list", json!({ "soundIds": [] })),
    ] {
        let response = order(ids).await;
        assert_eq!(response.status, 400, "{label}");
        assert_eq!(response.json::<Value>()["errors"]["soundIds"][0], "The list must contain every sound exactly once.", "{label}");
    }
    for bad in [json!({ "soundIds": [one, null] }), json!({ "soundIds": [one, "x"] }), json!({ "soundIds": "x" })] {
        assert_eq!(order(bad).await.status, 400);
    }
    assert_eq!(names().await, ["one", "two", "three"], "a refused order changes nothing");
    assert!(events(&server, "SoundboardChanged").len() == 3);

    let done = order(json!({ "soundIds": [three, one, two] })).await;

    assert_eq!(done.status, 204);
    assert!(done.body.is_empty());
    assert_eq!(names().await, ["three", "one", "two"]);
    assert_eq!(order(json!({ "soundIds": [three, one, two] })).await.status, 204);
    assert_eq!(events(&server, "SoundboardChanged").len(), 5);
}

#[tokio::test]
async fn an_empty_soundboard_has_an_empty_order() {
    let Board { server: _server, owner, .. } = board().await;

    let response = put(&owner, Method::PUT, "/api/admin/soundboard/order", json!({ "soundIds": [] })).await;

    assert_eq!(response.status, 204);
}

#[tokio::test]
async fn the_switches_change_one_at_a_time_and_say_so_in_the_audit_log() {
    let Board { server, owner, .. } = board().await;
    let settings = |body: Value| put(&owner, Method::PATCH, "/api/admin/soundboard/settings", body);

    let nothing = settings(json!({})).await;
    let first = settings(json!({ "everyoneCanPlay": false })).await;
    let second = settings(json!({ "rateLimited": false, "everyoneCanPlay": null })).await;
    let again = settings(json!({ "rateLimited": false })).await;
    let wrong = settings(json!({ "rateLimited": "no" })).await;
    let not_an_object = settings(json!([])).await;

    assert_eq!(nothing.text(), "{\"everyoneCanPlay\":true,\"rateLimited\":true}");
    assert_eq!(first.text(), "{\"everyoneCanPlay\":false,\"rateLimited\":true}");
    assert_eq!(second.text(), "{\"everyoneCanPlay\":false,\"rateLimited\":false}");
    assert_eq!(again.status, 200);
    assert_eq!((wrong.status.as_u16(), not_an_object.status.as_u16()), (400, 400));
    let db = server.database();
    let mut statement = db
        .prepare("SELECT \"Summary\", \"TargetType\", \"TargetName\", \"ChangesJson\" FROM \"ServerAuditEntries\" WHERE \"Action\" = 'soundboard.settings' ORDER BY \"CreatedAt\"")
        .unwrap();
    let rows: Vec<(String, Option<String>, Option<String>, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        [
            ("Maja changed the soundboard settings".to_string(), None, None, "[{\"field\":\"everyoneCanPlay\",\"from\":\"on\",\"to\":\"off\"}]".to_string()),
            ("Maja changed the soundboard settings".to_string(), None, None, "[{\"field\":\"rateLimited\",\"from\":\"on\",\"to\":\"off\"}]".to_string()),
        ]
    );
    assert_eq!(events(&server, "SoundboardChanged").len(), 4);
}

#[tokio::test]
async fn deleting_a_sound_takes_its_clip_and_the_join_sound_that_named_it() {
    let Board { server, owner, member } = board().await;
    let id = sound(&owner, "gong").await;
    let other = sound(&owner, "other").await;
    assert_eq!(put(&member, Method::PATCH, "/api/me/preferences", json!({ "joinSoundId": id })).await.status, 200);
    assert_eq!(put(&owner, Method::PATCH, "/api/me/preferences", json!({ "joinSoundId": other })).await.status, 200);

    let removed = owner.client.send(Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))).await;

    assert_eq!(removed.status, 204);
    let account: Value = member.client.get("/api/me").await.json();
    assert_eq!(account["preferences"]["joinSoundId"], Value::Null);
    let account: Value = owner.client.get("/api/me").await.json();
    assert_eq!(account["preferences"]["joinSoundId"], json!(other), "somebody else's choice stays");
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 1);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ObjectDeletions\""), 1);
    let again = owner.client.send(Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))).await;
    assert_eq!(again.status, 204, "a sound that is gone is gone");
    assert_eq!(events(&server, "SoundboardChanged").len(), 3, "and nothing is announced for it");
}

#[tokio::test]
async fn a_clip_that_a_message_still_carries_is_kept_when_its_sound_is_deleted() {
    let Board { server, owner, .. } = board().await;
    let clip = wave(&owner, 5.0).await;
    let created = put(&owner, Method::POST, SOUNDS, body(&clip, "gong", "g", 0.0, 1.0)).await;
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    let db = server.database();
    let channel = crate::uploads::channel_of(&db, first_server(&db));
    crate::uploads::post_message(&db, channel, &owner.id, &clip);

    let removed = owner.client.send(Request::new(Method::DELETE, &format!("{SOUNDS}/{id}"))).await;

    assert_eq!(removed.status, 204);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ImageAttachments\""), 1);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ObjectDeletions\""), 0);
    assert_eq!(owner.client.get(&format!("/assets/{clip}")).await.status, 200);
}

#[tokio::test]
async fn adding_a_sound_is_written_in_the_audit_log_and_announced_to_the_server() {
    let Board { server, owner, .. } = board().await;
    let id = sound(&owner, "Gong").await;
    let server_id = first_server(&server.database()).to_string();

    let told = events(&server, "SoundboardChanged");

    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["audience"]["server"], server_id);
    assert_eq!(told[0]["payload"], json!({ "serverId": server_id }));
    let row: (String, String, String, String, Option<String>) = server
        .database()
        .query_row(
            "SELECT \"Summary\", \"TargetType\", \"TargetId\", \"TargetName\", \"ChangesJson\" FROM \"ServerAuditEntries\" WHERE \"Action\" = 'sound.create'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert_eq!(row, ("Maja added the sound Gong".to_string(), "sound".to_string(), Id::parse(&id).unwrap().to_db_text(), "Gong".to_string(), None));
}

#[tokio::test]
async fn a_body_that_is_not_the_right_json_is_refused_and_a_big_one_is_cut_off() {
    let Board { server: _server, owner, .. } = board().await;
    let send = |method: Method, path: &str, content_type: Option<&str>, bytes: &[u8]| {
        let request = match content_type {
            Some(content_type) => Request::new(method, path).bytes(content_type, bytes.to_vec()),
            None => Request::new(method, path).untyped_body(&String::from_utf8_lossy(bytes)),
        };
        let client = &owner.client;
        let request = std::cell::Cell::new(Some(request));
        async move { client.send(request.take().unwrap()).await.status.as_u16() }
    };

    for path in [SOUNDS, "/api/admin/soundboard/settings", "/api/admin/soundboard/order"] {
        let method = if path.ends_with("order") { Method::PUT } else if path.ends_with("settings") { Method::PATCH } else { Method::POST };
        assert_eq!(send(method.clone(), path, Some("application/json"), b"{").await, 400, "{path}");
        assert_eq!(send(method.clone(), path, Some("application/json"), b"[]").await, 400, "{path}");
        assert_eq!(send(method.clone(), path, Some("application/json"), b"null").await, 400, "{path}");
        assert_eq!(send(method.clone(), path, Some("application/json"), b"").await, 400, "{path}");
        assert_eq!(send(method.clone(), path, Some("text/plain"), b"{}").await, 415, "{path}");
        assert_eq!(send(method.clone(), path, None, b"{}").await, 415, "{path}");
        let big = format!("{{\"name\":\"{}\"}}", "a".repeat(1024 * 1024));
        assert_eq!(send(method, path, Some("application/json"), big.as_bytes()).await, 413, "{path}");
    }
}

#[tokio::test]
async fn a_member_given_the_right_by_a_role_may_and_loses_it_with_the_role() {
    let Board { server, member, .. } = board().await;
    let own = wave(&member, 3.0).await;
    assert_eq!(put(&member, Method::POST, SOUNDS, body(&own, "mine", "m", 0.0, 1.0)).await.status, 403);
    give(&server, &member, 1);

    let allowed = put(&member, Method::POST, SOUNDS, body(&own, "mine", "m", 0.0, 1.0)).await;
    server.database().execute("DELETE FROM \"ServerMemberRoles\" WHERE \"UserId\" = ?1", [Id::parse(&member.id).unwrap()]).unwrap();
    let revoked = put(&member, Method::PATCH, "/api/admin/soundboard/settings", json!({ "everyoneCanPlay": false })).await;

    assert_eq!((allowed.status.as_u16(), revoked.status.as_u16()), (201, 403));
}

