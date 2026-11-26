//! A legacy PIN backup across the upgrade, a real full ZIP, and a fresh host.
use std::fs;

use aws_lc_rs::digest::{SHA256, digest};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::backups::restore;
use stellarr_api::values::{Id, Timestamp};
use stellarr_archive::ZipArchive;

use super::kit::{ACCOUNT, backup, bytes, post, put, read, set_up, text};
use crate::backups::settled;
use crate::support::{PASSWORD, Server, TempDir};

#[tokio::test]
async fn a_migrated_pin_backup_needs_the_separate_key_after_a_full_zip_restore() {
    let source = TempDir::new();
    let server = Server::start_in(source.path(), &[]).await;
    let owner = server.found("owner").await;
    let account = set_up(&owner.client).await;
    let db_path = server.database_path();
    assert!(server.stop().await);

    // Recreate the previous disk format, including superseded bytes on free
    // pages. The browser's backup, account key and prior attempts must survive.
    let verifier = digest(&SHA256, &account.auth_key);
    let previous = vec![0xa7u8; 1024];
    let ciphertext = bytes(64, 3);
    let user = Id::parse(&owner.id).unwrap();
    let updated = Timestamp::now();
    {
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(
            "PRAGMA secure_delete=OFF;
             DROP TABLE ProtectedAccountKeyBackups;
             CREATE TABLE AccountKeyBackups (UserId TEXT PRIMARY KEY, Salt BLOB, Iterations INTEGER,
                Verifier BLOB, Iv BLOB, Ciphertext BLOB, FailedAttempts INTEGER, UpdatedAt TEXT);
             DELETE FROM CodeRecoveryAccounts; PRAGMA user_version=1;",
        )
        .unwrap();
        db.execute(
            "INSERT INTO AccountKeyBackups VALUES (?1, ?2, 600000, ?3, ?4, ?5, 3, ?6)",
            params![
                user,
                bytes(16, 1),
                verifier.as_ref(),
                bytes(12, 2),
                ciphertext,
                updated
            ],
        )
        .unwrap();
        // Populate enough pages that a page-by-page snapshot could retain a
        // retired backup, even when the current row has been migrated.
        for _ in 0..16 {
            db.execute(
                "INSERT INTO AccountKeyBackups (UserId, Ciphertext) VALUES (?1, ?2)",
                params![Id::random(), previous],
            )
            .unwrap();
        }
        db.execute("DELETE FROM AccountKeyBackups WHERE UserId <> ?1", [user])
            .unwrap();
    }
    let legacy_file = fs::read(&db_path).unwrap();
    assert!(legacy_file.windows(64).any(|part| part == &previous[..64]));
    fs::remove_file(source.path().join("pin-recovery/master.key")).unwrap();
    let upgraded = Server::start_in(source.path(), &[]).await;
    let client = upgraded.client();
    assert_eq!(upgraded.login(&client, "owner", PASSWORD).await.status, 200);
    let status = read(&client, ACCOUNT).await;
    assert_eq!(status["backup"]["remainingAttempts"], 7);
    assert_eq!(status["backup"]["updatedAt"], updated.to_wire_text());

    let job = client
        .post_json(
            "/api/cockpit/backups",
            &json!({"database":true, "objects":true, "keys":true, "instance":true}),
        )
        .await;
    assert_eq!(job.status, 202);
    let list = settled(&client).await;
    assert_eq!(list["lastFailure"], Value::Null);
    let path = source
        .path()
        .join("backups")
        .join(job.json::<Value>()["id"].as_str().unwrap());
    let key_path = "pin-recovery/master.key";
    let key = fs::read(source.path().join(key_path)).unwrap();
    let mut zip = ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    assert!(zip.find(b"keys/master.key").is_some());
    assert_ne!(
        key,
        fs::read(source.path().join("keys/master.key")).unwrap()
    );
    for index in 0..zip.entries().len() {
        let entry = zip.read_to_vec(index, 32 * 1024 * 1024).unwrap();
        for secret in [&key[..], verifier.as_ref(), &ciphertext, &previous[..64]] {
            assert!(!entry.windows(secret.len()).any(|part| part == secret));
        }
    }

    let target = TempDir::new();
    let apply = || restore::apply(&path, target.path(), Timestamp::now());
    assert!(apply().unwrap_err().to_string().contains(key_path));
    assert!(!target.path().join(key_path).exists());
    fs::create_dir_all(target.path().join("pin-recovery")).unwrap();
    fs::copy(
        source.path().join("keys/master.key"),
        target.path().join(key_path),
    )
    .unwrap();
    assert!(apply().unwrap_err().to_string().contains(key_path));
    assert!(!target.path().join("db").exists());

    fs::copy(source.path().join(key_path), target.path().join(key_path)).unwrap();
    apply().unwrap();
    let restored = Server::start_in(target.path(), &[]).await;
    let device = restored.client();
    assert_eq!(restored.login(&device, "owner", PASSWORD).await.status, 200);
    let unlock = "/api/me/encryption/backup/unlock";
    let wrong = post(
        &device,
        unlock,
        &json!({"version": 1, "authKey":text(&bytes(32,9))}),
    )
    .await;
    assert_eq!(wrong.json::<Value>()["remainingAttempts"], 6);
    let right = post(
        &device,
        unlock,
        &json!({"version": 1, "authKey":text(&account.auth_key)}),
    )
    .await;
    assert_eq!(right.status, 200);
    let answer: Value = right.json();
    assert_eq!(answer["accountKeyId"], account.key_id);
    assert_eq!(answer["iv"], text(&bytes(12, 2)));
    assert_eq!(answer["ciphertext"], text(&ciphertext));

    let sealed_pin: String = restored
        .database()
        .query_row(
            "SELECT Sealed FROM ProtectedAccountKeyBackups WHERE UserId=?1",
            [user],
            |r| r.get(0),
        )
        .unwrap();
    let code_proof = bytes(32, 87);
    let replacement = backup(&code_proof);
    let reset = "/api/me/encryption/account-key/reset";
    let migrate = json!({ "currentPassword": PASSWORD, "algorithm": "x25519", "publicKey": text(&bytes(32, 17)),
        "migrateFromKeyId": account.key_id, "backup": replacement });
    let saved = post(&device, reset, &migrate).await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    let current = saved.json::<Value>()["accountKey"]["keyId"].clone();
    assert_ne!(current, account.key_id);
    assert_eq!(
        post(&device, reset, &migrate).await.json::<Value>()["accountKey"]["keyId"],
        current
    );
    let mut stale = migrate.clone();
    stale["publicKey"] = json!(text(&bytes(32, 18)));
    assert_eq!(post(&device, reset, &stale).await.status, 409);
    let old_proof = json!({"version":1, "authKey":text(&account.auth_key)});
    assert_eq!(post(&device, unlock, &old_proof).await.status, 404);
    assert_eq!(
        post(&device, unlock, &json!({"authKey":text(&account.auth_key)}))
            .await
            .status,
        404
    );
    let mut old_backup = replacement.clone();
    old_backup.as_object_mut().unwrap().remove("version");
    let downgrade =
        json!({ "currentPassword": PASSWORD, "accountKeyId": current, "backup": old_backup });
    assert_eq!(
        put(&device, "/api/me/encryption/backup", &downgrade)
            .await
            .status,
        400
    );
    assert_eq!(
        post(
            &device,
            unlock,
            &json!({"version":2, "authKey":text(&code_proof)})
        )
        .await
        .status,
        200
    );
    assert_eq!(read(&device, ACCOUNT).await["recoveryVersion"], 2);
    assert!(restored.stop().await);
    assert!(
        !fs::read(target.path().join("db/stellarr.db"))
            .unwrap()
            .windows(sealed_pin.len())
            .any(|w| w == sealed_pin.as_bytes())
    );
    assert!(
        apply()
            .unwrap_err()
            .to_string()
            .contains("reactivate a retired")
    );
    let current_host = Server::start_in(target.path(), &[]).await;
    let another = current_host.client();
    assert_eq!(
        current_host.login(&another, "owner", PASSWORD).await.status,
        200
    );
    assert_eq!(post(&another, unlock, &old_proof).await.status, 404);
    assert_eq!(
        post(
            &another,
            unlock,
            &json!({"version":2, "authKey":text(&code_proof)})
        )
        .await
        .status,
        200
    );
}
