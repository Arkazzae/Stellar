//! The bootstrap and the GIF configuration through the real binary.

use http::Method;
use rusqlite::params;
use serde_json::Value;
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Server};

const SECRET: &str = "a-secret-for-the-relay-of-the-tests";

fn bootstrap_of(person: &Person) -> impl std::future::Future<Output = crate::support::Response> + '_ {
    person.client.get("/api/bootstrap")
}

async fn answer(person: &Person) -> Value {
    let response = bootstrap_of(person).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

fn id(person: &Person) -> Id {
    Id::parse(&person.id).unwrap()
}

fn count(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn the_owner_of_a_new_instance_is_given_everything_in_the_order_clients_read_it() {
    let server = Server::start().await;
    let owner = server.found("owner").await;

    let response = bootstrap_of(&owner).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json; charset=utf-8"));
    // Bootstrap sends no explicit cache directive.
    assert!(response.header("cache-control").is_none());
    let text = response.text();
    let keys = ["\"user\"", "\"server\"", "\"servers\"", "\"channels\"", "\"sections\"", "\"members\"", "\"messages\"", "\"channelReads\"", "\"voice\"", "\"rtc\"", "\"permissions\""];
    let mut at = 0;
    for key in keys {
        at += text[at..].find(key).unwrap_or_else(|| panic!("{key} in order in {text}")) + key.len();
    }
    let answer: Value = response.json();
    assert_eq!(answer["user"]["username"], "owner");
    assert_eq!(answer["server"], answer["servers"][0]);
    let names: Vec<&str> = answer["channels"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["general", "voice"]);
    assert_eq!(answer["members"].as_array().unwrap().len(), 1);
    assert_eq!(answer["messages"][0]["content"], "**Owner** has joined the server. Say hello!");
    assert_eq!(answer["permissions"].as_array().unwrap().len(), 17);
    assert_eq!(answer["rtc"]["protocolVersion"], 1);
    assert_eq!(answer["rtc"]["signalingHub"], "/hubs/app");
    assert_eq!(answer["rtc"]["mediaServer"], "udp://localhost:49000");
    // Nobody sits anywhere; a voice room is listed all the same.
    let voice = answer["voice"].as_object().unwrap();
    assert_eq!(voice.len(), 1);
    assert!(voice.values().all(|room| room.as_array().unwrap().is_empty()));
}

#[tokio::test]
async fn nobody_signed_in_is_refused_and_only_a_get_is_answered() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    // A browser that is not signed in (a client that says nothing of itself is refused earlier).
    let anonymous = server.client();

    let refused = anonymous.get("/api/bootstrap").await;
    assert_eq!(refused.status, 401);
    assert!(refused.text().is_empty());
    assert_eq!(anonymous.get("/api/config/giphy").await.status, 401);
    for (method, path) in [(Method::HEAD, "/api/bootstrap"), (Method::POST, "/api/bootstrap"), (Method::POST, "/api/config/giphy")] {
        assert_eq!(owner.client.send(Request::new(method.clone(), path)).await.status, 405, "{method} {path}");
    }
}

#[tokio::test]
async fn a_person_in_no_server_is_forbidden_and_the_server_asked_for_must_be_theirs() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let home: String = server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    let home = home.to_lowercase();

    // The header and the query name a server, the header first; a name that
    // is no id is a bad request, a server the person is not in is forbidden.
    for (header, want) in [
        (Some(home.clone()), 200), (Some(home.replace('-', "")), 400), (Some(format!("{{{home}}}")), 400), (Some(home.to_uppercase()), 200),
        (Some(format!("  {home}  ")), 200),
        (Some("garbage".to_string()), 400), (Some(format!("{home},{home}")), 400),
        (Some("11111111-1111-1111-1111-111111111111".to_string()), 403),
    ] {
        let response = owner.client.send(Request::new(Method::GET, "/api/bootstrap").header("X-Stellarr-Server", header.as_deref().unwrap())).await;
        assert_eq!(response.status, want, "{header:?}");
    }
    let by_query = owner.client.get(&format!("/api/bootstrap?SERVERID={home}")).await;
    assert_eq!(by_query.status, 200);
    assert_eq!(owner.client.get("/api/bootstrap?serverId=zzz").await.status, 400);
    assert_eq!(owner.client.get("/api/bootstrap?serverId=").await.status, 200);

    server.database().execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", params![id(&maya)]).unwrap();
    let response = bootstrap_of(&maya).await;
    assert_eq!(response.status, 403);
    assert!(response.text().is_empty());
}

/// A server of the instance's own people, which `omar` joined first.
fn second_server(server: &Server, member: &Person) -> String {
    let db = server.database();
    let home: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    let beta = "22222222-BBBB-4BBB-8BBB-000000000001";
    db.execute_batch(&format!(
        "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"IconUrl\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
             VALUES ('{beta}', 'Beta', 'The second', 0, NULL, '2026-02-01 00:00:00', 1, 0);
         INSERT INTO \"ServerRoles\" SELECT 'BBBBBBBB' || substr(\"Id\", 9), '{beta}', \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\"
             FROM \"ServerRoles\" WHERE \"ServerId\" = '{home}' AND \"IsSystem\" = 1;
         INSERT INTO \"ServerMembers\" VALUES ('{beta}', '{member}', (SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = '{beta}' AND \"IsOwner\" = 1), 1, '2026-01-15 00:00:00');
         INSERT INTO \"Channels\" VALUES ('66666666-BBBB-4BBB-8BBB-000000000001', '{beta}', 'town-square', 0, 0, '', 0, 0, 0, NULL);
         INSERT INTO \"Channels\" VALUES ('66666666-BBBB-4BBB-8BBB-000000000002', '{beta}', 'hall', 1, 1, '', 0, 0, 0, NULL);",
        member = id(member).to_db_text()
    ))
    .unwrap();
    beta.to_lowercase()
}

#[tokio::test]
async fn a_member_of_two_servers_is_given_the_one_they_joined_first_or_the_one_they_ask_for() {
    let server = Server::start().await;
    server.found("owner").await;
    let omar = server.register("omar").await;
    let home: String = server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    let home = home.to_lowercase();
    let beta = second_server(&server, &omar);

    let first = answer(&omar).await;
    assert_eq!(first["server"]["id"], beta.as_str());
    let listed: Vec<&str> = first["servers"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(listed, [beta.as_str(), home.as_str()]);
    let names: Vec<&str> = first["channels"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["town-square", "hall"]);

    let at_home = omar.client.send(Request::new(Method::GET, "/api/bootstrap").header("X-Stellarr-Server", &home)).await;
    assert_eq!(at_home.json::<Value>()["server"]["id"], home.as_str());
    // The header wins over the query.
    let both = omar.client.send(Request::new(Method::GET, &format!("/api/bootstrap?serverId={beta}")).header("X-Stellarr-Server", &home)).await;
    assert_eq!(both.json::<Value>()["server"]["id"], home.as_str());
}

#[tokio::test]
async fn what_a_member_does_not_see_is_not_in_the_answer_and_has_no_marker() {
    let server = Server::start().await;
    server.found("owner").await;
    let maya = server.register("maya").await;
    let db = server.database();
    let home: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    let base: String = db.query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0)).unwrap();
    db.execute_batch(&format!(
        "INSERT INTO \"Sections\" VALUES ('22222222-AAAA-4AAA-8AAA-00000000000B', '{home}', 'Hidden', 1);
         INSERT INTO \"Channels\" VALUES ('33333333-AAAA-4AAA-8AAA-000000000015', '{home}', 'secret', 0, 6, '', 0, 0, 0, '22222222-AAAA-4AAA-8AAA-00000000000B');
         INSERT INTO \"Channels\" VALUES ('33333333-AAAA-4AAA-8AAA-000000000016', '{home}', 'ops', 1, 7, '', 0, 0, 0, '22222222-AAAA-4AAA-8AAA-00000000000B');
         INSERT INTO \"PermissionOverrides\" VALUES ('44444444-AAAA-4AAA-8AAA-000000000001', '{home}', 1, '22222222-AAAA-4AAA-8AAA-00000000000B', '{base}', 0, 16384);
         INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES ('B2000000-0000-0000-0000-000000000001', '33333333-AAAA-4AAA-8AAA-000000000015', '{owner}', 0, 'hidden words', '2026-03-06 00:00:00');
         INSERT INTO \"ChannelReads\" VALUES ('33333333-AAAA-4AAA-8AAA-000000000015', '{maya}', '2026-03-06 00:10:00');",
        owner = db.query_row::<String, _, _>("SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'owner'", [], |r| r.get(0)).unwrap(),
        maya = id(&maya).to_db_text()
    ))
    .unwrap();

    let answer = answer(&maya).await;

    let names: Vec<&str> = answer["channels"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["general", "voice"]);
    assert!(answer["sections"].as_array().unwrap().is_empty());
    assert!(!answer.to_string().contains("hidden words") && !answer.to_string().contains("secret"));
    assert!(!answer["voice"].to_string().contains("33333333-aaaa"));
    // A marker for the one text channel she sees, none for the hidden one, and the stale one is not shown.
    let reads = answer["channelReads"].as_object().unwrap();
    assert_eq!(reads.len(), 1);
    assert!(!reads.contains_key("33333333-aaaa-4aaa-8aaa-000000000015"));
}

#[tokio::test]
async fn a_marker_is_made_for_each_channel_never_opened_and_only_once() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let general: String = server.database().query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0", [], |row| row.get(0)).unwrap();
    let before = count(&server, "SELECT COUNT(*) FROM \"ChannelReads\" WHERE \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'owner')");

    let first = answer(&owner).await;
    let second = answer(&owner).await;
    let _ = before;

    let marker = first["channelReads"][general.to_lowercase()].as_str().unwrap().to_string();
    assert!(marker.ends_with('Z'));
    assert_eq!(second["channelReads"][general.to_lowercase()], marker.as_str(), "what is stored stands");
    assert_eq!(count(&server, "SELECT COUNT(*) FROM \"ChannelReads\" WHERE \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'owner')"), 1);
    // A marker that is there is the one given, whatever its moment.
    server.database().execute("UPDATE \"ChannelReads\" SET \"LastReadAt\" = '2026-01-02 03:04:05.5'", []).unwrap();
    assert_eq!(answer(&owner).await["channelReads"][general.to_lowercase()], "2026-01-02T03:04:05.5Z");
}

#[tokio::test]
async fn the_first_page_is_the_newest_hundred_of_the_first_text_channel_oldest_first() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let db = server.database();
    let general: String = db.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Name\" = 'general'", [], |row| row.get(0)).unwrap();
    let author: String = db.query_row("SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = 'owner'", [], |row| row.get(0)).unwrap();
    db.execute_batch(&format!(
        "CREATE TEMP TABLE n(i INTEGER PRIMARY KEY);
         WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<300) INSERT INTO n SELECT i FROM c;
         INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\")
           SELECT printf('B0000000-0000-0000-0000-%012d', i), '{general}', '{author}', 0, 'message ' || i, datetime('2026-03-01 00:00:00', printf('+%d seconds', i / 3)) FROM n;"
    ))
    .unwrap();

    let answer = answer(&owner).await;

    let messages = answer["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 100);
    // The welcome notice of today is the newest; the rest are the newest of the seeded ones, in order.
    assert_eq!(messages.last().unwrap()["content"], "**Owner** has joined the server. Say hello!");
    let created: Vec<&str> = messages.iter().map(|m| m["createdAt"].as_str().unwrap()).collect();
    assert!(created.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(messages[98]["content"], "message 300");
    // Of messages that share a moment, the ones that come later are listed later.
    let contents: Vec<&str> = messages.iter().map(|m| m["content"].as_str().unwrap()).collect();
    let at = contents.iter().position(|c| *c == "message 298").unwrap();
    assert_eq!(&contents[at..at + 3], ["message 298", "message 299", "message 300"]);
}

#[tokio::test]
async fn presence_is_what_the_caller_may_see_of_each_member() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let sam = server.register("sam").await;
    server
        .database()
        .execute(
            "INSERT OR REPLACE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
             VALUES (?1, 1, 1, 1, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 50)",
            params![id(&sam)],
        )
        .unwrap();
    let mut hub = crate::support::HubClient::connect(server.address(), &format!("stellarr-dev-session={}", sam.client.cookie("stellarr-dev-session").unwrap())).await;
    hub.quiet_for(std::time::Duration::from_millis(300)).await;

    let seen_by_owner = answer(&owner).await;
    let seen_by_sam = answer(&sam).await;

    let presence = |answer: &Value, name: &str| answer["members"].as_array().unwrap().iter().find(|m| m["username"] == name).unwrap()["presence"].clone();
    assert_eq!(presence(&seen_by_owner, "sam"), "offline", "a hidden status is offline to others");
    assert_eq!(presence(&seen_by_sam, "sam"), "online", "and online to its owner");
    assert_eq!(seen_by_sam["user"]["presence"], "online");
    assert_eq!(seen_by_owner["user"]["presence"], "offline");
}

// ---- voice

#[tokio::test]
async fn direct_media_does_not_advertise_a_relay_even_if_a_secret_is_configured() {
    let server = Server::start_with(&[
        ("Rtc__TurnSharedSecret", SECRET),
        ("Rtc__PublicHost", "voice.example.test"),
    ])
    .await;
    let owner = server.found("owner").await;

    let rtc = answer(&owner).await["rtc"].clone();
    assert_eq!(rtc["iceServers"], serde_json::json!([]));
    assert_eq!(rtc["mediaServer"], "udp://voice.example.test:49000");
}

#[tokio::test]
async fn the_relay_is_described_with_credentials_for_the_caller_that_last_an_hour() {
    let server = Server::start_with(&[("STELLARR_TURN_ENABLED", "true"), ("Rtc__TurnSharedSecret", SECRET), ("Rtc__PublicHost", "voice.example.test"), ("Rtc__TurnTcpEnabled", "true"), ("Rtc__TurnsEnabled", "true"), ("Rtc__TurnsPort", "443")]).await;
    let owner = server.found("owner").await;

    let rtc = answer(&owner).await["rtc"].clone();

    let ice = rtc["iceServers"].as_array().unwrap();
    assert_eq!(ice.len(), 3);
    assert_eq!(ice[0], serde_json::json!({ "urls": ["stun:voice.example.test:3478"], "username": null, "credential": null }));
    assert_eq!(ice[1]["urls"], serde_json::json!(["turn:voice.example.test:3478?transport=udp", "turn:voice.example.test:3478?transport=tcp"]));
    assert_eq!(ice[2]["urls"], serde_json::json!(["turns:voice.example.test:443?transport=tcp"]));
    assert_eq!(rtc["mediaServer"], "udp://voice.example.test:49000");
    for relay in &ice[1..] {
        let username = relay["username"].as_str().unwrap();
        let (ends, user) = username.split_once(':').unwrap();
        assert_eq!(user, owner.id.replace('-', ""));
        let left = ends.parse::<i64>().unwrap() - Timestamp::now().unix_seconds();
        assert!((3590..=3600).contains(&left), "{left}");
        // The credential is the MAC of the user name under the secret, which openssl makes of it:
        //   printf '<username>' | openssl dgst -sha1 -hmac '<secret>' -binary | base64
        let key = aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, SECRET.as_bytes());
        let want = stellarr_api::encoding::base64(aws_lc_rs::hmac::sign(&key, username.as_bytes()).as_ref());
        assert_eq!(relay["credential"], want.as_str());
    }
}

#[tokio::test]
async fn the_tls_port_of_the_relay_is_looked_at_only_while_tls_is_on() {
    let server = Server::start_with(&[("STELLARR_TURN_ENABLED", "true"), ("Rtc__TurnsPort", "0")]).await;
    let owner = server.found("owner").await;
    assert_eq!(bootstrap_of(&owner).await.status, 200);
}

// ---- GIFs

fn giphy_row(server: &Server, enabled: Option<bool>) {
    let db = server.database();
    let home: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    db.execute(
        "INSERT OR REPLACE INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"GiphyEnabled\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\")
         VALUES (?1, '', 'UTC', 0, '', ?2, 0, 0, 0, 7, 7, 7, 0, ?3)",
        params![home, enabled, Timestamp::now()],
    )
    .unwrap();
}

#[tokio::test]
async fn gifs_are_off_without_a_key_and_on_with_one_unless_the_operator_turned_them_off() {
    // No key anywhere.
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let text = owner.client.get("/api/config/giphy").await.text();
    assert_eq!(text, "{\"enabled\":false,\"apiKey\":\"\"}");

    // A key from the environment, an installation that never chose.
    let server = Server::start_with(&[("Giphy__ApiKey", "  env-key  ")]).await;
    let owner = server.found("owner").await;
    assert_eq!(owner.client.get("/api/config/giphy").await.text(), "{\"enabled\":true,\"apiKey\":\"env-key\"}");
    // Chosen on, chosen off: off hands out no key.
    giphy_row(&server, Some(true));
    assert_eq!(owner.client.get("/api/config/giphy").await.text(), "{\"enabled\":true,\"apiKey\":\"env-key\"}");
    giphy_row(&server, Some(false));
    assert_eq!(owner.client.get("/api/config/giphy").await.text(), "{\"enabled\":false,\"apiKey\":\"\"}");
    // The key is nowhere else.
    for path in ["/api/bootstrap", "/api/me", "/api/social", "/api/notifications"] {
        assert!(!owner.client.get(path).await.text().contains("env-key"), "{path}");
    }
}

#[tokio::test]
async fn a_key_stored_through_the_panel_wins_over_the_environment() {
    let server = Server::start_with(&[("Giphy__ApiKey", "env-key")]).await;
    let owner = server.found("owner").await;
    let sealer = stellarr_api::seal::Sealer::load_or_create(&server.folder().join("keys")).unwrap();
    stellarr_api::secrets::set(&server.database(), &sealer, &stellarr_api::secrets::GIPHY_API_KEY, "  panel-key ").unwrap();

    assert_eq!(owner.client.get("/api/config/giphy").await.text(), "{\"enabled\":true,\"apiKey\":\"panel-key\"}");
    giphy_row(&server, Some(false));
    assert_eq!(owner.client.get("/api/config/giphy").await.text(), "{\"enabled\":false,\"apiKey\":\"\"}");
}

// ---- cost

/// A server of `channels` channels, `members` members and `messages` messages in
/// its first channel, asked for the bootstrap of one of them: the statements
/// the reading took, the time it took, and the size of the answer as sent.
fn measure(channels: usize, members: usize, messages: usize, hidden: bool) -> (usize, std::time::Duration, usize) {
    use std::sync::{Arc, Mutex};
    use stellarr_api::bootstrap::{Context, read};

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let server = Server::start().await;
        let owner = server.found("owner").await;
        let maya = server.register("maya").await;
        let mut db = server.database();
        let home: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let role: String = db.query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0)).unwrap();
        let general: String = db.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Name\" = 'general'", [], |row| row.get(0)).unwrap();
        let author = id(&owner).to_db_text();
        db.execute_batch(&format!(
            "CREATE TEMP TABLE n(i INTEGER PRIMARY KEY);
             WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<{top}) INSERT INTO n SELECT i FROM c;
             INSERT INTO \"Channels\" SELECT printf('C0000000-0000-0000-0000-%012d', i), '{home}', 'channel-' || i, i % 3 = 0, i + 10, '', 0, 0, 0, NULL FROM n WHERE i <= {channels};
             INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
               SELECT printf('D0000000-0000-0000-0000-%012d', i), 'person' || i, 'PERSON' || i, 'Person ' || i, 0, 0, 0, '#777777', '2026-01-01 00:00:00' FROM n WHERE i <= {members};
             INSERT INTO \"ServerMembers\" SELECT '{home}', printf('D0000000-0000-0000-0000-%012d', i), '{role}', NULL, '2026-01-02 00:00:00' FROM n WHERE i <= {members};
             INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\")
               SELECT printf('B0000000-0000-0000-0000-%012d', i), '{general}', '{author}', 0, 'message ' || i, datetime('2026-03-01 00:00:00', printf('+%d seconds', i)) FROM n WHERE i <= {messages};",
            top = channels.max(members).max(messages).max(1),
        ))
        .unwrap();
        if hidden {
            // Somebody is denied view somewhere, so what each member sees is worked out.
            db.execute_batch(&format!(
                "INSERT INTO \"PermissionOverrides\" VALUES ('44444444-AAAA-4AAA-8AAA-000000000001', '{home}', 0, (SELECT \"Id\" FROM \"Channels\" WHERE \"Name\" = 'voice'), '{role}', 0, 16384);"
            ))
            .unwrap();
        }
        let started = std::time::Instant::now();
        let response = maya.client.get("/api/bootstrap").await;
        let took = started.elapsed();
        assert_eq!(response.status, 200);
        let size = response.text().len();
        // The markers are made now, so what follows reads and writes nothing more.
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&log);
        db.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(move |event: rusqlite::trace::TraceEvent<'_>| {
                if let rusqlite::trace::TraceEvent::Stmt(_, sql) = event
                    && !sql.starts_with("--")
                    && !sql.starts_with("PRAGMA")
                    && !sql.starts_with("SELECT k, v")
                {
                    sink.lock().unwrap().push(sql.to_string());
                }
            }),
        )
        .unwrap();
        let text = stellarr_messages::Messages::new(stellarr_messages::Language::English);
        let context = Context { messages: &text, presence: &stellarr_api::events::Unconnected, seats: None };
        db.execute_batch("BEGIN").unwrap();
        log.lock().unwrap().clear();
        let read = read(&db, &context, id(&maya), Id::parse(&home).unwrap()).map_err(|_| ()).unwrap();
        assert!(read.missing.is_empty());
        let statements = log.lock().unwrap().len();
        if std::env::var("SHOW_STATEMENTS").is_ok() {
            for line in log.lock().unwrap().iter() {
                println!("  {}", line.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect::<String>());
            }
        }
        db.execute_batch("ROLLBACK").unwrap();
        (statements, took, size)
    })
}

#[test]
fn the_cost_in_statements_does_not_grow_with_channels_members_or_messages() {
    for hidden in [false, true] {
        let (small, _, _) = measure(0, 0, 0, hidden);
        let (large, took, size) = measure(200, 2000, 2000, hidden);
        println!("hidden={hidden}: {small} statements on an empty server, {large} with 200 channels, 2000 members and 2000 messages; the answer took {took:?} and is {size} bytes");
        // Statements the permission model needs are its own; the rest is fixed.
        assert_eq!(small, large, "statements, with a view that is worked out: {hidden}");
        assert!(large <= 40, "{large}");
    }
}

#[test]
fn the_answer_for_a_big_server_is_made_quickly() {
    let (_, took, size) = measure(200, 2000, 2000, false);
    println!("a bootstrap of 200 channels, 2000 members and a full first page: {took:?}, {size} bytes");
    assert!(took.as_secs() < 2, "{took:?}");
    assert!(size < 2_000_000, "{size}");
}
