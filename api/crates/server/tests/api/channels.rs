//! Channels and sections through the real binary: making, ordering, changing
//! and removing them, who may, what is refused and how, what the audit log
//! keeps, who is told, the marker of how far a person has read, and the hub's
//! `JoinChannel` and `SendTyping`.
//!
//! A server is arranged the way a test needs it by writing rows, since the
//! routes that make roles and overrides belong to another group.

use std::time::Duration;

use http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::support::{HubClient, Person, Request, Response, Server};

const COOKIE: &str = "stellarr-dev-session";
const NOBODY: &str = "11111111-1111-1111-1111-111111111111";
const QUIET: Duration = Duration::from_millis(400);

const MANAGE_CHANNELS: i64 = 1 << 1;
const SEND_MESSAGES: i64 = 1 << 9;
const VIEW_CHANNEL: i64 = 1 << 14;

/// An instance with its owner, who sees and may do everything.
struct World {
    server: Server,
    owner: Person,
}

impl World {
    async fn new() -> World {
        let server = Server::start().await;
        let owner = server.found("owner").await;
        World { server, owner }
    }

    fn db(&self) -> Connection {
        self.server.database()
    }

    /// Somebody who has joined with the rights of the base role alone.
    async fn member(&self, name: &str) -> Person {
        self.server.register(name).await
    }

    /// Somebody who has joined and may manage channels on the whole server.
    async fn manager(&self, name: &str) -> Person {
        let person = self.member(name).await;
        let role = self.role(&format!("Manager {name}"), MANAGE_CHANNELS | SEND_MESSAGES | VIEW_CHANNEL);
        self.db()
            .execute(
                "UPDATE \"ServerMembers\" SET \"RoleId\" = ?1 WHERE \"UserId\" = upper(?2)",
                params![role, person.id],
            )
            .unwrap();
        person
    }

    /// A role below the owner's and above the base one.
    fn role(&self, name: &str, permissions: i64) -> String {
        let db = self.db();
        let server = self.server_id();
        let id = stellarr_api::values::Id::random().to_db_text();
        db.execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\")
             VALUES (?1, ?2, ?3, '#3366ff', ?4, 2, 0, 0)",
            params![id, server, name, permissions],
        )
        .unwrap();
        // The base role ranks last.
        db.execute(
            "UPDATE \"ServerRoles\" SET \"Position\" = 9 WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0",
            [&server],
        )
        .unwrap();
        id
    }

    fn server_id(&self) -> String {
        self.db()
            .query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0))
            .unwrap()
    }

    /// The id, in the way the clients write it, of the channel or section with this name.
    fn id_of(&self, table: &str, name: &str) -> String {
        self.db()
            .query_row(
                &format!("SELECT lower(\"Id\") FROM \"{table}\" WHERE \"Name\" = ?1"),
                [name],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| panic!("no {table} called {name}"))
    }

    fn channel(&self, name: &str) -> String {
        self.id_of("Channels", name)
    }

    /// Makes a channel by writing it, at the end, and returns its id.
    fn add_channel(&self, name: &str, kind: i64, section: Option<&str>) -> String {
        let db = self.db();
        let id = stellarr_api::values::Id::random();
        let position: i64 = db
            .query_row("SELECT COALESCE(MAX(\"Position\"), -1) + 1 FROM \"Channels\"", [], |row| row.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
             VALUES (?1, ?2, ?3, ?4, ?5, '', 0, 0, 0, upper(?6))",
            params![id.to_db_text(), self.server_id(), name, kind, position, section],
        )
        .unwrap();
        id.to_string()
    }

    fn add_section(&self, name: &str) -> String {
        let db = self.db();
        let id = stellarr_api::values::Id::random();
        let position: i64 = db
            .query_row("SELECT COALESCE(MAX(\"Position\"), -1) + 1 FROM \"Sections\"", [], |row| row.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, ?3, ?4)",
            params![id.to_db_text(), self.server_id(), name, position],
        )
        .unwrap();
        id.to_string()
    }

    /// Takes a channel (`target` 0) or a section (`target` 1) out of sight of one person.
    fn hide(&self, target: i64, place: &str, person: &Person) {
        self.member_override(target, place, person, 0, VIEW_CHANNEL);
    }

    /// Takes a section out of the list of one person who manages channels, and
    /// whose channels in it are out of sight: they neither view it nor manage it.
    fn unlist(&self, section: &str, person: &Person) {
        self.member_override(1, section, person, 0, MANAGE_CHANNELS | VIEW_CHANNEL);
    }

    fn member_override(&self, target: i64, place: &str, person: &Person, allow: i64, deny: i64) {
        self.db()
            .execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\")
                 VALUES (?1, ?2, ?3, upper(?4), upper(?5), ?6, ?7)",
                params![
                    stellarr_api::values::Id::random().to_db_text(),
                    self.server_id(),
                    target,
                    place,
                    person.id,
                    allow,
                    deny
                ],
            )
            .unwrap();
    }

    /// Every channel in its order: name and section name.
    fn layout(&self) -> Vec<(String, Option<String>)> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT c.\"Name\", s.\"Name\" FROM \"Channels\" c LEFT JOIN \"Sections\" s ON s.\"Id\" = c.\"SectionId\"
                 ORDER BY c.\"Position\", c.\"Id\"",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn order(&self) -> Vec<String> {
        self.layout().into_iter().map(|(name, _)| name).collect()
    }

    fn positions(&self) -> Vec<i64> {
        let db = self.db();
        let mut statement = db.prepare("SELECT \"Position\" FROM \"Channels\" ORDER BY \"Position\"").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    }

    fn section_order(&self) -> Vec<String> {
        let db = self.db();
        let mut statement = db.prepare("SELECT \"Name\" FROM \"Sections\" ORDER BY \"Position\", \"Id\"").unwrap();
        statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    }

    /// The audit lines, oldest first: action, target name, summary, changes.
    fn audit(&self) -> Vec<(String, Option<String>, String, Option<String>)> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT \"Action\", \"TargetName\", \"Summary\", \"ChangesJson\" FROM \"ServerAuditEntries\" ORDER BY \"CreatedAt\", \"Id\"",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter(|(action, ..): &(String, Option<String>, String, Option<String>)| !action.starts_with("server") && !action.starts_with("member"))
            .collect()
    }

    fn events(&self, name: &str) -> Vec<Value> {
        self.server.events().into_iter().filter(|event| event["name"] == name).collect()
    }
}

async fn send(person: &Person, method: Method, path: &str, body: Option<Value>) -> Response {
    let request = Request::new(method, path);
    person.client.send(match body {
        Some(body) => request.json(&body),
        None => request,
    })
    .await
}

async fn create(person: &Person, body: Value) -> Response {
    send(person, Method::POST, "/api/admin/channels", Some(body)).await
}

async fn patch(person: &Person, channel: &str, body: Value) -> Response {
    send(person, Method::PATCH, &format!("/api/admin/channels/{channel}"), Some(body)).await
}

async fn remove(person: &Person, channel: &str) -> Response {
    send(person, Method::DELETE, &format!("/api/admin/channels/{channel}"), None).await
}

async fn reorder(person: &Person, ids: &[&str], movement: Option<Value>) -> Response {
    let mut body = json!({ "channelIds": ids });
    if let Some(movement) = movement {
        body["move"] = movement;
    }
    send(person, Method::PUT, "/api/admin/channels/order", Some(body)).await
}

async fn section(person: &Person, body: Value) -> Response {
    send(person, Method::POST, "/api/admin/sections", Some(body)).await
}

fn error_of(response: &Response, field: &str) -> String {
    response.json::<Value>()["errors"][field][0].as_str().unwrap_or_default().to_string()
}

fn id_of(response: &Response) -> String {
    response.json::<Value>()["id"].as_str().unwrap().to_string()
}

// Making a channel.

#[tokio::test]
async fn a_channel_is_made_at_the_end_with_its_defaults_and_the_audit_keeps_it() {
    let world = World::new().await;

    let response = create(&world.owner, json!({ "name": "  Foo-Bar ", "kind": "text" })).await;

    assert_eq!(response.status, 201, "{}", response.text());
    let id = id_of(&response);
    assert_eq!(response.header("location"), Some(format!("/api/admin/channels/{id}").as_str()));
    assert_eq!(
        response.text(),
        format!(
            "{{\"id\":\"{id}\",\"name\":\"foo-bar\",\"kind\":\"text\",\"position\":2,\"topic\":\"\",\"slowModeSeconds\":0,\"hideAfterInactivityHours\":0,\"userLimit\":0,\"sectionId\":null}}"
        )
    );
    assert_eq!(world.order(), ["general", "voice", "foo-bar"]);
    let audit = world.audit();
    let (action, target, summary, changes) = audit.last().unwrap();
    assert_eq!(action, "channel.create");
    assert_eq!(target.as_deref(), Some("foo-bar"));
    assert_eq!(summary, "Owner created the channel #foo-bar");
    assert_eq!(changes.as_deref(), Some("[{\"field\":\"kind\",\"to\":\"text\"}]"));
}

#[tokio::test]
async fn a_channel_made_in_a_section_says_which_in_the_audit() {
    let world = World::new().await;
    let alpha = world.add_section("Alpha");

    let response = create(&world.owner, json!({ "name": "dev", "kind": "voice", "sectionId": alpha })).await;

    assert_eq!(response.status, 201, "{}", response.text());
    assert_eq!(response.json::<Value>()["sectionId"], alpha.as_str());
    assert_eq!(response.json::<Value>()["kind"], "voice");
    let audit = world.audit();
    assert_eq!(
        audit.last().unwrap().3.as_deref(),
        Some("[{\"field\":\"kind\",\"to\":\"voice\"},{\"field\":\"section\",\"to\":\"Alpha\"}]")
    );
}

#[tokio::test]
async fn a_name_or_kind_outside_the_rules_is_refused_with_the_reason() {
    let world = World::new().await;
    let cases = [
        (json!({ "name": "a b", "kind": "text" }), "spaces"),
        (json!({ "name": "", "kind": "text" }), "empty"),
        (json!({ "name": "   ", "kind": "text" }), "blank"),
        (json!({ "name": "x".repeat(61), "kind": "text" }), "too long"),
        (json!({ "name": "under_score", "kind": "text" }), "an underscore"),
        (json!({ "name": "\u{1D400}", "kind": "text" }), "a letter outside the basic plane"),
        (json!({ "name": "ok", "kind": "Text" }), "a kind in capitals"),
        (json!({ "name": "ok", "kind": "video" }), "an unknown kind"),
        (json!({ "name": "ok" }), "no kind"),
        (json!({ "kind": "text" }), "no name"),
        (json!({ "name": null, "kind": "text" }), "a null name"),
    ];
    for (body, why) in cases {
        let response = create(&world.owner, body).await;
        assert_eq!(response.status, 400, "{why}: {}", response.text());
        assert_eq!(error_of(&response, "channel"), "The channel name or type is invalid.", "{why}");
        assert_eq!(response.header("content-type"), Some("application/problem+json"));
    }
    assert_eq!(world.order(), ["general", "voice"]);
}

#[tokio::test]
async fn a_name_of_sixty_letters_and_one_with_accents_are_taken_in_lower_case() {
    let world = World::new().await;

    let long = create(&world.owner, json!({ "name": "A".repeat(60), "kind": "text" })).await;
    let accented = create(&world.owner, json!({ "name": "ŻÓŁĆ-1", "kind": "voice" })).await;

    assert_eq!(long.status, 201);
    assert_eq!(long.json::<Value>()["name"], "a".repeat(60).as_str());
    assert_eq!(accented.status, 201);
    assert_eq!(accented.json::<Value>()["name"], "żółć-1");
}

#[tokio::test]
async fn a_name_is_taken_whatever_its_case_and_by_a_channel_the_caller_cannot_see() {
    let world = World::new().await;
    let hidden = world.manager("hana").await;
    let secret = world.add_channel("secret", 0, None);
    world.hide(0, &secret, &hidden);

    let same = create(&world.owner, json!({ "name": "GENERAL", "kind": "text" })).await;
    let behind = create(&hidden, json!({ "name": "Secret", "kind": "text" })).await;

    assert_eq!(same.status, 409);
    assert!(same.body.is_empty());
    assert_eq!(behind.status, 409, "a name another channel has is taken, seen or not");
    assert_eq!(world.order().len(), 3);
}

#[tokio::test]
async fn somebody_who_may_not_manage_channels_is_forbidden_and_a_stranger_is_not_let_in() {
    let world = World::new().await;
    let maya = world.member("maya").await;

    let forbidden = create(&maya, json!({ "name": "mine", "kind": "text" })).await;
    let anonymous = world
        .server
        .anonymous()
        .send(Request::new(Method::POST, "/api/admin/channels").json(&json!({ "name": "x", "kind": "text" })))
        .await;

    assert_eq!(forbidden.status, 403);
    assert!(forbidden.body.is_empty());
    assert_eq!(anonymous.status, 401);
    assert_eq!(world.order(), ["general", "voice"]);
}

#[tokio::test]
async fn a_body_that_is_not_the_right_json_is_a_bad_request() {
    let world = World::new().await;

    let wrong = send(&world.owner, Method::POST, "/api/admin/channels", Some(json!({ "name": 5, "kind": "text" }))).await;
    let text = world
        .owner
        .client
        .send(Request::new(Method::POST, "/api/admin/channels").body("text/plain", "{}"))
        .await;

    assert_eq!(wrong.status, 400);
    assert_eq!(text.status, 415);
}

#[tokio::test]
async fn a_section_must_be_listed_for_the_caller_and_decides_who_may_add_to_it() {
    let world = World::new().await;
    let bravo = world.add_section("Bravo");
    let hidden_channel = world.add_channel("in-bravo", 0, Some(&bravo));
    let hana = world.manager("hana").await;
    world.hide(0, &hidden_channel, &hana);
    world.unlist(&bravo, &hana);
    // Dan manages nothing but Bravo.
    let dan = world.member("dan").await;
    world.member_override(1, &bravo, &dan, MANAGE_CHANNELS, 0);

    let unknown = create(&world.owner, json!({ "name": "a", "kind": "text", "sectionId": NOBODY })).await;
    let unlisted = create(&hana, json!({ "name": "b", "kind": "text", "sectionId": bravo })).await;
    let outside = create(&dan, json!({ "name": "c", "kind": "text" })).await;
    let inside = create(&dan, json!({ "name": "d", "kind": "text", "sectionId": bravo })).await;

    assert_eq!(unknown.status, 404);
    assert_eq!(unlisted.status, 404, "a section the caller does not see does not exist");
    assert_eq!(outside.status, 403);
    assert_eq!(inside.status, 201, "{}", inside.text());
    assert_eq!(inside.json::<Value>()["sectionId"], bravo.as_str());
}

#[tokio::test]
async fn two_requests_for_one_name_end_in_one_channel_and_no_server_error() {
    let world = World::new().await;
    let manager = world.manager("hana").await;

    let (a, b) = tokio::join!(
        create(&world.owner, json!({ "name": "race", "kind": "text" })),
        create(&manager, json!({ "name": "race", "kind": "text" })),
    );

    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 409]);
    assert_eq!(world.order().iter().filter(|name| *name == "race").count(), 1);
}

// Changing a channel.

#[tokio::test]
async fn a_channel_is_changed_in_part_and_each_change_is_in_the_audit() {
    let world = World::new().await;
    let general = world.channel("general");

    let response = patch(&world.owner, &general, json!({ "topic": "  Hello  ", "slowModeSeconds": 30, "name": " Lobby " })).await;

    assert_eq!(response.status, 200, "{}", response.text());
    assert_eq!(
        response.text(),
        format!(
            "{{\"id\":\"{general}\",\"name\":\"lobby\",\"kind\":\"text\",\"position\":0,\"topic\":\"Hello\",\"slowModeSeconds\":30,\"hideAfterInactivityHours\":0,\"userLimit\":0,\"sectionId\":null}}"
        )
    );
    let audit = world.audit();
    let (action, target, summary, changes) = audit.last().unwrap();
    assert_eq!(action, "channel.update");
    assert_eq!(target.as_deref(), Some("lobby"));
    assert_eq!(summary, "Owner updated the channel #lobby");
    assert_eq!(
        changes.as_deref(),
        Some("[{\"field\":\"name\",\"from\":\"general\",\"to\":\"lobby\"},{\"field\":\"topic\",\"from\":\"\",\"to\":\"Hello\"},{\"field\":\"slowMode\",\"from\":\"0\",\"to\":\"30\"}]")
    );
}

#[tokio::test]
async fn a_change_that_changes_nothing_is_answered_and_not_recorded() {
    let world = World::new().await;
    let general = world.channel("general");
    let before = world.audit().len();

    let response = patch(&world.owner, &general, json!({})).await;
    let same = patch(&world.owner, &general, json!({ "name": "General", "topic": "" })).await;

    assert_eq!(response.status, 200);
    assert_eq!(same.status, 200);
    assert_eq!(world.audit().len(), before);
}

#[tokio::test]
async fn a_setting_for_the_other_kind_of_channel_or_out_of_range_is_refused() {
    let world = World::new().await;
    let general = world.channel("general");
    let voice = world.channel("voice");

    for (channel, body, key) in [
        (&general, json!({ "userLimit": 5 }), "channel"),
        (&voice, json!({ "topic": "x" }), "channel"),
        (&voice, json!({ "slowModeSeconds": 0 }), "channel"),
    ] {
        let response = patch(&world.owner, channel, body).await;
        assert_eq!(response.status, 400, "{}", response.text());
        assert_eq!(error_of(&response, key), "This setting does not fit the type of the channel.");
    }
    for (channel, body) in [
        (&general, json!({ "slowModeSeconds": 21601 })),
        (&general, json!({ "slowModeSeconds": -1 })),
        (&general, json!({ "hideAfterInactivityHours": 8761 })),
        (&voice, json!({ "userLimit": 100 })),
        (&voice, json!({ "userLimit": -1 })),
        (&general, json!({ "topic": "t".repeat(1025) })),
    ] {
        let response = patch(&world.owner, channel, body.clone()).await;
        assert_eq!(response.status, 400, "{body}: {}", response.text());
        assert_eq!(error_of(&response, "channel"), "At least one channel setting is invalid.", "{body}");
    }
    let edge = patch(&world.owner, &general, json!({ "slowModeSeconds": 21600, "topic": "t".repeat(1024) })).await;
    let limit = patch(&world.owner, &voice, json!({ "userLimit": 99, "hideAfterInactivityHours": 8760 })).await;
    assert_eq!(edge.status, 200);
    assert_eq!(limit.status, 200);
}

#[tokio::test]
async fn a_new_name_is_checked_for_its_form_and_for_being_taken() {
    let world = World::new().await;
    let general = world.channel("general");
    let secret = world.add_channel("secret", 0, None);
    let hana = world.manager("hana").await;
    world.hide(0, &secret, &hana);

    let bad = patch(&world.owner, &general, json!({ "name": "a b" })).await;
    let taken = patch(&world.owner, &general, json!({ "name": "Voice" })).await;
    let hidden = patch(&hana, &general, json!({ "name": "SECRET" })).await;

    assert_eq!(bad.status, 400);
    assert_eq!(error_of(&bad, "name"), "The channel name is invalid.");
    assert_eq!(taken.status, 409);
    assert_eq!(hidden.status, 409);
    assert_eq!(world.order()[0], "general");
}

#[tokio::test]
async fn a_channel_the_caller_does_not_see_is_not_there_and_one_they_may_not_manage_is_forbidden() {
    let world = World::new().await;
    let secret = world.add_channel("secret", 0, None);
    let general = world.channel("general");
    let hana = world.manager("hana").await;
    let maya = world.member("maya").await;
    world.hide(0, &secret, &hana);
    world.hide(0, &secret, &maya);

    let unseen = patch(&hana, &secret, json!({ "topic": "x" })).await;
    let unseen_by_member = patch(&maya, &secret, json!({ "topic": "x" })).await;
    let forbidden = patch(&maya, &general, json!({ "topic": "x" })).await;
    let nowhere = patch(&world.owner, NOBODY, json!({})).await;
    let not_an_id = patch(&world.owner, "nope", json!({})).await;
    let removing = remove(&hana, &secret).await;

    assert_eq!(unseen.status, 404);
    assert_eq!(unseen_by_member.status, 404);
    assert_eq!(forbidden.status, 403);
    assert_eq!(nowhere.status, 404);
    assert_eq!(not_an_id.status, 404);
    assert_eq!(removing.status, 404);
    assert_eq!(world.order().len(), 3);
}

#[tokio::test]
async fn a_path_may_spell_the_id_without_hyphens() {
    let world = World::new().await;
    let general = world.channel("general").replace('-', "");

    let response = patch(&world.owner, &general, json!({ "topic": "x" })).await;

    assert_eq!(response.status, 200, "{}", response.text());
}

// Removing a channel.

#[tokio::test]
async fn a_channel_is_removed_with_what_belonged_to_it_and_the_last_of_a_kind_stays() {
    let world = World::new().await;
    let extra = world.add_channel("extra", 0, None);
    let general = world.channel("general");
    let voice = world.channel("voice");
    let marked = send(&world.owner, Method::PUT, &format!("/api/channels/{extra}/read"), None).await;
    assert_eq!(marked.status, 200);

    let removed = remove(&world.owner, &extra).await;
    let last_text = remove(&world.owner, &general).await;
    let last_voice = remove(&world.owner, &voice).await;

    assert_eq!(removed.status, 204);
    assert!(removed.body.is_empty());
    assert_eq!(last_text.status, 409);
    assert_eq!(last_voice.status, 409);
    assert_eq!(world.order(), ["general", "voice"]);
    let reads: i64 = world.db().query_row("SELECT COUNT(*) FROM \"ChannelReads\"", [], |row| row.get(0)).unwrap();
    assert_eq!(reads, 0, "a channel's read markers go with it");
    let audit = world.audit();
    let (action, target, summary, changes) = audit.iter().find(|line| line.0 == "channel.delete").unwrap();
    assert_eq!(action, "channel.delete");
    assert_eq!(target.as_deref(), Some("extra"));
    assert_eq!(summary, "Owner deleted the channel #extra");
    assert_eq!(*changes, None);
}

#[tokio::test]
async fn two_removals_of_the_last_two_channels_of_a_kind_leave_one() {
    let world = World::new().await;
    let other = world.add_channel("other", 0, None);
    let general = world.channel("general");

    let (a, b) = tokio::join!(remove(&world.owner, &general), remove(&world.owner, &other));

    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [204, 409]);
    let texts: i64 = world.db().query_row("SELECT COUNT(*) FROM \"Channels\" WHERE \"Kind\" = 0", [], |row| row.get(0)).unwrap();
    assert_eq!(texts, 1);
}

// Sections.

#[tokio::test]
async fn a_section_is_made_renamed_and_removed_and_each_is_in_the_audit() {
    let world = World::new().await;

    let made = section(&world.owner, json!({ "name": "  Alpha  " })).await;
    let id = id_of(&made);
    let renamed = send(&world.owner, Method::PATCH, &format!("/api/admin/sections/{id}"), Some(json!({ "name": " Beta " }))).await;
    let same = send(&world.owner, Method::PATCH, &format!("/api/admin/sections/{id}"), Some(json!({ "name": "Beta" }))).await;
    let removed = send(&world.owner, Method::DELETE, &format!("/api/admin/sections/{id}"), None).await;

    assert_eq!(made.status, 201, "{}", made.text());
    assert_eq!(made.header("location"), Some(format!("/api/admin/sections/{id}").as_str()));
    assert_eq!(made.text(), format!("{{\"id\":\"{id}\",\"name\":\"Alpha\",\"position\":0}}"));
    assert_eq!(renamed.status, 200);
    assert_eq!(renamed.text(), format!("{{\"id\":\"{id}\",\"name\":\"Beta\",\"position\":0}}"));
    assert_eq!(same.status, 200);
    assert_eq!(removed.status, 204);
    assert!(world.section_order().is_empty());
    let audit = world.audit();
    let lines: Vec<_> = audit.iter().filter(|line| line.0.starts_with("section")).collect();
    assert_eq!(lines.len(), 3, "the rename to the same name is not a change");
    assert_eq!(lines[0].2, "Owner created the section Alpha");
    assert_eq!(lines[1].2, "Owner renamed the section Beta");
    assert_eq!(lines[1].3.as_deref(), Some("[{\"field\":\"name\",\"from\":\"Alpha\",\"to\":\"Beta\"}]"));
    assert_eq!(lines[2].2, "Owner deleted the section Beta");
}

#[tokio::test]
async fn a_section_name_is_one_to_sixty_units_once_trimmed() {
    let world = World::new().await;

    for name in ["", "   ", &"x".repeat(61), &"\u{1D400}".repeat(31)] {
        let response = section(&world.owner, json!({ "name": name })).await;
        assert_eq!(response.status, 400, "{name:?}");
        assert_eq!(error_of(&response, "name"), "The section name is invalid.");
    }
    let missing = section(&world.owner, json!({})).await;
    let longest = section(&world.owner, json!({ "name": "\u{1D400}".repeat(30) })).await;
    let id = id_of(&longest);
    let blank = send(&world.owner, Method::PATCH, &format!("/api/admin/sections/{id}"), Some(json!({ "name": "" }))).await;
    let kept = send(&world.owner, Method::PATCH, &format!("/api/admin/sections/{id}"), Some(json!({}))).await;

    assert_eq!(missing.status, 400);
    assert_eq!(longest.status, 201);
    assert_eq!(blank.status, 400);
    assert_eq!(kept.status, 200);
    assert_eq!(world.section_order().len(), 1);
}

#[tokio::test]
async fn a_section_that_still_holds_channels_is_not_removed() {
    let world = World::new().await;
    let alpha = world.add_section("Alpha");
    world.add_channel("dev", 0, Some(&alpha));

    let response = send(&world.owner, Method::DELETE, &format!("/api/admin/sections/{alpha}"), None).await;

    assert_eq!(response.status, 409);
    assert_eq!(world.section_order(), ["Alpha"]);
}

#[tokio::test]
async fn a_section_the_caller_cannot_see_is_not_there_and_one_they_cannot_manage_is_forbidden() {
    let world = World::new().await;
    let alpha = world.add_section("Alpha");
    let bravo = world.add_section("Bravo");
    let secret = world.add_channel("secret", 0, Some(&bravo));
    let hana = world.manager("hana").await;
    let maya = world.member("maya").await;
    world.hide(0, &secret, &hana);
    // A section with nothing hidden in it is listed for everybody; one where
    // every channel is out of sight is not, unless they manage it.
    let dan = world.member("dan").await;
    world.hide(0, &secret, &dan);

    async fn rename(person: &Person, id: &str) -> Response {
        send(person, Method::PATCH, &format!("/api/admin/sections/{id}"), Some(json!({ "name": "No" }))).await
    }
    let unlisted = rename(&dan, &bravo).await;
    let forbidden = rename(&maya, &alpha).await;
    let missing = rename(&world.owner, NOBODY).await;
    let deleting = send(&dan, Method::DELETE, &format!("/api/admin/sections/{bravo}"), None).await;
    let making = section(&maya, json!({ "name": "Maya's" })).await;

    assert_eq!(unlisted.status, 404);
    assert_eq!(forbidden.status, 403);
    assert_eq!(missing.status, 404);
    assert_eq!(deleting.status, 404);
    assert_eq!(making.status, 403);
    assert_eq!(world.section_order(), ["Alpha", "Bravo"]);
}

// Putting channels in order.

#[tokio::test]
async fn an_order_names_every_channel_the_caller_sees_exactly_once() {
    let world = World::new().await;
    let ids: Vec<String> = ["general", "voice"].iter().map(|name| world.channel(name)).collect();
    let [general, voice] = [&ids[0], &ids[1]];

    let short = reorder(&world.owner, &[general], None).await;
    let twice = reorder(&world.owner, &[general, voice, general], None).await;
    let stranger = reorder(&world.owner, &[general, NOBODY], None).await;
    let empty = reorder(&world.owner, &[], None).await;

    for response in [&short, &twice, &stranger, &empty] {
        assert_eq!(response.status, 400, "{}", response.text());
        assert_eq!(error_of(response, "channelIds"), "The list must contain every channel exactly once.");
    }
    let missing = send(&world.owner, Method::PUT, "/api/admin/channels/order", Some(json!({}))).await;
    assert_eq!(missing.status, 400);
    assert_eq!(world.order(), ["general", "voice"], "nothing of a refused order is kept");
}

#[tokio::test]
async fn only_a_channel_manager_may_reorder_and_a_full_order_is_applied() {
    let world = World::new().await;
    let maya = world.member("maya").await;
    let (general, voice) = (world.channel("general"), world.channel("voice"));

    let forbidden = reorder(&maya, &[&voice, &general], None).await;
    let applied = reorder(&world.owner, &[&voice, &general], None).await;

    assert_eq!(forbidden.status, 403);
    assert_eq!(applied.status, 204);
    assert!(applied.body.is_empty());
    assert_eq!(world.order(), ["voice", "general"]);
    assert_eq!(world.positions(), [0, 1]);
    let audit = world.audit();
    assert_eq!(audit.last().unwrap().0, "channel.reorder");
    assert_eq!(audit.last().unwrap().2, "Owner reordered the channels");
}

#[tokio::test]
async fn an_order_that_changes_nothing_is_not_recorded() {
    let world = World::new().await;
    let (general, voice) = (world.channel("general"), world.channel("voice"));
    let before = world.audit().len();

    let response = reorder(&world.owner, &[&general, &voice], None).await;

    assert_eq!(response.status, 204);
    assert_eq!(world.audit().len(), before);
}

#[tokio::test]
async fn a_caller_who_sees_part_of_the_channels_orders_those_and_the_rest_stay_in_place() {
    let world = World::new().await;
    // general, voice, then c2 .. c5.
    let ids: Vec<String> = (2..6).map(|n| world.add_channel(&format!("c{n}"), 0, None)).collect();
    let hana = world.manager("hana").await;
    // Hana sees voice, c3 and c4: everything else is out of her sight.
    for hidden in [world.channel("general"), ids[0].clone(), ids[3].clone()] {
        world.hide(0, &hidden, &hana);
    }
    let (voice, c3, c4) = (world.channel("voice"), ids[1].clone(), ids[2].clone());

    let missing_one = reorder(&hana, &[&c4, &voice], None).await;
    let names_a_hidden_one = reorder(&hana, &[&c4, &voice, &c3, &ids[0]], None).await;
    let applied = reorder(&hana, &[&c4, &voice, &c3], None).await;

    assert_eq!(missing_one.status, 400);
    assert_eq!(names_a_hidden_one.status, 400, "a channel the caller does not see cannot be named");
    assert_eq!(applied.status, 204, "{}", applied.text());
    // Before: general voice c2 c3 c4 c5; Hana's places were voice, c3, c4.
    assert_eq!(world.order(), ["general", "c4", "c2", "voice", "c3", "c5"]);
    assert_eq!(world.positions(), [0, 1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn a_channel_dropped_into_a_section_changes_order_and_section_in_one_change() {
    let world = World::new().await;
    let alpha = world.add_section("Alpha");
    let (general, voice) = (world.channel("general"), world.channel("voice"));

    let response = reorder(&world.owner, &[&voice, &general], Some(json!({ "channelId": general, "sectionId": alpha }))).await;

    assert_eq!(response.status, 204, "{}", response.text());
    assert_eq!(world.layout(), [("voice".to_string(), None), ("general".to_string(), Some("Alpha".to_string()))]);
    let audit = world.audit();
    let (action, target, summary, changes) = audit.last().unwrap();
    assert_eq!(action, "channel.move");
    assert_eq!(target.as_deref(), Some("general"));
    assert_eq!(summary, "Owner moved #general to Alpha");
    assert_eq!(changes.as_deref(), Some("[{\"field\":\"section\",\"to\":\"Alpha\"}]"));
    assert_eq!(audit.iter().filter(|line| line.0 == "channel.reorder").count(), 0, "one line for the whole change");

    let out = reorder(&world.owner, &[&voice, &general], Some(json!({ "channelId": general, "sectionId": null }))).await;
    assert_eq!(out.status, 204);
    let audit = world.audit();
    assert_eq!(audit.last().unwrap().2, "Owner took #general out of its section");
    assert_eq!(audit.last().unwrap().3.as_deref(), Some("[{\"field\":\"section\",\"from\":\"Alpha\"}]"));
}

#[tokio::test]
async fn a_move_to_an_unknown_section_or_of_an_unknown_channel_or_one_not_seen_changes_nothing() {
    let world = World::new().await;
    let (general, voice) = (world.channel("general"), world.channel("voice"));
    let alpha = world.add_section("Alpha");
    let hana = world.manager("hana").await;
    world.hide(0, &voice, &hana);
    world.unlist(&alpha, &hana);

    let unknown_section = reorder(&world.owner, &[&voice, &general], Some(json!({ "channelId": general, "sectionId": NOBODY }))).await;
    let unknown_channel = reorder(&world.owner, &[&voice, &general], Some(json!({ "channelId": NOBODY, "sectionId": null }))).await;
    let no_channel = reorder(&world.owner, &[&voice, &general], Some(json!({ "sectionId": null }))).await;
    let unlisted = reorder(&hana, &[&general], Some(json!({ "channelId": general, "sectionId": alpha }))).await;
    let unseen = reorder(&hana, &[&general], Some(json!({ "channelId": voice, "sectionId": null }))).await;

    for response in [&unknown_section, &unknown_channel, &no_channel, &unlisted, &unseen] {
        assert_eq!(response.status, 404, "{}", response.text());
    }
    assert_eq!(world.layout(), [("general".to_string(), None), ("voice".to_string(), None)]);
}

#[tokio::test]
async fn a_move_the_caller_may_not_make_is_forbidden() {
    let world = World::new().await;
    let alpha = world.add_section("Alpha");
    let general = world.channel("general");
    let voice = world.channel("voice");
    // Hana manages channels, but not in Alpha, where her role may not.
    let hana = world.manager("hana").await;
    let role: String = world
        .db()
        .query_row("SELECT lower(\"RoleId\") FROM \"ServerMembers\" WHERE \"UserId\" = upper(?1)", [&hana.id], |row| row.get(0))
        .unwrap();
    world
        .db()
        .execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\")
             VALUES (?1, ?2, 1, upper(?3), upper(?4), 0, ?5)",
            params![stellarr_api::values::Id::random().to_db_text(), world.server_id(), alpha, role, MANAGE_CHANNELS],
        )
        .unwrap();

    let response = reorder(&hana, &[&general, &voice], Some(json!({ "channelId": general, "sectionId": alpha }))).await;

    assert_eq!(response.status, 403);
    assert_eq!(world.layout()[0], ("general".to_string(), None));
}

// What people are told.

fn audience_of(event: &Value) -> Value {
    event["audience"].clone()
}

#[tokio::test]
async fn where_nobody_is_denied_view_the_layout_goes_to_the_whole_server() {
    let world = World::new().await;
    let alpha = world.add_section("Alpha");
    let (general, voice) = (world.channel("general"), world.channel("voice"));
    let server = world.server_id().to_lowercase();

    let response = reorder(&world.owner, &[&voice, &general], Some(json!({ "channelId": voice, "sectionId": alpha }))).await;

    assert_eq!(response.status, 204);
    let layouts = world.events("ChannelLayoutChanged");
    assert_eq!(layouts.len(), 1);
    assert_eq!(audience_of(&layouts[0]), json!({ "server": server }));
    assert_eq!(
        layouts[0]["payload"],
        json!({ "serverId": server, "channels": [{ "id": voice, "sectionId": alpha }, { "id": general, "sectionId": null }] })
    );
    assert!(world.events("ChannelAccessChanged").is_empty(), "nobody saw something else");
}

#[tokio::test]
async fn where_somebody_is_denied_view_each_is_sent_the_layout_of_what_they_see() {
    let world = World::new().await;
    let hana = world.manager("hana").await;
    let maya = world.member("maya").await;
    let nils = world.member("nils").await;
    let secret = world.add_channel("secret", 0, None);
    world.hide(0, &secret, &hana);
    world.hide(0, &secret, &maya);
    let (general, voice) = (world.channel("general"), world.channel("voice"));
    let server = world.server_id().to_lowercase();

    let response = reorder(&hana, &[&voice, &general], None).await;

    assert_eq!(response.status, 204, "{}", response.text());
    let layouts = world.events("ChannelLayoutChanged");
    assert_eq!(layouts.len(), 2, "those who see the same share one event");
    let full = layouts.iter().find(|event| event["payload"]["channels"].as_array().unwrap().len() == 3).unwrap();
    let part = layouts.iter().find(|event| event["payload"]["channels"].as_array().unwrap().len() == 2).unwrap();
    let mut seeing_all = vec![world.owner.id.clone(), nils.id.clone()];
    seeing_all.sort();
    assert_eq!(audience_of(full), json!({ "followers": { "server": server, "users": seeing_all } }));
    let mut seeing_part = vec![hana.id.clone(), maya.id.clone()];
    seeing_part.sort();
    assert_eq!(audience_of(part), json!({ "followers": { "server": server, "users": seeing_part } }));
    assert_eq!(
        part["payload"]["channels"],
        json!([{ "id": voice, "sectionId": null }, { "id": general, "sectionId": null }]),
        "the channel they cannot see is not named to them"
    );
    assert_eq!(full["payload"]["channels"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn a_move_that_shows_or_hides_a_channel_tells_those_it_concerns_to_read_their_channels_again() {
    let world = World::new().await;
    let hana = world.manager("hana").await;
    let maya = world.member("maya").await;
    let nils = world.member("nils").await;
    // Bravo is hidden from Maya and Nils; Maya may see what sits outside.
    let bravo = world.add_section("Bravo");
    world.hide(1, &bravo, &maya);
    world.hide(1, &bravo, &nils);
    let (general, voice) = (world.channel("general"), world.channel("voice"));
    let server = world.server_id().to_lowercase();

    let response = reorder(&world.owner, &[&general, &voice], Some(json!({ "channelId": general, "sectionId": bravo }))).await;

    assert_eq!(response.status, 204, "{}", response.text());
    let access = world.events("ChannelAccessChanged");
    assert_eq!(access.len(), 1);
    let mut lost = vec![maya.id.clone(), nils.id.clone()];
    lost.sort();
    assert_eq!(audience_of(&access[0]), json!({ "followers": { "server": server, "users": lost } }));
    assert_eq!(access[0]["payload"], json!({ "serverId": server }));
    // Hana still sees both channels.
    let layouts = world.events("ChannelLayoutChanged");
    assert!(layouts.iter().any(|event| {
        event["audience"]["followers"]["users"].as_array().is_some_and(|users| users.iter().any(|user| *user == hana.id.as_str()))
    }));
    // The event named first is the one about access.
    let names: Vec<String> = world
        .server
        .events()
        .iter()
        .map(|event| event["name"].as_str().unwrap().to_string())
        .filter(|name| name.starts_with("Channel"))
        .collect();
    assert_eq!(names[..2], ["ChannelAccessChanged", "ChannelLayoutChanged"]);
}

#[tokio::test]
async fn a_new_section_order_is_announced_to_those_it_is_listed_for() {
    let world = World::new().await;
    let first = world.add_section("Pierwsza");
    let second = world.add_section("Druga");
    let server = world.server_id().to_lowercase();

    let response = send(&world.owner, Method::PUT, "/api/admin/sections/order", Some(json!({ "sectionIds": [second, first] }))).await;

    assert_eq!(response.status, 204, "{}", response.text());
    assert_eq!(world.section_order(), ["Druga", "Pierwsza"]);
    let events = world.events("SectionOrderChanged");
    assert_eq!(events.len(), 1);
    assert_eq!(audience_of(&events[0]), json!({ "server": server }));
    assert_eq!(events[0]["payload"], json!({ "serverId": server, "sectionIds": [second, first] }));
    assert_eq!(world.audit().last().unwrap().2, "Owner reordered the sections");
}

#[tokio::test]
async fn a_section_order_names_the_sections_the_caller_sees_and_leaves_the_others_in_place() {
    let world = World::new().await;
    let a = world.add_section("A");
    let b = world.add_section("B");
    let c = world.add_section("C");
    let d = world.add_section("D");
    let hana = world.manager("hana").await;
    // B and D hold a channel Hana may not see, and she may not manage them.
    for (name, id) in [("B", &b), ("D", &d)] {
        let channel = world.add_channel(&format!("in-{name}"), 0, Some(id));
        world.hide(0, &channel, &hana);
        world.unlist(id, &hana);
    }
    let maya = world.member("maya").await;

    let short = send(&hana, Method::PUT, "/api/admin/sections/order", Some(json!({ "sectionIds": [c, a] }))).await;
    let all = send(&hana, Method::PUT, "/api/admin/sections/order", Some(json!({ "sectionIds": [a, b, c, d] }))).await;
    let forbidden = send(&maya, Method::PUT, "/api/admin/sections/order", Some(json!({ "sectionIds": [a, b, c, d] }))).await;
    let applied = send(&hana, Method::PUT, "/api/admin/sections/order", Some(json!({ "sectionIds": [c, a] }))).await;

    // Hana is told of A and C, which are empty, and not of B and D.
    assert_eq!(short.status, 204, "{}", short.text());
    assert_eq!(all.status, 400);
    assert_eq!(error_of(&all, "sectionIds"), "The list must contain every section exactly once.");
    assert_eq!(forbidden.status, 403);
    assert_eq!(applied.status, 204);
    assert_eq!(world.section_order(), ["C", "B", "A", "D"]);
}

// How far a person has read.

#[tokio::test]
async fn reading_a_channel_moves_the_marker_forward_and_tells_the_persons_own_connections() {
    let world = World::new().await;
    let general = world.channel("general");

    let response = send(&world.owner, Method::PUT, &format!("/api/channels/{general}/read"), None).await;

    assert_eq!(response.status, 200, "{}", response.text());
    let body: Value = response.json();
    assert_eq!(body["channelId"], general.as_str());
    assert_eq!(response.text().split(',').next().unwrap(), format!("{{\"channelId\":\"{general}\""));
    assert!(body["lastReadAt"].as_str().unwrap().ends_with('Z'));
    let events = world.events("ChannelReadChanged");
    assert_eq!(events.len(), 1);
    assert_eq!(audience_of(&events[0]), json!({ "users": [world.owner.id] }));
    assert_eq!(events[0]["payload"], body);
    let stored: String = world.db().query_row("SELECT \"LastReadAt\" FROM \"ChannelReads\"", [], |row| row.get(0)).unwrap();
    assert!(!stored.contains('T') && !stored.ends_with('Z'), "stored as the first server wrote it: {stored}");
}

#[tokio::test]
async fn a_slower_request_never_moves_the_marker_back() {
    let world = World::new().await;
    let general = world.channel("general");
    world
        .db()
        .execute(
            "INSERT INTO \"ChannelReads\" (\"ChannelId\", \"UserId\", \"LastReadAt\") VALUES (upper(?1), upper(?2), '2099-01-01 00:00:00')",
            params![general, world.owner.id],
        )
        .unwrap();

    let response = send(&world.owner, Method::PUT, &format!("/api/channels/{general}/read"), None).await;

    assert_eq!(response.status, 200);
    let stored: String = world.db().query_row("SELECT \"LastReadAt\" FROM \"ChannelReads\"", [], |row| row.get(0)).unwrap();
    assert_eq!(stored, "2099-01-01 00:00:00");
    assert!(response.json::<Value>()["lastReadAt"].as_str().unwrap() < "2099", "the answer is the time that was offered");
}

#[tokio::test]
async fn a_channel_that_is_voice_unseen_or_unknown_cannot_be_read() {
    let world = World::new().await;
    let voice = world.channel("voice");
    let secret = world.add_channel("secret", 0, None);
    let maya = world.member("maya").await;
    world.hide(0, &secret, &maya);

    for (person, channel) in [(&world.owner, voice.as_str()), (&maya, secret.as_str()), (&world.owner, NOBODY)] {
        let response = send(person, Method::PUT, &format!("/api/channels/{channel}/read"), None).await;
        assert_eq!(response.status, 404, "{channel}");
        assert!(response.body.is_empty());
    }
    let anonymous = world.server.anonymous().send(Request::new(Method::PUT, &format!("/api/channels/{voice}/read"))).await;
    assert_eq!(anonymous.status, 401);
    assert!(world.events("ChannelReadChanged").is_empty());
}

// The hub.

async fn connect(world: &World, person: &Person) -> HubClient {
    let cookie = format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"));
    let mut client = HubClient::connect(world.server.address(), &cookie).await;
    // What the hub says about who is online is not what these tests are about.
    client.quiet_for(QUIET).await;
    client
}

fn error_text(completion: &Value) -> Option<&str> {
    completion["error"].as_str()
}

#[tokio::test]
async fn a_channel_is_joined_by_whoever_sees_it_and_only_a_text_channel() {
    let world = World::new().await;
    let secret = world.add_channel("secret", 0, None);
    let maya = world.member("maya").await;
    world.hide(0, &secret, &maya);
    let (general, voice) = (world.channel("general"), world.channel("voice"));
    let mut client = connect(&world, &maya).await;

    let seen = client.invoke("JoinChannel", json!([general])).await;
    let hidden = client.invoke("JoinChannel", json!([secret])).await;
    let call = client.invoke("JoinChannel", json!([voice])).await;
    let unknown = client.invoke("JoinChannel", json!([NOBODY])).await;
    let not_an_id = client.invoke("JoinChannel", json!(["nope"])).await;
    let nothing = client.invoke("JoinChannel", json!([null])).await;
    let two = client.invoke("JoinChannel", json!([general, general])).await;

    assert!(seen.get("error").is_none(), "{seen}");
    assert_eq!(seen["result"], Value::Null);
    for completion in [&hidden, &call, &unknown] {
        assert_eq!(
            error_text(completion),
            Some("An unexpected error occurred invoking 'JoinChannel' on the server. HubException: Unknown channel.")
        );
    }
    for completion in [&not_an_id, &nothing] {
        assert_eq!(
            error_text(completion),
            Some("An unexpected error occurred invoking 'JoinChannel' on the server. HubException: Invalid channelId.")
        );
    }
    assert_eq!(error_text(&two), Some("Failed to invoke 'JoinChannel' due to an error on the server."));
}

fn typing_events(records: &[Value]) -> Vec<&Value> {
    records.iter().filter(|record| record["target"] == "TypingChanged").collect()
}

#[tokio::test]
async fn typing_reaches_the_others_who_see_the_channel_and_the_senders_other_tabs() {
    let world = World::new().await;
    let maya = world.member("maya").await;
    let nils = world.member("nils").await;
    let secret = world.add_channel("secret", 0, None);
    world.hide(0, &secret, &nils);
    let mut owner_tab = connect(&world, &world.owner).await;
    let owner_again = world.server.sign_in(&world.owner).await;
    let mut other_tab = HubClient::connect(
        world.server.address(),
        &format!("{COOKIE}={}", owner_again.cookie(COOKIE).unwrap()),
    )
    .await;
    other_tab.quiet_for(QUIET).await;
    let mut maya_client = connect(&world, &maya).await;
    let mut nils_client = connect(&world, &nils).await;

    let completion = owner_tab.invoke("SendTyping", json!([secret])).await;

    assert!(completion.get("error").is_none(), "{completion}");
    let to_maya = maya_client.quiet_for(QUIET).await;
    let to_nils = nils_client.quiet_for(QUIET).await;
    let to_other_tab = other_tab.quiet_for(QUIET).await;
    let to_sender = owner_tab.quiet_for(QUIET).await;
    let heard = typing_events(&to_maya);
    assert_eq!(heard.len(), 1, "{to_maya:?}");
    let notice = &heard[0]["arguments"][0];
    assert_eq!(notice["channelId"], secret.as_str());
    assert_eq!(notice["userId"], world.owner.id.as_str());
    assert_eq!(notice["displayName"], "Owner");
    assert!(notice["expiresAt"].as_str().unwrap().ends_with('Z'));
    assert!(typing_events(&to_nils).is_empty(), "somebody who does not see the channel is not told");
    assert_eq!(typing_events(&to_other_tab).len(), 1, "the sender's other tab hears it");
    assert!(typing_events(&to_sender).is_empty(), "the connection that typed does not");
}

#[tokio::test]
async fn typing_on_an_open_channel_reaches_the_whole_server_but_the_sender() {
    let world = World::new().await;
    let maya = world.member("maya").await;
    let general = world.channel("general");
    let mut owner = connect(&world, &world.owner).await;
    let mut maya_client = connect(&world, &maya).await;

    let completion = maya_client.invoke("SendTyping", json!([general])).await;

    assert!(completion.get("error").is_none(), "{completion}");
    assert_eq!(typing_events(&owner.quiet_for(QUIET).await).len(), 1);
    assert!(typing_events(&maya_client.quiet_for(QUIET).await).is_empty());
}

#[tokio::test]
async fn typing_needs_the_channel_to_be_seen_and_the_right_to_send_there() {
    let world = World::new().await;
    let maya = world.member("maya").await;
    let nils = world.member("nils").await;
    let secret = world.add_channel("secret", 0, None);
    let announcements = world.add_channel("announcements", 0, None);
    world.hide(0, &secret, &maya);
    world.member_override(0, &announcements, &nils, 0, SEND_MESSAGES);
    let (voice, general) = (world.channel("voice"), world.channel("general"));
    let mut maya_client = connect(&world, &maya).await;
    let mut nils_client = connect(&world, &nils).await;

    let unseen = maya_client.invoke("SendTyping", json!([secret])).await;
    let call = maya_client.invoke("SendTyping", json!([voice])).await;
    let unknown = maya_client.invoke("SendTyping", json!([NOBODY])).await;
    let mute = nils_client.invoke("SendTyping", json!([announcements])).await;
    let not_an_id = nils_client.invoke("SendTyping", json!(["nope"])).await;
    let fine = nils_client.invoke("SendTyping", json!([general])).await;

    for completion in [&unseen, &call, &unknown] {
        assert_eq!(
            error_text(completion),
            Some("An unexpected error occurred invoking 'SendTyping' on the server. HubException: Unknown channel.")
        );
    }
    assert_eq!(
        error_text(&mute),
        Some("An unexpected error occurred invoking 'SendTyping' on the server. HubException: Not allowed to send messages here.")
    );
    assert_eq!(
        error_text(&not_an_id),
        Some("An unexpected error occurred invoking 'SendTyping' on the server. HubException: Invalid channelId.")
    );
    assert!(fine.get("error").is_none());
}

#[tokio::test]
async fn typing_is_throttled_for_each_connection() {
    let world = World::new().await;
    let general = world.channel("general");
    let mut owner = connect(&world, &world.owner).await;

    let mut answers = Vec::new();
    for _ in 0..14 {
        answers.push(owner.invoke("SendTyping", json!([general])).await);
    }

    let refused: Vec<_> = answers.iter().filter_map(error_text).collect();
    assert!(!refused.is_empty(), "a burst of typing is stopped");
    assert!(answers[0].get("error").is_none());
    assert!(refused.iter().all(|text| text.ends_with("HubException: Typing is too fast.")), "{refused:?}");
}
