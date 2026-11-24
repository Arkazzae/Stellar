//! What every route accepts and refuses by size and shape: each value at its
//! exact length, one byte short and one byte long, in a wrong encoding, empty
//! and null; the body ceiling; the number of devices; the PIN attempts.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use super::kit::*;
use crate::support::{Request, Server};

const SET_UP: &str = "/api/me/encryption/account-key";

/// Whether the account-key route took the request as a request: 409 says its
/// values were fine and an account key is there already, 400 that one was not.
async fn fits(crowd: &Crowd, body: &Value) -> bool {
    let response = post(&crowd.maja.client, SET_UP, body).await;
    match response.status.as_u16() {
        409 => true,
        400 => false,
        other => panic!("{other}: {} for {body}", response.text()),
    }
}

fn with_backup(change: impl Fn(&mut Value)) -> Value {
    let mut body = set_up_body(&bytes(32, 5));
    change(&mut body["backup"]);
    body
}

#[tokio::test]
async fn every_part_of_a_key_and_a_backup_is_checked_at_its_length_before_it_is_stored() {
    let crowd = Crowd::new().await;
    set_up(&crowd.maja.client).await;
    let sized = |field: &str, length: usize| with_backup(|backup| backup[field] = json!(text(&bytes(length, 1))));

    for (field, good, bad) in [
        ("salt", vec![16, 17, 64], vec![0, 15, 65]),
        ("verifier", vec![32], vec![0, 31, 33]),
        ("iv", vec![12], vec![0, 11, 13]),
        ("ciphertext", vec![17, 64, 1024], vec![0, 16, 1025]),
    ] {
        for length in good {
            assert!(fits(&crowd, &sized(field, length)).await, "{field} of {length} bytes");
        }
        for length in bad {
            assert!(!fits(&crowd, &sized(field, length)).await, "{field} of {length} bytes");
        }
    }
    for iterations in [600_000, 600_001, 10_000_000] {
        assert!(fits(&crowd, &with_backup(|b| b["iterations"] = json!(iterations))).await, "{iterations}");
    }
    for iterations in [0, -1, 599_999, 10_000_001] {
        assert!(!fits(&crowd, &with_backup(|b| b["iterations"] = json!(iterations))).await, "{iterations}");
    }

    let key = |length: usize| {
        let mut body = set_up_body(&bytes(32, 5));
        body["publicKey"] = json!(text(&bytes(length, 4)));
        body
    };
    assert!(fits(&crowd, &key(32)).await);
    for length in [0, 1, 31, 33, 64] {
        assert!(!fits(&crowd, &key(length)).await, "a key of {length} bytes");
    }
    for algorithm in [json!("X25519"), json!("rsa"), json!(""), json!("x25519 "), Value::Null] {
        let mut body = set_up_body(&bytes(32, 5));
        body["algorithm"] = algorithm.clone();
        assert!(!fits(&crowd, &body).await, "{algorithm}");
    }
    let mut missing = set_up_body(&bytes(32, 5));
    missing.as_object_mut().unwrap().remove("backup");
    assert!(!fits(&crowd, &missing).await);
    for part in ["salt", "iterations", "verifier", "iv", "ciphertext"] {
        assert!(!fits(&crowd, &with_backup(|b| { b.as_object_mut().unwrap().remove(part); })).await, "no {part}");
        assert!(!fits(&crowd, &with_backup(|b| b[part] = Value::Null)).await, "{part} null");
    }
    assert!(!fits(&crowd, &with_backup(|b| *b = Value::Null)).await);
}

#[tokio::test]
async fn a_value_in_a_wrong_encoding_makes_the_whole_body_unreadable_before_anything_is_checked() {
    let crowd = Crowd::new().await;
    set_up(&crowd.maja.client).await;
    let key = text(&bytes(32, 4));
    let body = |public_key: &str| {
        format!(
            "{{\"algorithm\":\"x25519\",\"publicKey\":{public_key},\"backup\":{}}}",
            backup(&bytes(32, 5))
        )
    };
    let send = |raw: String| {
        let client = &crowd.maja.client;
        async move {
            let response = client
                .send(Request::new(Method::POST, SET_UP).body("application/json", &raw))
                .await;
            (response.status.as_u16(), response.body.is_empty())
        }
    };

    // Taken as the bytes they are: spaces, tabs and line breaks are left out.
    for spelled in [
        format!("\"{} {}\"", &key[..10], &key[10..]),
        format!("\"{}\\n{}\"", &key[..10], &key[10..]),
        format!("\"{}\\t{}\"", &key[..10], &key[10..]),
        format!("\"{}\\r{}\"", &key[..10], &key[10..]),
        format!("\" {key} \""),
    ] {
        assert_eq!(send(body(&spelled)).await, (409, false), "{spelled}");
    }
    // Refused as unreadable: a 400 with no body.
    for spelled in [
        format!("\"{}\\u000b{}\"", &key[..10], &key[10..]),
        format!("\"{}\"", key.trim_end_matches('=')),
        format!("\"{key}=\""),
        "\"AAAA=AAA\"".to_string(),
        format!("\"{}B=\"", &text(&[0u8; 32])[..42]),
        format!("\"{}\"", key.replace('+', "-").replace('/', "_").replace("==", "")),
        "5".to_string(),
        "true".to_string(),
        "[1,2]".to_string(),
        "{}".to_string(),
    ] {
        let (status, empty) = send(body(&spelled)).await;
        assert_eq!((status, empty), (400, true), "{spelled}");
    }
    // Readable and not a key: a validation problem.
    for spelled in ["null", "\"\"", "\"   \""] {
        assert_eq!(send(body(spelled)).await, (400, false), "{spelled}");
    }
    // What the first server read in an integer: digits in a string are fine,
    // a number with a fraction or an exponent is not.
    let with_iterations = |iterations: &str| {
        format!(
            "{{\"algorithm\":\"x25519\",\"publicKey\":\"{key}\",\"backup\":{{\"salt\":\"{}\",\"iterations\":{iterations},\"verifier\":\"{}\",\"iv\":\"{}\",\"ciphertext\":\"{}\"}}}}",
            text(&bytes(16, 1)), text(&bytes(32, 1)), text(&bytes(12, 1)), text(&bytes(64, 1))
        )
    };
    for iterations in ["600000", "\"600000\"", "\"+600000\"", "\"0600000\""] {
        assert_eq!(send(with_iterations(iterations)).await, (409, false), "{iterations}");
    }
    for iterations in ["600000.0", "6e5", "\" 600000\"", "\"600000 \"", "\"\"", "\"abc\"", "true", "1e400", "600000000000", "null"] {
        let (status, empty) = send(with_iterations(iterations)).await;
        assert_eq!(status, 400, "{iterations}");
        assert_eq!(empty, iterations != "null", "{iterations}");
    }
    for raw in ["", "null", "[]", "5", "\"x\"", "{", "{} x", "{\"algorithm\":}"] {
        assert_eq!(send(raw.to_string()).await, (400, true), "{raw:?}");
    }
    // A member named twice is the last one, and a byte order mark is not part of the body.
    let twice = format!("{{\"algorithm\":\"rsa\",{}", &body(&format!("\"{key}\""))[1..]);
    assert_eq!(send(twice).await, (409, false));
    assert_eq!(send(format!("\u{feff}{}", body(&format!("\"{key}\"")))).await, (409, false));
}

#[tokio::test]
async fn a_body_over_the_ceiling_is_refused_before_it_is_read() {
    let crowd = Crowd::new().await;
    let big = format!("{{\"label\":\"{}\"}}", "a".repeat(1024 * 1024));
    let response = crowd
        .maja
        .client
        .send(Request::new(Method::POST, &format!("{ACCOUNT}/devices")).body("application/json", &big))
        .await;
    assert_eq!(response.status, 413);
    let small = r#"{"label":"a"}"#;
    let response = crowd
        .maja
        .client
        .send(Request::new(Method::POST, &format!("{ACCOUNT}/devices")).body("application/json", small))
        .await;
    assert_eq!(response.status, 400);
    let wrong_type = crowd
        .maja
        .client
        .send(Request::new(Method::POST, &format!("{ACCOUNT}/devices")).body("text/plain", small))
        .await;
    assert_eq!(wrong_type.status, 415);
}

#[tokio::test]
async fn a_device_has_a_key_that_fits_and_a_label_of_one_to_a_hundred_characters() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    set_up(maja).await;
    async fn register(maja: &crate::support::Client, key: Value, label: Value) -> crate::support::Response {
        post(maja, &format!("{ACCOUNT}/devices"), &json!({ "algorithm": "x25519", "publicKey": key, "label": label })).await
    }
    let key = json!(text(&bytes(32, 40)));

    assert_eq!(register(maja, key.clone(), json!("a")).await.status, 201);
    assert_eq!(register(maja, key.clone(), json!("a".repeat(100))).await.status, 200);
    // A hundred characters of two code units each is two hundred of them.
    let long = register(maja, key.clone(), json!("\u{1F600}".repeat(51))).await;
    assert_eq!(long.status, 400);
    assert!(long.text().contains("\"label\":[\"The device name must be 1 to 100 characters long.\"]"), "{}", long.text());
    assert_eq!(register(maja, key.clone(), json!("\u{1F600}".repeat(50))).await.status, 200);
    for label in [json!(""), json!("   "), json!("\t\n"), json!("\u{a0}"), json!("a".repeat(101)), Value::Null, json!(" ".repeat(10) + &"a".repeat(101))] {
        assert_eq!(register(maja, key.clone(), label.clone()).await.status, 400, "{label}");
    }
    // The label is stored without the space around it, and counted without it.
    let spaced = register(maja, key.clone(), json!(format!("  {}  ", "b".repeat(100)))).await;
    assert_eq!(spaced.status, 200);
    assert_eq!(spaced.json::<Value>()["label"], "b".repeat(100).as_str());
    for bad in [text(&bytes(31, 40)), text(&bytes(33, 40)), String::new()] {
        assert_eq!(register(maja, json!(bad), json!("a")).await.status, 400);
    }
    assert_eq!(register(maja, Value::Null, json!("a")).await.status, 400);
    let not_text = post(maja, &format!("{ACCOUNT}/devices"), &json!({ "algorithm": 5, "publicKey": key, "label": "a" })).await;
    assert_eq!((not_text.status.as_u16(), not_text.body.is_empty()), (400, true));
}

/// Gives `user` this many more devices, each in a session of its own.
pub fn seed_devices(server: &Server, user: &str, count: usize) {
    let user = Id::parse(user).unwrap();
    let now = Timestamp::now();
    let later = now.plus(std::time::Duration::from_secs(86_400));
    let connection = server.database();
    for index in 0..count {
        let session = Id::random();
        connection
            .execute(
                "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
                     \"AbsoluteExpiresAt\", \"PrimarySource\", \"AuthenticatedAt\", \"SecurityVersion\", \"ShortLived\")
                 VALUES (?1, ?2, 'Seeded', '203.0.113.7', ?3, ?3, ?4, ?4, 'password', ?3, (SELECT \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?2), 0)",
                params![session, user, now, later],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"EncryptionDevices\" (\"Id\", \"UserId\", \"SessionId\", \"Algorithm\", \"PublicKey\", \"Label\", \"CreatedAt\", \"LastSeenAt\")
                 VALUES (?1, ?2, ?3, 'x25519', ?4, 'Seeded', ?5, ?6)",
                params![
                    Id::random(),
                    user,
                    session,
                    bytes(32, index as u8),
                    now.minus(std::time::Duration::from_secs(1 + index as u64)),
                    now
                ],
            )
            .unwrap();
    }
}

#[tokio::test]
async fn an_account_holds_keys_in_a_bounded_number_of_browsers() {
    let crowd = Crowd::new().await;
    let maja = &crowd.maja.client;
    set_up(maja).await;
    seed_devices(&crowd.server, &crowd.maja.id, 99);

    // The hundredth takes the last place; registering it again changes nothing.
    let key = bytes(32, 50);
    assert_eq!(register_device_with(maja, &key, "Firefox").await.status, 201);
    assert_eq!(register_device_with(maja, &key, "Firefox").await.status, 200);
    assert_eq!(register_device_with(maja, &bytes(32, 51), "Firefox, again").await.status, 201, "a new key replaces the session's device");
    let laptop = crowd.server.sign_in(&crowd.maja).await;
    let refused = register_device_with(&laptop, &bytes(32, 52), "Laptop").await;
    assert_eq!(refusal(&refused), (409, Some("device-limit".into())));
    assert!(refused.text().contains("\"status\":409,\"max\":100,\"code\":\"device-limit\""), "{}", refused.text());

    // A device whose session is gone does not count against the limit.
    crowd
        .server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1 WHERE \"Id\" = (SELECT \"Id\" FROM \"UserSessions\" WHERE \"Device\" = 'Seeded' LIMIT 1)",
            params![Timestamp::now()],
        )
        .unwrap();
    assert_eq!(register_device_with(&laptop, &bytes(32, 53), "Laptop").await.status, 201);
    let phone = crowd.server.sign_in(&crowd.maja).await;
    assert_eq!(refusal(&register_device_with(&phone, &bytes(32, 54), "Phone").await).1.as_deref(), Some("device-limit"));
    let oldest = read(maja, &format!("{ACCOUNT}/devices")).await["devices"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(delete(maja, &format!("{ACCOUNT}/devices/{oldest}")).await.status, 204);
    assert_eq!(register_device_with(&phone, &bytes(32, 54), "Phone").await.status, 201);
    assert_eq!(read(maja, &format!("{ACCOUNT}/devices")).await["devices"].as_array().unwrap().len(), 100);
}

#[tokio::test]
async fn pin_attempts_are_limited_per_account_and_per_address() {
    let server = Server::start_with(&[
        ("ASPNETCORE_ENVIRONMENT", "Development"),
        ("RateLimiting__PinUnlock__PermitLimit", "2"),
    ])
    .await;
    let crowd = Crowd::on(server).await;
    set_up(&crowd.maja.client).await;
    set_up(&crowd.ada.client).await;
    let from = async |person: &crate::support::Person, address: &str| {
        crowd.server.sign_in(person).await.with_header("X-Forwarded-For", address)
    };
    let (maja_one, maja_two) = (from(&crowd.maja, "10.0.0.1").await, from(&crowd.maja, "10.0.0.2").await);
    let (ada_two, ada_three) = (from(&crowd.ada, "10.0.0.2").await, from(&crowd.ada, "10.0.0.3").await);
    async fn attempt(client: &crate::support::Client) -> crate::support::Response {
        post(client, UNLOCK, &json!({ "authKey": text(&bytes(32, 9)) })).await
    }

    assert_eq!(attempt(&maja_one).await.status, 403);
    assert_eq!(attempt(&maja_one).await.status, 403);
    // The account has used its tries, from wherever the next one comes.
    let spent = attempt(&maja_one).await;
    assert_eq!((spent.status.as_u16(), spent.body.is_empty()), (429, true));
    assert_eq!(attempt(&maja_two).await.status, 429);
    // Another account from that second address still gets one, and then the address has used its tries too.
    assert_eq!(attempt(&ada_two).await.status, 403);
    assert_eq!(attempt(&ada_two).await.status, 429);
    assert_eq!(attempt(&ada_three).await.status, 403);

    // Refused attempts are not counted against the backup.
    assert_eq!(read(&crowd.maja.client, BACKUP).await["remainingAttempts"], 8);
}

const UNLOCK: &str = "/api/me/encryption/backup/unlock";
const BACKUP: &str = "/api/me/encryption/backup";

#[tokio::test]
async fn an_attempt_that_is_cut_off_still_costs_the_one_it_made() {
    let server = Server::start_with(&[
        ("ASPNETCORE_ENVIRONMENT", "Development"),
        ("RateLimiting__PinUnlock__PermitLimit", "50"),
    ])
    .await;
    let crowd = Crowd::on(server).await;
    set_up(&crowd.maja.client).await;
    let wrong = json!({ "authKey": text(&bytes(32, 9)) }).to_string();

    // The client hangs up the moment it has sent the request. Whether the
    // server has read it by then is up to the network, so it is done until one
    // has been read: what that one did must not depend on anybody listening.
    let mut cut_off = 0;
    let mut remaining = 10;
    while remaining == 10 && cut_off < 20 {
        crowd.maja.client.post_json_and_hang_up(UNLOCK, &wrong).await;
        cut_off += 1;
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            remaining = read(&crowd.maja.client, BACKUP).await["remainingAttempts"].as_i64().unwrap();
            if remaining != 10 {
                break;
            }
        }
    }
    assert!(remaining < 10, "the wrong PIN was counted though nobody was there to hear it");
}
