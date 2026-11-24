//! The operator panel, `/api/cockpit`, through the real binary: who passes
//! the gate, what each route answers and what it leaves behind.
//!
//! What the answers are the first server's answers is proved by
//! `tests/cockpit-shared-database.sh`, which sends the same conversations to
//! both servers on copies of one database file; these say what they are.

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};

use crate::support::{HubClient, Person, Request, Response, Server};

const COOKIE: &str = "stellarr-dev-session";
const NIL: &str = "00000000-0000-0000-0000-000000000000";

fn cookie(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

/// A server with an owner, a plain member and a person who owns a server of
/// their own (and so passes for an administrator, but not for the operator).
struct Scene {
    server: Server,
    owner: Person,
    member: Person,
    admin: Person,
}

impl Scene {
    async fn new() -> Scene {
        Scene::with(&[]).await
    }

    async fn with(extra: &[(&str, &str)]) -> Scene {
        let server = Server::start_with(extra).await;
        let owner = server.found("owner").await;
        let member = server.register("joe").await;
        let admin = server.register("maya").await;
        let made = admin
            .client
            .post_json("/api/servers", &json!({"name": "Orbit", "description": "x", "textChannelName": "general", "voiceChannelName": "hall"}))
            .await;
        assert_eq!(made.status, 201, "{}", made.text());
        Scene { server, owner, member, admin }
    }

    fn rows(&self, sql: &str) -> Vec<Vec<String>> {
        let db = self.server.database();
        let mut statement = db.prepare(sql).expect("a statement");
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|c| {
                        Ok(match row.get::<_, rusqlite::types::Value>(c)? {
                            rusqlite::types::Value::Null => String::new(),
                            rusqlite::types::Value::Integer(number) => number.to_string(),
                            rusqlite::types::Value::Real(number) => number.to_string(),
                            rusqlite::types::Value::Text(text) => text,
                            rusqlite::types::Value::Blob(_) => "(bytes)".to_string(),
                        })
                    })
                    .collect()
            })
            .expect("rows")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    fn audit(&self) -> Vec<String> {
        self.rows("SELECT \"Summary\" FROM \"ServerAuditEntries\" ORDER BY \"CreatedAt\"")
            .into_iter()
            .flatten()
            .collect()
    }

    async fn instance_role(&self, name: &str) -> String {
        let role = self.rows(&format!("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"Name\" = '{name}' AND \"ServerId\" = (SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1 AND \"UserId\" = '{}' COLLATE NOCASE)", self.owner.id));
        role[0][0].to_lowercase()
    }

    async fn base_role(&self) -> String {
        let role = self.rows(&format!("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0 AND \"ServerId\" = (SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1 AND \"UserId\" = '{}' COLLATE NOCASE)", self.owner.id));
        role[0][0].to_lowercase()
    }
}

const READS: [&str; 12] = [
    "/api/cockpit/bootstrap",
    "/api/cockpit/activity",
    "/api/cockpit/users",
    "/api/cockpit/servers",
    "/api/cockpit/sessions",
    "/api/cockpit/runtime",
    "/api/cockpit/logs",
    "/api/cockpit/logs/export",
    "/api/cockpit/settings",
    "/api/cockpit/desktop-release",
    "/api/cockpit/secrets",
    "/api/cockpit/integrations/identity-providers",
];

#[tokio::test]
async fn only_the_owner_of_the_instance_passes_the_gate() {
    let scene = Scene::new().await;
    let anonymous = scene.server.client();
    for path in READS.iter().chain(["/api/cockpit/fresh-second-step"].iter()) {
        let owner = scene.owner.client.get(path).await;
        assert!(owner.status == 200 || owner.status == 204, "{path}: {}", owner.status);
        for (who, person) in [("member", &scene.member), ("admin of another server", &scene.admin)] {
            let refused = person.client.get(path).await;
            assert_eq!(refused.status, 403, "{who} {path}");
            assert!(refused.body.is_empty(), "{who} {path}");
        }
        assert_eq!(anonymous.get(path).await.status, 401, "{path}");
    }
}

#[tokio::test]
async fn an_id_in_a_path_is_read_before_the_caller_is_asked_who_they_are() {
    let scene = Scene::new().await;
    for path in ["/api/cockpit/users/x/security", "/api/cockpit/servers/x/members", "/api/cockpit/servers/x"] {
        let method = if path.ends_with("/x") { Method::DELETE } else { Method::GET };
        let asked = scene.member.client.send(Request::new(method, path)).await;
        assert_eq!(asked.status, 404, "{path}");
    }
}

#[tokio::test]
async fn a_body_is_bound_before_the_gate_and_an_empty_one_after_it() {
    let scene = Scene::new().await;
    async fn send(person: &Person, raw: &str, kind: &str) -> Response {
        person.client.send(Request::new(Method::POST, "/api/cockpit/users").body(kind, raw)).await
    }
    for person in [&scene.member, &scene.owner] {
        assert_eq!(send(person, "{", "application/json").await.status, 400);
        assert_eq!(send(person, r#"{"username":5}"#, "application/json").await.status, 400);
        assert_eq!(send(person, "[]", "application/json").await.status, 400);
        assert_eq!(send(person, "{}", "text/plain").await.status, 415);
    }
    assert_eq!(send(&scene.member, "", "application/json").await.status, 403);
    assert_eq!(send(&scene.member, "null", "application/json").await.status, 403);
    assert_eq!(send(&scene.owner, "", "application/json").await.status, 400);
    assert_eq!(send(&scene.owner, "null", "application/json").await.status, 400);
}

#[tokio::test]
async fn making_an_account_keeps_the_first_servers_rules_and_leaves_its_rows() {
    let scene = Scene::new().await;
    let role = scene.base_role().await;
    let create = |body: Value| scene.owner.client.send(Request::new(Method::POST, "/api/cockpit/users").json(&body));
    let good = json!({"username": "kim.lee", "displayName": " Kim ", "email": "Kim@Example.test", "password": "long-enough-1", "roleId": role});

    let made = create(good.clone()).await;
    assert_eq!(made.status, 201, "{}", made.text());
    let id = made.json::<Value>()["id"].as_str().unwrap().to_string();
    assert_eq!(made.header("location"), Some(format!("/api/cockpit/users/{id}").as_str()));
    let row = scene.rows("SELECT \"Username\", \"NormalizedUsername\", \"DisplayName\", \"Email\", \"NormalizedEmail\" FROM \"Users\" WHERE \"Username\" = 'kim.lee'");
    assert_eq!(row, [["kim.lee", "KIM.LEE", "Kim", "Kim@Example.test", "KIM@EXAMPLE.TEST"]]);
    assert!(scene.audit().iter().any(|line| line.contains("Kim") && line.starts_with("Owner")));

    // The same name or address in other capitals is taken.
    let mut again = good.clone();
    again["username"] = json!("KIM.LEE");
    assert_eq!(create(again).await.status, 409);
    let mut by_address = good.clone();
    by_address["username"] = json!("other");
    by_address["email"] = json!("kim@example.TEST");
    assert_eq!(create(by_address).await.status, 409);

    for (field, value) in [
        ("username", json!("a")),
        ("username", json!("a".repeat(33))),
        ("username", json!("a b")),
        ("username", json!("\u{10400}\u{10401}")),
        ("displayName", json!("   ")),
        ("displayName", json!("d".repeat(65))),
        ("password", json!("short")),
        ("password", json!("p".repeat(257))),
        ("email", json!("no address")),
    ] {
        let mut body = good.clone();
        body["username"] = json!("fresh");
        body["email"] = json!("fresh@example.test");
        body[field] = value.clone();
        let refused = create(body).await;
        assert_eq!(refused.status, 400, "{field} {value}");
        assert!(refused.json::<Value>()["errors"]["user"].is_array(), "{field} {value}");
    }
    // An address with a name beside it is an address, and an empty one is none.
    let named = json!({"username": "named", "displayName": "N", "email": "Ala <ala@example.test>", "password": "long-enough-1", "roleId": role});
    assert_eq!(create(named).await.status, 201);
    let none = json!({"username": "none", "displayName": "N", "email": "  ", "password": "long-enough-1", "roleId": role});
    assert_eq!(create(none).await.status, 201);

    // Never the owner's role, never one that is not the instance's.
    let owner_role = scene.instance_role("Owner").await;
    for role in [owner_role.as_str(), NIL] {
        let mut body = good.clone();
        body["username"] = json!("grab");
        body["email"] = json!("grab@example.test");
        body["roleId"] = json!(role);
        let refused = create(body).await;
        assert_eq!(refused.status, 400, "{role}");
        assert!(refused.json::<Value>()["errors"]["roleId"].is_array());
    }
}

#[tokio::test]
async fn disabling_a_person_ends_their_sessions_and_closes_their_connections() {
    let scene = Scene::with(&[("Testing__Workers", "realtime-sessions")]).await;
    let mut hub = HubClient::connect(scene.server.address(), &cookie(&scene.member)).await;
    let path = format!("/api/cockpit/users/{}/status", scene.member.id);

    let off = scene.owner.client.send(Request::new(Method::PATCH, &path).json(&json!({"disabled": true}))).await;

    assert_eq!(off.status, 204);
    let (_, closing) = hub.until_close().await;
    let _ = closing;
    assert_eq!(scene.member.client.get("/api/me").await.status, 401);
    let open = scene.rows(&format!("SELECT COUNT(*) FROM \"UserSessions\" WHERE \"UserId\" = '{}' COLLATE NOCASE AND \"RevokedAt\" IS NULL", scene.member.id));
    assert_eq!(open[0][0], "0");
    assert!(scene.audit().iter().any(|line| line.contains("Joe") || line.contains("joe")));
    // Doing it again changes nothing, and enabling lets the person in again.
    let again = scene.owner.client.send(Request::new(Method::PATCH, &path).json(&json!({"disabled": true}))).await;
    assert_eq!(again.status, 204);
    let on = scene.owner.client.send(Request::new(Method::PATCH, &path).json(&json!({"disabled": false}))).await;
    assert_eq!(on.status, 204);
    let login = scene.server.login(&scene.server.client(), "joe", crate::support::PASSWORD).await;
    assert_eq!(login.status, 200);
}

#[tokio::test]
async fn the_operator_cannot_disable_or_recover_themselves_or_change_their_own_role() {
    let scene = Scene::new().await;
    let me = &scene.owner.id;
    let status = scene.owner.client.send(Request::new(Method::PATCH, &format!("/api/cockpit/users/{me}/status")).json(&json!({"disabled": true}))).await;
    assert_eq!(status.status, 409);
    assert!(status.json::<Value>()["title"].is_string());
    let recover = scene.owner.client.send(Request::new(Method::POST, &format!("/api/cockpit/users/{me}/security/recover")).json(&json!({"reason": "lost"}))).await;
    assert_eq!(recover.status, 409);
    let role = scene.owner.client.send(Request::new(Method::PATCH, &format!("/api/cockpit/users/{me}/role")).json(&json!({"roleId": scene.base_role().await}))).await;
    assert_eq!(role.status, 409);
}

#[tokio::test]
async fn a_role_change_announces_what_the_person_now_holds_and_is_audited() {
    let scene = Scene::new().await;
    let made = scene
        .owner
        .client
        .post_json("/api/cockpit/roles", &json!({"name": "Helper", "color": "#112233", "permissions": ["manageMessages"]}))
        .await;
    assert_eq!(made.status, 201, "{}", made.text());
    let role = made.json::<Value>();
    assert_eq!(role["memberCount"], 0);
    let id = role["id"].as_str().unwrap().to_string();
    assert_eq!(made.header("location"), Some(format!("/api/cockpit/roles/{id}").as_str()));
    let again = scene.owner.client.post_json("/api/cockpit/roles", &json!({"name": "Helper", "color": "#000", "permissions": []})).await;
    assert_eq!(again.status, 409);
    let unknown = scene.owner.client.post_json("/api/cockpit/roles", &json!({"name": "X", "color": "#000", "permissions": ["flying"]})).await;
    assert_eq!(unknown.status, 400);
    let null = scene.owner.client.post_json("/api/cockpit/roles", &json!({"name": "X", "color": "#000", "permissions": [null]})).await;
    assert_eq!(null.status, 400);
    assert!(null.json::<Value>()["errors"]["role"].is_array());

    let set = scene
        .owner
        .client
        .send(Request::new(Method::PATCH, &format!("/api/cockpit/users/{}/role", scene.member.id)).json(&json!({"roleId": id})))
        .await;

    assert_eq!(set.status, 204);
    let events = scene.server.events();
    let told = events.iter().find(|event| event["name"] == "PermissionsChanged").expect("the person is told");
    assert!(told.to_string().contains("manageMessages"), "{told}");
    let users = scene.owner.client.get("/api/cockpit/users").await.json::<Value>();
    let joe = users["users"].as_array().unwrap().iter().find(|u| u["username"] == "joe").unwrap();
    assert_eq!(joe["roleName"], "Helper");
    let held = users["roles"].as_array().unwrap().iter().find(|r| r["name"] == "Helper").unwrap();
    assert_eq!(held["memberCount"], 1);
    let owner_role = scene.instance_role("Owner").await;
    let owner_edit = scene.owner.client.send(Request::new(Method::PATCH, &format!("/api/cockpit/roles/{owner_role}")).json(&json!({"name": "Boss"}))).await;
    assert_eq!(owner_edit.status, 409);
    let renamed = scene.owner.client.send(Request::new(Method::PATCH, &format!("/api/cockpit/roles/{id}")).json(&json!({"name": "Aide"}))).await;
    assert_eq!(renamed.status, 200);
    assert_eq!(renamed.json::<Value>()["permissions"], json!(["manageMessages"]));
}

#[tokio::test]
async fn a_server_is_handed_over_or_deleted_by_the_operator_and_the_instances_own_never() {
    let scene = Scene::new().await;
    let servers = scene.owner.client.get("/api/cockpit/servers").await.json::<Value>();
    let list = servers.as_array().unwrap();
    assert_eq!(list.len(), 2);
    let instance = list.iter().find(|s| s["isInstance"] == true).unwrap();
    let orbit = list.iter().find(|s| s["name"] == "Orbit").unwrap();
    assert_eq!(orbit["owner"]["username"], "maya");
    let orbit_id = orbit["id"].as_str().unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    for request in [
        Request::new(Method::POST, &format!("/api/cockpit/servers/{instance_id}/transfer")).json(&json!({"userId": scene.member.id})),
        Request::new(Method::DELETE, &format!("/api/cockpit/servers/{instance_id}")),
    ] {
        assert_eq!(scene.owner.client.send(request).await.status, 409);
    }
    let members = scene.owner.client.get(&format!("/api/cockpit/servers/{orbit_id}/members")).await;
    assert_eq!(members.json::<Value>().as_array().unwrap().len(), 1);
    let nobody = scene.owner.client.send(Request::new(Method::POST, &format!("/api/cockpit/servers/{orbit_id}/transfer")).json(&json!({"userId": NIL}))).await;
    assert_eq!(nobody.status, 400);
    assert!(nobody.json::<Value>()["errors"]["userId"].is_array());
    let to_self = scene.owner.client.send(Request::new(Method::POST, &format!("/api/cockpit/servers/{orbit_id}/transfer")).json(&json!({"userId": scene.admin.id}))).await;
    assert_eq!(to_self.status, 400);

    // Joe joins Orbit, then is made its owner.
    let db = scene.server.database();
    let (role, joined): (String, String) = (
        db.query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 COLLATE NOCASE AND \"IsSystem\" = 1 AND \"IsOwner\" = 0", [orbit_id], |r| r.get(0)).unwrap(),
        stellarr_api::values::Timestamp::now().to_db_text(),
    );
    db.execute(
        "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"OwnerSlot\", \"JoinedAt\") VALUES (?1, ?2, ?3, NULL, ?4)",
        params![orbit_id.to_uppercase(), scene.member.id.to_uppercase(), role, joined],
    )
    .unwrap();
    drop(db);
    let handed = scene.owner.client.send(Request::new(Method::POST, &format!("/api/cockpit/servers/{orbit_id}/transfer")).json(&json!({"userId": scene.member.id}))).await;
    assert_eq!(handed.status, 204, "{}", handed.text());
    let after = scene.owner.client.get(&format!("/api/cockpit/servers/{orbit_id}/members")).await.json::<Value>();
    let owners: Vec<&Value> = after.as_array().unwrap().iter().filter(|m| m["isOwner"] == true).collect();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0]["username"], "joe");
    assert_eq!(scene.audit().iter().filter(|line| line.contains("Orbit")).count(), 1);

    let gone = scene.owner.client.send(Request::new(Method::DELETE, &format!("/api/cockpit/servers/{orbit_id}"))).await;
    assert_eq!(gone.status, 204);
    assert_eq!(scene.owner.client.send(Request::new(Method::DELETE, &format!("/api/cockpit/servers/{orbit_id}"))).await.status, 404);
    assert_eq!(scene.owner.client.get(&format!("/api/cockpit/servers/{orbit_id}/members")).await.status, 404);
}

#[tokio::test]
async fn recovering_an_account_ends_what_it_had_and_hands_over_one_credential() {
    let scene = Scene::new().await;
    let path = format!("/api/cockpit/users/{}/security/recover", scene.member.id);
    for reason in ["", "ab", &"r".repeat(501)] {
        let refused = scene.owner.client.send(Request::new(Method::POST, &path).json(&json!({"reason": reason}))).await;
        assert_eq!(refused.status, 400, "{reason:?}");
        assert!(refused.json::<Value>()["errors"]["reason"].is_array());
    }
    let unknown = scene.owner.client.send(Request::new(Method::POST, &format!("/api/cockpit/users/{NIL}/security/recover")).json(&json!({"reason": "lost phone"}))).await;
    assert_eq!(unknown.status, 404);

    let done = scene.owner.client.send(Request::new(Method::POST, &path).json(&json!({"reason": "  lost phone  "}))).await;

    assert_eq!(done.status, 200, "{}", done.text());
    let answer = done.json::<Value>();
    assert!(answer["credential"].as_str().unwrap().len() >= 16);
    assert_eq!(answer["sessionsRevoked"], 1);
    assert_eq!(scene.member.client.get("/api/me").await.status, 401);
    let security = scene.owner.client.get(&format!("/api/cockpit/users/{}/security", scene.member.id)).await.json::<Value>();
    assert_eq!(security["recoveryRequired"], true);
    assert_eq!(security["hasPassword"], true);
    assert_eq!(security["events"][0]["kind"], "cli-recovery");
    let stored = scene.rows("SELECT \"Reason\" FROM \"CliRecoveryCredentials\"");
    assert_eq!(stored, [["lost phone"]]);
}

#[tokio::test]
async fn unlinking_the_only_way_in_is_refused_and_another_is_taken_away() {
    let scene = Scene::new().await;
    let db = scene.server.database();
    let id = stellarr_api::values::Id::random();
    db.execute(
        "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\") VALUES (?1, ?2, 'https://idp.example.test/realm', 's', NULL, ?3)",
        params![id.to_db_text(), scene.member.id.to_uppercase(), stellarr_api::values::Timestamp::now().to_db_text()],
    )
    .unwrap();
    drop(db);
    let security = scene.owner.client.get(&format!("/api/cockpit/users/{}/security", scene.member.id)).await.json::<Value>();
    assert_eq!(security["identities"][0]["name"], "idp.example.test");
    let path = format!("/api/cockpit/users/{}/identities/{id}/remove", scene.member.id);

    // Without a password or a passkey the identity is the last way in.
    scene.server.database().execute("UPDATE \"Users\" SET \"PasswordHash\" = NULL WHERE \"Id\" = ?1 COLLATE NOCASE", [&scene.member.id]).unwrap();
    let refused = scene.owner.client.send(Request::new(Method::POST, &path)).await;
    assert_eq!(refused.status, 409);
    assert_eq!(refused.json::<Value>()["code"], "last-way-in");
    scene.server.database().execute("UPDATE \"Users\" SET \"PasswordHash\" = 'x' WHERE \"Id\" = ?1 COLLATE NOCASE", [&scene.member.id]).unwrap();
    // With a password the identity is not the last way in.
    let removed = scene.owner.client.send(Request::new(Method::POST, &path)).await;
    assert_eq!(removed.status, 204, "{}", removed.text());
    assert_eq!(scene.rows("SELECT COUNT(*) FROM \"ExternalIdentities\"")[0][0], "0");
    assert_eq!(scene.rows("SELECT \"Kind\", \"Detail\" FROM \"SecurityEvents\" WHERE \"Kind\" = 'identity-unlinked'"), [["identity-unlinked", "operator: https://idp.example.test/realm"]]);
    assert_eq!(scene.owner.client.send(Request::new(Method::POST, &path)).await.status, 404);
}

#[tokio::test]
async fn settings_are_read_before_they_are_saved_and_saved_as_a_whole() {
    let scene = Scene::new().await;
    assert_eq!(scene.rows("SELECT COUNT(*) FROM \"OperationsSettings\"")[0][0], "0");
    let before = scene.owner.client.get("/api/cockpit/settings").await.json::<Value>();
    assert_eq!(scene.rows("SELECT COUNT(*) FROM \"OperationsSettings\"")[0][0], "0");
    assert_eq!(before["mfaRequirement"], "optional");
    assert_eq!(before["backupRetentionDays"], 14);
    assert_eq!(before["smtpPort"], 587);

    let mut body = json!({
        "instanceName": "Orbit HQ", "timeZone": "UTC", "mailingEnabled": false, "mailFromAddress": "",
        "inviteMailEnabled": true, "recoveryMailEnabled": true, "alertMailEnabled": false,
        "backupRetentionDays": 20, "logRetentionDays": 31, "orphanObjectRetentionDays": 2, "automaticCleanupEnabled": true,
        "publicUrl": "https://elsewhere.example", "nothing": 1
    });
    let saved = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&body)).await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    let answer = saved.json::<Value>();
    assert_eq!(answer["instanceName"], "Orbit HQ");
    assert_eq!(answer["backupRetentionDays"], 20);
    // What the environment fixes is not the request's to change.
    assert_eq!(answer["publicUrl"], before["publicUrl"]);
    assert!(answer["updatedAt"].as_str().unwrap().ends_with('Z'));
    let read = scene.owner.client.get("/api/cockpit/settings").await.json::<Value>();
    assert!(!read["updatedAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(read["logRetentionDays"], 31);
    assert_eq!(scene.rows("SELECT \"Name\" FROM \"Servers\" WHERE \"Name\" = 'Orbit HQ'").len(), 1);
    assert!(scene.audit().iter().any(|line| line.contains("settings")));

    for (field, value) in [("backupRetentionDays", json!(0)), ("backupRetentionDays", json!(3651)), ("logRetentionDays", json!(0)), ("orphanObjectRetentionDays", json!(3651)), ("instanceName", json!("")), ("instanceName", json!("n".repeat(81))), ("timeZone", json!("Nowhere/Land")), ("smtpPort", json!(0)), ("smtpPort", json!(65536)), ("smtpSecurity", json!("ssl")), ("mfaRequirement", json!("sometimes")), ("trustedDeviceDays", json!(366)), ("emailCodeCooldownSeconds", json!(601)), ("cliRecoveryMinutes", json!(61))] {
        let mut bad = body.clone();
        bad[field] = value.clone();
        let refused = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&bad)).await;
        assert_eq!(refused.status, 400, "{field} {value}");
        assert!(refused.json::<Value>()["errors"][field].is_array(), "{field} {value}");
    }
    for (field, value) in [("backupRetentionDays", json!(1)), ("backupRetentionDays", json!(3650)), ("smtpPort", json!(65535)), ("emailCodeCooldownSeconds", json!(0)), ("trustedDeviceDays", json!("365"))] {
        let mut edge = body.clone();
        edge[field] = value.clone();
        let accepted = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&edge)).await;
        assert_eq!(accepted.status, 200, "{field} {value}: {}", accepted.text());
    }
    body["mailingEnabled"] = json!(true);
    body["mailFromAddress"] = json!("not an address");
    let refused = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&body)).await;
    assert!(refused.json::<Value>()["errors"]["mailFromAddress"].is_array());
}

#[tokio::test]
async fn a_secret_goes_in_and_never_comes_out() {
    let scene = Scene::new().await;
    let listed = scene.owner.client.get("/api/cockpit/secrets").await.json::<Value>();
    let names: Vec<&str> = listed.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names[..3], ["mail.resend-api-key", "mail.smtp-password", "giphy.api-key"]);
    assert!(listed.as_array().unwrap().iter().all(|s| s["configured"] == false && s["source"] == "none"));
    let secret = "s3cr3t-value-that-must-not-leak";

    let set = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/secrets/giphy.api-key").json(&json!({"value": secret}))).await;

    assert_eq!(set.status, 200);
    let answer = set.json::<Value>();
    assert_eq!((answer["configured"].clone(), answer["source"].clone()), (json!(true), json!("panel")));
    assert_eq!(answer["variable"], "STELLARR_GIPHY_API_KEY");
    for path in ["/api/cockpit/secrets", "/api/cockpit/settings", "/api/cockpit/logs", "/api/cockpit/logs/export"] {
        let read = scene.owner.client.get(path).await;
        assert!(!read.text().contains(secret), "{path}");
    }
    assert!(scene.server.log().iter().all(|line| !line.contains(secret)));
    assert!(scene.audit().iter().any(|line| line.contains("giphy.api-key") && !line.contains(secret)));
    assert!(scene.rows("SELECT \"ProtectedValue\" FROM \"InstanceSecrets\"")[0][0] != secret);
    assert_eq!(scene.owner.client.get("/api/cockpit/settings").await.json::<Value>()["giphyKeyConfigured"], true);

    for bad in [json!({}), json!({"value": "   "}), json!({"value": "x".repeat(4097)})] {
        let refused = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/secrets/giphy.api-key").json(&bad)).await;
        assert_eq!(refused.status, 400);
        assert!(refused.json::<Value>()["errors"]["value"].is_array());
    }
    let longest = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/secrets/mail.smtp-password").json(&json!({"value": "x".repeat(4096)}))).await;
    assert_eq!(longest.status, 200);
    assert_eq!(scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/secrets/nope").json(&json!({"value": "x"}))).await.status, 404);

    let cleared = scene.owner.client.send(Request::new(Method::DELETE, "/api/cockpit/secrets/giphy.api-key")).await;
    assert_eq!(cleared.status, 200);
    assert_eq!(cleared.json::<Value>()["configured"], false);
    assert_eq!(scene.owner.client.send(Request::new(Method::DELETE, "/api/cockpit/secrets/nope")).await.status, 404);
}

#[tokio::test]
async fn the_log_view_filters_pages_and_exports_without_a_secret_in_it() {
    let scene = Scene::new().await;
    scene.server.login(&scene.server.client(), "joe", "wrong password").await;
    scene.owner.client.get("/api/cockpit/users?token=abc123tokenvalue").await;
    scene.owner.client.get("/api/%3Cb%3Ehello%3C/b%3E").await;

    let page = scene.owner.client.get("/api/cockpit/logs?limit=2&level=info").await.json::<Value>();
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);
    assert!(page["nextBefore"].is_u64());
    assert_eq!(page["capacity"], 10_000);
    let next = page["nextBefore"].as_u64().unwrap();
    let older = scene.owner.client.get(&format!("/api/cockpit/logs?limit=500&before={next}")).await.json::<Value>();
    assert!(older["entries"].as_array().unwrap().iter().all(|e| e["sequence"].as_u64().unwrap() < next));
    for (query, expected) in [("limit=0", 1), ("limit=-5", 1)] {
        let clamped = scene.owner.client.get(&format!("/api/cockpit/logs?{query}")).await.json::<Value>();
        assert_eq!(clamped["entries"].as_array().unwrap().len(), expected, "{query}");
    }
    assert_eq!(scene.owner.client.get("/api/cockpit/logs?limit=abc").await.status, 400);
    let found = scene.owner.client.get("/api/cockpit/logs?search=hello").await.json::<Value>();
    assert!(!found["entries"].as_array().unwrap().is_empty());

    let export = scene.owner.client.get("/api/cockpit/logs/export").await;
    assert_eq!(export.status, 200);
    assert_eq!(export.header("content-type"), Some("text/tab-separated-values; charset=utf-8"));
    let disposition = export.header("content-disposition").unwrap();
    assert!(disposition.starts_with("attachment; filename=stellarr-logs-") && disposition.contains(".tsv; filename*=UTF-8''"), "{disposition}");
    let text = export.text();
    assert!(text.starts_with("at\tlevel\tsource\ttrace\tmessage\n"));
    assert!(text.lines().skip(1).all(|line| line.split('\t').count() >= 5));
    assert!(!text.contains("abc123tokenvalue") && !text.contains("wrong password"));
    let quiet = scene.owner.client.get("/api/cockpit/logs?level=error&search=zzzzznothing").await.json::<Value>();
    assert!(quiet["entries"].as_array().unwrap().is_empty() && quiet["nextBefore"].is_null());
}

#[tokio::test]
async fn the_runtime_page_reports_the_services_and_the_samples() {
    let scene = Scene::with(&[("Testing__Workers", "cockpit-runtime")]).await;
    let runtime = scene.owner.client.get("/api/cockpit/runtime").await.json::<Value>();
    let ids: Vec<&str> = runtime["services"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["api", "database", "realtime", "maintenance", "storage"]);
    assert_eq!(runtime["sampleSeconds"], 30);
    assert!(!runtime["history"].as_array().unwrap().is_empty());
    assert_eq!(runtime["voiceRooms"], 0);
    let overview = scene.owner.client.get("/api/cockpit/bootstrap").await.json::<Value>();
    assert_eq!(overview["overview"]["userCount"], 3);
    assert_eq!(overview["overview"]["serverCount"], 2);
    let activity = scene.owner.client.get("/api/cockpit/activity").await.json::<Value>();
    assert_eq!(activity["days"].as_array().unwrap().len(), 30);
}

#[tokio::test]
async fn sessions_are_listed_and_revoked_but_never_the_callers_own() {
    let scene = Scene::new().await;
    let listed = scene.owner.client.get("/api/cockpit/sessions").await.json::<Value>();
    let all = listed.as_array().unwrap();
    assert_eq!(all.len(), 3);
    let own = all.iter().find(|s| s["current"] == true).unwrap();
    assert_eq!(own["userDisplayName"], "Owner");
    assert_eq!(scene.owner.client.send(Request::new(Method::DELETE, &format!("/api/cockpit/sessions/{}", own["id"].as_str().unwrap()))).await.status, 409);
    let joe = all.iter().find(|s| s["userDisplayName"] != "Owner" && s["current"] == false).unwrap();
    let id = joe["id"].as_str().unwrap();
    assert_eq!(scene.owner.client.send(Request::new(Method::DELETE, &format!("/api/cockpit/sessions/{id}"))).await.status, 204);
    assert_eq!(scene.owner.client.send(Request::new(Method::DELETE, &format!("/api/cockpit/sessions/{id}"))).await.status, 404);
    assert_eq!(scene.owner.client.get("/api/cockpit/sessions").await.json::<Value>().as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn the_desktop_versions_are_checked_and_stored() {
    let scene = Scene::new().await;
    let put = |body: Value| scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/desktop-release").json(&body));
    let saved = put(json!({"minimumVersion": " 0.2.0 ", "recommendedVersion": "0.10.0"})).await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    let answer = saved.json::<Value>();
    assert_eq!((answer["minimumVersion"].clone(), answer["recommendedVersion"].clone()), (json!("0.2.0"), json!("0.10.0")));
    assert!(answer["published"].is_null() && answer["androidPublished"].is_null());
    assert_eq!(scene.owner.client.get("/api/cockpit/desktop-release").await.json::<Value>()["minimumVersion"], "0.2.0");
    for body in [json!({"minimumVersion": "1.0", "recommendedVersion": "1.0.0"}), json!({"minimumVersion": "1.0.0", "recommendedVersion": "1.0.0-beta"}), json!({}), json!({"minimumVersion": "2.0.0", "recommendedVersion": "1.9.9"})] {
        let refused = put(body.clone()).await;
        assert_eq!(refused.status, 400, "{body}");
        assert!(refused.json::<Value>()["errors"].is_object());
    }
}

#[tokio::test]
async fn the_test_mail_and_the_gif_check_say_what_is_missing() {
    let scene = Scene::new().await;
    let mail = |to: &str| scene.owner.client.send(Request::new(Method::POST, "/api/cockpit/mail/test").json(&json!({"to": to})));
    for bad in ["", "no address", "Ann <ann@example.test>", "ann@example.test, bo@example.test"] {
        let refused = mail(bad).await;
        assert_eq!(refused.status, 400, "{bad:?}");
        assert!(refused.json::<Value>()["errors"]["to"].is_array());
    }
    let not_ready = mail("ann@example.test").await;
    assert_eq!(not_ready.status, 409);
    assert!(not_ready.json::<Value>()["detail"].is_string());
    let giphy = scene.owner.client.send(Request::new(Method::POST, "/api/cockpit/integrations/giphy/test")).await;
    assert_eq!(giphy.status, 409);
    assert!(giphy.json::<Value>()["title"].as_str().unwrap().contains("STELLARR_GIPHY_API_KEY"));
    let providers = scene.owner.client.get("/api/cockpit/integrations/identity-providers").await;
    assert_eq!(providers.text(), "[]");
}

/// Gives the owner a second step that no session of theirs has passed.
fn stale(scene: &Scene) {
    scene
        .server
        .database()
        .execute(
            "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\") VALUES (?1, ?2, 'totp', 'Phone', ?3, ?3)",
            params![stellarr_api::values::Id::random().to_db_text(), scene.owner.id.to_uppercase(), stellarr_api::values::Timestamp::now().to_db_text()],
        )
        .unwrap();
    scene.server.database().execute("UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = NULL", []).unwrap();
}

#[tokio::test]
async fn every_guarded_route_asks_a_stale_session_for_the_second_step_after_its_body() {
    let scene = Scene::new().await;
    stale(&scene);
    let me = &scene.owner.id;
    let guarded = [
        (Method::POST, "/api/cockpit/users".to_string(), Some(json!({}))),
        (Method::PATCH, format!("/api/cockpit/users/{me}/role"), Some(json!({}))),
        (Method::PATCH, format!("/api/cockpit/users/{me}/status"), Some(json!({}))),
        (Method::POST, format!("/api/cockpit/users/{me}/security/recover"), Some(json!({}))),
        (Method::POST, format!("/api/cockpit/users/{me}/identities/{NIL}/remove"), None),
        (Method::POST, "/api/cockpit/roles".to_string(), Some(json!({}))),
        (Method::PATCH, format!("/api/cockpit/roles/{NIL}"), Some(json!({}))),
        (Method::POST, format!("/api/cockpit/servers/{NIL}/transfer"), Some(json!({}))),
        (Method::DELETE, format!("/api/cockpit/servers/{NIL}"), None),
        (Method::PUT, "/api/cockpit/desktop-release".to_string(), Some(json!({}))),
        (Method::PUT, "/api/cockpit/integrations/identity-providers/x".to_string(), Some(json!({}))),
        (Method::PUT, "/api/cockpit/secrets/x".to_string(), Some(json!({}))),
        (Method::DELETE, "/api/cockpit/secrets/x".to_string(), None),
        (Method::GET, "/api/cockpit/fresh-second-step".to_string(), None),
    ];
    for (method, path, body) in guarded {
        let mut request = Request::new(method.clone(), &path);
        if let Some(body) = &body {
            request = request.json(body);
        }
        let asked = scene.owner.client.send(request).await;
        assert_eq!(asked.status, 403, "{method} {path}");
        assert_eq!(asked.json::<Value>()["code"], "fresh-mfa-required", "{method} {path}");
        // A body that cannot be read is told first, and a person who is not the owner is not told of the step.
        if body.is_some() {
            let broken = scene.owner.client.send(Request::new(method.clone(), &path).body("application/json", "{")).await;
            assert_eq!(broken.status, 400, "{method} {path}");
            let other = scene.member.client.send(Request::new(method, &path).json(&json!({}))).await;
            assert_eq!(other.status, 403);
            assert!(other.body.is_empty(), "{path}");
        }
    }
    // The reads and the settings that touch nothing of the account's security need no step.
    assert_eq!(scene.owner.client.get("/api/cockpit/users").await.status, 200);
}

#[tokio::test]
async fn the_settings_ask_for_the_second_step_only_for_security_and_the_mail_transport() {
    let scene = Scene::new().await;
    stale(&scene);
    let base = json!({
        "instanceName": "Orbit", "timeZone": "UTC", "mailingEnabled": false, "mailFromAddress": "",
        "inviteMailEnabled": true, "recoveryMailEnabled": true, "alertMailEnabled": false,
        "backupRetentionDays": 14, "logRetentionDays": 30, "orphanObjectRetentionDays": 1, "automaticCleanupEnabled": true
    });
    let put = |extra: Value| {
        let mut body = base.clone();
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&body))
    };
    assert_eq!(put(json!({})).await.status, 200);
    // Saying what is already so about the transport asks for nothing; changing it does.
    assert_eq!(put(json!({"smtpHost": "", "smtpPort": 587, "smtpSecurity": "STARTTLS", "smtpUsername": " "})).await.status, 200);
    for extra in [json!({"smtpHost": "mail.example.test"}), json!({"smtpPort": 25}), json!({"smtpSecurity": "tls"}), json!({"smtpUsername": "u"}), json!({"resendEnabled": false}), json!({"mfaRequirement": "optional"}), json!({"trustedDeviceDays": 14}), json!({"cliRecoveryMinutes": 10}), json!({"mailFromAddress": "ops@example.test"}), json!({"mailingEnabled": true, "mailFromAddress": "ops@example.test"}), json!({"inviteMailEnabled": false}), json!({"recoveryMailEnabled": false}), json!({"alertMailEnabled": true})] {
        let asked = put(extra.clone()).await;
        assert_eq!(asked.status, 403, "{extra}");
        assert_eq!(asked.json::<Value>()["code"], "fresh-mfa-required", "{extra}");
    }
    // Nothing of what was refused was saved.
    assert_eq!(scene.rows("SELECT \"MailFromAddress\", \"MailingEnabled\", \"InviteMailEnabled\", \"AlertMailEnabled\" FROM \"OperationsSettings\"")[0], ["", "0", "1", "0"]);
    // A request that is wrong is told so before it is asked for the step.
    let wrong = put(json!({"trustedDeviceDays": 0})).await;
    assert_eq!(wrong.status, 400);
    assert_eq!(scene.rows("SELECT \"MailSmtpHost\" FROM \"OperationsSettings\"")[0][0], "");
}

#[tokio::test]
async fn nobody_is_given_the_owners_role_by_a_role_change() {
    let scene = Scene::new().await;
    let owner_role = scene.instance_role("Owner").await;
    let before = scene.rows(&format!("SELECT \"RoleId\" FROM \"ServerMembers\" WHERE \"UserId\" = '{}' COLLATE NOCASE", scene.member.id));

    let refused = scene
        .owner
        .client
        .send(Request::new(Method::PATCH, &format!("/api/cockpit/users/{}/role", scene.member.id)).json(&json!({"roleId": owner_role})))
        .await;

    assert_eq!(refused.status, 404);
    let after = scene.rows(&format!("SELECT \"RoleId\" FROM \"ServerMembers\" WHERE \"UserId\" = '{}' COLLATE NOCASE", scene.member.id));
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_fresh_session_may_change_the_mail_settings_and_they_are_saved() {
    let scene = Scene::new().await;
    let body = json!({
        "instanceName": "Orbit", "timeZone": "UTC", "mailingEnabled": true, "mailFromAddress": " ops@example.test ",
        "inviteMailEnabled": false, "recoveryMailEnabled": true, "alertMailEnabled": true,
        "backupRetentionDays": 14, "logRetentionDays": 30, "orphanObjectRetentionDays": 1, "automaticCleanupEnabled": true
    });
    let saved = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&body)).await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    assert_eq!(scene.rows("SELECT \"MailFromAddress\", \"MailingEnabled\", \"InviteMailEnabled\", \"AlertMailEnabled\" FROM \"OperationsSettings\"")[0], ["ops@example.test", "1", "0", "1"]);
    // With a second step that is stale, saying the same again is not a change.
    stale(&scene);
    let again = scene.owner.client.send(Request::new(Method::PUT, "/api/cockpit/settings").json(&body)).await;
    assert_eq!(again.status, 200, "{}", again.text());
}
