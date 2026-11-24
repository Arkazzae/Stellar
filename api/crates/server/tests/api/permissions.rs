//! What the permission rules do inside a route, and who hears an event about
//! a private channel.

mod routes {
    use axum::extract::State;
    use http::Method;
    use rusqlite::params;
    use serde_json::{Value, json};
    use stellarr_api::auth::Caller;
    use stellarr_api::http::{ApiError, ApiResult, Endpoints, Path};
    use stellarr_api::permissions::authorization::{Scope, held_at};
    use stellarr_api::permissions::membership::{ActiveServer, InstanceOwner};
    use stellarr_api::permissions::privileges::{SEND_MESSAGES, CREATE_INVITE};
    use stellarr_api::state::AppState;
    use stellarr_api::values::Id;
    use stellarr_messages::MessageKey;

    use crate::support::{LogCapture, Person, Request, Server};

    fn routes() -> Endpoints {
        Endpoints::new()
            .get("/api/test/active-server", active_server)
            .authorized()
            .get("/api/test/instance", instance)
            .authorized()
            .get("/api/test/channels/{id}/send", send)
            .authorized()
            .get("/api/test/invite", invite)
            .authorized()
            .merge(stellarr_api::accounts::routes())
    }

    async fn active_server(_: Caller, ActiveServer(server): ActiveServer) -> String {
        server.to_string()
    }

    async fn instance(_: Caller, InstanceOwner(server): InstanceOwner) -> String {
        server.to_string()
    }

    /// What a route for a channel does: it is not there for who does not see
    /// it, and a missing permission is the problem that names it.
    async fn send(
        State(state): State<AppState>,
        caller: Caller,
        ActiveServer(server): ActiveServer,
        Path(channel): Path<Id>,
    ) -> ApiResult<String> {
        let held = state
            .database()
            .read(move |db| held_at(db, caller.user, &Scope::channel(server, channel)))
            .await?;
        match held.require(SEND_MESSAGES) {
            Ok(_) => Ok("sent".to_string()),
            Err(refusal) => Err(refusal
                .into_error(state.messages(), MessageKey::SendMessagesDenied, ApiError::not_found())?),
        }
    }

    async fn invite(State(state): State<AppState>, caller: Caller, ActiveServer(server): ActiveServer) -> ApiResult<String> {
        let held = state
            .database()
            .read(move |db| held_at(db, caller.user, &Scope::server(server)))
            .await?;
        match held.has(CREATE_INVITE) {
            true => Ok("made".to_string()),
            false => Err(stellarr_api::permissions::authorization::Refusal::Denied { permission: CREATE_INVITE }
                .into_error(state.messages(), MessageKey::CreateInviteDenied, ApiError::not_found())?),
        }
    }

    struct World {
        server: Server,
        owner: Person,
        jozef: Person,
        stranger: Person,
        general: String,
        hidden: String,
        _log: LogCapture,
    }

    async fn world() -> World {
        let log = LogCapture::start();
        let server = Server::in_process(routes(), &[]).await;
        let mut code = String::new();
        for _ in 0..100 {
            code = log
                .lines()
                .iter()
                .find_map(|line| line.split_once("Setup code: ").map(|(_, rest)| rest.split('.').next().unwrap_or("").to_string()))
                .unwrap_or_default();
            if !code.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let client = server.client();
        let response = client
            .post_json(
                "/api/auth/register",
                &json!({"username": "boss", "displayName": "Boss", "email": "boss@example.test",
                        "password": crate::support::PASSWORD, "setupCode": code}),
            )
            .await;
        assert_eq!(response.status.as_u16(), 201, "{}", response.text());
        let owner = Person { username: "boss".into(), id: response.json::<Value>()["id"].as_str().unwrap().into(), client };
        let jozef = server.register("jozef").await;
        let stranger = server.register("stranger").await;
        let database = server.database();
        let instance: String = database.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let channels = |name: &str| -> String {
            database.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Name\" = ?1", [name], |row| row.get(0)).unwrap()
        };
        let general = channels("general");
        // A private channel: view denied to the base role.
        let hidden = Id::random();
        database
            .execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'secret', 0, 9, '', 0, 0, 0, NULL)",
                params![hidden.to_db_text(), instance],
            )
            .unwrap();
        let base: String = database
            .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
            .unwrap();
        database
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
                params![Id::random().to_db_text(), instance, hidden.to_db_text(), base],
            )
            .unwrap();
        // The stranger leaves the server.
        database
            .execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [stranger.id.to_uppercase()])
            .unwrap();
        World { server, owner, jozef, stranger, general, hidden: hidden.to_string(), _log: log }
    }

    #[tokio::test]
    async fn active_server_routes_require_access_to_the_selected_server() {
        let w = world().await;
        let instance: String = w.server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let instance = Id::parse(&instance).unwrap().to_string();
        let named = |value: &str| Request::new(Method::GET, "/api/test/active-server").header("X-Stellarr-Server", value);

        let ok = w.jozef.client.get("/api/test/active-server").await;
        assert_eq!((ok.status.as_u16(), ok.text()), (200, instance.clone()));
        for value in [instance.clone(), instance.to_uppercase()] {
            let response = w.jozef.client.send(named(&value)).await;
            assert_eq!((response.status.as_u16(), response.text()), (200, instance.clone()), "{value}");
        }
        for value in [instance.replace('-', ""), format!("{{{}}}", instance.to_uppercase()), format!("({instance})")] {
            let response = w.jozef.client.send(named(&value)).await;
            assert_eq!((response.status.as_u16(), response.body.len()), (400, 0), "{value}");
        }
        let query = w.jozef.client.get(&format!("/api/test/active-server?SERVERID={instance}")).await;
        assert_eq!(query.status.as_u16(), 200);
        // A name that is no id is a 400 and an id of no server of theirs a 403, both with no body.
        let bad = w.jozef.client.send(named("not-an-id")).await;
        assert_eq!((bad.status.as_u16(), bad.body.len()), (400, 0));
        let other = w.jozef.client.send(named("11111111-1111-1111-1111-111111111111")).await;
        assert_eq!((other.status.as_u16(), other.body.len()), (403, 0));
        // A header that is there but empty names nobody, and the query is not asked.
        let empty = w.jozef.client.send(named("").header("X-Unused", "1")).await;
        assert_eq!(empty.status.as_u16(), 200);
        // Somebody who left is a stranger to every server; one who is not signed in is not asked.
        let left = w.stranger.client.get("/api/test/active-server").await;
        assert_eq!((left.status.as_u16(), left.body.len()), (403, 0));
        let nobody = w.server.anonymous().get("/api/test/active-server").await;
        assert_eq!(nobody.status.as_u16(), 401);
    }

    #[tokio::test]
    async fn a_refusal_names_the_permission_in_a_403_problem_and_a_hidden_channel_is_not_there() {
        let w = world().await;

        let allowed = w.jozef.client.get(&format!("/api/test/channels/{}/send", w.general)).await;
        assert_eq!((allowed.status.as_u16(), allowed.text()), (200, "sent".to_string()));
        // These routes return the same problem response apart from the trace ID.
        let denied = w.jozef.client.get("/api/test/invite").await;
        assert_eq!(denied.status.as_u16(), 403);
        assert_eq!(denied.header("content-type"), Some("application/problem+json"));
        let mut body: Value = denied.json();
        assert!(body.as_object_mut().unwrap().remove("traceId").is_some());
        assert_eq!(
            body,
            json!({"type": "https://tools.ietf.org/html/rfc9110#section-15.5.4",
                   "title": "You may not create invites here.", "status": 403, "permission": "createInvite"})
        );
        assert!(denied.text().starts_with("{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.4\",\"title\":\"You may not create invites here.\",\"status\":403,\"permission\":\"createInvite\",\"traceId\":"));
        // The owner holds it, and everything in the hidden channel too.
        assert_eq!(w.owner.client.get("/api/test/invite").await.status, 200);
        assert_eq!(w.owner.client.get(&format!("/api/test/channels/{}/send", w.hidden)).await.status, 200);
        // For somebody who does not see the channel it is the route's own answer for an id that does not exist.
        let hidden = w.jozef.client.get(&format!("/api/test/channels/{}/send", w.hidden)).await;
        let unknown = w.jozef.client.get(&format!("/api/test/channels/{}/send", Id::random())).await;
        assert_eq!((hidden.status.as_u16(), hidden.body.len()), (404, 0));
        assert_eq!((unknown.status.as_u16(), unknown.body.len()), (404, 0));
    }

    #[tokio::test]
    async fn a_channel_where_sending_is_denied_but_view_is_held_says_which_permission_is_missing() {
        let w = world().await;
        let database = w.server.database();
        let base: String = database
            .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
            .unwrap();
        let server: String = database.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        database
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 512)",
                params![Id::random().to_db_text(), server, w.general, base],
            )
            .unwrap();

        let denied = w.jozef.client.get(&format!("/api/test/channels/{}/send", w.general)).await;

        assert_eq!(denied.status.as_u16(), 403);
        assert_eq!(denied.json::<Value>()["permission"], "sendMessages");
        assert_eq!(denied.json::<Value>()["title"], "You may not send messages in this channel.");
    }

    #[tokio::test]
    async fn the_instance_panel_is_for_the_owner_of_the_instance_alone_and_says_when_only_a_second_step_is_missing() {
        let w = world().await;

        let owner = w.owner.client.get("/api/test/instance").await;
        let jozef = w.jozef.client.get("/api/test/instance").await;
        assert_eq!(owner.status.as_u16(), 200);
        assert_eq!((jozef.status.as_u16(), jozef.body.len()), (403, 0));

        // Where a second step is asked for, the session itself has to have passed one.
        let instance: String = w.server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        w.server
            .database()
            .execute(
                "INSERT INTO \"OperationsSettings\" (\"ServerId\",\"PublicUrl\",\"TimeZone\",\"MailingEnabled\",\"MailFromAddress\",\"InviteMailEnabled\",\"RecoveryMailEnabled\",\"AlertMailEnabled\",\"BackupRetentionDays\",\"LogRetentionDays\",\"OrphanObjectRetentionDays\",\"AutomaticCleanupEnabled\",\"UpdatedAt\",\"MfaRequirement\") VALUES (?1,'','UTC',0,'',0,0,0,7,7,7,0,'2026-11-24 00:00:00','cockpit')",
                params![instance],
            )
            .unwrap();
        let owner = w.owner.client.get("/api/test/instance").await;
        assert_eq!(owner.status.as_u16(), 403);
        assert_eq!(owner.header("content-type"), Some("application/problem+json"));
        assert_eq!(owner.json::<Value>()["code"], "cockpit-mfa-required");
        // Somebody who is not the owner is not told about second steps.
        let jozef = w.jozef.client.get("/api/test/instance").await;
        assert_eq!((jozef.status.as_u16(), jozef.body.len()), (403, 0));
        // Once the session has passed one, it opens.
        w.server
            .database()
            .execute("UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = ?1 WHERE \"UserId\" = ?2", params!["2026-11-24 00:00:00", w.owner.id.to_uppercase()])
            .unwrap();
        assert_eq!(w.owner.client.get("/api/test/instance").await.status, 200);
    }
}

// ---- Who hears an event about a private channel

mod audience {
    use std::time::Duration;

    use rusqlite::params;
    use serde_json::Value;
    use stellarr_api::values::{Id, Timestamp};

    use crate::support::{HubClient, Person, Server};

    fn cookie_of(person: &Person) -> String {
        format!("stellarr-dev-session={}", person.client.cookie("stellarr-dev-session").expect("signed in"))
    }

    fn messages(heard: &[Value]) -> Vec<&Value> {
        heard.iter().filter(|record| record["name"] == "MessageCreated").collect()
    }

    #[tokio::test]
    async fn an_event_about_a_private_channel_reaches_a_viewers_connection_on_that_server_and_nobody_elses() {
        let server = Server::start().await;
        let owner = server.found("owner").await;
        let maya = server.register("maya").await;
        let noor = server.register("noor").await;
        let database = server.database();
        let instance: String = database.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        // The first text channel is private to one person: noor's own override denies it.
        let channel: String = database
            .query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0))
            .unwrap();
        database
            .execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
                params![Id::random().to_db_text(), instance, channel, noor.id.to_uppercase()],
            )
            .unwrap();
        // Maya is on a second server as well, and has a tab open on it.
        let other = Id::random();
        database.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        database
            .execute(
                "INSERT INTO \"ServerMembers\" VALUES (?1, ?2, (SELECT \"RoleId\" FROM \"ServerMembers\" LIMIT 1), NULL, ?3)",
                params![other.to_db_text(), maya.id.to_uppercase(), Timestamp::now()],
            )
            .unwrap();

        let cookie_maya = cookie_of(&maya);
        let mut on_the_server = HubClient::connect_to(server.address(), "", &cookie_maya).await;
        let mut on_the_other = HubClient::connect_to(server.address(), &format!("?serverId={other}"), &cookie_maya).await;
        let mut denied = HubClient::connect_to(server.address(), "", &cookie_of(&noor)).await;
        for client in [&mut on_the_server, &mut on_the_other, &mut denied] {
            client.quiet_for(Duration::from_millis(300)).await;
        }

        // A newcomer is welcomed in the channel, which only some of the members see.
        server.register("zed").await;

        let seen = on_the_server.quiet_for(Duration::from_millis(500)).await;
        let elsewhere = on_the_other.quiet_for(Duration::from_millis(500)).await;
        let refused = denied.quiet_for(Duration::from_millis(500)).await;
        let welcome = messages(&seen);
        assert_eq!(welcome.len(), 1, "{seen:?}");
        assert_eq!(welcome[0]["data"]["channelId"], channel.to_lowercase());
        assert!(messages(&elsewhere).is_empty(), "{elsewhere:?}");
        assert!(messages(&refused).is_empty(), "{refused:?}");
        let _ = owner;
    }
}
