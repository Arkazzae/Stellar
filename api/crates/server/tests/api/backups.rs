//! The panel's routes for backups, against the binary: what the list says,
//! what each way of making a backup answers, verifying and downloading one,
//! and who may do any of it.

use std::time::Duration;

use axum::http::Method;
use serde_json::{Value, json};

use crate::support::{Client, Request, Server};

/// The list, until no backup is being written.
async fn settled(client: &Client) -> Value {
    for _ in 0..200 {
        let list: Value = client.get("/api/cockpit/backups").await.json();
        if list["job"].is_null() {
            return list;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the backup did not finish");
}

#[tokio::test]
async fn a_backup_of_the_database_alone_is_made_at_once_and_listed() {
    let server = Server::start().await;
    let owner = server.found("owner").await;

    let before: Value = owner.client.get("/api/cockpit/backups").await.json();
    assert_eq!(before["backups"], json!([]));
    assert_eq!(before["retentionDays"], 14);
    assert_eq!(before["job"], Value::Null);
    assert_eq!(before["lastFailure"], Value::Null);
    assert_eq!(before["automation"]["enabled"], false);

    let created = owner.client.send(Request::new(Method::POST, "/api/cockpit/backups")).await;
    assert_eq!(created.status, 201, "{}", created.text());
    let backup: Value = created.json();
    let id = backup["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("backup-") && id.ends_with(".db"), "{id}");
    assert_eq!(backup["parts"], json!(["database"]));
    assert_eq!(backup["verifiedAt"], Value::Null);
    assert_eq!(created.header("location"), Some(format!("/api/cockpit/backups/{id}").as_str()));

    let after = settled(&owner.client).await;
    assert_eq!(after["backups"][0]["id"], id.as_str());
}

#[tokio::test]
async fn a_full_backup_is_written_in_the_background_verified_and_downloaded() {
    let server = Server::start().await;
    let owner = server.found("owner").await;

    let started = owner
        .client
        .post_json(
            "/api/cockpit/backups",
            &json!({"database": true, "objects": true, "keys": true, "instance": true}),
        )
        .await;
    assert_eq!(started.status, 202, "{}", started.text());
    let job: Value = started.json();
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["parts"], json!(["database", "objects", "keys", "instance"]));

    let list = settled(&owner.client).await;
    assert_eq!(list["lastFailure"], Value::Null, "{list}");
    assert_eq!(list["backups"][0]["id"], id.as_str());

    let verified = owner
        .client
        .send(Request::new(Method::POST, &format!("/api/cockpit/backups/{id}/verify")))
        .await;
    assert_eq!(verified.status, 200, "{}", verified.text());
    assert!(verified.json::<Value>()["verifiedAt"].is_string());

    let download = owner.client.get(&format!("/api/cockpit/backups/{id}/download")).await;
    assert_eq!(download.status, 200);
    assert_eq!(download.header("content-type"), Some("application/zip"));
    assert_eq!(&download.body[..2], b"PK");
}

#[tokio::test]
async fn nobody_but_the_owner_gets_in() {
    let server = Server::start().await;
    let _owner = server.found("owner").await;
    let member = server.register("maya").await;

    assert_eq!(member.client.get("/api/cockpit/backups").await.status, 403);
    assert_eq!(server.anonymous().get("/api/cockpit/backups").await.status, 401);
}
