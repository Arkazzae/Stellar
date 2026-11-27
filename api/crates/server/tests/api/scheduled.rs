//! Scheduled messages through the real binary: who sees them and hears of
//! them, what making one refuses, sending one now, and the worker sending one
//! whose time has come. What the worker makes of each refusal is in the
//! crate's own tests, which move the clock.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Response, Server};

fn ahead(by: Duration) -> String {
    Timestamp::now().plus(by).to_wire_text()
}

const MINUTE: Duration = Duration::from_secs(60);

fn general(server: &Server) -> String {
    server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .to_lowercase()
}

async fn schedule(person: &Person, channel: &str, body: Value) -> Response {
    person
        .client
        .post_json(&format!("/api/channels/{channel}/scheduled"), &body)
        .await
}

async fn scheduled(person: &Person, channel: &str, content: &str, after: Duration) -> String {
    let response = schedule(person, channel, json!({ "content": content, "sendAt": ahead(after) })).await;
    assert_eq!(response.status, 201, "{}", response.text());
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

fn posted(server: &Server, content: &str) -> Vec<String> {
    let db = server.database();
    let mut statement = db
        .prepare("SELECT \"Id\" FROM \"Messages\" WHERE \"Content\" = ?1")
        .unwrap();
    statement
        .query_map([content], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|id| id.unwrap().to_lowercase())
        .collect()
}

#[tokio::test]
async fn only_the_author_sees_hears_of_or_touches_a_scheduled_message() {
    let server = Server::start_with(&[("Testing__Workers", "scheduled-messages")]).await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let general = general(&server);

    let made = schedule(&owner, &general, json!({ "content": "good morning", "sendAt": ahead(5 * MINUTE) })).await;
    assert_eq!(made.status, 201, "{}", made.text());
    let made: Value = made.json();
    let id = made["id"].as_str().unwrap().to_string();
    assert_eq!(made["state"], "pending");
    let (instance, instance_name): (String, String) = server
        .database()
        .query_row("SELECT \"Id\", \"Name\" FROM \"Servers\"", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    let instance = instance.to_lowercase();
    let general_name: String = server
        .database()
        .query_row(
            "SELECT \"Name\" FROM \"Channels\" WHERE \"Id\" = ?1",
            [Id::parse(&general).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        made["target"],
        json!({ "kind": "channel", "serverId": instance, "serverName": instance_name, "channelId": general, "channelName": general_name })
    );
    assert_eq!(made["failure"], Value::Null);

    // One scheduled on another server is in the same list, whichever server
    // the client names; only the filters narrow it.
    let orbit = owner
        .client
        .post_json("/api/servers", &json!({ "name": "Orbit", "textChannelName": "lobby", "voiceChannelName": "hall" }))
        .await;
    assert_eq!(orbit.status, 201, "{}", orbit.text());
    let orbit = orbit.json::<Value>()["id"].as_str().unwrap().to_string();
    let lobby: String = server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = 0",
            [Id::parse(&orbit).unwrap()],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .to_lowercase();
    let elsewhere = owner
        .client
        .send(
            Request::new(Method::POST, &format!("/api/channels/{lobby}/scheduled"))
                .header("X-Stellarr-Server", &orbit)
                .json(&json!({ "content": "over in orbit", "sendAt": ahead(60 * MINUTE) })),
        )
        .await;
    assert_eq!(elsewhere.status, 201, "{}", elsewhere.text());
    let listed: Value = owner
        .client
        .send(Request::new(Method::GET, "/api/scheduled").header("X-Stellarr-Server", &instance))
        .await
        .json();
    let targets: Vec<&Value> = listed.as_array().unwrap().iter().map(|message| &message["target"]).collect();
    assert_eq!(targets.len(), 2);
    assert_eq!(
        targets[1],
        &json!({ "kind": "channel", "serverId": orbit, "serverName": "Orbit", "channelId": lobby, "channelName": "lobby" })
    );
    let narrowed: Value = owner.client.get(&format!("/api/scheduled?channelId={general}")).await.json();
    assert_eq!(narrowed.as_array().unwrap().len(), 1);
    assert_eq!(narrowed[0]["id"], id.as_str());

    // Maya learns nothing of it and can do nothing with it.
    assert_eq!(maya.client.get("/api/scheduled").await.json::<Value>(), json!([]));
    for request in [
        Request::new(Method::PATCH, &format!("/api/scheduled/{id}")).json(&json!({ "content": "hijacked" })),
        Request::new(Method::POST, &format!("/api/scheduled/{id}/send")),
        Request::new(Method::DELETE, &format!("/api/scheduled/{id}")),
    ] {
        assert_eq!(maya.client.send(request).await.status, 404);
    }

    // The owner changes it and sends it now.
    let edited = owner
        .client
        .send(Request::new(Method::PATCH, &format!("/api/scheduled/{id}")).json(&json!({ "content": "good morning, all" })))
        .await;
    assert_eq!(edited.status, 200, "{}", edited.text());
    assert_eq!(edited.json::<Value>()["content"], "good morning, all");
    let sent = owner.client.send(Request::new(Method::POST, &format!("/api/scheduled/{id}/send"))).await;
    assert_eq!(sent.status, 201, "{}", sent.text());
    assert_eq!(sent.json::<Value>()["id"], id.as_str(), "the message has the scheduled message's id");
    assert_eq!(posted(&server, "good morning, all"), vec![id.clone()]);
    let left: Value = owner.client.get("/api/scheduled").await.json();
    assert_eq!(left.as_array().unwrap().len(), 1, "only the one in orbit waits");

    // One whose time has come goes out once the worker looks, which making
    // another one makes it do.
    let due = scheduled(&owner, &general, "from the worker", 5 * MINUTE).await;
    let past = Timestamp::now().minus(Duration::from_secs(1));
    server
        .database()
        .execute(
            "UPDATE \"ScheduledMessages\" SET \"SendAt\" = ?2, \"DueAt\" = ?2 WHERE \"Id\" = ?1",
            params![Id::parse(&due).unwrap(), past],
        )
        .unwrap();
    let later = scheduled(&owner, &general, "much later", 10 * MINUTE).await;
    let gone = |event: &Value| event["name"] == "ScheduledMessageRemoved" && event["payload"]["id"] == due.as_str();
    for _ in 0..200 {
        if server.events().iter().any(gone) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(server.events().iter().any(gone), "the worker sent it");
    assert_eq!(posted(&server, "from the worker"), vec![due.clone()]);
    assert!(posted(&server, "much later").is_empty());

    // Sending it now once it has gone answers with the message as stored.
    let again = owner.client.send(Request::new(Method::POST, &format!("/api/scheduled/{due}/send"))).await;
    assert_eq!(again.status, 200, "{}", again.text());
    assert_eq!(again.json::<Value>()["id"], due.as_str());
    assert_eq!(posted(&server, "from the worker").len(), 1);

    let removed = owner.client.send(Request::new(Method::DELETE, &format!("/api/scheduled/{later}"))).await;
    assert_eq!(removed.status, 204);

    // Every event about a scheduled message went to the owner alone.
    let events: Vec<Value> = server
        .events()
        .into_iter()
        .filter(|event| event["name"].as_str().unwrap().starts_with("ScheduledMessage"))
        .collect();
    assert!(events.len() >= 7, "{events:?}");
    for event in events {
        assert_eq!(event["audience"], json!({ "users": [owner.id] }), "{event}");
    }
}

#[tokio::test]
async fn a_message_is_not_scheduled_outside_the_window_past_the_limit_or_in_an_encrypted_conversation() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let general = general(&server);

    for send_at in [Some(ahead(Duration::from_secs(10))), Some(ahead(31 * 24 * 60 * MINUTE)), None] {
        let refused = schedule(&owner, &general, json!({ "content": "when?", "sendAt": send_at })).await;
        assert_eq!(refused.status, 400, "{send_at:?}");
        assert!(refused.json::<Value>()["errors"]["sendAt"].is_array(), "{}", refused.text());
    }

    for n in 0..25 {
        scheduled(&owner, &general, &format!("number {n}"), 5 * MINUTE).await;
    }
    let over = schedule(&owner, &general, json!({ "content": "one too many", "sendAt": ahead(5 * MINUTE) })).await;
    assert_eq!(over.status, 409);
    assert_eq!(over.json::<Value>()["code"], "scheduled-limit");

    let opened = maya
        .client
        .post_json("/api/social/conversations", &json!({ "userId": owner.id }))
        .await;
    assert_eq!(opened.status, 200, "{}", opened.text());
    let conversation = opened.json::<Value>()["id"].as_str().unwrap().to_string();
    server
        .database()
        .execute(
            "UPDATE \"DirectConversations\" SET \"EncryptionState\" = 'On' WHERE \"Id\" = ?1",
            [Id::parse(&conversation).unwrap()],
        )
        .unwrap();
    let encrypted = maya
        .client
        .post_json(
            &format!("/api/social/conversations/{conversation}/scheduled"),
            &json!({ "content": "secret", "sendAt": ahead(5 * MINUTE) }),
        )
        .await;
    assert_eq!(encrypted.status, 409, "{}", encrypted.text());
    assert_eq!(encrypted.json::<Value>()["code"], "scheduled-encrypted");
}
