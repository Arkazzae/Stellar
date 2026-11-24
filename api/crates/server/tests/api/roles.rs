//! Roles, who holds them and the overrides of a role or a member in a channel
//! or section, through the real binary.
//!
//! Every row of the delegation table is a pair: somebody who may, and the
//! nearest who may not. The tests say what the answers are and what the
//! routes leave behind.

use std::collections::HashMap;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::permissions::ownership;
use stellarr_api::permissions::privileges::{self, ADMINISTRATOR};
use stellarr_api::permissions::roles::{self, Role, ServerRoles};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Response, Server};

const ORDINARY: i64 = privileges::SEND_MESSAGES | privileges::VIEW_CHANNEL;
const GATE: i64 = privileges::MANAGE_ROLES
    | privileges::MANAGE_MEMBERS
    | privileges::MODERATE_MEMBERS
    | privileges::MANAGE_BANS
    | privileges::VIEW_CHANNEL;

/// A server with people at every rank: `boss` owns it; `adm` holds Admin,
/// `sen` and `peer` Senior, `jun` Junior, `big` Powerful, `plain` only the
/// base role. Roles by position: Owner 0, Admin 1, Senior and Powerful 2,
/// Junior 3, Spare 4, the base role last.
pub struct Scene {
    pub server: Server,
    pub id: Id,
    pub boss: Person,
    people: HashMap<&'static str, Person>,
    roles: HashMap<&'static str, Id>,
    pub channel: Id,
    pub section: Id,
}

impl Scene {
    pub async fn new() -> Scene {
        let server = Server::start().await;
        let boss = server.found("boss").await;
        let mut people = HashMap::new();
        for name in ["adm", "sen", "peer", "jun", "big", "plain"] {
            people.insert(name, server.register(name).await);
        }
        let db = server.database();
        let id: Id = db
            .query_row("SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"OwnerSlot\" = 1", [], |row| row.get(0))
            .expect("the instance server");
        let mut roles: HashMap<&'static str, Id> = HashMap::new();
        let mut make = |name: &'static str, position: i64, permissions: i64| {
            let role = Id::random();
            db.execute(
                "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, ?5, 0, 0)",
                params![role, id, name, permissions, position],
            )
            .expect("a role");
            roles.insert(name, role);
        };
        make("Admin", 1, GATE | privileges::MANAGE_CHANNELS | ORDINARY);
        make("Senior", 2, GATE | ORDINARY);
        make("Powerful", 2, GATE | ORDINARY | privileges::MANAGE_SERVER);
        make("Junior", 3, ORDINARY);
        make("Spare", 4, privileges::SEND_MESSAGES);
        db.execute("UPDATE \"ServerRoles\" SET \"Position\" = 9 WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0", [id])
            .expect("the base role ranks last");
        let all = ServerRoles::load(&db, id).expect("roles");
        let base = all.base.expect("a base role");
        let by_id = all.by_id();
        for (name, role) in [("adm", "Admin"), ("sen", "Senior"), ("peer", "Senior"), ("jun", "Junior"), ("big", "Powerful")] {
            let user = Id::parse(&people[name].id).expect("an id");
            let held: Vec<Role> = vec![by_id[&roles[role]]];
            ownership::set_roles(&db, id, user, &held, &base).expect("held roles");
        }
        let section = Id::random();
        let channel = Id::random();
        db.execute("INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, 'Section', 0)", params![section, id]).expect("a section");
        db.execute(
            "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'room', 0, 0, '', 0, 0, 0, NULL)",
            params![channel, id],
        )
        .expect("a channel");
        drop(db);
        Scene { server, id, boss, people, roles, channel, section }
    }

    pub fn person(&self, name: &str) -> &Person {
        if name == "boss" { &self.boss } else { &self.people[name] }
    }

    pub fn user(&self, name: &str) -> String {
        self.person(name).id.clone()
    }

    pub fn role(&self, name: &str) -> String {
        if name == "Member" {
            return self.base().to_string();
        }
        self.roles[name].to_string()
    }

    pub fn base(&self) -> Id {
        let db = self.server.database();
        ServerRoles::load(&db, self.id).expect("roles").base.expect("a base role").id
    }

    pub async fn send(&self, who: &str, method: Method, path: &str, body: Option<Value>) -> Response {
        let mut request = Request::new(method, path);
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.person(who).client.send(request).await
    }

    pub async fn status(&self, who: &str, method: Method, path: &str, body: Option<Value>) -> u16 {
        self.send(who, method, path, body).await.status.as_u16()
    }

    /// The roles a member holds besides the base role, by name, highest first, and the role that ranks them.
    pub fn held(&self, who: &str) -> (String, Vec<String>) {
        let db = self.server.database();
        let user = Id::parse(&self.user(who)).expect("an id");
        let member = roles::Member::load(&db, self.id, user).expect("a read").expect("a member");
        let (held, _) = roles::held_of_member(&db, &member).expect("held");
        let name = |id: Id| -> String { db.query_row("SELECT \"Name\" FROM \"ServerRoles\" WHERE \"Id\" = ?1", [id], |row| row.get(0)).expect("a name") };
        (name(member.role.id), held.iter().map(|role| name(role.id)).collect())
    }

    /// Role names with positions, as the database holds them.
    pub fn positions(&self) -> Vec<(String, i64)> {
        let db = self.server.database();
        let mut statement = db.prepare("SELECT \"Name\", \"Position\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 ORDER BY \"Position\", \"Name\"").unwrap();
        statement.query_map([self.id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().map(Result::unwrap).collect()
    }

    pub fn audit(&self) -> Vec<(String, String)> {
        let db = self.server.database();
        let mut statement = db.prepare("SELECT \"Action\", \"Summary\" FROM \"ServerAuditEntries\" WHERE \"ServerId\" = ?1 ORDER BY \"CreatedAt\"").unwrap();
        statement.query_map([self.id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().map(Result::unwrap).collect()
    }
}

fn names(mask: i64) -> Vec<&'static str> {
    privileges::to_names(mask)
}

#[tokio::test]
async fn the_roles_are_listed_by_position_with_their_holders() {
    let scene = Scene::new().await;
    let response = scene.send("plain", Method::GET, "/api/admin/roles", None).await;
    assert_eq!(response.status, 200);
    let roles: Vec<Value> = response.json();
    let ids: Vec<&str> = roles.iter().map(|role| role["id"].as_str().unwrap()).collect();
    assert_eq!(ids[0], scene.user_role_of_owner());
    assert_eq!(ids[1], scene.role("Admin"));
    assert_eq!(*ids.last().unwrap(), scene.role("Member"), "the base role ranks last");
    let count = |name: &str| roles.iter().find(|role| role["id"] == scene.role(name).as_str()).unwrap()["memberCount"].clone();
    assert_eq!(count("Senior"), 2);
    assert_eq!(count("Junior"), 1);
    assert_eq!(count("Spare"), 0);
    assert_eq!(count("Member"), 7, "the base role counts everybody");
    let first = response.text();
    assert!(first.starts_with("[{\"id\":\""), "{first}");
    assert!(first.contains("\"memberCount\":1,\"isSystem\":true,\"isOwner\":true"), "{first}");
}

impl Scene {
    fn user_role_of_owner(&self) -> String {
        let db = self.server.database();
        db.query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND \"IsOwner\" = 1", [self.id], |row| row.get::<_, Id>(0))
            .expect("an owner role")
            .to_string()
    }
}

#[tokio::test]
async fn the_members_are_listed_by_name_with_their_roles_and_the_one_that_ranks_them() {
    let scene = Scene::new().await;
    let response = scene.send("jun", Method::GET, "/api/admin/members", None).await;
    assert_eq!(response.status, 200);
    let members: Vec<Value> = response.json();
    assert_eq!(members.len(), 7);
    let names: Vec<&str> = members.iter().map(|member| member["user"]["displayName"].as_str().unwrap()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    let sen = members.iter().find(|member| member["user"]["id"] == scene.user("sen").as_str()).unwrap();
    assert_eq!(sen["roleId"], scene.role("Senior"));
    assert_eq!(sen["roleIds"], json!([scene.role("Senior")]));
    let plain = members.iter().find(|member| member["user"]["id"] == scene.user("plain").as_str()).unwrap();
    assert_eq!(plain["roleId"], scene.role("Member"));
    assert_eq!(plain["roleIds"], json!([]));
    // A moment says it is UTC.
    assert!(sen["joinedAt"].as_str().unwrap().ends_with('Z'), "{sen}");
}

#[tokio::test]
async fn creating_a_role_needs_manage_roles_and_every_power_it_grants() {
    let scene = Scene::new().await;
    let body = |permissions: i64| Some(json!({ "name": "New", "color": " #fff ", "permissions": names(permissions) }));
    // Somebody who may.
    let created = scene.send("sen", Method::POST, "/api/admin/roles", body(ORDINARY)).await;
    assert_eq!(created.status, 201, "{}", created.text());
    let role: Value = created.json();
    assert_eq!(created.header("location").unwrap(), format!("/api/admin/roles/{}", role["id"].as_str().unwrap()));
    assert_eq!((role["name"].as_str(), role["color"].as_str(), role["memberCount"].as_i64()), (Some("New"), Some("#fff"), Some(0)));
    // The nearest who may not: manages roles, and lacks a power the role would grant.
    assert_eq!(scene.status("sen", Method::POST, "/api/admin/roles", Some(json!({ "name": "Greedy", "color": "#fff", "permissions": names(privileges::MANAGE_SERVER) }))).await, 403);
    assert_eq!(scene.status("adm", Method::POST, "/api/admin/roles", Some(json!({ "name": "Fine", "color": "#fff", "permissions": names(privileges::MANAGE_CHANNELS) }))).await, 201);
    assert_eq!(scene.status("jun", Method::POST, "/api/admin/roles", body(0)).await, 403);
    // The new role ranks above the base role and below the others.
    let positions = scene.positions();
    let at = |name: &str| positions.iter().find(|(role, _)| role == name).unwrap().1;
    assert!(at("New") > at("Spare") && at("Fine") > at("New") && at("Fine") < 9, "{positions:?}");
    let audit = scene.audit();
    assert_eq!(audit.iter().filter(|(action, _)| action == "role.create").count(), 2);
}

#[tokio::test]
async fn a_role_that_is_not_valid_or_taken_is_refused_before_the_decision() {
    let scene = Scene::new().await;
    for (body, status) in [
        (json!({ "name": "", "color": "#fff", "permissions": [] }), 400),
        (json!({ "name": "x".repeat(65), "color": "#fff", "permissions": [] }), 400),
        (json!({ "name": "A", "color": "", "permissions": [] }), 400),
        (json!({ "name": "A", "color": "#fff", "permissions": ["nonsense"] }), 400),
        (json!({ "name": "  Admin ", "color": "#fff", "permissions": [] }), 409),
        (json!({ "name": "x".repeat(64), "color": "#fff", "permissions": [] }), 201),
    ] {
        assert_eq!(scene.status("boss", Method::POST, "/api/admin/roles", Some(body.clone())).await, status, "{body}");
    }
    let refused = scene.send("boss", Method::POST, "/api/admin/roles", Some(json!({ "name": "", "color": "#fff" }))).await;
    assert_eq!(refused.header("content-type"), Some("application/problem+json"));
    assert_eq!(refused.json::<Value>()["errors"]["role"], json!(["The name, color or list of permissions of the role is invalid."]));
    // Somebody who may not manage roles is told so before anything else.
    assert_eq!(scene.status("jun", Method::POST, "/api/admin/roles", Some(json!({ "name": "", "color": "" }))).await, 403);
}

#[tokio::test]
async fn editing_a_role_needs_outranking_it_and_every_power_it_has_or_gets() {
    let scene = Scene::new().await;
    let path = |name: &str| format!("/api/admin/roles/{}", scene.role(name));
    // Who may: above the role, holding every power.
    let renamed = scene.send("adm", Method::PATCH, &path("Junior"), Some(json!({ "name": "Renamed" }))).await;
    assert_eq!(renamed.status, 200, "{}", renamed.text());
    assert_eq!(renamed.json::<Value>()["memberCount"], 1);
    // The nearest who may not: a role at the same rank, one of the actor's own, one with a power they lack.
    assert_eq!(scene.status("sen", Method::PATCH, &path("Powerful"), Some(json!({ "name": "X" }))).await, 403);
    assert_eq!(scene.status("sen", Method::PATCH, &path("Senior"), Some(json!({ "name": "X" }))).await, 403);
    assert_eq!(scene.status("adm", Method::PATCH, &path("Powerful"), Some(json!({ "name": "X" }))).await, 403, "it holds a power the actor lacks");
    assert_eq!(scene.status("sen", Method::PATCH, &path("Junior"), Some(json!({ "permissions": names(ORDINARY | privileges::MANAGE_CHANNELS) }))).await, 403, "a power the actor lacks");
    assert_eq!(scene.status("sen", Method::PATCH, &path("Junior"), Some(json!({ "permissions": names(privileges::SEND_MESSAGES) }))).await, 200);
    assert_eq!(scene.status("boss", Method::PATCH, &path("Spare"), Some(json!({ "name": "Admin" }))).await, 409);
    assert_eq!(scene.status("boss", Method::PATCH, &format!("/api/admin/roles/{}", scene.user_role_of_owner()), Some(json!({ "name": "Admin" }))).await, 409, "the owner role is not edited");
    let audit = scene.audit();
    assert!(audit.iter().any(|(action, summary)| action == "role.update" && summary == "Adm updated the role Renamed"), "{audit:?}");
}

#[tokio::test]
async fn deleting_a_role_moves_its_holders_to_the_next_role_they_hold() {
    let scene = Scene::new().await;
    let junior = scene.role("Junior");
    let jun = scene.user("jun");
    assert_eq!(scene.status("boss", Method::PUT, &format!("/api/admin/members/{jun}/roles"), Some(json!({ "roleIds": [junior, scene.role("Spare")] }))).await, 204);
    assert_eq!(scene.held("jun").0, "Junior");
    // The nearest who may not: at the same rank as the role, or lacking a power it grants.
    assert_eq!(scene.status("sen", Method::DELETE, &format!("/api/admin/roles/{}", scene.role("Senior")), None).await, 403);
    assert_eq!(scene.status("adm", Method::DELETE, &format!("/api/admin/roles/{}", scene.role("Powerful")), None).await, 403);
    assert_eq!(scene.status("adm", Method::DELETE, &format!("/api/admin/roles/{}", scene.role("Member")), None).await, 409);
    // Who may.
    assert_eq!(scene.status("adm", Method::DELETE, &format!("/api/admin/roles/{junior}"), None).await, 204);
    assert_eq!(scene.held("jun"), ("Spare".to_string(), vec!["Spare".to_string()]));
    assert_eq!(scene.status("adm", Method::DELETE, &format!("/api/admin/roles/{junior}"), None).await, 404);
    let audit = scene.audit();
    assert!(audit.iter().any(|(action, _)| action == "role.delete"), "{audit:?}");
}

#[tokio::test]
async fn reordering_moves_only_what_the_actor_outranks_and_ranks_everybody_again() {
    let scene = Scene::new().await;
    let order = |names: &[&str]| Some(json!({ "roleIds": names.iter().map(|name| scene.role(name)).collect::<Vec<_>>() }));
    let put = |who: &'static str, names: &[&str]| scene.status(who, Method::PUT, "/api/admin/roles/order", order(names));
    // Who may: moves roles below their own.
    assert_eq!(put("adm", &["Admin", "Senior", "Powerful", "Spare", "Junior"]).await, 204);
    let positions = scene.positions();
    let at = |name: &str| positions.iter().find(|(role, _)| role == name).unwrap().1;
    assert!(at("Spare") < at("Junior"), "{positions:?}");
    assert!(at("Junior") < 9, "the base role stays last");
    // The nearest who may not: putting something above their own top role, or moving a peer.
    assert_eq!(put("sen", &["Senior", "Admin", "Powerful", "Junior", "Spare"]).await, 403);
    assert_eq!(put("sen", &["Admin", "Powerful", "Senior", "Junior", "Spare"]).await, 403);
    assert_eq!(put("jun", &["Admin", "Senior", "Powerful", "Junior", "Spare"]).await, 403);
    // Every role once, no more and no fewer.
    assert_eq!(put("boss", &["Admin", "Senior", "Powerful", "Junior"]).await, 400);
    assert_eq!(put("boss", &["Admin", "Senior", "Powerful", "Junior", "Junior"]).await, 400);
    // An order that changes nothing writes nothing.
    let before = scene.audit().len();
    assert_eq!(put("boss", &["Admin", "Senior", "Powerful", "Spare", "Junior"]).await, 204);
    assert_eq!(scene.audit().len(), before);
    // Everybody is ranked by the new order at once.
    let jun = scene.user("jun");
    let spare = scene.role("Spare");
    assert_eq!(scene.status("boss", Method::PUT, &format!("/api/admin/members/{jun}/roles"), Some(json!({ "roleIds": [scene.role("Junior"), spare] }))).await, 204);
    assert_eq!(scene.held("jun").0, "Spare");
    assert_eq!(put("boss", &["Admin", "Senior", "Powerful", "Junior", "Spare"]).await, 204);
    assert_eq!(scene.held("jun").0, "Junior");
    assert!(scene.audit().iter().any(|(action, _)| action == "role.reorder"));
}

#[tokio::test]
async fn giving_one_role_needs_outranking_the_member_and_the_role() {
    let scene = Scene::new().await;
    let scene = &scene;
    let path = |who: &str| format!("/api/admin/members/{}/role", scene.user(who));
    let give = |actor: &'static str, to: &str, role: &str| {
        let (path, body) = (path(to), json!({ "roleId": scene.role(role) }));
        async move { scene.status(actor, Method::PATCH, &path, Some(body)).await }
    };
    assert_eq!(give("adm", "plain", "Junior").await, 204);
    assert_eq!(scene.held("plain"), ("Junior".to_string(), vec!["Junior".to_string()]));
    // The nearest who may not.
    assert_eq!(give("sen", "plain", "Powerful").await, 403, "a role that holds a power the actor lacks and ranks level");
    assert_eq!(give("adm", "plain", "Powerful").await, 403, "the same power");
    assert_eq!(give("sen", "peer", "Junior").await, 403, "a member at the actor's own rank");
    assert_eq!(give("adm", "adm", "Junior").await, 403, "oneself");
    assert_eq!(give("jun", "plain", "Spare").await, 403, "no manageMembers");
    assert_eq!(give("adm", "boss", "Junior").await, 409, "the owner");
    assert_eq!(give("adm", "plain", "Spare").await, 204);
    // The base role's id leaves only the base role.
    assert_eq!(scene.status("adm", Method::PATCH, &path("plain"), Some(json!({ "roleId": scene.role("Member") }))).await, 204);
    assert_eq!(scene.held("plain").1, Vec::<String>::new());
    // A role that is not there, and one that is left out.
    assert_eq!(scene.status("adm", Method::PATCH, &path("plain"), Some(json!({ "roleId": Id::random() }))).await, 404);
    assert_eq!(scene.status("adm", Method::PATCH, &path("plain"), Some(json!({}))).await, 404);
    assert_eq!(scene.status("adm", Method::PATCH, &path("plain"), Some(json!({ "roleId": null }))).await, 400);
    assert!(scene.audit().iter().any(|(action, summary)| action == "member.role" && summary == "Adm changed the role of Plain to Junior"), "{:?}", scene.audit());
}

#[tokio::test]
async fn setting_the_list_of_roles_checks_each_one_given_or_taken() {
    let scene = Scene::new().await;
    let scene = &scene;
    let path = |who: &str| format!("/api/admin/members/{}/roles", scene.user(who));
    let set = |actor: &'static str, to: &str, roles: &[&str]| {
        let (path, body) = (path(to), json!({ "roleIds": roles.iter().map(|name| scene.role(name)).collect::<Vec<_>>() }));
        async move { scene.status(actor, Method::PUT, &path, Some(body)).await }
    };
    assert_eq!(set("adm", "plain", &["Junior", "Spare"]).await, 204);
    assert_eq!(scene.held("plain"), ("Junior".to_string(), vec!["Junior".to_string(), "Spare".to_string()]));
    // Taking a role away is checked as giving it is.
    assert_eq!(set("sen", "big", &[]).await, 403, "a member at the actor's own rank");
    assert_eq!(set("adm", "big", &[]).await, 403, "a role with a power the actor lacks");
    assert_eq!(set("sen", "plain", &["Junior", "Powerful"]).await, 403);
    assert_eq!(set("sen", "sen", &[]).await, 403, "oneself");
    assert_eq!(set("adm", "plain", &["Junior", "Spare"]).await, 204, "nothing changes");
    assert_eq!(set("adm", "plain", &[]).await, 204);
    assert_eq!(scene.held("plain").1, Vec::<String>::new());
    assert_eq!(set("adm", "boss", &[]).await, 409);
    assert_eq!(scene.status("adm", Method::PUT, &path("plain"), Some(json!({ "roleIds": [Id::random()] }))).await, 404);
    assert_eq!(scene.status("adm", Method::PUT, &path("plain"), Some(json!({}))).await, 204);
}

#[tokio::test]
async fn a_role_override_is_set_and_cleared_by_somebody_who_outranks_the_role_and_holds_what_it_says() {
    let scene = Scene::new().await;
    for (collection, target) in [("channels", scene.channel), ("sections", scene.section)] {
        let base = format!("/api/admin/{collection}/{target}/permissions");
        let junior = format!("{base}/{}", scene.role("Junior"));
        let body = |allow: i64, deny: i64| Some(json!({ "allow": names(allow), "deny": names(deny) }));
        if collection == "sections" {
            // A section is listed for those who see a channel in it, or hold viewChannel in an empty one.
        }
        let set = scene.send("sen", Method::PUT, &junior, body(privileges::SEND_MESSAGES, 0)).await;
        assert_eq!(set.status, 200, "{}", set.text());
        assert_eq!(set.json::<Value>(), json!({ "roleId": scene.role("Junior"), "allow": ["sendMessages"], "deny": [] }));
        let listed: Vec<Value> = scene.send("sen", Method::GET, &base, None).await.json();
        assert_eq!(listed, vec![json!({ "roleId": scene.role("Junior"), "allow": ["sendMessages"], "deny": [] })]);
        // The nearest who may not.
        assert_eq!(scene.status("sen", Method::PUT, &format!("{base}/{}", scene.role("Senior")), body(privileges::SEND_MESSAGES, 0)).await, 403, "the actor's own role");
        assert_eq!(scene.status("sen", Method::PUT, &junior, body(privileges::MANAGE_SERVER, 0)).await, 403, "a power the actor lacks");
        assert_eq!(scene.status("jun", Method::PUT, &junior, body(0, privileges::SEND_MESSAGES)).await, 403);
        assert_eq!(scene.status("jun", Method::GET, &base, None).await, 403);
        // Refusals of the request itself.
        assert_eq!(scene.status("sen", Method::PUT, &junior, body(privileges::SEND_MESSAGES, privileges::SEND_MESSAGES)).await, 400);
        assert_eq!(scene.status("sen", Method::PUT, &junior, body(ADMINISTRATOR, 0)).await, 400);
        assert_eq!(scene.status("sen", Method::PUT, &junior, Some(json!({ "allow": ["nonsense"] }))).await, 400);
        assert_eq!(scene.status("sen", Method::PUT, &format!("{base}/{}", Id::random()), body(0, 0)).await, 404);
        assert_eq!(scene.status("sen", Method::PUT, &format!("/api/admin/{collection}/{}/permissions/{}", Id::random(), scene.role("Junior")), body(0, 0)).await, 404);
        // Clearing needs holding what was said, a deny included.
        assert_eq!(scene.status("sen", Method::PUT, &junior, body(0, privileges::MANAGE_CHANNELS)).await, 403, "a deny of a power the actor lacks");
        assert_eq!(scene.status("boss", Method::PUT, &junior, body(0, privileges::MANAGE_CHANNELS)).await, 200);
        assert_eq!(scene.status("sen", Method::DELETE, &junior, None).await, 403);
        assert_eq!(scene.status("adm", Method::DELETE, &junior, None).await, 204);
        assert_eq!(scene.status("adm", Method::DELETE, &junior, None).await, 204, "clearing nothing is not an error");
        // Setting both empty clears it.
        assert_eq!(scene.status("sen", Method::PUT, &junior, body(privileges::SEND_MESSAGES, 0)).await, 200);
        let cleared = scene.send("sen", Method::PUT, &junior, body(0, 0)).await;
        assert_eq!(cleared.json::<Value>(), json!({ "roleId": scene.role("Junior"), "allow": [], "deny": [] }));
        assert_eq!(scene.send("sen", Method::GET, &base, None).await.json::<Vec<Value>>().len(), 0);
    }
    assert!(scene.audit().iter().any(|(action, _)| action == "channel.permissions"));
    assert!(scene.audit().iter().any(|(action, _)| action == "section.permissions"));
}

#[tokio::test]
async fn a_member_override_needs_outranking_the_member_and_never_touches_the_owner() {
    let scene = Scene::new().await;
    for (collection, target) in [("channels", scene.channel), ("sections", scene.section)] {
        let base = format!("/api/admin/{collection}/{target}/permissions/members");
        let body = |allow: i64| Some(json!({ "allow": names(allow), "deny": [] }));
        let at = |who: &str| format!("{base}/{}", scene.user(who));
        let set = scene.send("sen", Method::PUT, &at("jun"), body(privileges::SEND_MESSAGES)).await;
        assert_eq!(set.status, 200, "{}", set.text());
        assert_eq!(set.json::<Value>(), json!({ "userId": scene.user("jun"), "allow": ["sendMessages"], "deny": [] }));
        let listed: Vec<Value> = scene.send("sen", Method::GET, &base, None).await.json();
        assert_eq!(listed.len(), 1);
        assert_eq!(scene.status("sen", Method::PUT, &at("peer"), body(0)).await, 403, "a member at the actor's rank");
        assert_eq!(scene.status("sen", Method::PUT, &at("sen"), body(0)).await, 403, "oneself");
        assert_eq!(scene.status("sen", Method::PUT, &at("boss"), body(0)).await, 409, "the owner");
        assert_eq!(scene.status("sen", Method::PUT, &at("big"), body(privileges::SEND_MESSAGES)).await, 403);
        assert_eq!(scene.status("adm", Method::PUT, &at("big"), body(privileges::SEND_MESSAGES)).await, 200);
        assert_eq!(scene.status("adm", Method::PUT, &at("plain"), body(privileges::MANAGE_SERVER)).await, 403, "a power the actor lacks");
        assert_eq!(scene.status("sen", Method::PUT, &format!("{base}/{}", Id::random()), body(0)).await, 404);
        assert_eq!(scene.status("sen", Method::DELETE, &at("peer"), None).await, 403);
        assert_eq!(scene.status("sen", Method::DELETE, &at("boss"), None).await, 409);
        assert_eq!(scene.status("sen", Method::DELETE, &at("jun"), None).await, 204);
        assert_eq!(scene.status("sen", Method::DELETE, &at("jun"), None).await, 204);
    }
    assert!(scene.audit().iter().any(|(action, _)| action == "channel.permissions.member"));
}

#[tokio::test]
async fn a_change_of_a_role_tells_each_holder_their_own_permissions_and_nobody_else() {
    let scene = Scene::new().await;
    let before = scene.server.events().len();
    let update = scene
        .send("adm", Method::PATCH, &format!("/api/admin/roles/{}", scene.role("Junior")), Some(json!({ "permissions": names(privileges::SEND_MESSAGES) })))
        .await;
    assert_eq!(update.status, 200);
    let events: Vec<Value> = scene.server.events().into_iter().skip(before).collect();
    let told: Vec<(&str, Value)> = events.iter().map(|event| (event["name"].as_str().unwrap(), event["audience"].clone())).collect();
    assert_eq!(told, vec![("PermissionsChanged", json!({ "users": [scene.user("jun")] }))], "{events:?}");
    assert_eq!(events[0]["payload"], json!({ "serverId": scene.id.to_string(), "permissions": ["sendMessages", "attachFiles", "addReactions", "connect", "viewChannel"] }));
}

#[tokio::test]
async fn whoever_sees_something_else_after_a_change_is_told_to_read_their_channels_again() {
    let scene = Scene::new().await;
    // A channel the base role may not view is private; Junior may.
    let base = scene.base();
    let path = |role: Id| format!("/api/admin/channels/{}/permissions/{role}", scene.channel);
    let before = scene.server.events().len();
    assert_eq!(scene.status("boss", Method::PUT, &path(base), Some(json!({ "allow": [], "deny": ["viewChannel"] }))).await, 200);
    let events: Vec<Value> = scene.server.events().into_iter().skip(before).collect();
    let access: Vec<&Value> = events.iter().filter(|event| event["name"] == "ChannelAccessChanged").collect();
    assert_eq!(access.len(), 1, "{events:?}");
    let told = access[0]["audience"]["followers"]["users"].as_array().unwrap();
    // Everybody who held nothing else that shows the channel loses it: the plain member, and nobody who holds a role with view.
    assert!(told.contains(&json!(scene.user("plain"))), "{told:?}");
    assert_eq!(access[0]["audience"]["followers"]["server"], scene.id.to_string());
    assert_eq!(access[0]["payload"], json!({ "serverId": scene.id.to_string() }));
    // The channel does not exist for the plain member: its overrides are not theirs to read.
    assert_eq!(scene.status("plain", Method::GET, &format!("/api/admin/channels/{}/permissions", scene.channel), None).await, 404);
    // Giving the view back to a role shows it again, and the person who gains it is told.
    let before = scene.server.events().len();
    assert_eq!(scene.status("boss", Method::DELETE, &path(base), None).await, 204);
    let events: Vec<Value> = scene.server.events().into_iter().skip(before).collect();
    assert!(events.iter().any(|event| event["name"] == "ChannelAccessChanged"), "{events:?}");
    let _ = Timestamp::now();
}

// ---------------------------------------------------------------------------
// The second step, asked on the instance's own server only.

/// Gives the person a confirmed second step, and their sessions the moment it
/// was passed (never, when none).
fn second_step(scene: &Scene, who: &str, passed_seconds_ago: Option<u64>) {
    let db = scene.server.database();
    let user = Id::parse(&scene.user(who)).expect("an id");
    db.execute(
        "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\") VALUES (?1, ?2, 'totp', 'Phone', ?3, ?3)",
        params![Id::random(), user, Timestamp::now()],
    )
    .expect("a method");
    let passed = passed_seconds_ago.map(|seconds| Timestamp::now().minus(std::time::Duration::from_secs(seconds)));
    db.execute("UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = ?2 WHERE \"UserId\" = ?1", params![user, passed]).expect("sessions");
}

/// The fourteen routes that need it, each as a request that would succeed for
/// somebody who may: the path, the method and the body.
fn second_step_routes(scene: &Scene) -> Vec<(Method, String, Option<Value>)> {
    let (channel, section) = (scene.channel, scene.section);
    let junior = scene.role("Junior");
    let plain = scene.user("plain");
    let order: Vec<String> = ["Admin", "Senior", "Powerful", "Junior", "Spare"].iter().map(|name| scene.role(name)).collect();
    let mut routes = vec![
        (Method::POST, "/api/admin/roles".to_string(), Some(json!({ "name": "Late", "color": "#fff", "permissions": [] }))),
        (Method::PUT, "/api/admin/roles/order".to_string(), Some(json!({ "roleIds": order }))),
        (Method::PATCH, format!("/api/admin/roles/{junior}"), Some(json!({ "name": "Renamed" }))),
        (Method::DELETE, format!("/api/admin/roles/{}", scene.role("Spare")), None),
        (Method::PATCH, format!("/api/admin/members/{plain}/role"), Some(json!({ "roleId": junior }))),
        (Method::PUT, format!("/api/admin/members/{plain}/roles"), Some(json!({ "roleIds": [junior] }))),
    ];
    for (collection, place) in [("channels", channel.to_string()), ("sections", section.to_string())] {
        let base = format!("/api/admin/{collection}/{place}/permissions");
        let masks = Some(json!({ "allow": ["sendMessages"], "deny": [] }));
        routes.push((Method::PUT, format!("{base}/{junior}"), masks.clone()));
        routes.push((Method::DELETE, format!("{base}/{junior}"), None));
        routes.push((Method::PUT, format!("{base}/members/{plain}"), masks));
        routes.push((Method::DELETE, format!("{base}/members/{plain}"), None));
    }
    routes
}

fn writes(scene: &Scene) -> i64 {
    let db = scene.server.database();
    db.query_row("SELECT (SELECT COUNT(*) FROM \"ServerAuditEntries\") + (SELECT COUNT(*) FROM \"ServerRoles\") + (SELECT SUM(\"Position\") FROM \"ServerRoles\")", [], |row| row.get(0)).unwrap()
}

#[tokio::test]
async fn a_stale_second_step_refuses_the_fourteen_routes_and_writes_nothing() {
    let scene = Scene::new().await;
    second_step(&scene, "adm", None);
    let before = writes(&scene);
    for (method, path, body) in second_step_routes(&scene) {
        let refused = scene.send("adm", method.clone(), &path, body).await;
        assert_eq!(refused.status, 403, "{method} {path}: {}", refused.text());
        let problem: Value = refused.json();
        assert_eq!(problem["code"], "fresh-mfa-required", "{method} {path}");
    }
    assert_eq!(writes(&scene), before);
    // Reading is not asked for the step.
    assert_eq!(scene.status("adm", Method::GET, "/api/admin/roles", None).await, 200);
    assert_eq!(scene.status("adm", Method::GET, &format!("/api/admin/channels/{}/permissions", scene.channel), None).await, 200);
}

#[tokio::test]
async fn a_second_step_passed_lately_or_none_to_pass_leaves_the_routes_open() {
    let scene = Scene::new().await;
    second_step(&scene, "adm", Some(60));
    // `sen` has no second step at all.
    for (who, route) in [("adm", 0), ("sen", 2), ("adm", 4), ("sen", 6)] {
        let (method, path, body) = second_step_routes(&scene).remove(route);
        let answer = scene.send(who, method.clone(), &path, body).await;
        assert!(answer.status.is_success(), "{who} {method} {path}: {} {}", answer.status, answer.text());
    }
}

#[tokio::test]
async fn the_order_of_refusals_puts_the_second_step_after_the_request_is_read_and_before_the_decision() {
    let scene = Scene::new().await;
    second_step(&scene, "jun", None);
    second_step(&scene, "adm", None);
    // A person who may not manage roles is asked for the step first.
    let create = Some(json!({ "name": "Late", "color": "#fff", "permissions": [] }));
    let refused = scene.send("jun", Method::POST, "/api/admin/roles", create).await;
    assert_eq!((refused.status.as_u16(), refused.json::<Value>()["code"].clone()), (403, json!("fresh-mfa-required")));
    // So is one whose request names a role that is not there, or one above them.
    let unknown = format!("/api/admin/roles/{}", Id::random());
    assert_eq!(scene.send("adm", Method::PATCH, &unknown, Some(json!({}))).await.json::<Value>()["code"], "fresh-mfa-required");
    let above = format!("/api/admin/roles/{}", scene.role("Admin"));
    assert_eq!(scene.send("adm", Method::PATCH, &above, Some(json!({ "name": "X" }))).await.json::<Value>()["code"], "fresh-mfa-required");
    // A body that cannot be read is refused first, as it was bound first.
    let broken = crate::support::Request::new(Method::POST, "/api/admin/roles").body("application/json", "{");
    assert_eq!(scene.person("adm").client.send(broken).await.status, 400);
    // The step is not asked for on a server that is not the instance's own.
    let other = Id::random();
    {
        let db = scene.server.database();
        db.execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Other', '', 0, ?2, 0, 0)",
            params![other, Timestamp::now()],
        )
        .unwrap();
        let owner_role = Id::random();
        db.execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, 'Owner', '#777', ?3, 0, 1, 1)",
            params![owner_role, other, privileges::ALL],
        )
        .unwrap();
        let base = Id::random();
        db.execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, 'Member', '#777', ?3, 5, 1, 0)",
            params![base, other, privileges::BASE_ROLE_DEFAULTS],
        )
        .unwrap();
        let user = Id::parse(&scene.user("adm")).unwrap();
        db.execute(
            "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"OwnerSlot\", \"JoinedAt\") VALUES (?1, ?2, ?3, 1, ?4)",
            params![other, user, owner_role, Timestamp::now()],
        )
        .unwrap();
        db.execute("INSERT INTO \"ServerMemberRoles\" (\"ServerId\", \"UserId\", \"RoleId\") VALUES (?1, ?2, ?3)", params![other, user, owner_role]).unwrap();
    }
    let named = crate::support::Request::new(Method::POST, "/api/admin/roles")
        .header("X-Stellarr-Server", &other.to_string())
        .json(&json!({ "name": "Elsewhere", "color": "#fff", "permissions": [] }));
    assert_eq!(scene.person("adm").client.send(named).await.status, 201);
}
