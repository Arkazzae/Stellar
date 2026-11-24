//! The hub, through the real binary: who is let in, what a connection is told
//! when it starts and ends, the methods that need nobody else, and who hears
//! about a status.
//!
//! Connections are raw sockets that speak the hub protocol, signed in with
//! the cookie of an account that logged in over HTTP.

use std::time::Duration;

use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{HubClient, PASSWORD, Person, Server, upgrade};

const COOKIE: &str = "stellarr-dev-session";
const QUIET: Duration = Duration::from_millis(400);

/// The cookie header of a person's sign-in.
fn cookie_of(person: &Person) -> String {
    format!("{COOKIE}={}", person.client.cookie(COOKIE).expect("signed in"))
}

async fn connect(server: &Server, person: &Person) -> HubClient {
    HubClient::connect(server.address(), &cookie_of(person)).await
}

/// What a `PresenceChanged` says, as (account, status).
fn presence(event: &Value) -> (String, String) {
    (
        event["userId"].as_str().expect("an account").to_string(),
        event["presence"].as_str().expect("a status").to_string(),
    )
}

fn is_presence_of(record: &Value, person: &Person, status: &str) -> bool {
    record["target"] == "PresenceChanged"
        && record["arguments"][0]["userId"] == person.id.as_str()
        && record["arguments"][0]["presence"] == status
}

/// Two people on the instance's server.
async fn two_people() -> (Server, Person, Person) {
    let server = Server::start().await;
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    (server, owner, maya)
}

/// Reads until the connection is settled: its own status has come, which the
/// hub says last of what it does when a connection starts.
async fn settle(client: &mut HubClient, who: &Person) -> Vec<Value> {
    let mut seen = Vec::new();
    // Joining the server's group says the status once to the group, so a
    // connection that follows a server hears it twice.
    loop {
        let record = client.record().await;
        let own = is_presence_of(&record, who, "online");
        seen.push(record);
        if own && seen.len() >= 2 {
            return seen;
        }
    }
}

#[tokio::test]
async fn a_connecting_account_is_told_its_own_status_once_for_each_group_that_carries_it() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;

    let first = client.record().await;
    let second = client.record().await;

    // The server's group, which the connection is in, and then the connection
    // itself: the first server said it twice, and so does this one.
    assert_eq!(first["type"], 1);
    assert_eq!(first["target"], "PresenceChanged");
    assert_eq!(first["arguments"], json!([{"userId": owner.id, "presence": "online"}]));
    assert_eq!(second, first);
    assert!(client.quiet_for(QUIET).await.is_empty());
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

    assert_eq!(answer["type"], 3);
    assert!(answer.get("error").is_none(), "{answer}");
    assert_eq!(presence(&observer.event("PresenceChanged").await), (maya.id.clone(), "dnd".to_string()));
    // The caller hears it through the server's group and again directly.
    assert_eq!(presence(&actor.event("PresenceChanged").await), (maya.id.clone(), "dnd".to_string()));
    assert_eq!(presence(&actor.event("PresenceChanged").await), (maya.id.clone(), "dnd".to_string()));
    assert!(observer.quiet_for(QUIET).await.is_empty());

    // Choosing what is already chosen says nothing to anybody.
    actor.invoke("SetPresence", json!(["dnd"])).await;
    assert!(actor.quiet_for(QUIET).await.is_empty());
    assert!(observer.quiet_for(Duration::from_millis(50)).await.is_empty());

    // The choice outlives the connection: coming back announces it.
    actor.hang_up().await;
    observer.event("PresenceChanged").await;
    let mut again = connect(&server, &maya).await;
    let own = settle_any(&mut again).await;
    assert_eq!(own["presence"], "dnd");
    assert_eq!(presence(&observer.event("PresenceChanged").await), (maya.id.clone(), "dnd".to_string()));
}

/// The first status record of a connection that has just started.
async fn settle_any(client: &mut HubClient) -> Value {
    let first = client.event("PresenceChanged").await;
    client.event("PresenceChanged").await;
    first
}

#[tokio::test]
async fn an_unknown_or_malformed_status_is_refused_and_changes_nothing() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;

    let unknown = client.invoke("SetPresence", json!(["invisible"])).await;
    assert_eq!(
        unknown["error"],
        "An unexpected error occurred invoking 'SetPresence' on the server. HubException: Unknown presence."
    );
    let wrong_case = client.invoke("setpresence", json!(["Idle"])).await;
    assert!(wrong_case["error"].as_str().unwrap().ends_with("HubException: Unknown presence."));
    let nothing = client.invoke("SetPresence", json!([])).await;
    assert_eq!(nothing["error"], "Failed to invoke 'SetPresence' due to an error on the server.");
    let two = client.invoke("SetPresence", json!(["idle", "dnd"])).await;
    assert_eq!(two["error"], "Failed to invoke 'SetPresence' due to an error on the server.");
    let number = client.invoke("SetPresence", json!([3])).await;
    assert_eq!(number["error"], "Failed to invoke 'SetPresence' due to an error on the server.");
    assert!(client.quiet_for(Duration::from_millis(100)).await.is_empty());
}

#[tokio::test]
async fn the_clock_answers_in_milliseconds_and_takes_no_arguments() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;
    let before = Timestamp::now().unix_millis();

    let answer = client.invoke("GetRtcClock", json!([])).await;

    let clock = answer["result"].as_i64().expect("a number of milliseconds");
    assert!(clock >= before - 5_000 && clock <= Timestamp::now().unix_millis() + 5_000);
    let extra = client.invoke("GetRtcClock", json!([1])).await;
    assert_eq!(extra["error"], "Failed to invoke 'GetRtcClock' due to an error on the server.");
}

#[tokio::test]
async fn a_method_nobody_has_registered_is_answered_as_one_that_does_not_exist() {
    let (server, owner, _) = two_people().await;
    let mut client = connect(&server, &owner).await;
    settle(&mut client, &owner).await;

    for method in ["NoSuchMethod", "JoinChannel", "SendTyping", "JoinVoice", "OpenMediaSession", "PlaySoundboard"] {
        let answer = client.invoke(method, json!([])).await;
        assert_eq!(
            answer["error"],
            format!("Failed to invoke '{method}' due to an error on the server. HubException: Method does not exist.")
        );
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
    // The server's group got the public status, which the connection is in,
    // and then the connection its own.
    assert_eq!(actor.event("PresenceChanged").await["presence"], "offline");
    assert_eq!(actor.event("PresenceChanged").await["presence"], "online");
    assert!(observer.quiet_for(QUIET).await.is_empty());
}

#[tokio::test]
async fn the_hub_refuses_whoever_is_not_signed_in() {
    let (server, _, _) = two_people().await;

    let (answer, _) = upgrade(server.address(), "", None).await;
    assert_eq!(answer.status, 401);

    let (answer, _) = upgrade(server.address(), "", Some("stellarr-dev-session=garbage")).await;
    assert_eq!(answer.status, 401);

    let anonymous = server.anonymous();
    let negotiation = anonymous
        .send(crate::support::Request::new(http::Method::POST, "/hubs/app/negotiate?negotiateVersion=1"))
        .await;
    assert_eq!(negotiation.status, 401);
}

#[tokio::test]
async fn the_clients_negotiation_is_answered_for_a_signed_in_account() {
    let (server, owner, _) = two_people().await;

    let negotiation = owner
        .client
        .send(crate::support::Request::new(http::Method::POST, "/hubs/app/negotiate?negotiateVersion=1"))
        .await;

    assert_eq!(negotiation.status, 200);
    let body: Value = negotiation.json();
    assert_eq!(body["negotiateVersion"], 1);
    assert_eq!(body["availableTransports"][0]["transport"], "WebSockets");
    drop(server);
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
    assert_eq!(client.invoke("GetRtcClock", json!([])).await["type"], 3);
}
