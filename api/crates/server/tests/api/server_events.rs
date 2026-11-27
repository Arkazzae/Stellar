//! Server events through the real binary: who may make them, their limits,
//! the card an announcement carries, a seat that goes to the waitlist with a
//! notice in the inbox, and cancelling. What the worker does with the clock
//! is in the crate's own tests.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Timestamp;

use crate::support::{Person, Request, Response, Server};

const MINUTE: Duration = Duration::from_secs(60);

struct World {
    server: Server,
    owner: Person,
    maya: Person,
    joe: Person,
    general: String,
    instance: String,
}

async fn world() -> World {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let joe = server.register("joe").await;
    let db = server.database();
    let general: String = db
        .query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    World { server, owner, maya, joe, general: general.to_lowercase(), instance: instance.to_lowercase() }
}

fn ahead(by: Duration) -> String {
    Timestamp::now().plus(by).to_wire_text()
}

fn elsewhere(seats: Option<i64>) -> Value {
    json!({
        "name": "Board games",
        "place": { "kind": "elsewhere", "text": "The usual table" },
        "startsAt": ahead(120 * MINUTE),
        "endsAt": ahead(240 * MINUTE),
        "timeZone": "Europe/Warsaw",
        "seats": seats,
        "repeat": "none",
    })
}

impl World {
    async fn create(&self, who: &Person, body: Value) -> Response {
        who.client.post_json("/api/events", &body).await
    }

    async fn made(&self, who: &Person, body: Value) -> Value {
        let response = self.create(who, body).await;
        assert_eq!(response.status, 201, "{}", response.text());
        response.json()
    }

    fn signup_path(event: &Value) -> String {
        format!(
            "/api/events/{}/occurrences/{}/signup",
            event["id"].as_str().unwrap(),
            event["occurrenceId"].as_str().unwrap()
        )
    }

    async fn sign(&self, who: &Person, event: &Value, method: Method) -> Response {
        who.client.send(Request::new(method, &Self::signup_path(event))).await
    }

    fn events(&self, name: &str) -> Vec<Value> {
        self.server.events().into_iter().filter(|event| event["name"] == name).collect()
    }
}

#[tokio::test]
async fn only_holders_of_the_permission_create_events_and_no_channel_can_grant_it() {
    let w = world().await;
    let refused = w.create(&w.maya, elsewhere(None)).await;
    assert_eq!(refused.status, 403, "plain members do not create events by default");
    w.made(&w.owner, elsewhere(None)).await;

    // A channel override cannot give it: it is the server's alone.
    let base: String = w
        .server
        .database()
        .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
        .unwrap();
    let base = base.to_lowercase();
    let override_ = w
        .owner
        .client
        .send(
            Request::new(Method::PUT, &format!("/api/admin/channels/{}/permissions/{base}", w.general))
                .json(&json!({ "allow": ["createEvents"], "deny": [] })),
        )
        .await;
    assert_eq!(override_.status, 400, "{}", override_.text());

    // A role can.
    w.server
        .database()
        .execute(
            "UPDATE \"ServerRoles\" SET \"Permissions\" = \"Permissions\" | ?2 WHERE \"Id\" = ?1",
            params![base.to_uppercase(), 1_i64 << 15],
        )
        .unwrap();
    w.made(&w.maya, elsewhere(None)).await;
    let me: Value = w.maya.client.get("/api/bootstrap").await.json();
    assert!(me.to_string().contains("createEvents"), "the member's permissions name it");
}

#[tokio::test]
async fn an_event_is_refused_past_each_limit() {
    let w = world().await;
    let with = |field: &str, value: Value| {
        let mut body = elsewhere(None);
        body[field] = value;
        body
    };
    let start = Timestamp::now().plus(120 * MINUTE);
    let cases = [
        (with("name", json!("n".repeat(101))), "name"),
        (with("name", json!("  ")), "name"),
        (with("description", json!("d".repeat(1001))), "description"),
        (with("place", json!({ "kind": "elsewhere", "text": " " })), "place.text"),
        (with("place", json!({ "kind": "voice", "channelId": w.general })), "place.channelId"),
        (with("timeZone", json!("Mars/Olympus")), "timeZone"),
        (with("startsAt", json!(ahead(366 * 24 * 60 * MINUTE))), "startsAt"),
        (with("startsAt", json!(Timestamp::now().minus(10 * MINUTE).to_wire_text())), "startsAt"),
        (with("endsAt", Value::Null), "endsAt"),
        (with("endsAt", json!(start.minus(MINUTE).to_wire_text())), "endsAt"),
        (with("endsAt", json!(start.plus(8 * 24 * 60 * MINUTE).to_wire_text())), "endsAt"),
        (with("seats", json!(0)), "seats"),
        (with("seats", json!(1000)), "seats"),
        (with("repeat", json!("daily")), "repeat"),
    ];
    for (mut body, field) in cases {
        if field != "startsAt" && field != "endsAt" {
            body["startsAt"] = json!(start.to_wire_text());
            body["endsAt"] = json!(start.plus(60 * MINUTE).to_wire_text());
        }
        let refused = w.create(&w.owner, body).await;
        assert_eq!(refused.status, 400, "{field}: {}", refused.text());
        assert!(refused.json::<Value>()["errors"][field].is_array(), "{field}: {}", refused.text());
    }
    // An occurrence of a weekly event ends before the next one starts.
    let mut weekly = with("repeat", json!("weekly"));
    weekly["startsAt"] = json!(start.to_wire_text());
    weekly["endsAt"] = json!(start.plus(7 * 24 * 60 * MINUTE).to_wire_text());
    let refused = w.create(&w.owner, weekly).await;
    assert!(refused.json::<Value>()["errors"]["endsAt"].is_array());
    let stored: i64 = w.server.database().query_row("SELECT COUNT(*) FROM \"ServerEvents\"", [], |row| row.get(0)).unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn a_freed_seat_goes_to_the_first_waiting_with_a_notice_and_the_card_follows() {
    let w = world().await;
    let mut body = elsewhere(Some(1));
    body["announceChannelId"] = json!(w.general);
    let event = w.made(&w.owner, body).await;
    let message = event["announceMessageId"].as_str().unwrap().to_string();

    // The announcement carries the card, shared with nobody's own standing.
    let created = w.events("MessageCreated").into_iter().find(|created| created["payload"]["id"] == message.as_str()).unwrap();
    assert_eq!(created["payload"]["event"]["id"], event["id"]);
    assert_eq!(created["payload"]["event"]["mine"], Value::Null);

    let signed: Value = w.sign(&w.maya, &event, Method::PUT).await.json();
    assert_eq!((signed["mine"].clone(), signed["signedUp"].clone()), (json!("signed"), json!(1)));
    let waiting: Value = w.sign(&w.joe, &event, Method::PUT).await.json();
    assert_eq!((waiting["mine"].clone(), waiting["myPosition"].clone()), (json!("waiting"), json!(1)));
    let page: Value = w.joe.client.get(&format!("/api/channels/{}/messages", w.general)).await.json();
    let sync_from = page["syncCursor"].as_str().unwrap().to_string();

    let out = w.sign(&w.maya, &event, Method::DELETE).await;
    assert_eq!(out.status, 200, "{}", out.text());
    assert_eq!(out.json::<Value>()["mine"], "none");

    // Joe has the seat, and his inbox says so with a link to the event.
    let inbox: Value = w.joe.client.get("/api/notifications").await.json();
    let notice = inbox["items"].as_array().unwrap().iter().find(|item| item["source"]["moduleId"] == "seat").unwrap();
    assert_eq!(
        notice["target"],
        json!({
            "type": "event",
            "channelId": null,
            "inviteCode": null,
            "serverId": w.instance,
            "serverName": null,
            "invitedByName": null,
            "eventId": event["id"],
        })
    );
    let mine: Vec<Value> = w.events("EventSignupChanged").into_iter().filter(|told| told["payload"]["mine"] == "signed").collect();
    assert_eq!(mine.last().unwrap()["audience"], json!({ "users": [w.joe.id] }), "where Joe stands goes to Joe");
    let changes = w.events("EventChanged");
    let revisions: Vec<i64> = changes.iter().map(|told| told["payload"]["revision"].as_i64().unwrap()).collect();
    assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]), "{revisions:?}");
    assert!(changes.iter().all(|told| told["payload"]["mine"].is_null()));

    // A client that missed it all catches up through the channel's journal.
    let sync: Value = w
        .joe
        .client
        .get(&format!("/api/channels/{}/messages/sync?after={sync_from}", w.general))
        .await
        .json();
    let card = sync["changes"].as_array().unwrap().iter().rev().find(|change| change["messageId"] == message.as_str()).unwrap();
    assert_eq!((card["message"]["event"]["mine"].clone(), card["message"]["event"]["signedUp"].clone()), (json!("signed"), json!(1)));
}

#[tokio::test]
async fn a_cancelled_event_leaves_the_list_its_card_says_so_and_nobody_signs_up_after() {
    let w = world().await;
    let event = w.made(&w.owner, elsewhere(None)).await;
    w.sign(&w.maya, &event, Method::PUT).await;
    let listed: Value = w.joe.client.get("/api/events").await.json();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let path = format!("/api/events/{}", event["id"].as_str().unwrap());
    assert_eq!(w.joe.client.send(Request::new(Method::DELETE, &path)).await.status, 403, "neither its creator nor a manager");
    assert_eq!(w.owner.client.send(Request::new(Method::DELETE, &path)).await.status, 204);

    assert_eq!(w.joe.client.get("/api/events").await.json::<Value>(), json!([]));
    let card: Value = w.maya.client.get(&path).await.json();
    assert_eq!(card["state"], "cancelled");
    let inbox: Value = w.maya.client.get("/api/notifications").await.json();
    assert!(inbox["items"].as_array().unwrap().iter().any(|item| item["source"]["moduleId"] == "cancelled"));
    let refused = w.sign(&w.joe, &event, Method::PUT).await;
    assert_eq!(refused.status, 409);
    assert_eq!(refused.json::<Value>()["code"], "event-over");
}
