//! A device belongs to a signed-in session and ends with it: signing out, a
//! session revoked from elsewhere or run out, a changed password, a reset by
//! mail and a disabled account each take it out of what the people of an
//! encrypted conversation are told and asked to wrap keys for, without a
//! write to the device itself.

use rusqlite::{Connection, params};
use serde_json::Value;
use stellarr_api::auth::recovery;
use stellarr_api::values::{Id, Timestamp};

use super::kit::*;
use crate::support::Client;

/// The row of a device as it is stored, every column.
fn row_of(connection: &Connection, device: &str) -> String {
    connection
        .query_row(
            "SELECT \"Id\" || '|' || \"UserId\" || '|' || \"SessionId\" || '|' || \"Algorithm\" || '|' || hex(\"PublicKey\") || '|'
                || \"Label\" || '|' || \"CreatedAt\" || '|' || \"LastSeenAt\" || '|' || COALESCE(\"RevokedAt\", 'null')
             FROM \"EncryptionDevices\" WHERE \"Id\" = ?1",
            [Id::parse(device).unwrap()],
            |row| row.get(0),
        )
        .unwrap()
}

struct Scene {
    crowd: Crowd,
    conversation: String,
    laptop: Client,
    laptop_device: String,
    own_device: String,
}

/// Maja with two browsers, in an encrypted conversation with Ada whose epoch
/// reaches both.
async fn scene() -> Scene {
    let crowd = Crowd::ready().await;
    let laptop = crowd.server.sign_in(&crowd.maja).await;
    let laptop_device = register_device_with(&laptop, &bytes(32, 60), "Laptop").await.json::<Value>()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let own_device = read(&crowd.maja.client, &format!("{ACCOUNT}/devices")).await["devices"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    rotate(&crowd, &crowd.ada.client, &conversation).await;
    Scene { crowd, conversation, laptop, laptop_device, own_device }
}

impl Scene {
    async fn maja_devices_as_ada_sees_them(&self) -> Vec<String> {
        let keys = self.crowd.keys(&self.crowd.ada.client, &self.conversation).await;
        keys["peer"]["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|device| device["keyId"].as_str().unwrap().to_string())
            .collect()
    }
}

/// What must hold once the laptop's session has ended, whatever ended it.
async fn assert_laptop_is_gone(scene: &Scene, before: &str) {
    let crowd = &scene.crowd;
    assert_eq!(row_of(&crowd.server.database(), &scene.laptop_device), before, "nothing is written on the device");

    let seen = scene.maja_devices_as_ada_sees_them().await;
    assert!(!seen.contains(&scene.laptop_device), "{seen:?}");
    let recipients = crowd.recipients(&crowd.ada.client, &scene.conversation).await;
    assert!(!recipients.contains(&scene.laptop_device), "{recipients:?}");
    // An epoch that is still wrapped for it is refused as wrapped for what does not belong.
    let current = crowd.keys(&crowd.ada.client, &scene.conversation).await["encryption"]["currentEpochId"].as_str().unwrap().to_string();
    let mut with_it = recipients.clone();
    with_it.push(scene.laptop_device.clone());
    let refused = post_epoch(&crowd.ada.client, &scene.conversation, &epoch_body(&Id::random().to_string(), Some(&current), &with_it)).await;
    assert_eq!(refusal(&refused), (409, Some("epoch-coverage".into())));
    assert_eq!(refused.json::<Value>()["extra"][0], scene.laptop_device.as_str());
    // And one wrapped for the keys that are left is taken.
    let fresh = post_epoch(&crowd.ada.client, &scene.conversation, &epoch_body(&Id::random().to_string(), Some(&current), &recipients)).await;
    assert_eq!(fresh.status, 201, "{}", fresh.text());
}

#[tokio::test]
async fn signing_out_ends_the_device() {
    let scene = scene().await;
    let before = row_of(&scene.crowd.server.database(), &scene.laptop_device);

    assert_eq!(scene.laptop.post_json("/api/auth/logout", &serde_json::json!({})).await.status, 204);

    assert_laptop_is_gone(&scene, &before).await;
    let own = read(&scene.crowd.maja.client, &format!("{ACCOUNT}/devices")).await;
    assert_eq!(own["devices"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_session_revoked_from_elsewhere_or_run_out_ends_the_device() {
    for (what, statement) in [
        ("revoked", "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1"),
        ("run out", "UPDATE \"UserSessions\" SET \"ExpiresAt\" = ?2 WHERE \"Id\" = ?1"),
        ("past its absolute end", "UPDATE \"UserSessions\" SET \"AbsoluteExpiresAt\" = ?2 WHERE \"Id\" = ?1"),
        ("of an older version", "UPDATE \"UserSessions\" SET \"SecurityVersion\" = 99 WHERE \"Id\" = ?1 AND ?2 IS NOT NULL"),
    ] {
        let scene = scene().await;
        let connection = scene.crowd.server.database();
        let before = row_of(&connection, &scene.laptop_device);
        let session: Id = connection
            .query_row("SELECT \"SessionId\" FROM \"EncryptionDevices\" WHERE \"Id\" = ?1", [Id::parse(&scene.laptop_device).unwrap()], |row| row.get(0))
            .unwrap();

        connection.execute(statement, params![session, Timestamp::now().minus(std::time::Duration::from_secs(5))]).unwrap();

        assert_laptop_is_gone(&scene, &before).await;
        assert_eq!(scene.laptop.get("/api/me").await.status, 401, "{what}");
    }
}

#[tokio::test]
async fn a_changed_password_ends_the_devices_of_every_other_session() {
    let scene = scene().await;
    let mut connection = scene.crowd.server.database();
    let before = row_of(&connection, &scene.laptop_device);
    let own_before = row_of(&connection, &scene.own_device);
    let main: Id = connection
        .query_row("SELECT \"SessionId\" FROM \"EncryptionDevices\" WHERE \"Id\" = ?1", [Id::parse(&scene.own_device).unwrap()], |row| row.get(0))
        .unwrap();

    let tx = connection.transaction().unwrap();
    recovery::credentials_changed(&tx, Id::parse(&scene.crowd.maja.id).unwrap(), Some(main), Timestamp::now()).unwrap();
    tx.commit().unwrap();

    assert_laptop_is_gone(&scene, &before).await;
    // The session that changed it keeps its device, and its row is as it was.
    assert_eq!(scene.maja_devices_as_ada_sees_them().await, std::slice::from_ref(&scene.own_device));
    assert_eq!(row_of(&connection, &scene.own_device), own_before);
}

#[tokio::test]
async fn a_reset_by_mail_ends_every_device_of_the_account_and_keeps_its_key() {
    let scene = scene().await;
    let mut connection = scene.crowd.server.database();
    let before = row_of(&connection, &scene.laptop_device);
    let own_before = row_of(&connection, &scene.own_device);

    let tx = connection.transaction().unwrap();
    recovery::credentials_changed(&tx, Id::parse(&scene.crowd.maja.id).unwrap(), None, Timestamp::now()).unwrap();
    tx.commit().unwrap();

    assert_laptop_is_gone(&scene, &before).await;
    assert!(scene.maja_devices_as_ada_sees_them().await.is_empty());
    assert_eq!(row_of(&connection, &scene.own_device), own_before);
    let keys = scene.crowd.keys(&scene.crowd.ada.client, &scene.conversation).await;
    assert!(keys["peer"]["accountKey"]["keyId"].is_string(), "the account key stays");
    assert_eq!(scene.crowd.maja.client.get("/api/me").await.status, 401);
}

#[tokio::test]
async fn a_disabled_account_has_no_devices_that_take_part() {
    let scene = scene().await;
    let connection = scene.crowd.server.database();
    let before = row_of(&connection, &scene.laptop_device);
    let own_before = row_of(&connection, &scene.own_device);

    connection
        .execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1", [Id::parse(&scene.crowd.maja.id).unwrap()])
        .unwrap();

    assert_laptop_is_gone(&scene, &before).await;
    assert!(scene.maja_devices_as_ada_sees_them().await.is_empty());
    assert_eq!(row_of(&connection, &scene.own_device), own_before);

    // And it takes part again when the account does.
    connection
        .execute("UPDATE \"Users\" SET \"IsDisabled\" = 0 WHERE \"Id\" = ?1", [Id::parse(&scene.crowd.maja.id).unwrap()])
        .unwrap();
    assert!(scene.maja_devices_as_ada_sees_them().await.contains(&scene.own_device));
}
