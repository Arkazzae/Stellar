//! The hub, through the real binary: who is let in, what a connection is told
//! when it starts and ends, the methods that need nobody else, and who hears
//! about a status.
//!
//! Connections are raw sockets that speak the hub protocol, signed in with
//! the cookie of an account that logged in over HTTP.

use std::time::Duration;

use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{HubClient, PASSWORD, Person, Server, upgrade, upgrade_with};

const COOKIE: &str = "stellarr-dev-session";
const QUIET: Duration = Duration::from_millis(400);

/// The cookie header of a person's sign-in.
fn cookie_of(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

async fn connect(server: &Server, person: &Person) -> HubClient {
    HubClient::connect(server.address(), &cookie_of(person)).await
}

#[tokio::test]
async fn native_hub_calls_and_fans_out_to_other_tabs_until_logout() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let cookie = cookie_of(&owner);
    let mut actor = HubClient::connect(server.address(), &cookie).await;
    let mut observer = HubClient::connect(server.address(), &cookie).await;
    let online = json!({"userId": owner.id, "presence": "online"});
    assert_eq!(
        observer.record().await,
        json!({"type":"event", "name":"PresenceChanged", "data":online})
    );

    let before = Timestamp::now().unix_millis();
    let clock = actor.invoke("GetRtcClock", json!([])).await;
    assert_eq!(clock["type"], "result");
    let clock = clock["value"].as_i64().expect("milliseconds");
    assert!((before..=Timestamp::now().unix_millis()).contains(&clock));

    let rejected = actor.invoke("GetRtcClock", json!([0])).await;
    assert_eq!(rejected["type"], "error");
    assert_eq!(rejected["code"], "invalid_arguments");

    assert_eq!(
        actor.invoke("SetPresence", json!(["dnd"])).await,
        json!({"type":"result", "id":3, "value":null})
    );
    let dnd = json!({"userId": owner.id, "presence": "dnd"});
    assert_eq!(
        observer.record().await,
        json!({"type":"event", "name":"PresenceChanged", "data":dnd})
    );

    assert_eq!(
        owner
            .client
            .post_json("/api/auth/logout", &json!({}))
            .await
            .status,
        204
    );
    assert_eq!(actor.finish_close().await, Some(4401));
    assert_eq!(observer.finish_close().await, Some(4401));

    let host = server.address().to_string();
    let origin = format!("http://{host}");
    let (refused, _) = upgrade_with(
        server.address(),
        "",
        Some(&cookie),
        &host,
        &[
            ("Origin", &origin),
            ("Sec-WebSocket-Protocol", "stellarr.v1"),
        ],
    )
    .await;
    assert_eq!(refused.status, 401, "{}", refused.head);
}

#[tokio::test]
async fn native_voice_signaling_opens_a_media_session_for_a_seated_caller() {
    let rig = crate::voice::Rig::start().await;
    let maya = rig.person("maya").await;
    let mut observer = rig.connect(&maya).await;
    let mut actor = HubClient::connect(rig.server.address(), &cookie_of(&rig.owner)).await;

    let denied = actor
        .invoke("OpenMediaSession", json!([rig.salon]))
        .await;
    assert_eq!(denied["type"], "error");
    assert_eq!(denied["code"], "visible");
    assert_eq!(denied["message"], "Not seated in this voice room.");
    assert_eq!(rig.media.count("POST", "/v1/sessions"), 0);

    let joined = actor.invoke("JoinVoice", json!([rig.salon])).await;
    assert_eq!(joined["type"], "result", "{joined}");
    let snapshot = observer.next("VoiceParticipantChanged").await;
    assert_eq!(snapshot["channelId"], json!(rig.salon));
    assert_eq!(snapshot["participants"][0]["userId"], rig.owner.id);

    let opened = actor
        .invoke("OpenMediaSession", json!([rig.salon]))
        .await;
    assert_eq!(opened["type"], "result", "{opened}");
    let session = opened["value"]["sessionId"]
        .as_str()
        .expect("media session id");
    assert_eq!(
        rig.media.session_of(session),
        Some((rig.salon.to_string(), rig.owner.id.clone()))
    );
}

#[tokio::test]
async fn native_hub_checks_origin_and_keeps_the_public_port() {
    let server =
        Server::start_with(&[("Desktop__CanonicalOrigin", "https://chat.example.test:8443")]).await;
    let owner = server.found("owner").await;
    let cookie = cookie_of(&owner);
    let peer = server.address().to_string();
    let cases = [
        (
            Some(cookie.as_str()),
            "preview.test:4180",
            Some("http://preview.test:4180"),
            None,
            101,
        ),
        (
            Some(cookie.as_str()),
            "preview.test:4180",
            Some("http://preview.test"),
            None,
            403,
        ),
        (
            Some(cookie.as_str()),
            "tls.test:9443",
            Some("https://tls.test:9443"),
            Some("https"),
            101,
        ),
        (
            Some(cookie.as_str()),
            peer.as_str(),
            Some("https://chat.example.test:8443"),
            Some("https"),
            101,
        ),
        (
            Some(cookie.as_str()),
            peer.as_str(),
            Some("https://chat.example.test"),
            Some("https"),
            403,
        ),
        (
            Some(cookie.as_str()),
            peer.as_str(),
            Some("http://evil.example.test"),
            None,
            403,
        ),
        (
            Some(cookie.as_str()),
            "preview.test:4180",
            Some("http://preview.test:4180/path"),
            None,
            403,
        ),
        (
            Some(cookie.as_str()),
            peer.as_str(),
            Some("null"),
            None,
            403,
        ),
        (Some(cookie.as_str()), peer.as_str(), None, None, 403),
        (
            None,
            "preview.test:4180",
            Some("http://preview.test:4180"),
            None,
            401,
        ),
    ];
    for (cookie, host, origin, scheme, expected) in cases {
        let mut headers = vec![
            ("Sec-WebSocket-Protocol", "stellarr.v1"),
            ("X-Forwarded-Host", "evil.example.test"),
        ];
        headers.extend(origin.map(|origin| ("Origin", origin)));
        headers.extend(scheme.map(|scheme| ("X-Forwarded-Proto", scheme)));
        let (answer, _) = upgrade_with(server.address(), "", cookie, host, &headers).await;
        assert_eq!(answer.status, expected, "{origin:?}: {}", answer.head);
    }

    let (duplicate, _) = upgrade_with(
        server.address(),
        "",
        Some(&cookie),
        "preview.test:4180",
        &[
            ("Sec-WebSocket-Protocol", "stellarr.v1"),
            ("Origin", "http://preview.test:4180"),
            ("Origin", "http://evil.example.test"),
        ],
    )
    .await;
    assert_eq!(duplicate.status, 403, "{}", duplicate.head);
}

/// What a `PresenceChanged` says, as (account, status).
fn presence(event: &Value) -> (String, String) {
    (
        event["userId"].as_str().expect("an account").to_string(),
        event["presence"].as_str().expect("a status").to_string(),
    )
}

fn is_presence_of(record: &Value, person: &Person, status: &str) -> bool {
    record["name"] == "PresenceChanged"
        && record["data"]["userId"] == person.id.as_str()
        && record["data"]["presence"] == status
}

/// Two people on the instance's server.
async fn two_people() -> (Server, Person, Person) {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    (server, owner, maya)
}

/// Consumes the startup state up to a call's completion.
async fn settle(client: &mut HubClient, who: &Person) {
    assert_eq!(client.invoke("GetRtcClock", json!([])).await["type"], "result");
    let seen = client.quiet_for(Duration::ZERO).await;
    let own = seen.iter().rfind(|event| {
        event["name"] == "PresenceChanged" && event["data"]["userId"] == who.id.as_str()
    });
    assert!(own.is_some_and(|event| is_presence_of(event, who, "online")), "{seen:?}");
}

#[tokio::test]
async fn a_connecting_account_is_told_its_own_status() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    let expected = json!({"userId": owner.id, "presence": "online"});
    assert_eq!(client.event("PresenceChanged").await, expected);
    let other_startup_records = client.quiet_for(QUIET).await;
    assert!(other_startup_records.iter().all(|record| is_presence_of(record, &owner, "online")), "{other_startup_records:?}");
}

#[tokio::test]
async fn others_see_an_account_come_online_and_go_offline() {
    let (server, owner, maya) = two_people().await;
    let mut observer = connect(&server, &owner).await;
    settle(&mut observer, &owner).await;

    let actor = connect(&server, &maya).await;
    let seen = observer.event("PresenceChanged").await;
    assert_eq!(presence(&seen), (maya.id.clone(), "online".to_string()));

    actor.hang_up().await;
    let seen = observer.event("PresenceChanged").await;
    assert_eq!(presence(&seen), (maya.id.clone(), "offline".to_string()));
    assert!(observer.quiet_for(QUIET).await.is_empty());
}

#[tokio::test]
async fn another_connection_of_an_online_account_is_not_announced_again() {
    let (server, owner, maya) = two_people().await;
    let mut observer = connect(&server, &owner).await;
    settle(&mut observer, &owner).await;
    let mut first = connect(&server, &maya).await;
    settle(&mut first, &maya).await;
    observer.event("PresenceChanged").await;

    let mut second = connect(&server, &maya).await;
    // The second tab is told its own status, once: the account did not just
    // come online, so no group is told, and nobody else hears a thing.
    let told = second.event("PresenceChanged").await;
    assert_eq!(presence(&told), (maya.id.clone(), "online".to_string()));
    assert!(second.quiet_for(QUIET).await.is_empty());
    assert!(observer.quiet_for(QUIET).await.is_empty());

    // Going away is not going offline while another tab is open.
    second.hang_up().await;
    assert!(observer.quiet_for(QUIET).await.is_empty());
    first.hang_up().await;
    let seen = observer.event("PresenceChanged").await;
    assert_eq!(presence(&seen), (maya.id.clone(), "offline".to_string()));
}

#[tokio::test]
async fn a_chosen_status_reaches_other_members_and_the_caller_and_is_remembered() {
    let (server, owner, maya) = two_people().await;
    let mut observer = connect(&server, &owner).await;
    settle(&mut observer, &owner).await;
    let mut actor = connect(&server, &maya).await;
    settle(&mut actor, &maya).await;
    observer.event("PresenceChanged").await;

    let answer = actor.invoke("SetPresence", json!(["dnd"])).await;

    assert_eq!(answer["type"], "result", "{answer}");
    assert_eq!(presence(&observer.event("PresenceChanged").await), (maya.id.clone(), "dnd".to_string()));
    let to_caller = actor.quiet_for(Duration::from_millis(50)).await;
    assert!(to_caller.iter().any(|event| is_presence_of(event, &maya, "dnd")), "{to_caller:?}");
    assert!(observer.quiet_for(QUIET).await.is_empty());

    // Choosing what is already chosen says nothing to anybody.
    actor.invoke("SetPresence", json!(["dnd"])).await;
    assert!(actor.quiet_for(QUIET).await.is_empty());
    assert!(observer.quiet_for(Duration::from_millis(50)).await.is_empty());

    // The choice outlives the connection: coming back announces it.
    actor.hang_up().await;
    observer.event("PresenceChanged").await;
    let mut again = connect(&server, &maya).await;
    let own = again.event("PresenceChanged").await;
    assert_eq!(own["presence"], "dnd");
    assert_eq!(presence(&observer.event("PresenceChanged").await), (maya.id.clone(), "dnd".to_string()));
}

#[tokio::test]
async fn an_unknown_or_malformed_status_is_refused_and_changes_nothing() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;

    for status in ["invisible", "Idle"] {
        let rejected = client.invoke("SetPresence", json!([status])).await;
        assert_eq!(rejected["type"], "error", "{rejected}");
        assert_eq!(rejected["code"], "visible", "{rejected}");
        assert_eq!(rejected["message"], "Unknown presence.");
    }
    for args in [json!([]), json!(["idle", "dnd"]), json!([3])] {
        let rejected = client.invoke("SetPresence", args).await;
        assert_eq!(rejected["type"], "error", "{rejected}");
        assert_eq!(rejected["code"], "invalid_arguments", "{rejected}");
    }
    assert!(client.quiet_for(Duration::from_millis(100)).await.is_empty());
}

#[tokio::test]
async fn the_clock_answers_in_milliseconds_and_takes_no_arguments() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;
    let before = Timestamp::now().unix_millis();

    let answer = client.invoke("GetRtcClock", json!([])).await;

    let clock = answer["value"].as_i64().expect("a number of milliseconds");
    assert!(clock >= before - 5_000 && clock <= Timestamp::now().unix_millis() + 5_000);
    let extra = client.invoke("GetRtcClock", json!([1])).await;
    assert_eq!(extra["type"], "error");
    assert_eq!(extra["code"], "invalid_arguments");
}

#[tokio::test]
async fn a_method_nobody_has_registered_is_an_unknown_method_and_names_are_exact() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;

    for method in ["NoSuchMethod", "setpresence", "SETPRESENCE", "getrtcclock"] {
        let answer = client.invoke(method, json!([])).await;
        assert_eq!(answer["type"], "error", "{method}: {answer}");
        assert_eq!(answer["code"], "unknown_method", "{method}: {answer}");
    }
}

#[tokio::test]
async fn an_account_that_hides_its_status_is_offline_to_others_and_online_to_itself() {
    let (server, owner, maya) = two_people().await;
    server
        .database()
        .execute(
            "INSERT OR REPLACE INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
             VALUES (?1, 1, 1, 1, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 50)",
            [Id::parse(&maya.id).unwrap()],
        )
        .expect("preferences are written");
    let mut observer = connect(&server, &owner).await;
    settle(&mut observer, &owner).await;

    let mut actor = connect(&server, &maya).await;

    let heard = observer.event("PresenceChanged").await;
    assert_eq!(presence(&heard), (maya.id.clone(), "offline".to_string()));
    settle(&mut actor, &maya).await;
    assert!(observer.quiet_for(QUIET).await.is_empty());
}

#[tokio::test]
async fn the_hub_refuses_whoever_is_not_signed_in() {
    let (server, _, _) = two_people().await;

    let (answer, _) = upgrade(server.address(), "", None).await;
    assert_eq!(answer.status, 401);

    let (answer, _) = upgrade(server.address(), "", Some("stellarr-dev-session=garbage")).await;
    assert_eq!(answer.status, 401);

}

#[tokio::test]
async fn a_signed_in_account_must_offer_stellarr_v1_and_cannot_negotiate() {
    let (server, owner, _) = two_people().await;
    let cookie = cookie_of(&owner);
    let host = server.address().to_string();
    let origin = format!("http://{host}");
    for offered in [None, Some("stellarr.v2")] {
        let mut headers = vec![("Origin", origin.as_str())];
        headers.extend(offered.map(|value| ("Sec-WebSocket-Protocol", value)));
        let (answer, _) = upgrade_with(server.address(), "", Some(&cookie), &host, &headers).await;
        assert_eq!(answer.status, 400, "offer {offered:?}: {}", answer.head);
    }

    let negotiation = owner.client
        .send(crate::support::Request::new(http::Method::POST, "/hubs/app/negotiate?negotiateVersion=1"))
        .await;
    assert_eq!(negotiation.status, 404);
}

#[tokio::test]
async fn the_password_is_not_needed_again_to_stay_connected() {
    // Signing in again elsewhere does not end the first connection.
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;

    let other = server.sign_in(&owner).await;
    assert_eq!(server.login(&other, &owner.username, PASSWORD).await.status, 200);

    assert!(client.quiet_for(QUIET).await.is_empty());
    assert_eq!(client.invoke("GetRtcClock", json!([])).await["type"], "result");
}

// What ends a session while it has a connection open.

const WATCHER: [(&str, &str); 1] = [("Testing__Workers", "realtime-sessions")];

/// The id of the session a client's cookie names, as the database has it.
fn session_of(server: &Server, person: &Person, client: &crate::support::Client) -> Id {
    let _ = client;
    let newest: String = server
        .database()
        .query_row(
            "SELECT \"Id\" FROM \"UserSessions\" WHERE \"UserId\" = ?1 ORDER BY \"CreatedAt\" DESC, rowid DESC LIMIT 1",
            [Id::parse(&person.id).unwrap()],
            |row| row.get(0),
        )
        .expect("the person has a session");
    Id::parse(&newest).expect("an id")
}

/// An account with two sessions, each with a connection open and settled.
async fn two_sessions() -> (Server, Person, Id, HubClient, Id, HubClient) {
    let server = Server::start_with(&WATCHER).await;
    let owner = server.found("owner").await;
    let first_session = session_of(&server, &owner, &owner.client);
    let mut first = connect(&server, &owner).await;
    settle(&mut first, &owner).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let second_client = server.sign_in(&owner).await;
    let second_session = session_of(&server, &owner, &second_client);
    assert_ne!(first_session, second_session);
    let cookie = format!("{COOKIE}={}", second_client.cookie(COOKIE).unwrap());
    let mut second = HubClient::connect(server.address(), &cookie).await;
    second.event("PresenceChanged").await;
    (server, owner, first_session, first, second_session, second)
}

/// Changes the database as another process would, then says how long the
/// connection of `closed` took to be closed, and what it was told.
async fn closed_within_two_seconds(connection: &mut HubClient) -> Option<u16> {
    let started = std::time::Instant::now();
    let close = connection.finish_close().await;
    assert!(started.elapsed() < Duration::from_secs(2), "closed after {:?}", started.elapsed());
    close
}

#[tokio::test]
async fn a_session_revoked_in_the_database_loses_its_connection_and_only_its_own() {
    let (server, _, first, mut closing, _, mut staying) = two_sessions().await;

    server
        .database()
        .execute(
            "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
            rusqlite::params![first, Timestamp::now()],
        )
        .unwrap();

    assert_eq!(closed_within_two_seconds(&mut closing).await, Some(4401));
    assert!(staying.quiet_for(Duration::from_millis(1500)).await.is_empty());
    assert_eq!(staying.invoke("GetRtcClock", json!([])).await["type"], "result");
}

#[tokio::test]
async fn a_session_that_ran_out_or_reached_its_absolute_end_loses_its_connection() {
    for column in ["ExpiresAt", "AbsoluteExpiresAt"] {
        let (server, _, first, mut closing, _, mut staying) = two_sessions().await;

        server
            .database()
            .execute(
                &format!("UPDATE \"UserSessions\" SET \"{column}\" = ?2 WHERE \"Id\" = ?1"),
                rusqlite::params![first, Timestamp::now().minus(Duration::from_secs(1))],
            )
            .unwrap();

        assert_eq!(closed_within_two_seconds(&mut closing).await, Some(4401), "{column}");
        assert_eq!(staying.invoke("GetRtcClock", json!([])).await["type"], "result", "{column}");
    }
}

#[tokio::test]
async fn a_disabled_account_loses_every_connection() {
    let (server, owner, _, mut first, _, mut second) = two_sessions().await;

    server
        .database()
        .execute(
            "UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1",
            [Id::parse(&owner.id).unwrap()],
        )
        .unwrap();

    assert_eq!(closed_within_two_seconds(&mut first).await, Some(4401));
    assert_eq!(closed_within_two_seconds(&mut second).await, Some(4401));
}

#[tokio::test]
async fn a_bump_of_the_accounts_session_version_ends_every_connection_it_made() {
    let (server, owner, _, mut first, _, mut second) = two_sessions().await;

    server
        .database()
        .execute(
            "UPDATE \"Users\" SET \"SessionVersion\" = \"SessionVersion\" + 1 WHERE \"Id\" = ?1",
            [Id::parse(&owner.id).unwrap()],
        )
        .unwrap();

    assert_eq!(closed_within_two_seconds(&mut first).await, Some(4401));
    assert_eq!(closed_within_two_seconds(&mut second).await, Some(4401));
}

/// Makes the instance ask for the second step of `requirement`.
fn require_second_step(server: &Server, requirement: &str) {
    let instance: String = server
        .database()
        .query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |row| row.get(0))
        .unwrap();
    server
        .database()
        .execute(
            "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\")
             VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, ?2, ?3)
             ON CONFLICT (\"ServerId\") DO UPDATE SET \"MfaRequirement\" = excluded.\"MfaRequirement\"",
            rusqlite::params![instance, Timestamp::now(), requirement],
        )
        .unwrap();
}

#[tokio::test]
async fn a_policy_that_starts_demanding_a_second_step_ends_the_connections_that_have_none() {
    let (server, _, _, mut first, _, mut second) = two_sessions().await;

    // Asking only the instance's administrators leaves the others alone, and
    // the owner is one of them.
    require_second_step(&server, "everyone");

    assert_eq!(closed_within_two_seconds(&mut first).await, Some(4401));
    assert_eq!(closed_within_two_seconds(&mut second).await, Some(4401));
}

#[tokio::test]
async fn a_policy_that_does_not_reach_the_account_leaves_its_connections_alone() {
    let server = Server::start_with(&WATCHER).await;
    let _owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let mut connection = connect(&server, &maya).await;
    settle(&mut connection, &maya).await;

    require_second_step(&server, "admins");

    assert!(connection.quiet_for(Duration::from_millis(2500)).await.is_empty());
    assert_eq!(connection.invoke("GetRtcClock", json!([])).await["type"], "result");
}

#[tokio::test]
async fn signing_out_closes_the_connections_of_that_session_at_once_without_the_watcher() {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let mut connection = connect(&server, &owner).await;
    settle(&mut connection, &owner).await;
    let started = std::time::Instant::now();

    let response = owner.client.post_json("/api/auth/logout", &json!({})).await;
    assert_eq!(response.status, 204);

    assert_eq!(closed_within_two_seconds(&mut connection).await, Some(4401));
    assert!(started.elapsed() < Duration::from_millis(900), "{:?}", started.elapsed());
}

#[tokio::test]
async fn a_session_that_stays_valid_keeps_its_connection_through_many_sweeps() {
    let (server, _, _, mut first, _, _second) = two_sessions().await;
    drop(server.database());

    assert!(first.quiet_for(Duration::from_millis(3500)).await.is_empty());
    assert_eq!(first.invoke("GetRtcClock", json!([])).await["type"], "result");
}

// What handlers publish, and who hears it. These run the library in the
// test's own process, with a route that publishes whatever it is told, next
// to the accounts' own.

mod published {
    use axum::extract::State;
    use axum::http::StatusCode;
    use serde::Deserialize;
    use serde_json::{Value, json};
    use stellarr_api::Services;
    use stellarr_api::events::{Audience, Event};
    use stellarr_api::http::{Endpoints, Json};
    use stellarr_api::realtime::Realtime;
    use stellarr_api::state::AppState;
    use stellarr_api::values::Id;

    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum Wanted {
        Server(Id),
        Users(Vec<Id>),
        Followers { server: Id, users: Vec<Id> },
        Reach { servers: Vec<Id>, users: Vec<Id> },
        Everyone,
        Connection(String),
    }

    #[derive(Deserialize)]
    struct Publish {
        audience: Wanted,
        name: String,
        /// The JSON of the payload, as text, so that it is sent as it is.
        payload: String,
    }

    async fn publish(State(state): State<AppState>, Json(body): Json<Publish>) -> StatusCode {
        let audience = match body.audience {
            Wanted::Server(server) => Audience::Server(server),
            Wanted::Users(users) => Audience::Users(users),
            Wanted::Followers { server, users } => Audience::Followers { server, users },
            Wanted::Reach { servers, users } => Audience::Reach { servers, users },
            Wanted::Everyone => Audience::Everyone,
            Wanted::Connection(id) => Audience::Connection(id),
        };
        state.events().publish(Event {
            audience,
            name: body.name,
            payload: body.payload,
        });
        StatusCode::NO_CONTENT
    }

    #[derive(Deserialize)]
    struct Ended {
        user: Id,
        session: Id,
    }

    async fn session_ended(State(state): State<AppState>, Json(body): Json<Ended>) -> StatusCode {
        state.events().session_ended(body.user, body.session);
        StatusCode::NO_CONTENT
    }

    #[derive(Deserialize)]
    struct Left {
        user: Id,
        server: Id,
    }

    async fn server_left(State(state): State<AppState>, Json(body): Json<Left>) -> StatusCode {
        state.events().server_left(body.user, body.server);
        StatusCode::NO_CONTENT
    }

    async fn setup_code(State(state): State<AppState>) -> String {
        state.setup_code().current().expect("the instance is empty")
    }

    async fn connections(State(state): State<AppState>) -> Json<Value> {
        let live = state.realtime().expect("a hub").registry().snapshot();
        Json(json!(live.iter().map(|c| json!({"id": c.connection_id, "user": c.user})).collect::<Vec<_>>()))
    }

    #[derive(Deserialize)]
    struct Visibility {
        user: Id,
        visible: bool,
    }

    async fn visibility(State(state): State<AppState>, Json(body): Json<Visibility>) -> Json<Value> {
        Json(json!(state.presence().set_visibility(body.user, body.visible)))
    }

    async fn presence(State(state): State<AppState>, Json(body): Json<Visibility>) -> Json<Value> {
        let _ = body.visible;
        Json(json!({
            "own": state.presence().presence_of(body.user),
            "public": state.presence().public_presence_of(body.user, None),
            "connections": state.presence().connection_ids(body.user).len(),
        }))
    }

    fn routes() -> Endpoints {
        stellarr_api::accounts::routes().merge(
            Endpoints::new()
                .get("/api/test/setup-code", setup_code)
                .get("/api/test/connections", connections)
                .post("/api/test/publish", publish)
                .json_body()
                .post("/api/test/session-ended", session_ended)
                .json_body()
                .post("/api/test/server-left", server_left)
                .json_body()
                .post("/api/test/visibility", visibility)
                .json_body()
                .post("/api/test/presence", presence)
                .json_body(),
        )
    }

    pub struct Instance {
        pub server: Server,
        pub owner: Person,
        pub maya: Person,
    }

    impl Instance {
        /// A hub in this process, an owner who founded the instance, and maya.
        pub async fn start() -> Instance {
            let services = Services {
                realtime: Some(Realtime::new()),
                workers: stellarr_api::workers(),
                ..Services::default()
            };
            let server = Server::in_process_with(routes(), services, &[]).await;
            let code = server.anonymous().get("/api/test/setup-code").await.text();
            let client = server.client();
            let response = client
                .post_json(
                    "/api/auth/register",
                    &json!({"username": "owner", "displayName": "Owner", "email": "owner@example.test",
                            "password": PASSWORD, "setupCode": code}),
                )
                .await;
            assert_eq!(response.status, 201, "{}", response.text());
            let owner = Person {
                username: "owner".into(),
                id: response.json::<Value>()["id"].as_str().unwrap().into(),
                client,
            };
            let maya = server.register("maya").await;
            Instance { server, owner, maya }
        }

        pub async fn publish(&self, audience: Value, name: &str, payload: &str) {
            let response = self
                .server
                .anonymous()
                .send(
                    crate::support::Request::new(http::Method::POST, "/api/test/publish")
                        .json(&json!({"audience": audience, "name": name, "payload": payload})),
                )
                .await;
            assert_eq!(response.status, 204, "{}", response.text());
        }

        pub async fn post(&self, path: &str, body: Value) -> Value {
            let response = self
                .server
                .anonymous()
                .send(crate::support::Request::new(http::Method::POST, path).json(&body))
                .await;
            assert!(matches!(response.status.as_u16(), 200 | 204), "{}", response.text());
            if response.body.is_empty() { Value::Null } else { response.json() }
        }

        pub async fn connect(&self, person: &Person, query: &str) -> HubClient {
            HubClient::connect_to(self.server.address(), query, &cookie_of(person)).await
        }
    }

    /// A tab that has been told its own status and is settled.
    async fn settled(client: &mut HubClient) {
        client.event("PresenceChanged").await;
    }

    /// Makes `person` a member of a server that has no row of its own, which
    /// is all the hub looks at.
    fn join_another_server(server: &Server, person: &str, other: &str) {
        let database = server.database();
        database.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        database
            .execute(
                "INSERT INTO \"ServerMembers\" VALUES (?1, ?2, (SELECT \"RoleId\" FROM \"ServerMembers\" LIMIT 1), NULL, '2026-11-24 02:00:00')",
                [other.to_uppercase(), Id::parse(person).unwrap().to_db_text()],
            )
            .unwrap();
    }

    #[tokio::test]
    async fn a_server_event_reaches_the_connections_that_follow_that_server_and_no_others() {
        let instance = Instance::start().await;
        let server_id = instance.server.database().query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap();
        let other = "11111111-2222-4333-8444-555555555555";
        join_another_server(&instance.server, &instance.maya.id, other);
        let mut follower = instance.connect(&instance.owner, "").await;
        let mut elsewhere = instance.connect(&instance.maya, &format!("?serverId={other}")).await;
        let mut also_follower = instance.connect(&instance.maya, &format!("?serverId={server_id}")).await;
        // Maya's first tab announced her; the owner's tab was told of her.
        follower.quiet_for(Duration::from_millis(300)).await;
        elsewhere.quiet_for(Duration::from_millis(300)).await;
        also_follower.quiet_for(Duration::from_millis(300)).await;
        let server = Id::parse(&server_id.to_lowercase()).unwrap();

        instance.publish(json!({"server": server}), "ChannelCreated", "{\"id\":\"c1\"}").await;

        assert_eq!(follower.event("ChannelCreated").await, json!({"id": "c1"}));
        assert_eq!(also_follower.event("ChannelCreated").await, json!({"id": "c1"}));
        assert!(elsewhere.quiet_for(QUIET).await.is_empty());
        assert!(follower.quiet_for(Duration::from_millis(50)).await.is_empty());
    }

    #[tokio::test]
    async fn an_event_for_people_reaches_every_tab_of_each_of_them_and_nobody_else() {
        let instance = Instance::start().await;
        let (mut owner_one, mut owner_two) = (instance.connect(&instance.owner, "").await, instance.connect(&instance.owner, "?serverId=nonsense").await);
        let mut maya = instance.connect(&instance.maya, "").await;
        for client in [&mut owner_one, &mut owner_two, &mut maya] {
            client.quiet_for(Duration::from_millis(300)).await;
        }
        let owner = Id::parse(&instance.owner.id).unwrap();

        instance.publish(json!({"users": [owner]}), "DirectMessageCreated", "{\"n\":1}").await;

        assert_eq!(owner_one.event("DirectMessageCreated").await, json!({"n": 1}));
        assert_eq!(owner_two.event("DirectMessageCreated").await, json!({"n": 1}));
        assert!(maya.quiet_for(QUIET).await.is_empty());
        assert!(owner_one.quiet_for(Duration::from_millis(50)).await.is_empty());
    }

    #[tokio::test]
    async fn an_event_for_followers_leaves_out_the_tabs_of_the_same_person_on_other_servers() {
        let instance = Instance::start().await;
        let server_id = instance.server.database().query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap();
        let other = "11111111-2222-4333-8444-555555555555";
        join_another_server(&instance.server, &instance.owner.id, other);
        let server = Id::parse(&server_id.to_lowercase()).unwrap();
        let mut here = instance.connect(&instance.owner, &format!("?serverId={server}")).await;
        let mut there = instance.connect(&instance.owner, &format!("?serverId={other}")).await;
        let mut maya = instance.connect(&instance.maya, "").await;
        for client in [&mut here, &mut there, &mut maya] {
            client.quiet_for(Duration::from_millis(300)).await;
        }
        let owner = Id::parse(&instance.owner.id).unwrap();

        instance
            .publish(json!({"followers": {"server": server, "users": [owner]}}), "TypingChanged", "{\"who\":\"x\"}")
            .await;

        assert_eq!(here.event("TypingChanged").await, json!({"who": "x"}));
        assert!(there.quiet_for(QUIET).await.is_empty());
        assert!(maya.quiet_for(Duration::from_millis(50)).await.is_empty());
    }

    #[tokio::test]
    async fn a_person_named_twice_hears_an_event_once_on_each_tab() {
        let instance = Instance::start().await;
        let server_id = instance.server.database().query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap();
        let server = Id::parse(&server_id.to_lowercase()).unwrap();
        let mut owner = instance.connect(&instance.owner, "").await;
        owner.quiet_for(Duration::from_millis(300)).await;
        let id = Id::parse(&instance.owner.id).unwrap();

        instance.publish(json!({"users": [id, id]}), "DirectMessageCreated", "{\"n\":1}").await;
        instance.publish(json!({"followers": {"server": server, "users": [id, id]}}), "TypingChanged", "{\"n\":2}").await;

        let heard = owner.quiet_for(Duration::from_millis(400)).await;
        let named: Vec<&str> = heard.iter().filter_map(|record| record["name"].as_str()).collect();
        assert_eq!(named, ["DirectMessageCreated", "TypingChanged"], "{heard:?}");
    }

    #[tokio::test]
    async fn a_tab_that_follows_a_server_and_belongs_to_a_person_hears_what_reaches_both_once() {
        let instance = Instance::start().await;
        let server_id = instance.server.database().query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap();
        let server = Id::parse(&server_id.to_lowercase()).unwrap();
        let mut owner = instance.connect(&instance.owner, "").await;
        let mut maya = instance.connect(&instance.maya, "").await;
        owner.quiet_for(Duration::from_millis(300)).await;
        maya.quiet_for(Duration::from_millis(300)).await;
        let id = Id::parse(&instance.owner.id).unwrap();

        instance.publish(json!({"reach": {"servers": [server, server], "users": [id]}}), "Probe", "1").await;

        for client in [&mut owner, &mut maya] {
            let heard = client.quiet_for(Duration::from_millis(400)).await;
            assert_eq!(heard.len(), 1, "{heard:?}");
            assert_eq!(heard[0]["name"], "Probe");
        }
    }

    #[tokio::test]
    async fn a_change_of_status_is_heard_once_by_a_tab_that_is_in_two_of_the_groups_it_goes_to() {
        let instance = Instance::start().await;
        let (owner_id, maya_id) = (Id::parse(&instance.owner.id).unwrap(), Id::parse(&instance.maya.id).unwrap());
        let (a, b) = stellarr_api::social::pair(owner_id, maya_id);
        instance
            .server
            .database()
            .execute(
                "INSERT INTO \"Friendships\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![Id::random(), a, b, stellarr_api::values::Timestamp::now()],
            )
            .unwrap();
        let mut owner = instance.connect(&instance.owner, "").await;
        owner.quiet_for(Duration::from_millis(300)).await;

        // Maya shares the server with the owner and is the owner's friend: the
        // owner's tab is in the server's group and in the owner's own.
        let _maya = instance.connect(&instance.maya, "").await;

        let heard = owner.quiet_for(Duration::from_millis(500)).await;
        let of_maya = heard
            .iter()
            .filter(|record| record["name"] == "PresenceChanged" && record["data"]["userId"] == instance.maya.id.as_str())
            .count();
        assert_eq!(of_maya, 1, "{heard:?}");
    }

    #[tokio::test]
    async fn everyone_and_one_connection_are_addressed_as_they_say() {
        let instance = Instance::start().await;
        let mut owner = instance.connect(&instance.owner, "").await;
        let mut maya = instance.connect(&instance.maya, "").await;
        owner.quiet_for(Duration::from_millis(300)).await;
        maya.quiet_for(Duration::from_millis(300)).await;

        instance.publish(json!("everyone"), "ServerUpdated", "{\"name\":\"X\"}").await;
        assert_eq!(owner.event("ServerUpdated").await, json!({"name": "X"}));
        assert_eq!(maya.event("ServerUpdated").await, json!({"name": "X"}));

        let live = instance.server.anonymous().get("/api/test/connections").await.json::<Value>();
        let maya_connection = live
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["user"] == instance.maya.id.as_str())
            .unwrap()["id"]
            .clone();
        instance.publish(json!({"connection": maya_connection}), "Probe", "1").await;
        assert_eq!(maya.event("Probe").await, json!(1));
        assert!(owner.quiet_for(QUIET).await.is_empty());
    }

    #[tokio::test]
    async fn the_payload_goes_out_as_its_publisher_wrote_it() {
        let instance = Instance::start().await;
        let mut owner = instance.connect(&instance.owner, "").await;
        owner.quiet_for(Duration::from_millis(300)).await;

        instance.publish(json!("everyone"), "Probe", "{\"zebra\":1,\"apple\":[2,1],\"mango\":\"é\"}").await;

        assert_eq!(
            owner.raw_record().await,
            r#"{"type":"event","name":"Probe","data":{"zebra":1,"apple":[2,1],"mango":"é"}}"#
        );
    }

    #[tokio::test]
    async fn a_session_that_ended_closes_its_connections_and_is_told_to_nobody() {
        let instance = Instance::start().await;
        let session = session_of(&instance.server, &instance.owner, &instance.owner.client);
        let mut leaving = instance.connect(&instance.owner, "").await;
        let mut staying = instance.connect(&instance.maya, "").await;
        leaving.quiet_for(Duration::from_millis(300)).await;
        staying.quiet_for(Duration::from_millis(300)).await;

        instance
            .post("/api/test/session-ended", json!({"user": instance.owner.id, "session": session}))
            .await;

        assert_eq!(closed_within_two_seconds(&mut leaving).await, Some(4401));
        // The owner going offline is the only thing the others hear.
        let heard = staying.quiet_for(QUIET).await;
        assert!(heard.iter().all(|record| record["name"] == "PresenceChanged"), "{heard:?}");
        assert!(heard.iter().all(|record| record["name"] != "SessionEnded"));
    }

    #[tokio::test]
    async fn somebody_who_left_a_server_stops_hearing_what_is_sent_to_it() {
        let instance = Instance::start().await;
        let server_id = instance.server.database().query_row("SELECT \"Id\" FROM \"Servers\" LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap();
        let server = Id::parse(&server_id.to_lowercase()).unwrap();
        let mut owner = instance.connect(&instance.owner, "").await;
        let mut maya = instance.connect(&instance.maya, "").await;
        owner.quiet_for(Duration::from_millis(300)).await;
        maya.quiet_for(Duration::from_millis(300)).await;

        instance.post("/api/test/server-left", json!({"user": instance.maya.id, "server": server})).await;
        instance.publish(json!({"server": server}), "ServerUpdated", "{}").await;
        let owner_id = Id::parse(&instance.owner.id).unwrap();
        instance
            .publish(json!({"followers": {"server": server, "users": [owner_id, Id::parse(&instance.maya.id).unwrap()]}}), "Probe", "2")
            .await;

        assert_eq!(owner.event("ServerUpdated").await, json!({}));
        assert_eq!(owner.event("Probe").await, json!(2));
        assert!(maya.quiet_for(QUIET).await.is_empty());
        let _ = settled;
    }

    #[tokio::test]
    async fn presence_is_what_the_hub_says_and_a_hidden_status_reads_offline_to_others() {
        let instance = Instance::start().await;
        let maya = Id::parse(&instance.maya.id).unwrap();
        let asked = |body: Value| async { instance.post("/api/test/presence", body).await };
        assert_eq!(
            asked(json!({"user": maya, "visible": true})).await,
            json!({"own": "offline", "public": "offline", "connections": 0})
        );

        let mut client = instance.connect(&instance.maya, "").await;
        client.quiet_for(Duration::from_millis(300)).await;
        assert_eq!(
            asked(json!({"user": maya, "visible": true})).await,
            json!({"own": "online", "public": "online", "connections": 1})
        );
        assert_eq!(instance.post("/api/test/visibility", json!({"user": maya, "visible": false})).await, json!(true));
        assert_eq!(instance.post("/api/test/visibility", json!({"user": maya, "visible": false})).await, json!(false));
        assert_eq!(
            asked(json!({"user": maya, "visible": true})).await,
            json!({"own": "online", "public": "offline", "connections": 1})
        );
    }
}

// Who hears about a status beyond the people of a server.

/// Makes owner and maya know each other without sharing a server: maya has
/// left every server, and they are `relationship` ("friend", "conversation"
/// or "both").
fn befriend_without_a_server(server: &Server, owner: &Person, maya: &Person, relationship: &str) {
    let database = server.database();
    let (owner, maya) = (Id::parse(&owner.id).unwrap(), Id::parse(&maya.id).unwrap());
    database
        .execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [maya])
        .unwrap();
    let now = Timestamp::now();
    if matches!(relationship, "friend" | "both") {
        database
            .execute(
                "INSERT INTO \"Friendships\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![Id::random(), owner, maya, now],
            )
            .unwrap();
    }
    if matches!(relationship, "conversation" | "both") {
        database
            .execute(
                "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\") VALUES (?1, ?2, ?3, ?4, 'Off')",
                rusqlite::params![Id::random(), owner, maya, now],
            )
            .unwrap();
    }
}

#[tokio::test]
async fn social_peers_hear_a_status_without_sharing_a_server() {
    for relationship in ["friend", "conversation", "both"] {
        let (server, owner, maya) = two_people().await;
        befriend_without_a_server(&server, &owner, &maya, relationship);
        let mut observer = connect(&server, &owner).await;
        settle(&mut observer, &owner).await;

        let mut actor = connect(&server, &maya).await;
        // Without a server the tab is told its own status once, and the
        // friend is told once too: only one group reaches the friend.
        assert_eq!(presence(&observer.event("PresenceChanged").await).1, "online", "{relationship}");
        assert_eq!(presence(&actor.event("PresenceChanged").await).1, "online", "{relationship}");

        actor.invoke("SetPresence", json!(["idle"])).await;
        assert_eq!(presence(&observer.event("PresenceChanged").await).1, "idle", "{relationship}");
        assert_eq!(presence(&actor.event("PresenceChanged").await).1, "idle", "{relationship}");

        actor.hang_up().await;
        assert_eq!(presence(&observer.event("PresenceChanged").await).1, "offline", "{relationship}");
        assert!(observer.quiet_for(QUIET).await.is_empty(), "{relationship}");
    }
}

#[tokio::test]
async fn people_who_blocked_each_other_hear_nothing_of_each_others_status() {
    for actor_blocks in [true, false] {
        let (server, owner, maya) = two_people().await;
        befriend_without_a_server(&server, &owner, &maya, "both");
        let (blocker, blocked) = if actor_blocks { (&maya, &owner) } else { (&owner, &maya) };
        server
            .database()
            .execute(
                "INSERT INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
                rusqlite::params![Id::parse(&blocker.id).unwrap(), Id::parse(&blocked.id).unwrap(), Timestamp::now()],
            )
            .unwrap();
        let mut observer = connect(&server, &owner).await;
        settle(&mut observer, &owner).await;
        let mut actor = connect(&server, &maya).await;
        actor.event("PresenceChanged").await;

        actor.invoke("SetPresence", json!(["dnd"])).await;
        actor.hang_up().await;

        assert!(observer.quiet_for(QUIET).await.is_empty(), "actor blocks: {actor_blocks}");
    }
}

#[tokio::test]
async fn a_connection_follows_the_server_it_asked_for_when_it_belongs_to_it() {
    let (server, owner, maya) = two_people().await;
    // The owner is on a second server that maya is not on, and the status of
    // maya is heard by whoever follows a server she is on.
    let other = "11111111-aaaa-4aaa-8aaa-333333333333".to_uppercase();
    let database = server.database();
    database.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
    database
        .execute(
            "INSERT INTO \"ServerMembers\" VALUES (?1, ?2, (SELECT \"RoleId\" FROM \"ServerMembers\" LIMIT 1), NULL, ?3)",
            rusqlite::params![other, Id::parse(&owner.id).unwrap(), Timestamp::now()],
        )
        .unwrap();
    let instance_server: String = database
        .query_row("SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"UserId\" = ?1 AND \"ServerId\" <> ?2", rusqlite::params![Id::parse(&owner.id).unwrap(), other], |r| r.get(0))
        .unwrap();
    let cookie = cookie_of(&owner);
    let mut actor = connect(&server, &maya).await;
    settle(&mut actor, &maya).await;

    for (query, hears) in [
        (String::new(), true),
        (format!("?serverId={}", instance_server.to_lowercase()), true),
        (format!("?serverId={}", other.to_lowercase()), false),
        // The name is `serverId`; in another case it names nothing.
        (format!("?SERVERID={}", other.to_lowercase()), true),
        // Only the hyphenated spelling is an id; any other names no server and
        // the connection follows the first.
        (format!("?serverId={}", other.to_lowercase().replace('-', "")), true),
        (format!("?serverId=%7B{}%7D", other.to_lowercase()), true),
        ("?serverId=garbage".to_string(), true),
        ("?serverId=11111111-1111-1111-1111-111111111111".to_string(), true),
    ] {
        let mut watcher = HubClient::connect_to(server.address(), &query, &cookie).await;
        // The order the database lists the owner's servers in decides which
        // one a connection follows when it names none that counts; the
        // instance's own server comes first because it was joined first.
        watcher.quiet_for(Duration::from_millis(200)).await;
        actor.invoke("SetPresence", json!(["idle"])).await;
        actor.invoke("SetPresence", json!(["online"])).await;
        actor.quiet_for(Duration::from_millis(100)).await;
        let heard = watcher.quiet_for(Duration::from_millis(300)).await;
        let counted = heard.iter().filter(|r| r["name"] == "PresenceChanged").count();
        assert_eq!(counted > 0, hears, "{query}: {heard:?}");
        watcher.hang_up().await;
        actor.quiet_for(Duration::from_millis(100)).await;
    }
}
