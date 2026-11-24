//! Friend requests, friendships, blocks and the state the friends screen
//! loads, through the real binary. The comparison with the first server is
//! `tests/social-oracle.sh`.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Client, HubClient, Person, Request, Server};

const COOKIE: &str = "stellarr-dev-session";
const QUIET: Duration = Duration::from_millis(400);

struct People {
    server: Server,
    ada: Person,
    ben: Person,
    cy: Person,
}

async fn people() -> People {
    let server = Server::start().await;
    let ada = server.found("ada").await;
    let ben = server.register("ben").await;
    let cy = server.register("cyd").await;
    People { server, ada, ben, cy }
}

async fn ask(client: &Client, username: &str) -> String {
    let response = client.post_json("/api/social/requests", &json!({ "username": username })).await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json::<Value>()["result"].as_str().unwrap().to_string()
}

async fn state(client: &Client) -> Value {
    let response = client.get("/api/social").await;
    assert_eq!(response.status, 200, "{}", response.text());
    response.json()
}

async fn status(client: &Client, method: Method, path: &str) -> u16 {
    client.send(Request::new(method, path)).await.status.as_u16()
}

fn names(list: &Value, user_in: Option<&str>) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let user = user_in.map_or(entry, |key| &entry[key]);
            user["username"].as_str().unwrap().to_string()
        })
        .collect()
}

async fn only_request(client: &Client) -> String {
    let requests = state(client).await["requests"].clone();
    assert_eq!(requests.as_array().unwrap().len(), 1, "{requests}");
    requests[0]["id"].as_str().unwrap().to_string()
}

async fn befriend(from: &Person, to: &Person) {
    assert_eq!(ask(&from.client, &to.username).await, "sent");
    let id = only_request(&to.client).await;
    assert_eq!(status(&to.client, Method::POST, &format!("/api/social/requests/{id}/accept")).await, 204);
}

fn social_events(server: &Server) -> Vec<Value> {
    server.events().into_iter().filter(|e| e["name"] == "SocialChanged").collect()
}

fn rows(server: &Server, sql: &str) -> i64 {
    server.database().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn accepting_a_request_makes_both_people_friends() {
    let p = people().await;
    assert_eq!(ask(&p.ada.client, "ben").await, "sent");

    let outgoing = &state(&p.ada.client).await["requests"][0];
    assert_eq!(outgoing["direction"], "outgoing");
    assert_eq!(outgoing["user"]["username"], "ben");
    let id = only_request(&p.ben.client).await;
    let incoming = state(&p.ben.client).await["requests"][0].clone();
    assert_eq!(incoming["direction"], "incoming");
    assert_eq!(incoming["user"]["username"], "ada");

    // Only the person it was sent to may accept it.
    let path = format!("/api/social/requests/{id}/accept");
    assert_eq!(status(&p.ada.client, Method::POST, &path).await, 404);
    assert_eq!(status(&p.cy.client, Method::POST, &path).await, 404);
    assert_eq!(status(&p.ben.client, Method::POST, &path).await, 204);
    assert_eq!(status(&p.ben.client, Method::POST, &path).await, 404);

    for (client, friend) in [(&p.ada.client, "ben"), (&p.ben.client, "ada")] {
        let state = state(client).await;
        assert!(state["requests"].as_array().unwrap().is_empty());
        assert_eq!(names(&state["friends"], Some("user")), [friend]);
    }
    assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"FriendRequests\""), 0);
}

#[tokio::test]
async fn a_friendship_is_stored_once_in_the_order_of_the_ids() {
    let p = people().await;
    befriend(&p.ada, &p.ben).await;
    let (a, b): (String, String) = p
        .server
        .database()
        .query_row("SELECT \"UserAId\", \"UserBId\" FROM \"Friendships\"", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert!(a < b, "{a} {b}");
    assert_eq!(
        [a.to_lowercase(), b.to_lowercase()].to_vec(),
        {
            let mut ids = vec![p.ada.id.clone(), p.ben.id.clone()];
            ids.sort();
            ids
        }
    );
}

#[tokio::test]
async fn the_pair_key_of_a_request_is_the_same_whoever_asks() {
    let p = people().await;
    ask(&p.ada.client, "ben").await;
    let key: String = p
        .server
        .database()
        .query_row("SELECT \"PairKey\" FROM \"FriendRequests\"", [], |r| r.get(0))
        .unwrap();
    let mut ids = [p.ada.id.replace('-', ""), p.ben.id.replace('-', "")];
    ids.sort();
    assert_eq!(key, ids.join(":"));
}

#[tokio::test]
async fn a_request_is_declined_by_its_receiver_and_withdrawn_by_its_sender() {
    let p = people().await;
    ask(&p.ada.client, "ben").await;
    let id = only_request(&p.ben.client).await;
    let path = format!("/api/social/requests/{id}");

    assert_eq!(status(&p.cy.client, Method::DELETE, &path).await, 404);
    assert_eq!(status(&p.ben.client, Method::DELETE, &path).await, 204);
    assert_eq!(status(&p.ben.client, Method::DELETE, &path).await, 404);
    assert!(state(&p.ada.client).await["requests"].as_array().unwrap().is_empty());
    assert!(state(&p.ben.client).await["friends"].as_array().unwrap().is_empty());

    assert_eq!(ask(&p.ada.client, "ben").await, "sent");
    let id = only_request(&p.ada.client).await;
    assert_eq!(status(&p.ada.client, Method::DELETE, &format!("/api/social/requests/{id}")).await, 204);
    assert!(state(&p.ben.client).await["requests"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn each_outcome_of_a_request_has_its_own_result_code() {
    let p = people().await;
    assert_eq!(ask(&p.ada.client, "nobody-by-that-name").await, "unknown-user");
    assert_eq!(ask(&p.ada.client, "@ADA").await, "self");
    assert_eq!(ask(&p.ada.client, "  @Ben  ").await, "sent");
    assert_eq!(ask(&p.ada.client, "ben").await, "already-sent");
    assert_eq!(ask(&p.ben.client, "ada").await, "already-sent");
    let id = only_request(&p.ben.client).await;
    p.ben.client.send(Request::new(Method::POST, &format!("/api/social/requests/{id}/accept"))).await;
    assert_eq!(ask(&p.ada.client, "ben").await, "already-friends");
    assert_eq!(ask(&p.ben.client, "ada").await, "already-friends");

    status(&p.cy.client, Method::POST, &format!("/api/social/blocks/{}", p.ada.id)).await;
    assert_eq!(ask(&p.cy.client, "ada").await, "blocked");
    assert_eq!(ask(&p.ada.client, "cyd").await, "blocked");
    // Every leading "@" is dropped, not only the first.
    assert_eq!(ask(&p.ada.client, "@@cyd").await, "blocked");
    // An "@" after a space is part of the name, which is nobody's.
    assert_eq!(ask(&p.ada.client, "@ ben").await, "unknown-user");
}

#[tokio::test]
async fn a_request_without_a_username_is_refused_in_the_words_of_the_instance() {
    let p = people().await;
    for body in [json!({ "username": "" }), json!({ "username": "   " }), json!({ "username": "@" }), json!({ "username": null }), json!({})] {
        let response = p.ada.client.post_json("/api/social/requests", &body).await;
        assert_eq!(response.status, 400, "{body}");
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
        let problem: Value = response.json();
        assert_eq!(problem["errors"]["username"][0], "Enter a username.");
    }
    for body in ["null", "", "[]", "{\"username\":5}", "not json"] {
        let response = p.ada.client.send(Request::new(Method::POST, "/api/social/requests").body("application/json", body)).await;
        assert_eq!(response.status, 400, "{body}");
        assert!(response.text().is_empty(), "{body}");
    }
    let response = p.ada.client.send(Request::new(Method::POST, "/api/social/requests").body("text/plain", "{\"username\":\"ben\"}")).await;
    assert_eq!(response.status, 415);
}

#[tokio::test]
async fn the_body_is_read_the_way_the_first_server_read_it() {
    let p = people().await;
    for (body, want) in [
        ("{\"USERNAME\":\"ben\"}", "sent"),
        ("{\"username\":\"xx\",\"Username\":\"ben\"}", "already-sent"),
        ("{\"other\":[1,{\"a\":null}],\"username\":\"ben\"}", "already-sent"),
        // The long s is a capital S, as the stored names are.
        ("{\"ſername\":\"ben\"}", "x"),
    ] {
        let response = p.ada.client.send(Request::new(Method::POST, "/api/social/requests").body("application/json", body)).await;
        if want == "x" {
            assert_eq!(response.status, 400, "{body}");
        } else {
            assert_eq!(response.json::<Value>()["result"], want, "{body}");
        }
    }
    // Spaces the first server trimmed, and characters it did not.
    for (name, want) in [("\u{85}ben\u{a0}", "already-sent"), ("\u{3000}@ben\u{2028}", "already-sent"), ("\u{1f}ben", "unknown-user"), ("\u{200b}ben", "unknown-user"), ("\u{feff}ben", "unknown-user")] {
        assert_eq!(ask(&p.ada.client, name).await, want, "{name:?}");
    }
}

#[tokio::test]
async fn names_are_compared_in_capitals_the_way_the_first_server_stored_them() {
    let server = Server::start().await;
    let ada = server.found("ada").await;
    let bill = server.register("bill").await;
    let dotless = server.register("bıll").await;
    let _ = bill;
    // A dotless i is not capitalised, so the two names are not each other's.
    assert_eq!(ask(&ada.client, "BILL").await, "sent");
    assert_eq!(ask(&ada.client, "bıll").await, "sent");
    assert_eq!(ask(&ada.client, "BıLL").await, "already-sent");
    let _ = dotless;
    assert_eq!(state(&ada.client).await["requests"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_account_that_turned_requests_off_reads_like_one_that_does_not_exist() {
    let p = people().await;
    p.server
        .database()
        .execute(
            "INSERT OR REPLACE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
             VALUES (?1, 1, 1, 1, 0, 0, 0, 1, 1, 1, 0, 1, 0, 0, 50)",
            params![Id::parse(&p.ben.id).unwrap()],
        )
        .unwrap();
    assert_eq!(ask(&p.ada.client, "ben").await, "unknown-user");
    assert_eq!(ask(&p.ben.client, "ben").await, "unknown-user");
    assert!(state(&p.ben.client).await["requests"].as_array().unwrap().is_empty());
    // Turning requests off does not stop the account from asking others.
    assert_eq!(ask(&p.ben.client, "ada").await, "sent");
}

#[tokio::test]
async fn accounts_that_stand_in_for_somebody_cannot_be_befriended_and_a_disabled_one_can() {
    let p = people().await;
    let database = p.server.database();
    database.execute("UPDATE \"Users\" SET \"Origin\" = 'discord' WHERE \"Username\" = 'ben'", []).unwrap();
    database.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Username\" = 'cy'", []).unwrap();
    assert_eq!(ask(&p.ada.client, "ben").await, "unknown-user");
    assert_eq!(status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", p.ben.id)).await, 404);
    // The first server never looked at whether the account was disabled.
    assert_eq!(ask(&p.ada.client, "cyd").await, "sent");
    assert_eq!(status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", p.cy.id)).await, 204);
}

#[tokio::test]
async fn removing_a_friend_ends_the_friendship_for_both() {
    let p = people().await;
    befriend(&p.ada, &p.ben).await;
    assert_eq!(status(&p.ada.client, Method::DELETE, &format!("/api/social/friends/{}", p.cy.id)).await, 404);
    assert_eq!(status(&p.ben.client, Method::DELETE, &format!("/api/social/friends/{}", p.ada.id)).await, 204);
    assert_eq!(status(&p.ada.client, Method::DELETE, &format!("/api/social/friends/{}", p.ben.id)).await, 404);
    assert!(state(&p.ada.client).await["friends"].as_array().unwrap().is_empty());
    assert!(state(&p.ben.client).await["friends"].as_array().unwrap().is_empty());
    assert_eq!(ask(&p.ada.client, "ben").await, "sent");
}

#[tokio::test]
async fn blocking_ends_the_friendship_and_every_request_between_the_two() {
    let p = people().await;
    let dee = p.server.register("dee").await;
    befriend(&p.ada, &p.ben).await;
    assert_eq!(ask(&p.cy.client, "ada").await, "sent");
    assert_eq!(ask(&p.ada.client, "dee").await, "sent");
    for id in [&p.ben.id, &p.cy.id, &dee.id] {
        assert_eq!(status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{id}")).await, 204);
    }
    let mine = state(&p.ada.client).await;
    assert!(mine["friends"].as_array().unwrap().is_empty());
    assert!(mine["requests"].as_array().unwrap().is_empty());
    let mut blocked = names(&mine["blocked"], None);
    blocked.sort();
    assert_eq!(blocked, ["ben", "cyd", "dee"]);
    for other in [&p.ben, &p.cy, &dee] {
        let theirs = state(&other.client).await;
        assert!(theirs["friends"].as_array().unwrap().is_empty());
        assert!(theirs["requests"].as_array().unwrap().is_empty());
        // Being blocked is not something the other side is shown.
        assert!(theirs["blocked"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn blocking_twice_keeps_one_block_and_only_real_people_can_be_blocked() {
    let p = people().await;
    let path = format!("/api/social/blocks/{}", p.ben.id);
    assert_eq!(status(&p.ada.client, Method::POST, &path).await, 204);
    assert_eq!(status(&p.ada.client, Method::POST, &path).await, 204);
    assert_eq!(status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", p.ada.id)).await, 404);
    assert_eq!(status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", Id::random())).await, 404);
    assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"UserBlocks\""), 1);
    assert_eq!(names(&state(&p.ada.client).await["blocked"], None), ["ben"]);
}

#[tokio::test]
async fn unblocking_restores_nothing_and_only_the_blocker_can_lift_a_block() {
    let p = people().await;
    befriend(&p.ada, &p.ben).await;
    status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", p.ben.id)).await;

    assert_eq!(status(&p.ben.client, Method::DELETE, &format!("/api/social/blocks/{}", p.ada.id)).await, 404);
    assert_eq!(ask(&p.ben.client, "ada").await, "blocked");
    assert_eq!(status(&p.ada.client, Method::DELETE, &format!("/api/social/blocks/{}", p.ben.id)).await, 204);
    assert_eq!(status(&p.ada.client, Method::DELETE, &format!("/api/social/blocks/{}", p.ben.id)).await, 404);

    let state = state(&p.ada.client).await;
    assert!(state["friends"].as_array().unwrap().is_empty());
    assert!(state["blocked"].as_array().unwrap().is_empty());
    assert_eq!(ask(&p.ada.client, "ben").await, "sent");
}

#[tokio::test]
async fn an_id_in_a_path_is_read_in_every_spelling_the_first_server_took() {
    let p = people().await;
    let plain = p.ben.id.replace('-', "");
    for (method, path) in [
        (Method::POST, format!("/api/social/blocks/{{{}}}", p.ben.id)),
        (Method::DELETE, format!("/api/social/blocks/{plain}")),
        (Method::POST, format!("/api/social/blocks/({})", p.ben.id.to_uppercase())),
    ] {
        assert_eq!(status(&p.ada.client, method, &path).await, 204, "{path}");
    }
    assert_eq!(status(&p.ada.client, Method::POST, "/api/social/blocks/not-an-id").await, 404);
    assert_eq!(status(&p.ada.client, Method::HEAD, "/api/social").await, 405);
    assert_eq!(status(&p.ada.client, Method::PUT, "/api/social").await, 405);
}

#[tokio::test]
async fn every_route_needs_somebody_signed_in() {
    let server = Server::start().await;
    let anonymous = server.anonymous();
    for (method, path) in [
        (Method::GET, "/api/social"),
        (Method::POST, "/api/social/requests"),
        (Method::POST, "/api/social/requests/3d82da44-8aa5-4bdf-9ee1-2902df99880c/accept"),
        (Method::DELETE, "/api/social/requests/3d82da44-8aa5-4bdf-9ee1-2902df99880c"),
        (Method::DELETE, "/api/social/friends/3d82da44-8aa5-4bdf-9ee1-2902df99880c"),
        (Method::POST, "/api/social/blocks/3d82da44-8aa5-4bdf-9ee1-2902df99880c"),
        (Method::DELETE, "/api/social/blocks/3d82da44-8aa5-4bdf-9ee1-2902df99880c"),
    ] {
        assert_eq!(status(&anonymous, method.clone(), path).await, 401, "{method} {path}");
    }
}

#[tokio::test]
async fn the_state_lists_friends_requests_with_their_direction_and_blocked_people() {
    let p = people().await;
    let dee = p.server.register("dee").await;
    befriend(&p.ada, &p.ben).await;
    ask(&p.cy.client, "ada").await;
    ask(&p.ada.client, "dee").await;
    let eve = p.server.register("eve").await;
    status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", eve.id)).await;

    let state = state(&p.ada.client).await;
    assert_eq!(names(&state["friends"], Some("user")), ["ben"]);
    assert_eq!(names(&state["blocked"], None), ["eve"]);
    let mut requests: Vec<(String, String)> = state["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["user"]["username"].as_str().unwrap().into(), r["direction"].as_str().unwrap().into()))
        .collect();
    requests.sort();
    assert_eq!(requests, [("cyd".into(), "incoming".into()), ("dee".into(), "outgoing".into())]);
    assert!(state["conversations"].as_array().unwrap().is_empty());
    let _ = dee;
}

#[tokio::test]
async fn members_of_the_state_come_in_the_order_the_first_server_wrote_them() {
    let p = people().await;
    befriend(&p.ada, &p.ben).await;
    let body = p.ada.client.get("/api/social").await.text();
    let want = ["\"friends\"", "\"requests\"", "\"blocked\"", "\"conversations\""];
    let positions: Vec<usize> = want.iter().map(|w| body.find(w).unwrap()).collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{body}");
    let friend = &body[body.find("\"friends\":[{").unwrap()..];
    assert!(friend.starts_with("\"friends\":[{\"user\":{\"id\":\""), "{friend}");
    for key in ["\"username\"", "\"displayName\"", "\"avatarColor\"", "\"presence\"", "\"avatarUrl\"", "\"bannerUrl\"", "\"customStatus\"", "\"bio\"", "\"pronouns\"", "\"origin\"", "\"since\""] {
        assert!(friend.contains(key), "{key}");
    }
    // A moment read from the database carries no zone, as the first server wrote it.
    let since = p.ada.client.get("/api/social").await.json::<Value>()["friends"][0]["since"].as_str().unwrap().to_string();
    assert!(!since.ends_with('Z') && since.contains('T'), "{since}");
}

fn conversation(server: &Server, a: &Person, b: &Person, created: &str) -> Id {
    let (x, y) = stellarr_api::social::pair(Id::parse(&a.id).unwrap(), Id::parse(&b.id).unwrap());
    let id = Id::random();
    server
        .database()
        .execute(
            "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\") VALUES (?1, ?2, ?3, ?4, 'Off')",
            params![id, x, y, Timestamp::parse(created).unwrap()],
        )
        .unwrap();
    id
}

fn message(server: &Server, conversation: Id, author: &Person, at: &str, content: &str, kind: Option<&str>) -> Id {
    let id = Id::random();
    server
        .database()
        .execute(
            "INSERT INTO \"DirectMessages\" (\"Id\", \"ConversationId\", \"AuthorId\", \"Content\", \"CreatedAt\", \"SystemKind\") VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, conversation, Id::parse(&author.id).unwrap(), content, Timestamp::parse(at).unwrap(), kind],
        )
        .unwrap();
    id
}

#[tokio::test]
async fn a_conversation_is_listed_with_its_newest_message_and_what_is_unread() {
    let p = people().await;
    let first = conversation(&p.server, &p.ada, &p.ben, "2026-01-01 10:00:00");
    let second = conversation(&p.server, &p.ada, &p.cy, "2026-01-01 10:00:00");
    message(&p.server, first, &p.ada, "2026-01-01 10:01:00", "one", None);
    message(&p.server, first, &p.ben, "2026-01-01 10:02:00", "two", None);
    message(&p.server, first, &p.ben, "2026-01-01 10:03:00", "three", None);
    // A line about somebody's keys neither counts nor becomes the last message.
    message(&p.server, first, &p.ben, "2026-01-01 10:04:00", "", Some("device_added"));
    message(&p.server, first, &p.ben, "2026-01-01 10:05:00", "", Some("encryption_enabled"));
    message(&p.server, second, &p.cy, "2026-01-01 11:00:00", "newer", None);
    message(&p.server, second, &p.cy, "2026-01-01 11:01:00", "", Some("encryption_requested"));

    let listed = state(&p.ada.client).await["conversations"].clone();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 2);
    // The one with the newest line is first; the line about keys never moves it.
    assert_eq!(listed[0]["peer"]["username"], "cyd");
    assert_eq!(listed[0]["unread"], 2);
    assert_eq!(listed[0]["lastMessage"]["system"]["kind"], "encryption_requested");
    assert_eq!(listed[0]["lastMessage"]["system"]["actorId"], p.cy.id.as_str());
    assert_eq!(listed[1]["peer"]["username"], "ben");
    assert_eq!(listed[1]["unread"], 2);
    assert_eq!(listed[1]["id"], first.to_string());
    let last = &listed[1]["lastMessage"];
    assert_eq!(last["content"], "");
    assert_eq!(last["system"]["kind"], "encryption_enabled");
    assert_eq!(last["channelId"], first.to_string());
    assert_eq!(last["author"]["presence"], "online");
    assert_eq!(last["createdAt"], "2026-01-01T10:05:00Z");
    assert_eq!(listed[1]["updatedAt"], "2026-01-01T10:05:00");
    assert_eq!(listed[1]["encryption"], json!({ "state": "off", "requestedById": null, "requestedAt": null, "enabledAt": null, "currentEpochId": null }));
    let _ = second;

    // A read marker moves what is unread.
    p.server
        .database()
        .execute(
            "INSERT INTO \"DirectConversationReads\" (\"ConversationId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)",
            params![first, Id::parse(&p.ada.id).unwrap(), Timestamp::parse("2026-01-01 10:02:30").unwrap()],
        )
        .unwrap();
    let listed = state(&p.ada.client).await["conversations"].clone();
    assert_eq!(listed[1]["unread"], 1);
}

#[tokio::test]
async fn a_conversation_without_a_message_is_as_old_as_itself_and_one_with_a_blocked_person_is_left_out() {
    let p = people().await;
    conversation(&p.server, &p.ada, &p.ben, "2026-02-03 04:05:06.123");
    conversation(&p.server, &p.ada, &p.cy, "2026-02-03 04:05:07");
    let listed = state(&p.ada.client).await["conversations"].clone();
    assert_eq!(listed[0]["peer"]["username"], "cyd");
    assert_eq!(listed[1]["updatedAt"], "2026-02-03T04:05:06.123");
    assert!(listed[1]["lastMessage"].is_null());
    assert_eq!(listed[1]["unread"], 0);

    status(&p.cy.client, Method::POST, &format!("/api/social/blocks/{}", p.ada.id)).await;
    let listed = state(&p.ada.client).await["conversations"].clone();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    // The blocked side's list is the same, and shows the blocked person nowhere else.
    assert_eq!(state(&p.cy.client).await["conversations"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn an_attachment_and_an_encrypted_message_are_shown_as_the_first_server_showed_them() {
    let p = people().await;
    let room = conversation(&p.server, &p.ada, &p.ben, "2026-01-01 10:00:00");
    let said = message(&p.server, room, &p.ben, "2026-01-01 10:01:00", "", None);
    let database = p.server.database();
    let attachment = Id::random();
    database
        .execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"Width\", \"Height\", \"IsVoiceMessage\", \"PosterObjectKey\", \"IsEncrypted\", \"CreatedAt\")
             VALUES (?1, ?2, 'k', 'cat.png', 'image/png', 12, 3, 4, 0, 'p', 0, ?3)",
            params![attachment, Id::parse(&p.ben.id).unwrap(), Timestamp::now()],
        )
        .unwrap();
    database.execute("INSERT INTO \"DirectMessageAttachments\" VALUES (?1, ?2)", params![said, attachment]).unwrap();

    let last = state(&p.ada.client).await["conversations"][0]["lastMessage"].clone();
    assert_eq!(
        last["attachments"],
        json!([{ "id": attachment.to_string(), "name": "cat.png", "contentType": "image/png", "size": 12, "width": 3, "height": 4,
                 "url": format!("/assets/{attachment}"), "durationSeconds": null, "posterUrl": format!("/assets/{attachment}/poster"),
                 "isVoiceMessage": false, "previewUrl": null, "placeholder": null }])
    );
    for (key, want) in [("embeds", json!([])), ("editedAt", Value::Null), ("pinnedAt", Value::Null), ("replyTo", Value::Null), ("reactions", Value::Null), ("encrypted", Value::Null), ("system", Value::Null)] {
        assert_eq!(last[key], want, "{key}");
    }
}

#[tokio::test]
async fn every_change_is_announced_to_the_two_people_it_concerns_and_to_nobody_else() {
    let p = people().await;
    let mut ben = HubClient::connect(p.server.address(), &cookie(&p.ben)).await;
    let mut cy = HubClient::connect(p.server.address(), &cookie(&p.cy)).await;
    drain(&mut ben).await;
    drain(&mut cy).await;

    ask(&p.ada.client, "ben").await;
    assert_eq!(ben.event("SocialChanged").await, json!({}));
    let id = only_request(&p.ben.client).await;
    status(&p.ben.client, Method::POST, &format!("/api/social/requests/{id}/accept")).await;
    assert_eq!(ben.event("SocialChanged").await, json!({}));
    status(&p.ada.client, Method::DELETE, &format!("/api/social/friends/{}", p.ben.id)).await;
    assert_eq!(ben.event("SocialChanged").await, json!({}));
    status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", p.ben.id)).await;
    assert_eq!(ben.event("SocialChanged").await, json!({}));
    status(&p.ada.client, Method::DELETE, &format!("/api/social/blocks/{}", p.ben.id)).await;
    assert_eq!(ben.event("SocialChanged").await, json!({}));
    assert!(ben.quiet_for(QUIET).await.is_empty());
    assert!(cy.quiet_for(QUIET).await.is_empty());

    // The raw record has an empty object, not null.
    let events = social_events(&p.server);
    assert_eq!(events.len(), 5);
    assert!(events.iter().all(|e| e["payload"] == json!({}) && e["audience"]["users"].as_array().unwrap().len() == 2));
}

#[tokio::test]
async fn a_refused_change_announces_nothing() {
    let p = people().await;
    ask(&p.ada.client, "nobody").await;
    ask(&p.ada.client, "ada").await;
    status(&p.ada.client, Method::DELETE, &format!("/api/social/friends/{}", p.ben.id)).await;
    status(&p.ada.client, Method::DELETE, &format!("/api/social/blocks/{}", p.ben.id)).await;
    status(&p.ada.client, Method::POST, &format!("/api/social/blocks/{}", Id::random())).await;
    status(&p.ada.client, Method::POST, &format!("/api/social/requests/{}/accept", Id::random())).await;
    ask(&p.ada.client, "ben").await;
    ask(&p.ada.client, "ben").await;
    assert_eq!(social_events(&p.server).len(), 1);
}

fn cookie(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

/// Reads what a connection is told when it starts.
async fn drain(client: &mut HubClient) {
    client.quiet_for(QUIET).await;
}

fn presence_of(records: &[Value], user: &str) -> Vec<String> {
    records
        .iter()
        .filter(|r| r["target"] == "PresenceChanged" && r["arguments"][0]["userId"] == user)
        .map(|r| r["arguments"][0]["presence"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_friendship_or_a_block_changes_who_hears_a_status_on_connections_already_open() {
    let server = Server::start().await;
    let ada = server.found("ada").await;
    let ben = server.register("ben").await;
    // Without a server of their own in common, nothing but the relationship
    // carries a status from one to the other.
    server.database().execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", params![Id::parse(&ben.id).unwrap()]).unwrap();
    let mut watcher = HubClient::connect(server.address(), &cookie(&ada)).await;
    let mut actor = HubClient::connect(server.address(), &cookie(&ben)).await;
    drain(&mut watcher).await;
    drain(&mut actor).await;

    actor.invoke("SetPresence", json!(["idle"])).await;
    let heard = watcher.quiet_for(QUIET).await;
    assert!(presence_of(&heard, &ben.id).is_empty(), "strangers hear nothing: {heard:?}");

    befriend(&ada, &ben).await;
    drain(&mut watcher).await;
    actor.invoke("SetPresence", json!(["dnd"])).await;
    let heard = watcher.quiet_for(QUIET).await;
    assert_eq!(presence_of(&heard, &ben.id), ["dnd"], "friends hear: {heard:?}");

    status(&ada.client, Method::POST, &format!("/api/social/blocks/{}", ben.id)).await;
    drain(&mut watcher).await;
    actor.invoke("SetPresence", json!(["online"])).await;
    let heard = watcher.quiet_for(QUIET).await;
    assert!(presence_of(&heard, &ben.id).is_empty(), "a block silences: {heard:?}");

    status(&ada.client, Method::DELETE, &format!("/api/social/blocks/{}", ben.id)).await;
    befriend(&ben, &ada).await;
    drain(&mut watcher).await;
    actor.invoke("SetPresence", json!(["idle"])).await;
    let heard = watcher.quiet_for(QUIET).await;
    assert_eq!(presence_of(&heard, &ben.id), ["idle"]);

    // And what the friends screen shows follows what the hub knows.
    let shown = state(&ada.client).await["friends"][0]["user"]["presence"].clone();
    assert_eq!(shown, "idle");
}

// Races. Every one of them is run several times, since a lucky order proves
// nothing.

#[tokio::test]
async fn two_people_asking_each_other_at_once_leave_exactly_one_request() {
    for _ in 0..5 {
        let p = people().await;
        let (one, two) = tokio::join!(ask(&p.ada.client, "ben"), ask(&p.ben.client, "ada"));
        let mut got = [one, two];
        got.sort();
        assert_eq!(got, ["already-sent", "sent"]);
        assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"FriendRequests\""), 1);
        assert_eq!(social_events(&p.server).len(), 1);
    }
}

#[tokio::test]
async fn the_same_request_sent_eight_times_at_once_is_sent_once() {
    let p = people().await;
    let answers = futures_all(&p).await;
    assert_eq!(answers.iter().filter(|a| *a == "sent").count(), 1, "{answers:?}");
    assert_eq!(answers.iter().filter(|a| *a == "already-sent").count(), 7, "{answers:?}");
    assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"FriendRequests\""), 1);
    assert_eq!(social_events(&p.server).len(), 1);
}

async fn futures_all(p: &People) -> Vec<String> {
    let c = &p.ada.client;
    let (a, b, c2, d, e, f, g, h) = tokio::join!(
        ask(c, "ben"), ask(c, "ben"), ask(c, "ben"), ask(c, "ben"), ask(c, "ben"), ask(c, "ben"), ask(c, "ben"), ask(c, "ben")
    );
    vec![a, b, c2, d, e, f, g, h]
}

#[tokio::test]
async fn accepting_and_withdrawing_the_same_request_at_once_never_leaves_both_a_friendship_and_a_request() {
    for _ in 0..8 {
        let p = people().await;
        ask(&p.ada.client, "ben").await;
        let id = only_request(&p.ben.client).await;
        let accept = format!("/api/social/requests/{id}/accept");
        let withdraw = format!("/api/social/requests/{id}");
        let (a, w) = tokio::join!(
            status(&p.ben.client, Method::POST, &accept),
            status(&p.ada.client, Method::DELETE, &withdraw)
        );
        // Whoever came second finds the request gone or the work already done.
        assert!([204, 404].contains(&a) && [204, 404].contains(&w), "{a} {w}");
        assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"FriendRequests\""), 0);
        assert!(rows(&p.server, "SELECT COUNT(*) FROM \"Friendships\"") <= 1);
    }
}

#[tokio::test]
async fn blocking_while_the_other_accepts_leaves_a_block_and_no_friendship() {
    for _ in 0..8 {
        let p = people().await;
        ask(&p.ben.client, "ada").await;
        let id = only_request(&p.ada.client).await;
        let accept = format!("/api/social/requests/{id}/accept");
        let block = format!("/api/social/blocks/{}", p.ben.id);
        let (a, b) = tokio::join!(
            status(&p.ada.client, Method::POST, &accept),
            status(&p.ada.client, Method::POST, &block)
        );
        assert_eq!(b, 204);
        assert!([204, 404].contains(&a), "{a}");
        // Accepting first and blocking after ends the friendship it made.
        assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"Friendships\""), 0);
        assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"FriendRequests\""), 0);
        assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"UserBlocks\""), 1);
    }
}

#[tokio::test]
async fn two_blocks_at_once_leave_one_block() {
    let p = people().await;
    let path = format!("/api/social/blocks/{}", p.ben.id);
    let (a, b, c) = tokio::join!(
        status(&p.ada.client, Method::POST, &path),
        status(&p.ada.client, Method::POST, &path),
        status(&p.ada.client, Method::POST, &path)
    );
    assert_eq!([a, b, c], [204, 204, 204]);
    assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"UserBlocks\""), 1);
}

#[tokio::test]
async fn removing_the_same_friend_twice_at_once_answers_no_content_to_whoever_found_it() {
    let p = people().await;
    befriend(&p.ada, &p.ben).await;
    let path = format!("/api/social/friends/{}", p.ben.id);
    let other = format!("/api/social/friends/{}", p.ada.id);
    let (a, b) = tokio::join!(status(&p.ada.client, Method::DELETE, &path), status(&p.ben.client, Method::DELETE, &other));
    assert!([204, 404].contains(&a) && [204, 404].contains(&b) && (a == 204 || b == 204), "{a} {b}");
    assert_eq!(rows(&p.server, "SELECT COUNT(*) FROM \"Friendships\""), 0);
}
