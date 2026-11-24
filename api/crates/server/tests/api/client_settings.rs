//! The settings a person's client keeps on the server.

use http::Method;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::support::{Person, Request, Response, Server};
use crate::uploads::started;

const SECTIONS: [&str; 5] = ["appearance", "language", "keybinds", "voice", "privacy"];

async fn put(person: &Person, section: &str, body: &[u8]) -> Response {
    person.client.send(Request::new(Method::PUT, &format!("/api/me/settings/{section}")).bytes("application/json", body.to_vec())).await
}

async fn read(person: &Person) -> Response {
    person.client.get("/api/me/settings").await
}

fn stored(server: &Server, column: &str, user: &str) -> Option<String> {
    server
        .database()
        .query_row(&format!("SELECT \"{column}\" FROM \"UserPreferences\" p JOIN \"Users\" u ON u.\"Id\" = p.\"UserId\" WHERE u.\"Username\" = ?1"), [user], |row| row.get(0))
        .optional()
        .unwrap()
        .flatten()
}

#[tokio::test]
async fn an_account_that_never_saved_any_section_reads_every_one_as_null() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    server.database().execute("DELETE FROM \"UserPreferences\"", []).unwrap();

    let settings = read(&maja).await;

    assert_eq!(settings.status, 200);
    assert_eq!(settings.text(), "{\"appearance\":null,\"language\":null,\"keybinds\":null,\"voice\":null,\"privacy\":null}");
    assert_eq!(settings.header("content-type"), Some("application/json; charset=utf-8"));
    // Reading makes nothing, and the answer carries no word about caching.
    assert_eq!(settings.header("cache-control"), None);
    assert_eq!(settings.header("etag"), None);
    assert_eq!(server.database().query_row::<i64, _, _>("SELECT COUNT(*) FROM \"UserPreferences\"", [], |row| row.get(0)).unwrap(), 0);
}

#[tokio::test]
async fn a_section_is_replaced_whole_and_the_others_are_left_alone() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    assert_eq!(put(&maja, "appearance", br#"{ "palette": "ember", "size": "lg" }"#).await.status, 204);
    assert_eq!(put(&maja, "keybinds", br#"{ "toggleMute": null }"#).await.status, 204);

    let saved = put(&maja, "appearance", br#"{ "palette": "tide" }"#).await;

    assert_eq!(saved.status, 204);
    assert!(saved.body.is_empty());
    assert_eq!(read(&maja).await.text(), "{\"appearance\":{\"palette\":\"tide\"},\"language\":null,\"keybinds\":{\"toggleMute\":null},\"voice\":null,\"privacy\":null}");
    assert_eq!(stored(&server, "AppearanceSettingsJson", "maja").as_deref(), Some("{\"palette\":\"tide\"}"));
}

#[tokio::test]
async fn every_section_saved_at_once_by_an_account_with_no_row_is_kept() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    server.database().execute("DELETE FROM \"UserPreferences\"", []).unwrap();
    let save = |section: &'static str| put(&maja, section, br#"{ "v": 1 }"#);

    let (a, b, c, d, e) = tokio::join!(save("appearance"), save("language"), save("keybinds"), save("voice"), save("privacy"));

    for response in [a, b, c, d, e] {
        assert_eq!(response.status, 204);
    }
    let settings: Value = read(&maja).await.json();
    for section in SECTIONS {
        assert_eq!(settings[section], json!({ "v": 1 }), "{section}");
    }
}

#[tokio::test]
async fn saves_of_one_section_at_once_leave_the_last_that_was_written_whole() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let save = |who: u32| put(&maja, "voice", format!("{{\"who\":{who},\"padding\":\"{}\"}}", "x".repeat(4000)).as_bytes().to_vec().leak());

    let all = tokio::join!(save(1), save(2), save(3), save(4), save(5), save(6));

    for response in [&all.0, &all.1, &all.2, &all.3, &all.4, &all.5] {
        assert_eq!(response.status, 204);
    }
    let text = stored(&server, "VoiceSettingsJson", "maja").unwrap();
    assert!(text.starts_with("{\"who\":") && text.ends_with("\"}") && text.len() > 4000, "{}", &text[..40]);
    let read_back: Value = read(&maja).await.json();
    assert_eq!(read_back["voice"]["padding"].as_str().unwrap().len(), 4000);
}

#[tokio::test]
async fn settings_belong_to_the_account_that_saved_them_and_every_session_of_it_reads_them() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let ilya = server.register("ilya").await;
    put(&maja, "language", br#"{ "locale": "en" }"#).await;

    let theirs = read(&ilya).await;
    let again = server.sign_in(&maja).await;
    let from_another_session = again.get("/api/me/settings").await.json::<Value>();

    assert_eq!(theirs.json::<Value>()["language"], Value::Null);
    assert_eq!(from_another_session["language"], json!({ "locale": "en" }));
}

#[tokio::test]
async fn only_a_json_object_is_stored() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let message = "Settings must be a JSON object of at most 8192 characters.";
    let refused: Vec<(&str, Vec<u8>)> = vec![
        ("an array", b"[1, 2]".to_vec()),
        ("a string", b"\"dark\"".to_vec()),
        ("a number", b"1".to_vec()),
        ("null", b"null".to_vec()),
        ("true", b"true".to_vec()),
        ("broken", b"{ not json".to_vec()),
        ("nothing", Vec::new()),
        ("blank", b"   ".to_vec()),
        ("junk after", br#"{"a":1} x"#.to_vec()),
        ("two", br#"{"a":1}{"b":2}"#.to_vec()),
        ("a comment", br#"{"a":1 /* c */}"#.to_vec()),
        ("a trailing comma", br#"{"a":1,}"#.to_vec()),
        ("single quotes", b"{'a':1}".to_vec()),
        ("a name that is not a number", br#"{"a":NaN}"#.to_vec()),
        ("a leading zero", br#"{"a":01}"#.to_vec()),
        ("a bare control character", b"{\"a\":\"\x01\"}".to_vec()),
        ("a bad escape", br#"{"a":"\x"}"#.to_vec()),
        ("a short escape", br#"{"a":"\u12"}"#.to_vec()),
        ("a surrogate with no partner", br#"{"a":"\ud800"}"#.to_vec()),
        ("a byte order mark", b"\xef\xbb\xbf{}".to_vec()),
        ("a second byte order mark", b"\xef\xbb\xbf\xef\xbb\xbf{}".to_vec()),
        ("a text in UTF-16", b"\xff\xfe{\0}\0".to_vec()),
        ("bytes that are not UTF-8", b"{\"a\":\"\xff\"}".to_vec()),
    ];

    for (label, body) in refused {
        let response = put(&maja, "voice", &body).await;
        assert_eq!(response.status, 400, "{label}");
        assert_eq!(response.json::<Value>()["errors"]["voice"][0], message, "{label}");
    }
    assert_eq!(stored(&server, "VoiceSettingsJson", "maja"), None);
    for (label, body) in [
        ("spaces around", b" \n\t{ }\r\n ".to_vec()),
        ("a number as long as you like", b"{\"a\":123456789012345678901234567890,\"b\":1e999}".to_vec()),
    ] {
        assert_eq!(put(&maja, "voice", &body).await.status, 204, "{label}");
    }
}

#[tokio::test]
async fn what_is_kept_is_compact_and_written_as_the_web_client_wrote_it() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    for (sent, kept) in [
        (&br#"{ "b" : 1 ,
"a":[ true , null , "x" ], "b": {} }"#[..], r#"{"b":1,"a":[true,null,"x"],"b":{}}"#),
        (&br#"{"n":[1.0,-0,1E5,2e+3,0.10,-1.5e-7]}"#[..], r#"{"n":[1.0,-0,1E5,2e+3,0.10,-1.5e-7]}"#),
        ("{\"a\":\"é<a href='x'>&\\n\\t\\\"\\\\\\/\\u0041 \u{1f680} \u{7f} \u{2028}`+\"}".as_bytes(), "{\"a\":\"é<a href='x'>&\\n\\t\\\"\\\\/A \u{1f680} \u{7f} \u{2028}`+\"}"),
        ("{\"é<\":1}".as_bytes(), "{\"é<\":1}"),
        (&b"{\"a\":\"\\ud83d\\ude80\\u00e9\"}"[..], "{\"a\":\"\u{1f680}é\"}"),
    ] {
        let sent_text = String::from_utf8_lossy(sent).into_owned();
        assert_eq!(put(&maja, "appearance", sent).await.status, 204, "{sent_text}");
        assert_eq!(stored(&server, "AppearanceSettingsJson", "maja").as_deref(), Some(kept), "{sent_text}");
    }
}

#[tokio::test]
async fn a_section_may_be_eight_thousand_one_hundred_and_ninety_two_characters_of_what_is_kept() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let of = |filler: &str, times: usize| format!("{{\"a\":\"{}\"}}", filler.repeat(times));

    // The limit counts what is kept, in UTF-16 units as the web client counts
    // them: a letter outside ASCII is one unit, one outside the plane is two.
    let at = put(&maja, "voice", of("x", 8192 - 8).as_bytes()).await;
    let over = put(&maja, "voice", of("x", 8192 - 7).as_bytes()).await;
    let escapes_at = put(&maja, "voice", of("\u{e9}", 8192 - 8).as_bytes()).await;
    let escapes_over = put(&maja, "voice", of("\u{e9}", 8192 - 7).as_bytes()).await;
    let pairs_at = put(&maja, "voice", of("\u{1f680}", (8192 - 8) / 2).as_bytes()).await;
    let pairs_over = put(&maja, "voice", of("\u{1f680}", (8192 - 8) / 2 + 1).as_bytes()).await;
    let padded = put(&maja, "appearance", format!("{{{}\"palette\": \"ember\"}}", " ".repeat(9000)).as_bytes()).await;

    assert_eq!((at.status.as_u16(), over.status.as_u16()), (204, 400));
    assert_eq!((escapes_at.status.as_u16(), escapes_over.status.as_u16()), (204, 400));
    assert_eq!((pairs_at.status.as_u16(), pairs_over.status.as_u16()), (204, 400));
    assert_eq!(padded.status, 204, "whitespace does not count");
    assert_eq!(stored(&server, "AppearanceSettingsJson", "maja").as_deref(), Some("{\"palette\":\"ember\"}"));
    assert_eq!(stored(&server, "VoiceSettingsJson", "maja").unwrap().encode_utf16().count(), 8192);
}

#[tokio::test]
async fn a_body_is_taken_to_thirty_two_thousand_seven_hundred_and_sixty_eight_and_no_further() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let padded = |length: usize| [&br#"{"a":1}"#[..], &vec![b' '; length - 7][..]].concat();

    let at = put(&maja, "voice", &padded(32768)).await;
    let over = put(&maja, "voice", &padded(32769)).await;
    let big = put(&maja, "voice", &[&br#"{"a":""#[..], &vec![b'x'; 200_000][..], &br#""}"#[..]].concat()).await;
    let huge = put(&maja, "voice", &[&br#"{"a":""#[..], &vec![b'x'; 2 * 1024 * 1024][..], &br#""}"#[..]].concat()).await;

    assert_eq!(at.status, 204);
    for refused in [over, big, huge] {
        assert_eq!(refused.status, 400);
        assert_eq!(refused.json::<Value>()["errors"]["voice"][0], "Settings must be a JSON object of at most 8192 characters.");
    }
}

#[tokio::test]
async fn nesting_may_be_sixty_four_deep_and_no_deeper() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    let objects = |levels: usize| format!("{}1{}", "{\"a\":".repeat(levels), "}".repeat(levels));
    let arrays = |levels: usize| format!("{{\"a\":{}{}}}", "[".repeat(levels), "]".repeat(levels));

    assert_eq!(put(&maja, "voice", objects(64).as_bytes()).await.status, 204);
    assert_eq!(put(&maja, "voice", objects(65).as_bytes()).await.status, 400);
    assert_eq!(put(&maja, "language", arrays(63).as_bytes()).await.status, 204);
    assert_eq!(put(&maja, "language", arrays(64).as_bytes()).await.status, 400);
}

#[tokio::test]
async fn a_section_that_is_damaged_in_the_database_reads_as_never_saved() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;
    put(&maja, "voice", br#"{"a":1}"#).await;
    server
        .database()
        .execute("UPDATE \"UserPreferences\" SET \"AppearanceSettingsJson\" = '[1]', \"LanguageSettingsJson\" = '{broken', \"KeybindSettingsJson\" = '', \"PrivacySettingsJson\" = '{ \"é\" : 1 }'", [])
        .unwrap();

    let settings = read(&maja).await;

    assert_eq!(settings.text(), "{\"appearance\":null,\"language\":null,\"keybinds\":null,\"voice\":{\"a\":1},\"privacy\":{\"é\":1}}");
}

#[tokio::test]
async fn only_the_five_sections_exist_and_by_their_exact_names() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for section in SECTIONS {
        assert_eq!(put(&maja, section, b"{}").await.status, 204, "{section}");
    }
    for section in ["Appearance", "VOICE", "passwords", "settings", "voice%20", "a%2Fb", "voice%2F"] {
        let response = put(&maja, section, b"{}").await;
        assert_eq!(response.status, 404, "{section}");
        assert!(response.body.is_empty());
    }
    // The section is looked at before the body is read.
    assert_eq!(put(&maja, "passwords", b"{ not json").await.status, 404);
    assert_eq!(put(&maja, "%76oice", b"{}").await.status, 204, "an escape is the character it stands for");
    for method in [Method::POST, Method::PATCH, Method::DELETE] {
        let response = maja.client.send(Request::new(method, "/api/me/settings/voice").bytes("application/json", b"{}".to_vec())).await;
        assert_eq!(response.status, 405);
    }
}

#[tokio::test]
async fn the_body_need_not_say_what_it_is_and_may_arrive_in_pieces() {
    let server = started(&[]).await;
    let maja = server.found("maja").await;

    for content_type in ["text/plain", "application/x-www-form-urlencoded", "application/json; charset=utf-16"] {
        let response = maja.client.send(Request::new(Method::PUT, "/api/me/settings/privacy").bytes(content_type, br#"{"giphy":false}"#.to_vec())).await;
        assert_eq!(response.status, 204, "{content_type}");
    }
    let untyped = maja.client.send(Request::new(Method::PUT, "/api/me/settings/language").untyped_body(r#"{"locale":"pl"}"#)).await;
    assert_eq!(untyped.status, 204);
    assert_eq!(read(&maja).await.json::<Value>()["language"], json!({ "locale": "pl" }));
}

#[tokio::test]
async fn nobody_reads_or_writes_settings_without_signing_in() {
    let server = started(&[]).await;
    let anonymous = server.client_of("web", "2.0.0");

    assert_eq!(anonymous.get("/api/me/settings").await.status, 401);
    let put = anonymous.send(Request::new(Method::PUT, "/api/me/settings/voice").bytes("application/json", b"{}".to_vec())).await;
    assert_eq!(put.status, 401);
}
