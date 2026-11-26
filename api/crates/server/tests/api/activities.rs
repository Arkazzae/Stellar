//! Personal sessions are account-scoped even when both users launch one release.
use crate::support::{Person, Request, Response, Server};
use http::Method;
use rusqlite::params;
use serde_json::{Value, json};

const ORIGIN: &str = "https://chat.test";
const INSTALL: &str = "9f77e982-01b8-4934-ada0-1fb14cc3dd71";
const VOICE_ONLY: &str = "18f8c6a1-26ea-471b-a6db-eb11c353095c";
const FRAME: &str = "05245c20-5a7a-4464-a422-595ff87e9e84";

async fn post(person: &Person, path: &str, data: Value) -> Response {
    person
        .client
        .send(
            Request::new(Method::POST, path)
                .header("Origin", ORIGIN)
                .header("X-Forwarded-Host", "chat.test")
                .header("X-Forwarded-Proto", "https")
                .json(&data),
        )
        .await
}
fn seed(server: &Server, id: &str, personal: bool) {
    let db = server.database();
    let mut manifest = json!({"id":if personal{"personal-demo"}else{"voice-demo"},"name":"Demo","version":"1.0.0","description":"Board","protocol":1});
    if personal {
        manifest["launchModes"] = json!(["voice", "personal"]);
    }
    let digest = "b".repeat(64);
    db.execute("INSERT INTO ActivityInstalls(Id,Source,Activity,Digest,Enabled) VALUES(?1,'https://github.com/test/demo',?2,?3,1)",
        params![id.to_uppercase(),manifest["id"].as_str().unwrap(),digest]).unwrap();
    db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?1,?2,'commit','release',?3,200)",
        params![id.to_uppercase(),digest,manifest.to_string()]).unwrap();
}
async fn launch(who: &Person, id: &str) -> Response {
    post(
        who,
        "/api/activities/personal/launch",
        json!({"install":id,"device":"desktop"}),
    )
    .await
}
async fn room(who: &Person, token: &Value, mut action: Value) -> Response {
    action["frame"] = json!(FRAME);
    post(
        who,
        "/api/activities/personal/room",
        json!({"install":INSTALL,"access":token,"action":action}),
    )
    .await
}

#[tokio::test]
async fn independent_authenticated_personal_sessions_enforce_launch_modes_tokens_and_revocation() {
    let server = Server::start_with(&[
        ("STELLARR_ACTIVITY_DOMAIN", "games.test"),
        ("STELLARR_ACTIVITY_ORIGIN", ORIGIN),
    ])
    .await;
    let alice = server.found("owner").await;
    let bob = server.register("visitor").await;
    seed(&server, INSTALL, true);
    seed(&server, VOICE_ONLY, false);
    let list = post(
        &bob,
        "/api/activities/personal/list",
        json!({"device":"desktop"}),
    )
    .await;
    assert_eq!(list.status, 200, "{}", list.text());
    let entries = list.json::<Value>();
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["install"], INSTALL);
    assert_eq!(launch(&bob, VOICE_ONLY).await.status, 403);
    let first = launch(&alice, INSTALL).await;
    let second = launch(&bob, INSTALL).await;
    assert_eq!(first.status, 200, "{}", first.text());
    assert_eq!(second.status, 200, "{}", second.text());
    let a = first.json::<Value>();
    let b = second.json::<Value>();
    let open = json!({"op":"open","initialState":{"score":0}});
    let a_open = room(&alice, &a["access"], open.clone())
        .await
        .json::<Value>();
    let b_open = room(&bob, &b["access"], open.clone()).await.json::<Value>();
    assert_ne!(a_open["sessionId"], b_open["sessionId"]);
    assert_eq!(a_open["launchMode"], "personal");
    assert!(a_open["place"].is_null());
    assert_eq!(a_open["standalone"], false);
    assert_eq!(b_open["participants"].as_array().unwrap().len(), 1);
    assert_eq!(
        room(
            &alice,
            &a["access"],
            json!({"op":"write","sequence":1,"patch":{"score":8}})
        )
        .await
        .status,
        200
    );
    let snapshot = room(&bob, &b["access"], json!({"op":"read","after":-1}))
        .await
        .json::<Value>();
    assert_eq!(snapshot["changes"][0]["state"]["score"], 0);
    assert_eq!(
        room(&bob, &a["access"], json!({"op":"read","after":-1}))
            .await
            .status,
        403
    );
    let as_channel = format!("/api/activities/{}/launch", alice.id);
    assert_eq!(
        post(
            &alice,
            &as_channel,
            json!({"install":INSTALL,"device":"desktop"})
        )
        .await
        .status,
        403
    );
    let manage = format!("/api/cockpit/activities/{INSTALL}/manage");
    assert_eq!(
        post(&alice, &manage, json!({"action":"disable"}))
            .await
            .status,
        200
    );
    assert_eq!(
        room(&bob, &b["access"], json!({"op":"read","after":-1}))
            .await
            .status,
        410
    );
    assert_eq!(
        post(&alice, &manage, json!({"action":"enable"}))
            .await
            .status,
        200
    );
    let resumed = launch(&alice, INSTALL).await.json::<Value>();
    let fresh = room(&alice, &resumed["access"], open).await.json::<Value>();
    assert_ne!(a_open["sessionId"], fresh["sessionId"]);
    assert_eq!(fresh["changes"][0]["state"]["score"], 0);
    // A newly approved, voice-only release cannot change an existing personal
    // room's grants. After the update only the account with that pin may resume.
    {
        let db = server.database();
        let manifest = json!({"id":"personal-demo","name":"Demo","version":"2.0.0","description":"New voice version","protocol":1,"launchModes":["voice"]});
        let digest = "c".repeat(64);
        db.execute("INSERT INTO ActivityVersions(Install,Digest,Revision,Release,Manifest,Bytes) VALUES(?1,?2,'commit2','release2',?3,201)",
            params![INSTALL.to_uppercase(),digest,manifest.to_string()]).unwrap();
        db.execute(
            "UPDATE ActivityInstalls SET Digest=?2 WHERE Id=?1",
            params![INSTALL.to_uppercase(), digest],
        )
        .unwrap();
    }
    let pinned = post(
        &alice,
        "/api/activities/personal/list",
        json!({"device":"desktop"}),
    )
    .await
    .json::<Value>();
    assert_eq!(pinned[0]["version"], "1.0.0");
    assert_eq!(launch(&alice, INSTALL).await.status, 200);
    assert_eq!(launch(&bob, INSTALL).await.status, 403);
    assert_eq!(
        post(
            &bob,
            "/api/activities/personal/list",
            json!({"device":"desktop"})
        )
        .await
        .json::<Value>(),
        json!([])
    );
}
