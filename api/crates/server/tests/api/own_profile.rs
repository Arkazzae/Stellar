//! What a person keeps for themselves on their own account: how their profile
//! reads, their preferences and the sound that plays when they join a voice
//! room.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Id;

use crate::support::{HubClient, Person, Request, Response, Server};
use crate::uploads::{Form, WAVE, first_server, started};

const COOKIE: &str = "stellarr-dev-session";

async fn patch(person: &Person, path: &str, body: Value) -> Response {
    person.client.send(Request::new(Method::PATCH, path).json(&body)).await
}

async fn me(person: &Person) -> Value {
    person.client.get("/api/me").await.json()
}

fn events(server: &Server, name: &str) -> Vec<Value> {
    server.events().into_iter().filter(|event| event["name"] == name).collect()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

async fn wave(person: &Person) -> String {
    let response = person
        .client
        .send(Form::new().file("file", "s.wav", "audio/wav", WAVE).text("durationSeconds", "20").post("/api/uploads/images"))
        .await;
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

/// A sound on the server the request is about (the person's first, unless a
/// server is named).
async fn sound_on(person: &Person, server: Option<&str>, name: &str, start: f64, end: f64) -> String {
    let clip = wave(person).await;
    let mut request = Request::new(Method::POST, "/api/admin/soundboard/sounds")
        .json(&json!({ "attachmentId": clip, "name": name, "glyph": "g", "trimStartSeconds": start, "trimEndSeconds": end }));
    if let Some(server) = server {
        request = request.header("X-Stellarr-Server", server);
    }
    let response = person.client.send(request).await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn changing_the_email_is_refused_and_nothing_else_in_the_request_is_applied() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    let refused = patch(&maja, "/api/me", json!({ "email": "other@example.test", "displayName": "Changed" })).await;
    let blank = patch(&maja, "/api/me", json!({ "email": "" })).await;
    let null = patch(&maja, "/api/me", json!({ "email": null, "bio": "kept" })).await;

    assert_eq!(refused.status, 400);
    assert_eq!(refused.header("content-type"), Some("application/problem+json"));
    assert_eq!(refused.json::<Value>()["errors"]["email"][0], "The address changes on its own, through a link sent to the new address.");
    assert_eq!(blank.status, 400, "an empty address is an address");
    assert_eq!(null.status, 200);
    let account = me(&maja).await;
    assert_eq!(account["displayName"], "Maja");
    assert_eq!(account["email"], "maja@example.test");
    assert_eq!(account["bio"], "kept");
}

#[tokio::test]
async fn empty_profile_text_is_stored_as_nothing_and_a_field_left_out_is_kept() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    patch(&maja, "/api/me", json!({ "bio": " About me ", "customStatus": " busy ", "pronouns": " she/her " })).await;

    let kept = patch(&maja, "/api/me", json!({ "displayName": "  Maja Nowak  " })).await;
    let cleared = patch(&maja, "/api/me", json!({ "bio": "  ", "customStatus": "", "pronouns": "\t" })).await;

    let account: Value = kept.json();
    assert_eq!(
        (account["displayName"].clone(), account["bio"].clone(), account["customStatus"].clone(), account["pronouns"].clone()),
        (json!("Maja Nowak"), json!("About me"), json!("busy"), json!("she/her"))
    );
    let account: Value = cleared.json();
    assert_eq!((account["bio"].clone(), account["customStatus"].clone(), account["pronouns"].clone()), (json!(""), json!(""), json!("")));
    let stored: (Option<String>, Option<String>, Option<String>) = server
        .database()
        .query_row("SELECT \"Bio\", \"CustomStatus\", \"Pronouns\" FROM \"Users\" WHERE \"Username\" = 'maja'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    assert_eq!(stored, (None, None, None));
}

#[tokio::test]
async fn every_field_of_the_profile_is_counted_after_trimming_in_utf16_units() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for (field, limit) in [("displayName", 64), ("bio", 2000), ("customStatus", 128), ("pronouns", 32)] {
        let at = patch(&maja, "/api/me", json!({ field: format!("  {}  ", "a".repeat(limit)) })).await;
        let over = patch(&maja, "/api/me", json!({ field: "a".repeat(limit + 1) })).await;
        let emoji_at = patch(&maja, "/api/me", json!({ field: "\u{1f680}".repeat(limit / 2) })).await;
        let emoji_over = patch(&maja, "/api/me", json!({ field: "\u{1f680}".repeat(limit / 2 + 1) })).await;
        assert_eq!((at.status.as_u16(), over.status.as_u16(), emoji_at.status.as_u16(), emoji_over.status.as_u16()), (200, 400, 200, 400), "{field}");
        assert!(over.json::<Value>()["errors"][field][0].as_str().unwrap().contains(&limit.to_string()), "{field}");
    }
    let empty = patch(&maja, "/api/me", json!({ "displayName": " \u{a0}\u{3000} " })).await;
    assert_eq!(empty.json::<Value>()["errors"]["displayName"][0], "Display name must be 1 to 64 characters long.");
    // Every refusal at once, in the order the first server listed them.
    let all = patch(&maja, "/api/me", json!({ "pronouns": "p".repeat(33), "customStatus": "s".repeat(129), "bio": "b".repeat(2001), "displayName": "" })).await;
    let errors = all.text();
    let at = |name: &str| errors.find(&format!("\"{name}\":")).unwrap();
    assert!(at("displayName") < at("bio") && at("bio") < at("customStatus") && at("customStatus") < at("pronouns"), "{errors}");
    // Nothing of a refused request is applied.
    let account = me(&maja).await;
    assert_ne!(account["bio"].as_str().unwrap().len(), 2001);
}

#[tokio::test]
async fn a_change_of_the_profile_is_told_to_everybody_and_to_the_person_as_they_see_themselves() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    patch(&maja, "/api/me/preferences", json!({ "showPresence": false })).await;

    let response = patch(&maja, "/api/me", json!({ "displayName": "Maja N", "bio": "hi" })).await;
    let nothing = patch(&maja, "/api/me", json!({})).await;
    let refused = patch(&maja, "/api/me", json!({ "displayName": "" })).await;

    assert_eq!((response.status.as_u16(), nothing.status.as_u16(), refused.status.as_u16()), (200, 200, 400));
    let told = events(&server, "UserUpdated");
    assert_eq!(told.len(), 4, "two for each save that was kept, none for the refused one");
    assert_eq!(told[0]["audience"], "everyone");
    assert_eq!(told[1]["audience"]["users"], json!([maja.id]));
    assert_eq!(told[2]["audience"], "everyone");
    assert_eq!(told[0]["payload"]["displayName"], "Maja N");
    assert_eq!(told[0]["payload"]["bio"], "hi");
    assert_eq!(told[0]["payload"]["id"], json!(maja.id));
    assert_eq!(told[0]["payload"]["customStatus"], Value::Null);
}

#[tokio::test]
async fn an_account_that_never_saved_a_preference_has_the_defaults_and_changes_only_what_was_sent() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    server.database().execute("DELETE FROM \"UserPreferences\"", []).unwrap();
    let defaults = json!({
        "desktopNotifications": true, "emailNotifications": false, "pushNotifications": false, "mentionsOnly": true, "messageSound": true,
        "joinLeaveSound": true, "allowDirectMessages": true, "allowFriendRequests": true, "showPresence": true, "developerMode": false,
        "reducedMotion": false, "mutedChannelIds": [], "vadSensitivity": 60, "joinSoundId": null, "mentionPopups": true, "callRingtone": true,
    });
    assert_eq!(me(&maja).await["preferences"], defaults);

    let changed = patch(&maja, "/api/me/preferences", json!({ "emailNotifications": true, "mentionsOnly": false, "developerMode": null })).await;

    assert_eq!(changed.status, 200);
    let mut expected = defaults.clone();
    expected["emailNotifications"] = json!(true);
    expected["mentionsOnly"] = json!(false);
    assert_eq!(changed.json::<Value>(), expected);
    assert_eq!(me(&maja).await["preferences"], expected);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"UserPreferences\""), 1);
    // The members come in the order of the first server's answer.
    let text = changed.text();
    assert!(text.starts_with("{\"desktopNotifications\":true,\"emailNotifications\":true,\"pushNotifications\":false,\"mentionsOnly\":false,"), "{text}");
    assert!(text.ends_with("\"vadSensitivity\":60,\"joinSoundId\":null,\"mentionPopups\":true,\"callRingtone\":true}"), "{text}");
}

#[tokio::test]
async fn every_switch_changes_alone_and_the_two_that_are_newer_apart_from_call_popups() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for (name, default) in [
        ("desktopNotifications", true), ("emailNotifications", false), ("pushNotifications", false), ("mentionsOnly", true), ("messageSound", true),
        ("joinLeaveSound", true), ("allowDirectMessages", true), ("allowFriendRequests", true), ("showPresence", true), ("developerMode", false),
        ("reducedMotion", false), ("mentionPopups", true), ("callRingtone", true),
    ] {
        let response = patch(&maja, "/api/me/preferences", json!({ name: !default })).await;
        let preferences: Value = response.json();
        assert_eq!(preferences[name], json!(!default), "{name}");
        patch(&maja, "/api/me/preferences", json!({ name: default })).await;
    }
    for bad in [json!("true"), json!(1), json!("false"), json!(0), json!([])] {
        assert_eq!(patch(&maja, "/api/me/preferences", json!({ "developerMode": bad })).await.status, 400);
    }
}

#[tokio::test]
async fn the_sensitivity_of_speech_detection_is_a_whole_number_from_nothing_to_a_hundred() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let message = "Speech detection sensitivity must be between 0 and 100.";

    for (sent, kept) in [(json!(0), 0), (json!(100), 100), (json!("42"), 42), (json!("+7"), 7), (json!("07"), 7)] {
        let response = patch(&maja, "/api/me/preferences", json!({ "vadSensitivity": sent })).await;
        assert_eq!(response.json::<Value>()["vadSensitivity"], json!(kept), "{sent}");
    }
    for refused in [json!(101), json!(-1), json!(2147483647), json!("101")] {
        let response = patch(&maja, "/api/me/preferences", json!({ "vadSensitivity": refused })).await;
        assert_eq!(response.status, 400, "{refused}");
        assert_eq!(response.json::<Value>()["errors"]["vadSensitivity"][0], message, "{refused}");
    }
    for unreadable in [json!(50.5), json!(5.0), json!(1e1), json!("5.0"), json!("1e1"), json!(" 5"), json!(true), json!(2147483648_i64), json!("x")] {
        assert_eq!(patch(&maja, "/api/me/preferences", json!({ "vadSensitivity": unreadable })).await.status, 400, "{unreadable}");
    }
    assert_eq!(patch(&maja, "/api/me/preferences", json!({ "vadSensitivity": null })).await.json::<Value>()["vadSensitivity"], json!(7));
}

#[tokio::test]
async fn muted_channels_are_kept_once_each_in_the_order_they_came() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let (a, b) = (Id::random().to_string(), Id::random().to_string());

    let response = patch(&maja, "/api/me/preferences", json!({ "mutedChannelIds": [b, a, b, a] })).await;

    assert_eq!(response.json::<Value>()["mutedChannelIds"], json!([b, a]));
    let stored: String = server.database().query_row("SELECT \"MutedChannelIdsJson\" FROM \"UserPreferences\" WHERE \"MutedChannelIdsJson\" <> '[]'", [], |row| row.get(0)).unwrap();
    assert_eq!(stored, format!("[\"{b}\",\"{a}\"]"));
    assert_eq!(patch(&maja, "/api/me/preferences", json!({})).await.json::<Value>()["mutedChannelIds"], json!([b, a]));
    for bad in [json!(["x"]), json!([a.replace('-', "")]), json!([format!("{{{a}}}")]), json!([null]), json!([5]), json!("x")] {
        assert_eq!(patch(&maja, "/api/me/preferences", json!({ "mutedChannelIds": bad })).await.status, 400);
    }
    assert_eq!(patch(&maja, "/api/me/preferences", json!({ "mutedChannelIds": [] })).await.json::<Value>()["mutedChannelIds"], json!([]));
}

#[tokio::test]
async fn a_join_sound_is_one_the_account_can_already_play_and_blank_clears_it() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let ilya = server.register("ilya").await;
    let sound = sound_on(&maja, None, "Fanfara", 1.0, 12.0).await;
    let message = "Pick a sound from the soundboard of a server you are on.";

    let chosen = patch(&ilya, "/api/me/preferences", json!({ "joinSoundId": format!("  {{{sound}}}  ") })).await;
    assert_eq!(chosen.json::<Value>()["joinSoundId"], json!(sound));
    let left_alone = patch(&ilya, "/api/me/preferences", json!({ "developerMode": true })).await;
    let nulled = patch(&ilya, "/api/me/preferences", json!({ "joinSoundId": null })).await;
    assert_eq!(left_alone.json::<Value>()["joinSoundId"], json!(sound), "absent changes nothing");
    assert_eq!(nulled.json::<Value>()["joinSoundId"], json!(sound), "so does null");
    for refused in [json!(Id::random().to_string()), json!("x"), json!("{x}"), json!("not-a-guid-at-all")] {
        let response = patch(&ilya, "/api/me/preferences", json!({ "joinSoundId": refused })).await;
        assert_eq!(response.status, 400, "{refused}");
        assert_eq!(response.json::<Value>()["errors"]["joinSoundId"][0], message);
    }
    assert_eq!(me(&ilya).await["preferences"]["joinSoundId"], json!(sound), "a refused request changes nothing");
    let cleared = patch(&ilya, "/api/me/preferences", json!({ "joinSoundId": "   " })).await;
    assert_eq!(cleared.json::<Value>()["joinSoundId"], Value::Null);
    // Somebody who is on no server with that sound may not choose it.
    let stranger = server.register("sam").await;
    server.database().execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [Id::parse(&stranger.id).unwrap()]).unwrap();
    let refused = patch(&stranger, "/api/me/preferences", json!({ "joinSoundId": sound })).await;
    assert_eq!(refused.status, 400, "a sound is no sound of a person who is on no server");
    assert_eq!(refused.json::<Value>()["errors"]["joinSoundId"][0], message);
}

#[tokio::test]
async fn the_first_refusal_is_the_sensitivity_and_a_refused_request_makes_no_row() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    server.database().execute("DELETE FROM \"UserPreferences\"", []).unwrap();

    let both = patch(&maja, "/api/me/preferences", json!({ "vadSensitivity": 101, "joinSoundId": "x" })).await;
    let sound = patch(&maja, "/api/me/preferences", json!({ "joinSoundId": "x", "vadSensitivity": 5 })).await;

    assert_eq!(both.json::<Value>()["errors"].as_object().unwrap().keys().collect::<Vec<_>>(), ["vadSensitivity"]);
    assert_eq!(sound.json::<Value>()["errors"].as_object().unwrap().keys().collect::<Vec<_>>(), ["joinSoundId"]);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"UserPreferences\""), 0);
    let empty = patch(&maja, "/api/me/preferences", json!({})).await;
    assert_eq!(empty.status, 200);
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"UserPreferences\""), 1, "an empty request still makes the row");
}

#[tokio::test]
async fn hiding_presence_tells_the_servers_the_person_is_on_and_the_person_what_they_still_see() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let ilya = server.register("ilya").await;
    let first = first_server(&server.database()).to_string();
    let cookie = format!("{COOKIE}={}", maja.client.cookie(COOKIE).unwrap());
    let connected = HubClient::connect(server.address(), &cookie).await;
    let before = events(&server, "PresenceChanged").len();

    patch(&maja, "/api/me/preferences", json!({ "showPresence": false })).await;
    patch(&maja, "/api/me/preferences", json!({ "showPresence": false })).await;
    patch(&maja, "/api/me/preferences", json!({ "developerMode": true })).await;
    patch(&maja, "/api/me/preferences", json!({ "showPresence": true })).await;

    let told: Vec<Value> = events(&server, "PresenceChanged").split_off(before);
    assert_eq!(told.len(), 4, "{told:?}");
    assert_eq!(
        (told[0]["audience"]["reach"]["servers"].clone(), told[0]["payload"].clone()),
        (json!([first]), json!({ "userId": maja.id, "presence": "offline" }))
    );
    assert_eq!((told[1]["audience"]["users"].clone(), told[1]["payload"].clone()), (json!([maja.id]), json!({ "userId": maja.id, "presence": "online" })));
    assert_eq!(told[2]["payload"]["presence"], "online");
    assert_eq!(told[3]["payload"]["presence"], "online");
    drop((connected, ilya));
}

#[tokio::test]
async fn presence_is_not_told_for_a_person_who_is_not_connected() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    patch(&maja, "/api/me/preferences", json!({ "showPresence": false })).await;

    assert!(events(&server, "PresenceChanged").is_empty());
    assert_eq!(me(&maja).await["preferences"]["showPresence"], json!(false));
}

#[tokio::test]
async fn the_library_of_join_sounds_is_every_servers_sounds_cut_to_the_window_of_an_arrival() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let ilya = server.register("ilya").await;
    let long = sound_on(&maja, None, "Fanfara", 1.0, 12.0).await;
    let short = sound_on(&maja, None, "Kliknięcie", 0.5, 2.0).await;
    let second = maja.client.send(Request::new(Method::POST, "/api/servers").json(&json!({ "name": "Alpha" }))).await;
    let second = second.json::<Value>()["id"].as_str().unwrap().to_string();
    let beta = maja.client.send(Request::new(Method::POST, "/api/servers").json(&json!({ "name": "Beta" }))).await;
    assert_eq!(beta.status, 201);
    let far = sound_on(&maja, Some(&second), "Daleki", 0.0, 3.0).await;

    let library = ilya.client.get("/api/me/join-sound").await;
    let mine = maja.client.get("/api/me/join-sound").await;
    assert!(mine.text().starts_with("{\"maxSeconds\":5,\"selectedId\":null,\"servers\":[{\"serverId\":\""), "{}", mine.text());

    let ilya_sees: Value = library.json();
    assert_eq!(ilya_sees["maxSeconds"], json!(5));
    assert_eq!(ilya_sees["selectedId"], Value::Null);
    let servers = ilya_sees["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 1, "a server with no sounds, and one the person is not on, are left out");
    let sounds = servers[0]["sounds"].as_array().unwrap();
    assert_eq!(sounds[0]["id"], json!(long));
    // Eleven seconds on the soundboard, five in a doorway.
    assert_eq!((sounds[0]["trimStartSeconds"].clone(), sounds[0]["trimEndSeconds"].clone()), (json!(1), json!(6)));
    assert_eq!((sounds[1]["id"].clone(), sounds[1]["trimEndSeconds"].clone()), (json!(short), json!(2)));
    assert_eq!(sounds[0]["durationSeconds"], json!(20));
    let mine: Value = mine.json();
    let names: Vec<&str> = mine["servers"].as_array().unwrap().iter().map(|server| server["serverName"].as_str().unwrap()).collect();
    assert_eq!(names, ["Alpha", "Stellarr"], "by the name of the server");
    assert_eq!(mine["servers"][0]["sounds"][0]["id"], json!(far));
    assert_eq!(mine["servers"][0]["serverId"], json!(second));
    assert_eq!(mine["servers"][0]["iconUrl"], Value::Null);
}

#[tokio::test]
async fn a_selection_whose_server_has_been_left_is_still_reported() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let ilya = server.register("ilya").await;
    let first = first_server(&server.database());
    let sound = sound_on(&maja, None, "Fanfara", 0.0, 2.0).await;
    patch(&ilya, "/api/me/preferences", json!({ "joinSoundId": sound })).await;
    let other = maja.client.send(Request::new(Method::POST, "/api/servers").json(&json!({ "name": "Other" }))).await;
    assert_eq!(other.status, 201);
    server
        .database()
        .execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1 AND \"ServerId\" = ?2", params![Id::parse(&ilya.id).unwrap(), first])
        .unwrap();
    let elsewhere = ilya.client.send(Request::new(Method::GET, "/api/me/join-sound")).await;

    assert_eq!(elsewhere.status, 200);
    let library: Value = elsewhere.json();
    assert_eq!(library["selectedId"], json!(sound));
    assert_eq!(library["servers"], json!([]));
}

#[tokio::test]
async fn nobody_changes_an_account_without_signing_in_and_a_body_must_be_json() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let anonymous = server.client_of("web", "2.0.0");

    for (method, path) in [(Method::PATCH, "/api/me"), (Method::PATCH, "/api/me/preferences"), (Method::GET, "/api/me/join-sound")] {
        assert_eq!(anonymous.send(Request::new(method.clone(), path).json(&json!({}))).await.status, 401, "{path}");
        if method != Method::GET {
            assert_eq!(maja.client.send(Request::new(method.clone(), path).body("text/plain", "{}")).await.status, 415, "{path}");
            assert_eq!(maja.client.send(Request::new(method.clone(), path).bytes("application/json", b"{".to_vec())).await.status, 400, "{path}");
            assert_eq!(maja.client.send(Request::new(method.clone(), path).bytes("application/json", b"[]".to_vec())).await.status, 400, "{path}");
            assert_eq!(maja.client.send(Request::new(method.clone(), path).bytes("application/json", b"null".to_vec())).await.status, 400, "{path}");
            let big = format!("{{\"bio\":\"{}\"}}", "a".repeat(1024 * 1024));
            assert_eq!(maja.client.send(Request::new(method.clone(), path).bytes("application/json", big.into_bytes())).await.status, 413, "{path}");
        }
    }
}
