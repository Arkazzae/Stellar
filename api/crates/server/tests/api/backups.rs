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

    let created = owner
        .client
        .send(Request::new(Method::POST, "/api/cockpit/backups"))
        .await;
    assert_eq!(created.status, 201, "{}", created.text());
    let backup: Value = created.json();
    let id = backup["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("backup-") && id.ends_with(".db"), "{id}");
    assert_eq!(backup["parts"], json!(["database"]));
    assert_eq!(backup["verifiedAt"], Value::Null);
    assert_eq!(
        created.header("location"),
        Some(format!("/api/cockpit/backups/{id}").as_str())
    );

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
    assert_eq!(
        job["parts"],
        json!(["database", "objects", "keys", "instance"])
    );

    let list = settled(&owner.client).await;
    assert_eq!(list["lastFailure"], Value::Null, "{list}");
    assert_eq!(list["backups"][0]["id"], id.as_str());

    let verified = owner
        .client
        .send(Request::new(
            Method::POST,
            &format!("/api/cockpit/backups/{id}/verify"),
        ))
        .await;
    assert_eq!(verified.status, 200, "{}", verified.text());
    assert!(verified.json::<Value>()["verifiedAt"].is_string());

    let download = owner
        .client
        .get(&format!("/api/cockpit/backups/{id}/download"))
        .await;
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
    assert_eq!(
        server.anonymous().get("/api/cockpit/backups").await.status,
        401
    );
}

const GATED: [(&str, &str); 9] = [
    ("GET", "/api/cockpit/backups"),
    ("POST", "/api/cockpit/backups"),
    ("GET", "/api/cockpit/backups/settings"),
    ("PUT", "/api/cockpit/backups/settings"),
    ("DELETE", "/api/cockpit/backups/settings"),
    ("GET", "/api/cockpit/integrations/cloudflare/r2"),
    ("PUT", "/api/cockpit/integrations/cloudflare/r2"),
    ("DELETE", "/api/cockpit/integrations/cloudflare/r2"),
    ("POST", "/api/cockpit/backups/backup-x.zip/verify"),
];

#[tokio::test]
async fn every_route_is_closed_to_a_member_and_to_nobody() {
    let server = Server::start().await;
    let _owner = server.found("owner").await;
    let member = server.register("maya").await;
    let anonymous = server.anonymous();

    let routes = GATED
        .iter()
        .chain(&[("GET", "/api/cockpit/backups/backup-x.zip/download")]);
    for (method, path) in routes {
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let request = || Request::new(method.clone(), path).json(&json!({}));
        assert_eq!(
            member.client.send(request()).await.status,
            403,
            "{method} {path}"
        );
        assert_eq!(
            anonymous.send(request()).await.status,
            401,
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn a_download_honours_ranges_and_names_the_file() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let created: Value = owner
        .client
        .send(Request::new(Method::POST, "/api/cockpit/backups"))
        .await
        .json();
    let id = created["id"].as_str().unwrap();
    let path = format!("/api/cockpit/backups/{id}/download");

    let whole = owner.client.get(&path).await;
    assert_eq!(whole.status, 200);
    assert_eq!(
        whole.header("content-type"),
        Some("application/vnd.sqlite3")
    );
    assert_eq!(whole.header("accept-ranges"), Some("bytes"));
    assert!(whole.header("content-disposition").unwrap().contains(id));

    let part = owner
        .client
        .send(Request::new(Method::GET, &path).header("Range", "bytes=0-15"))
        .await;
    assert_eq!(part.status, 206);
    assert_eq!(&part.body[..], b"SQLite format 3\0");
    assert_eq!(
        part.header("content-range"),
        Some(format!("bytes 0-15/{}", whole.body.len()).as_str())
    );
}

#[tokio::test]
async fn an_id_that_is_no_backup_is_not_found() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    for id in [
        "backup-missing.zip",
        "backup-x.txt",
        "other.zip",
        "..%2f..%2fetc%2fpasswd",
        "backup-%2e%2e%2fx.zip",
    ] {
        let verify = owner
            .client
            .send(Request::new(
                Method::POST,
                &format!("/api/cockpit/backups/{id}/verify"),
            ))
            .await;
        assert_eq!(verify.status, 404, "verify {id}");
        let download = owner
            .client
            .get(&format!("/api/cockpit/backups/{id}/download"))
            .await;
        assert_eq!(download.status, 404, "download {id}");
    }
}

#[tokio::test]
async fn verifying_a_file_that_is_not_a_database_is_refused_and_leaves_no_mark() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let folder = server.folder().join("backups");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("backup-20260101-000000-x.db"),
        b"this is not a database at all",
    )
    .unwrap();

    let refused = owner
        .client
        .send(Request::new(
            Method::POST,
            "/api/cockpit/backups/backup-20260101-000000-x.db/verify",
        ))
        .await;
    assert_eq!(refused.status, 409);
    assert_eq!(
        refused.json::<Value>()["title"],
        "The backup failed the integrity check."
    );
    assert!(!folder.join("backup-20260101-000000-x.db.verified").exists());
}

#[tokio::test]
async fn the_schedule_is_saved_read_back_and_reset_and_a_bad_one_changes_nothing() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let path = "/api/cockpit/backups/settings";
    let put = |body: Value| Request::new(Method::PUT, path).json(&body);

    let saved = owner
        .client
        .send(put(json!({"enabled": true, "intervalHours": 6, "retryMinutes": 10, "initialDelayMinutes": 0, "retentionDays": 30})))
        .await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    assert_eq!(
        saved.text(),
        r#"{"enabled":true,"intervalHours":6,"retryMinutes":10,"initialDelayMinutes":0,"retentionDays":30,"cleanupEnabled":true,"source":"panel"}"#
    );
    assert_eq!(owner.client.get(path).await.text(), saved.text());

    for (interval, retention) in [(0, 30), (8761, 30), (6, 0), (6, 3651)] {
        let refused = owner
            .client
            .send(put(json!({"enabled": false, "intervalHours": interval, "retryMinutes": 10, "initialDelayMinutes": 0, "retentionDays": retention})))
            .await;
        assert_eq!(refused.status, 400, "{interval} {retention}");
    }
    assert_eq!(owner.client.get(path).await.text(), saved.text());

    let reset = owner.client.send(Request::new(Method::DELETE, path)).await;
    let reset: Value = reset.json();
    assert_eq!(reset["source"], "environment");
    // Only the schedule goes back to the environment's: the retention stays.
    assert_eq!(reset["retentionDays"], 30);

    let lines: i64 = server
        .database()
        .query_row(
            "SELECT COUNT(*) FROM \"ServerAuditEntries\" WHERE \"Action\" = 'operator'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(lines, 2);
}

#[tokio::test]
async fn the_offsite_destination_needs_its_credentials_and_never_shows_them() {
    let server = Server::start_with(&[
        ("Cloudflare__R2__AccessKey", "AKIATESTACCESSKEY"),
        ("Cloudflare__R2__SecretKey", "TESTSECRETVALUE123"),
    ])
    .await;
    let owner = server.found("owner").await;
    let path = "/api/cockpit/integrations/cloudflare/r2";
    let body = json!({"enabled": true, "mode": "s3", "endpoint": "https://acct.r2.example", "region": "auto", "bucket": "bk", "prefix": "stellarr"});

    let saved = owner
        .client
        .send(Request::new(Method::PUT, path).json(&body))
        .await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    assert_eq!(saved.json::<Value>()["source"], "panel");
    let worker = owner.client.send(Request::new(Method::PUT, path).json(&json!({"enabled": true, "mode": "worker", "endpoint": "https://relay.example", "region": "auto", "bucket": "", "prefix": "stellarr"}))).await;
    assert_eq!(worker.status, 400, "no token is set");
    assert!(worker.json::<Value>()["errors"]["destination"].is_array());
    let http = owner.client.send(Request::new(Method::PUT, path).json(&json!({"enabled": true, "mode": "s3", "endpoint": "http://acct.r2.example", "region": "auto", "bucket": "bk", "prefix": "stellarr"}))).await;
    assert_eq!(http.status, 400);

    for answer in [
        saved.text(),
        owner.client.get(path).await.text(),
        worker.text(),
        http.text(),
    ] {
        assert!(
            !answer.contains("TESTSECRETVALUE123") && !answer.contains("AKIATESTACCESSKEY"),
            "{answer}"
        );
    }
    let log = server.log().join("\n");
    assert!(!log.contains("TESTSECRETVALUE123") && !log.contains("AKIATESTACCESSKEY"));
    let reset: Value = owner
        .client
        .send(Request::new(Method::DELETE, path))
        .await
        .json();
    assert_eq!(reset["source"], "environment");
    assert_eq!(reset["enabled"], false);
}

/// Gives the account a confirmed second step, and the session the time it
/// passed it (none: it did not).
fn add_second_step(server: &Server, username: &str, passed: Option<&str>) {
    let database = server.database();
    database
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\")
             SELECT lower(hex(randomblob(4))) || '-0000-4000-8000-' || lower(hex(randomblob(6))), \"Id\", 'totp', 'Phone', datetime('now'), datetime('now') FROM \"Users\" WHERE \"Username\" = ?1",
            [username],
        )
        .unwrap();
    database
        .execute(
            "UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = CASE WHEN ?2 IS NULL THEN NULL ELSE strftime('%Y-%m-%d %H:%M:%f', 'now', ?2) END
             WHERE \"UserId\" = (SELECT \"Id\" FROM \"Users\" WHERE \"Username\" = ?1)",
            rusqlite::params![username, passed],
        )
        .unwrap();
}

#[tokio::test]
async fn without_a_fresh_second_step_the_changes_and_the_download_are_refused_and_the_rest_is_not()
{
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let member = server.register("maya").await;
    let good = json!({"enabled": true, "intervalHours": 6, "retryMinutes": 10, "initialDelayMinutes": 0, "retentionDays": 30});
    add_second_step(&server, "owner", None);
    add_second_step(&server, "maya", None);

    for (method, path) in [
        ("PUT", "/api/cockpit/backups/settings"),
        ("DELETE", "/api/cockpit/backups/settings"),
        ("PUT", "/api/cockpit/integrations/cloudflare/r2"),
        ("DELETE", "/api/cockpit/integrations/cloudflare/r2"),
        ("GET", "/api/cockpit/backups/backup-missing.zip/download"),
    ] {
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let stale = owner
            .client
            .send(Request::new(method.clone(), path).json(&good))
            .await;
        assert_eq!(stale.status, 403, "{method} {path}");
        assert_eq!(
            stale.json::<Value>()["code"],
            "fresh-mfa-required",
            "{method} {path}"
        );
        // A member who is not the owner is told they are not, whatever else is wrong.
        assert_eq!(
            member
                .client
                .send(Request::new(method.clone(), path).json(&good))
                .await
                .status,
            403
        );
        assert!(
            member
                .client
                .send(Request::new(method, path).json(&good))
                .await
                .body
                .is_empty()
        );
    }
    // A body that cannot be read is refused before anybody is asked who they are.
    let broken =
        Request::new(Method::PUT, "/api/cockpit/backups/settings").body("application/json", "{");
    assert_eq!(owner.client.send(broken).await.status, 400);
    // Reading, making and verifying ask no fresh step.
    assert_eq!(owner.client.get("/api/cockpit/backups").await.status, 200);
    assert_eq!(
        owner
            .client
            .send(Request::new(Method::POST, "/api/cockpit/backups"))
            .await
            .status,
        201
    );
}

#[tokio::test]
async fn a_second_step_passed_a_minute_ago_lets_the_changes_through() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    add_second_step(&server, "owner", Some("-60 seconds"));
    let saved = owner
        .client
        .send(Request::new(Method::PUT, "/api/cockpit/backups/settings").json(&json!({"enabled": false, "intervalHours": 1, "retryMinutes": 1, "initialDelayMinutes": 0, "retentionDays": 1})))
        .await;
    assert_eq!(saved.status, 200, "{}", saved.text());
}

#[tokio::test]
async fn when_the_policy_asks_a_second_step_of_the_panel_the_owner_without_one_is_told_so() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let member = server.register("maya").await;
    let instance: String = server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\" LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    server
        .database()
        .execute(
            "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\")
             VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 14, 30, 1, 1, datetime('now'), 'cockpit')",
            [instance],
        )
        .unwrap();
    let refused = owner.client.get("/api/cockpit/backups").await;
    assert_eq!(refused.status, 403);
    assert_eq!(refused.json::<Value>()["code"], "cockpit-mfa-required");
    assert_eq!(member.client.get("/api/cockpit/backups").await.status, 403);
    assert!(
        member
            .client
            .get("/api/cockpit/backups")
            .await
            .body
            .is_empty()
    );
}

/// A writer that inserts one row after another, each in a transaction of its
/// own, until told to stop; the names of its rows are the order it wrote them.
fn start_writer(
    database: std::path::PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<u64> {
    std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(database).unwrap();
        connection.busy_timeout(Duration::from_secs(30)).unwrap();
        let mut written = 0u64;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            written += 1;
            connection
                .execute(
                    "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, 'w', '2026-11-24 00:00:00')",
                    [format!("w{written:09}")],
                )
                .unwrap();
        }
        written
    })
}

/// A full backup taken while a writer posts rows throughout restores to a
/// database that is whole and holds exactly the first rows the writer wrote:
/// a state the database really had, and not a mixture of two.
#[tokio::test]
async fn a_backup_taken_while_rows_are_being_written_is_a_state_that_existed() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = Server::start().await;
    let owner = server.found("owner").await;
    // Enough data that the copy takes a while.
    {
        let connection = server.database();
        connection.execute_batch("BEGIN").unwrap();
        for at in 0..20_000 {
            connection
                .execute(
                    "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, ?2, '2026-11-24 00:00:00')",
                    (format!("pad{at:06}"), "p".repeat(300)),
                )
                .unwrap();
        }
        connection.execute_batch("COMMIT").unwrap();
    }
    for round in 0..3 {
        // Each round's writer numbers its rows from one.
        server
            .database()
            .execute(
                "DELETE FROM \"InstanceSecrets\" WHERE \"Name\" LIKE 'w%'",
                [],
            )
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let writer = start_writer(server.database_path(), stop.clone());
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = std::time::Instant::now();
        let response = owner
            .client
            .post_json(
                "/api/cockpit/backups",
                &json!({"database": true, "objects": true, "keys": true, "instance": true}),
            )
            .await;
        assert_eq!(response.status, 202, "{}", response.text());
        let id = response.json::<Value>()["id"].as_str().unwrap().to_string();
        let list = settled(&owner.client).await;
        let took = started.elapsed();
        assert_eq!(list["lastFailure"], Value::Null, "{list}");
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.store(true, Ordering::Relaxed);
        let written = writer.join().unwrap();

        let download = owner
            .client
            .get(&format!("/api/cockpit/backups/{id}/download"))
            .await;
        assert_eq!(download.status, 200);
        let folder = crate::support::TempDir::new();
        let archive = folder.path().join("backup.zip");
        std::fs::write(&archive, &download.body).unwrap();
        let root = folder.path().join("restored");
        stellarr_api::backups::restore::apply(
            &archive,
            &root,
            stellarr_api::values::Timestamp::now(),
        )
        .unwrap();

        let restored = rusqlite::Connection::open(root.join("db/stellarr.db")).unwrap();
        let verdict: String = restored
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(verdict, "ok");
        let broken: i64 = restored
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0);
        let mut names = restored
            .prepare("SELECT \"Name\" FROM \"InstanceSecrets\" WHERE \"Name\" LIKE 'w%' ORDER BY \"Name\"")
            .unwrap();
        let names: Vec<String> = names
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        // The writer's rows from the start of the sequence, with none missing
        // in the middle and none past what the writer had written.
        for (at, name) in names.iter().enumerate() {
            assert_eq!(*name, format!("w{:09}", at as u64 + 1), "gap at {at}");
        }
        assert!(names.len() as u64 <= written);
        println!(
            "round {round}: the writer wrote {written} rows in the round; backup took {took:?}; restored database holds {} of the writer's rows",
            names.len()
        );
    }
}

#[tokio::test]
async fn a_partial_backup_left_by_a_stop_is_removed_at_the_next_start() {
    let folder = crate::support::TempDir::new();
    let backups = folder.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    for name in [
        ".backup-1.zip.partial",
        ".snapshot-2.partial",
        ".snapshot-2.partial-wal",
        "backup-3.zip",
        "backup-3.zip.verified",
    ] {
        std::fs::write(backups.join(name), b"x").unwrap();
    }
    let server = Server::start_in(folder.path(), &[]).await;
    let mut left: Vec<String> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["backup-3.zip", "backup-3.zip.verified"]);
    server.stop().await;
}

#[tokio::test]
async fn after_a_restore_the_old_cookie_is_refused_and_a_sign_in_with_the_password_works() {
    let folder = crate::support::TempDir::new();
    // The database sits where the container puts it, which is where a restore
    // puts it back.
    std::fs::create_dir_all(folder.path().join("db")).unwrap();
    let database = format!(
        "Data Source={};Pooling=True",
        folder.path().join("db/stellarr.db").display()
    );
    let settings = [("ConnectionStrings__Database", database.as_str())];
    let first = Server::start_in(folder.path(), &settings).await;
    let owner = first.found("owner").await;
    assert_eq!(owner.client.get("/api/cockpit/backups").await.status, 200);
    let started = owner
        .client
        .post_json(
            "/api/cockpit/backups",
            &json!({"database": true, "objects": true, "keys": true, "instance": true}),
        )
        .await;
    assert_eq!(started.status, 202, "{}", started.text());
    let list = settled(&owner.client).await;
    let id = list["backups"][0]["id"].as_str().unwrap().to_string();
    let archive = folder.path().join("backups").join(&id);
    assert!(first.stop().await);

    // The container's next start does this before any service runs.
    let applied = stellarr_api::backups::restore::apply(
        &archive,
        folder.path(),
        stellarr_api::values::Timestamp::now(),
    );
    assert!(applied.is_ok(), "{applied:?}");

    let second = Server::start_in(folder.path(), &settings).await;
    let old = second.client().with_cookies_of(&owner.client);
    assert_eq!(
        old.get("/api/cockpit/backups").await.status,
        401,
        "the cookie of a session that was running is refused"
    );
    let again = second.client();
    let signed_in = second.login(&again, "owner", crate::support::PASSWORD).await;
    assert_eq!(signed_in.status, 200, "{}", signed_in.text());
    assert_eq!(again.get("/api/cockpit/backups").await.status, 200);
    second.stop().await;
}
