//! Kicking, banning and lifting a ban, through the real binary, on the same
//! scene as the roles. The pairs here are the nearest who may and may not for
//! the kick and the ban.

use std::time::Duration;

use http::Method;
use serde_json::{Value, json};

use crate::roles::Scene;
use crate::support::{HubClient, Person};

const COOKIE: &str = "stellarr-dev-session";

fn cookie_of(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

fn member_row(scene: &Scene, who: &str) -> bool {
    let db = scene.server.database();
    db.query_row(
        "SELECT COUNT(*) FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
        rusqlite::params![scene.id, Id404::parse(&scene.user(who)).unwrap()],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[tokio::test]
async fn kicking_and_banning_need_a_higher_rank_and_the_power_for_each() {
    let scene = Scene::new().await;
    let kick = |who: &'static str, target: &str| {
        let path = format!("/api/admin/members/{}", scene.user(target));
        let scene = &scene;
        async move { scene.status(who, Method::DELETE, &path, None).await }
    };
    let ban = |who: &'static str, target: &str| {
        let path = format!("/api/admin/members/{}/ban", scene.user(target));
        let scene = &scene;
        async move { scene.status(who, Method::POST, &path, Some(json!({ "reason": "because" }))).await }
    };
    // The nearest who may not: the same rank, a higher one, the owner, oneself, nobody who is a member.
    for target in ["peer", "adm", "boss", "sen"] {
        assert_eq!(kick("sen", target).await, 403, "kick {target}");
        assert_eq!(ban("sen", target).await, 403, "ban {target}");
    }
    assert_eq!(kick("jun", "plain").await, 403, "no power");
    assert_eq!(ban("jun", "plain").await, 403, "no power");
    let stranger = Id404::random();
    assert_eq!(scene.status("sen", Method::DELETE, &format!("/api/admin/members/{stranger}"), None).await, 404);
    assert_eq!(scene.status("sen", Method::POST, &format!("/api/admin/members/{stranger}/ban"), Some(json!({}))).await, 404);
    // Who may.
    assert_eq!(kick("sen", "jun").await, 204);
    assert!(!member_row(&scene, "jun"));
    assert_eq!(ban("adm", "peer").await, 201);
    assert!(!member_row(&scene, "peer"));
    assert_eq!(kick("sen", "jun").await, 404, "already gone");
    let actions: Vec<String> = scene.audit().into_iter().map(|(action, _)| action).collect();
    assert!(actions.contains(&"member.kick".to_string()) && actions.contains(&"member.ban".to_string()), "{actions:?}");
}

type Id404 = stellarr_api::values::Id;

#[tokio::test]
async fn a_member_is_banned_once_and_the_reason_is_trimmed_defaulted_and_limited() {
    let scene = Scene::new().await;
    let ban = |target: &str, body: Option<Value>| {
        let path = format!("/api/admin/members/{}/ban", scene.user(target));
        let scene = &scene;
        async move { scene.send("boss", Method::POST, &path, body).await }
    };
    let long = ban("plain", Some(json!({ "reason": "x".repeat(501) }))).await;
    assert_eq!(long.status, 400);
    assert_eq!(long.json::<Value>()["errors"]["reason"], json!(["A ban reason can be at most 500 characters long."]));
    assert!(member_row(&scene, "plain"));
    let first = ban("plain", Some(json!({ "reason": "  spam  " }))).await;
    assert_eq!(first.status, 201, "{}", first.text());
    assert_eq!(first.header("location").unwrap(), format!("/api/admin/bans/{}", scene.user("plain")));
    let dto: Value = first.json();
    assert_eq!((dto["displayName"].as_str(), dto["bannedByName"].as_str(), dto["reason"].as_str()), (Some("Plain"), Some("Boss"), Some("spam")));
    assert!(dto["bannedAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(ban("plain", Some(json!({}))).await.status, 404, "somebody who is not a member cannot be banned again");
    let defaulted = ban("big", Some(json!({ "reason": null }))).await;
    assert_eq!(defaulted.status, 201);
    assert_eq!(defaulted.json::<Value>()["reason"], "No reason given");
    assert_eq!(ban("jun", Some(json!({ "reason": "x".repeat(500) }))).await.status, 201);
    assert_eq!(ban("peer", None).await.status, 400, "a request with no body");
    // The list, newest first, for those who manage bans.
    let list: Vec<Value> = scene.send("boss", Method::GET, "/api/admin/bans", None).await.json();
    let who: Vec<&str> = list.iter().map(|ban| ban["displayName"].as_str().unwrap()).collect();
    assert_eq!(who, ["Jun", "Big", "Plain"]);
    assert_eq!(scene.status("jun", Method::GET, "/api/admin/bans", None).await, 403);
    // Lifting a ban does not make them a member again.
    let lift = |target: &str| {
        let path = format!("/api/admin/bans/{}", scene.user(target));
        let scene = &scene;
        async move { scene.status("sen", Method::DELETE, &path, None).await }
    };
    assert_eq!(lift("plain").await, 204);
    assert_eq!(lift("plain").await, 404);
    assert!(!member_row(&scene, "plain"));
    assert_eq!(scene.status("jun", Method::DELETE, &format!("/api/admin/bans/{}", scene.user("big")), None).await, 403);
    assert!(scene.audit().iter().any(|(action, summary)| action == "member.unban" && summary == "Sen lifted the ban on Plain"), "{:?}", scene.audit());
}

#[tokio::test]
async fn a_banned_member_loses_their_roles_and_overrides_with_the_membership() {
    let scene = Scene::new().await;
    let db = scene.server.database();
    let user = Id404::parse(&scene.user("jun")).unwrap();
    db.execute(
        "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 1, 0)",
        rusqlite::params![stellarr_api::values::Id::random(), scene.id, scene.channel, user],
    )
    .unwrap();
    assert_eq!(scene.status("boss", Method::POST, &format!("/api/admin/members/{user}/ban"), Some(json!({}))).await, 201);
    let count = |table: &str| -> i64 {
        db.query_row(&format!("SELECT COUNT(*) FROM \"{table}\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2"), rusqlite::params![scene.id, user], |row| row.get(0)).unwrap()
    };
    assert_eq!((count("ServerMemberRoles"), count("MemberPermissionOverrides"), count("ServerBans")), (0, 0, 1));
}

#[tokio::test]
async fn a_removed_member_hears_of_it_and_nothing_more_from_the_server() {
    let scene = Scene::new().await;
    let cookie = cookie_of(scene.person("jun"));
    let server_id = scene.id.to_string();
    let mut removed = HubClient::connect_to(scene.server.address(), &format!("?serverId={server_id}"), &cookie).await;
    let mut other = HubClient::connect_to(scene.server.address(), &format!("?serverId={server_id}"), &cookie_of(scene.person("plain"))).await;
    // What the connections were told by connecting is not what this is about.
    removed.quiet_for(Duration::from_millis(400)).await;
    other.quiet_for(Duration::from_millis(400)).await;

    let user = scene.user("jun");
    assert_eq!(scene.status("boss", Method::POST, &format!("/api/admin/members/{user}/ban"), Some(json!({}))).await, 201);
    let told = removed.quiet_for(Duration::from_millis(500)).await;
    let names: Vec<&str> = told.iter().filter_map(|record| record["name"].as_str()).collect();
    assert!(names.contains(&"ServerMemberRemoved") && names.contains(&"ServerAccessRevoked"), "{told:?}");
    let revoked = told.iter().find(|record| record["name"] == "ServerAccessRevoked").unwrap();
    assert_eq!(revoked["data"], json!({ "serverId": server_id, "kind": "banned" }));
    let others = other.quiet_for(Duration::from_millis(100)).await;
    assert!(others.iter().any(|record| record["name"] == "ServerMemberRemoved"), "{others:?}");

    // A later event of the server does not reach the connection that was cut off.
    assert_eq!(scene.status("boss", Method::POST, "/api/admin/roles", Some(json!({ "name": "Later", "color": "#fff", "permissions": [] }))).await, 201);
    let base = scene.base();
    assert_eq!(
        scene.status("boss", Method::PUT, &format!("/api/admin/channels/{}/permissions/{base}", scene.channel), Some(json!({ "allow": [], "deny": ["viewChannel"] }))).await,
        200
    );
    assert!(removed.quiet_for(Duration::from_millis(500)).await.iter().all(|record| record["name"] != "ChannelAccessChanged"));
    assert!(other.quiet_for(Duration::from_millis(100)).await.iter().any(|record| record["name"] == "ChannelAccessChanged"));
}

#[tokio::test]
async fn the_old_session_of_a_banned_member_is_not_let_into_the_server_again() {
    let scene = Scene::new().await;
    let user = scene.user("jun");
    assert_eq!(scene.status("jun", Method::GET, "/api/admin/roles", None).await, 200);
    assert_eq!(scene.status("boss", Method::POST, &format!("/api/admin/members/{user}/ban"), Some(json!({}))).await, 201);
    // The session is still theirs, but the server is not: with no server named, or with its name.
    assert_eq!(scene.status("jun", Method::GET, "/api/admin/roles", None).await, 403);
    let named = crate::support::Request::new(Method::GET, "/api/admin/roles").header("X-Stellarr-Server", &scene.id.to_string());
    assert_eq!(scene.person("jun").client.send(named).await.status, 403);
    assert_eq!(scene.status("jun", Method::POST, &format!("/api/admin/members/{}/ban", scene.user("plain")), Some(json!({}))).await, 403);
}

#[tokio::test]
async fn a_body_that_cannot_be_read_is_answered_before_a_person_who_is_no_member_is_refused() {
    let scene = Scene::new().await;
    assert_eq!(scene.status("boss", Method::POST, &format!("/api/admin/members/{}/ban", scene.user("jun")), Some(json!({}))).await, 201);
    let broken = crate::support::Request::new(Method::POST, "/api/admin/roles").body("application/json", "{");
    assert_eq!(scene.person("jun").client.send(broken).await.status, 400);
    let named = crate::support::Request::new(Method::POST, "/api/admin/roles")
        .header("X-Stellarr-Server", &scene.id.to_string())
        .json(&json!({ "name": "x", "color": "#fff", "permissions": [] }));
    assert_eq!(scene.person("jun").client.send(named).await.status, 403);
    // Without a body to read, the first refusal is the person's.
    assert_eq!(scene.status("jun", Method::GET, "/api/admin/roles", None).await, 403);
}
