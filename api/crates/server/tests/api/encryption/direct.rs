//! The encryption of one private conversation: the switch both people agree
//! on, the keys, the epochs, and who is told what.

use std::time::Duration;

use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use super::kit::*;
use crate::support::{HubClient, Person};

const QUIET: Duration = Duration::from_millis(500);

fn path(conversation: &str, what: &str) -> String {
    format!("/api/social/conversations/{conversation}/encryption/{what}")
}

/// What a connection was told since it last looked, but who is online.
async fn told(client: &mut HubClient) -> Vec<(String, Value)> {
    client
        .quiet_for(QUIET)
        .await
        .into_iter()
        .filter(|record| record["type"] == 1 && record["target"] != "PresenceChanged")
        .map(|record| (record["target"].as_str().unwrap().to_string(), record["arguments"][0].clone()))
        .collect()
}

fn names(events: &[(String, Value)]) -> Vec<&str> {
    events.iter().map(|(name, _)| name.as_str()).collect()
}

async fn listening(crowd: &Crowd, person: &Person) -> HubClient {
    let mut client = crowd.hub(person).await;
    told(&mut client).await;
    client
}

#[tokio::test]
async fn encryption_is_on_only_once_the_other_person_confirms_with_a_key_of_their_own() {
    let crowd = Crowd::new().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    let (maja, ada) = (&crowd.maja.client, &crowd.ada.client);

    assert_eq!(refusal(&action(maja, &conversation, "request").await), (409, Some("account-key-missing".into())));
    set_up(maja).await;
    let requested = action(maja, &conversation, "request").await;
    assert_eq!(requested.status, 200);
    let state: Value = requested.json();
    assert_eq!((state["state"].as_str(), state["requestedById"].as_str()), (Some("requested"), Some(crowd.maja.id.as_str())));
    assert!(state["requestedAt"].is_string() && state["enabledAt"].is_null() && state["currentEpochId"].is_null());
    assert!(requested.text().starts_with("{\"state\":\"requested\",\"requestedById\":\""), "{}", requested.text());

    let twice = action(maja, &conversation, "request").await;
    assert_eq!(refusal(&twice), (409, Some("encryption-state".into())));
    assert_eq!(twice.json::<Value>()["encryption"]["state"], "requested");
    assert!(
        twice.text().contains("\"status\":409,\"encryption\":{\"state\":\"requested\",\"requestedById\":\"")
            && twice.text().contains("\"currentEpochId\":null},\"code\":\"encryption-state\",\"traceId\""),
        "{}",
        twice.text()
    );
    assert_eq!(refusal(&action(maja, &conversation, "confirm").await), (403, Some("encryption-not-allowed".into())));
    assert_eq!(refusal(&action(ada, &conversation, "confirm").await), (409, Some("account-key-missing".into())));

    set_up(ada).await;
    let confirmed = action(ada, &conversation, "confirm").await;
    assert_eq!(confirmed.status, 200);
    let state: Value = confirmed.json();
    assert_eq!(state["state"], "on");
    assert!(state["enabledAt"].is_string() && state["currentEpochId"].is_null() && state["requestedById"].is_null());
    assert_eq!(crowd.timeline(&conversation), ["encryption_requested:maja", "encryption_enabled:ada"]);
}

#[tokio::test]
async fn a_conversation_or_a_device_is_named_in_the_hyphenated_spelling_only() {
    let crowd = Crowd::ready().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    let maja = &crowd.maja.client;
    let id = Id::parse(&conversation).unwrap().to_string();

    let upper = maja.get(&path(&id.to_uppercase(), "keys")).await;
    assert_eq!(upper.status, 200);
    assert_eq!(upper.json::<Value>()["conversationId"], id.as_str());
    for spelled in [
        id.replace('-', ""),
        format!("{{{id}}}"),
        format!("({id})"),
        format!("%7B{id}%7D"),
    ] {
        assert_eq!(maja.get(&path(&spelled, "keys")).await.status, 404, "{spelled}");
    }
    assert_eq!(action(maja, &id.replace('-', ""), "request").await.status, 404);
    for bad in [&id[1..], &format!("{id}0"), "x"] {
        assert_eq!(maja.get(&path(bad, "keys")).await.status, 404, "{bad}");
    }
    let device = read(maja, ACCOUNT).await["device"]["id"].as_str().unwrap().to_string();
    let other = crowd.server.sign_in(&crowd.maja).await;
    assert_eq!(register_device_with(&other, &bytes(32, 3), "Other").await.status, 201);
    let spelled = read(&other, ACCOUNT).await["device"]["id"].as_str().unwrap().replace('-', "");
    assert_eq!(delete(maja, &format!("{ACCOUNT}/devices/{spelled}")).await.status, 404);
    assert_eq!(refusal(&delete(maja, &format!("{ACCOUNT}/devices/{}", device.to_uppercase())).await), (409, Some("current-device".into())));
}

#[tokio::test]
async fn only_the_requester_cancels_either_person_switches_off_and_on_again_takes_both_again() {
    let crowd = Crowd::new().await;
    let (maja, ada) = (&crowd.maja.client, &crowd.ada.client);
    set_up(maja).await;
    set_up(ada).await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);

    assert_eq!(action(maja, &conversation, "request").await.status, 200);
    assert_eq!(refusal(&action(ada, &conversation, "cancel").await), (403, Some("encryption-not-allowed".into())));
    assert_eq!(action(maja, &conversation, "cancel").await.status, 200);
    assert_eq!(refusal(&action(maja, &conversation, "cancel").await), (409, Some("encryption-state".into())));

    // Whoever confirmed may switch it off as well as whoever asked.
    crowd.enable(&crowd.ada, &crowd.maja, &conversation).await;
    assert_eq!(action(maja, &conversation, "disable").await.status, 200);
    assert_eq!(refusal(&action(ada, &conversation, "confirm").await), (409, Some("encryption-state".into())));
    assert_eq!(refusal(&action(ada, &conversation, "disable").await), (409, Some("encryption-state".into())));
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    assert_eq!(action(ada, &conversation, "disable").await.status, 200);
    // Turning a request down is switching off before it was ever on.
    assert_eq!(action(maja, &conversation, "request").await.status, 200);
    assert_eq!(action(ada, &conversation, "disable").await.status, 200);

    assert_eq!(
        crowd.timeline(&conversation),
        [
            "encryption_requested:maja", "encryption_request_cancelled:maja",
            "encryption_requested:ada", "encryption_enabled:maja", "encryption_disabled:maja",
            "encryption_requested:maja", "encryption_enabled:ada", "encryption_disabled:ada",
            "encryption_requested:maja", "encryption_disabled:ada",
        ]
    );
    let stored: (String, Option<String>, Option<String>, Option<String>, Option<String>) = crowd
        .server
        .database()
        .query_row(
            "SELECT \"EncryptionState\", \"EncryptionRequestedById\", \"EncryptionRequestedAt\", \"EncryptionEnabledAt\", \"CurrentEpochId\"
             FROM \"DirectConversations\" WHERE \"Id\" = ?1",
            [Id::parse(&conversation).unwrap()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert_eq!(stored, ("Off".to_string(), None, None, None, None));
}

fn only_friends(crowd: &Crowd, person: &Person) {
    crowd
        .server
        .database()
        .execute(
            "INSERT INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\", \"EmailNotifications\",
                 \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\", \"AllowDirectMessages\", \"AllowFriendRequests\",
                 \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\", \"VadSensitivity\")
             VALUES (?1, 1, 1, 1, 0, 0, 0, 1, 1, 0, 1, 1, 0, 0, 50)",
            [Id::parse(&person.id).unwrap()],
        )
        .unwrap();
}

#[tokio::test]
async fn somebody_who_takes_messages_only_from_friends_takes_requests_only_from_them_too() {
    let crowd = Crowd::ready().await;
    only_friends(&crowd, &crowd.ada);
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);

    let refused = action(&crowd.maja.client, &conversation, "request").await;
    assert_eq!((refused.status.as_u16(), refused.body.is_empty()), (403, true));
    assert!(crowd.timeline(&conversation).is_empty());

    let (a, b) = {
        let (x, y) = (Id::parse(&crowd.maja.id).unwrap(), Id::parse(&crowd.ada.id).unwrap());
        if x < y { (x, y) } else { (y, x) }
    };
    crowd
        .server
        .database()
        .execute(
            "INSERT INTO \"Friendships\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
            params![Id::random(), a, b, Timestamp::now()],
        )
        .unwrap();
    assert_eq!(action(&crowd.maja.client, &conversation, "request").await.status, 200);
    // Only a request is held back by it: she may still turn it down, and ask.
    assert_eq!(action(&crowd.ada.client, &conversation, "disable").await.status, 200);
}

fn block(crowd: &Crowd, blocker: &Person, blocked: &Person) {
    crowd
        .server
        .database()
        .execute(
            "INSERT INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
            params![Id::parse(&blocker.id).unwrap(), Id::parse(&blocked.id).unwrap(), Timestamp::now()],
        )
        .unwrap();
}

#[tokio::test]
async fn outsiders_find_nothing_and_a_block_closes_the_encryption_too() {
    let crowd = Crowd::ready().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let epoch = rotate(&crowd, &crowd.maja.client, &conversation).await;
    let ilya = &crowd.ilya.client;
    let nothing = |response: &crate::support::Response| (response.status.as_u16(), response.body.is_empty());

    for what in ["request", "cancel", "confirm", "disable"] {
        assert_eq!(nothing(&action(ilya, &conversation, what).await), (404, true), "{what}");
    }
    assert_eq!(nothing(&ilya.get(&path(&conversation, "keys")).await), (404, true));
    assert_eq!(nothing(&ilya.get(&path(&conversation, "epochs")).await), (404, true));
    let strange = post_epoch(ilya, &conversation, &epoch_body(&Id::random().to_string(), Some(&epoch), &[Id::random().to_string()])).await;
    assert_eq!(nothing(&strange), (404, true));
    // A conversation that does not exist is no different from one that is not theirs.
    let unknown = Id::random().to_string();
    for what in ["request", "disable"] {
        assert_eq!(nothing(&action(&crowd.maja.client, &unknown, what).await), (404, true));
    }
    assert_eq!(nothing(&crowd.maja.client.get(&path(&unknown, "keys")).await), (404, true));
    // A malformed epoch is refused the same for an outsider as for anybody: it is read first.
    let malformed = post(ilya, &path(&conversation, "epochs"), &json!({ "epochId": "x", "envelopes": [] })).await;
    assert_eq!(malformed.status, 400);

    block(&crowd, &crowd.maja, &crowd.ada);
    assert_eq!(nothing(&action(&crowd.ada.client, &conversation, "disable").await), (403, true));
    assert_eq!(nothing(&action(&crowd.maja.client, &conversation, "request").await), (403, true));
    assert_eq!(nothing(&crowd.ada.client.get(&path(&conversation, "keys")).await), (404, true));
    assert_eq!(nothing(&crowd.maja.client.get(&path(&conversation, "epochs")).await), (404, true));
    let blocked = post_epoch(&crowd.ada.client, &conversation, &epoch_body(&Id::random().to_string(), Some(&epoch), &[Id::random().to_string()])).await;
    assert_eq!(nothing(&blocked), (403, true));
}

#[tokio::test]
async fn an_epoch_follows_the_current_one_and_reaches_exactly_every_key_of_both_people() {
    let crowd = Crowd::ready().await;
    let (maja, ada) = (&crowd.maja.client, &crowd.ada.client);
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);

    let off = post_epoch(maja, &conversation, &epoch_body(&Id::random().to_string(), None, &[Id::random().to_string()])).await;
    assert_eq!(refusal(&off), (409, Some("encryption-off".into())));
    assert_eq!(off.json::<Value>()["encryption"]["state"], "off");
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;

    let keys = crowd.keys(maja, &conversation).await;
    let every = crowd.recipients(maja, &conversation).await;
    assert_eq!(every.len(), 4);
    let mut own: Vec<String> = vec![
        keys["self"]["accountKey"]["keyId"].as_str().unwrap().to_string(),
        keys["currentDeviceKeyId"].as_str().unwrap().to_string(),
    ];
    own.sort();
    let envelope_ids = |epoch: &Value| -> Vec<String> {
        let mut ids: Vec<String> = epoch["envelopes"].as_array().unwrap().iter().map(|e| e["recipientKeyId"].as_str().unwrap().to_string()).collect();
        ids.sort();
        ids
    };
    // Whoever asks is told them in the one order: by person, then kind, then id.
    let order: Vec<(String, String, String)> = keys["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| (key["userId"].as_str().unwrap().to_string(), key["kind"].as_str().unwrap().to_string(), key["keyId"].as_str().unwrap().to_string()))
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
    let ada_device = keys["peer"]["devices"][0]["keyId"].as_str().unwrap().to_string();
    let stranger = Id::random().to_string();

    let stale = post_epoch(maja, &conversation, &epoch_body(&Id::random().to_string(), Some(&Id::random().to_string()), &every)).await;
    assert_eq!(refusal(&stale), (409, Some("epoch-stale".into())));
    assert!(stale.text().contains("\"status\":409,\"currentEpochId\":null,\"code\":\"epoch-stale\""), "{}", stale.text());

    let wrapped: Vec<String> = every.iter().filter(|key| **key != ada_device).cloned().chain([stranger.clone()]).collect();
    let uncovered = post_epoch(maja, &conversation, &epoch_body(&Id::random().to_string(), None, &wrapped)).await;
    assert_eq!(refusal(&uncovered), (409, Some("epoch-coverage".into())));
    let coverage: Value = uncovered.json();
    assert_eq!((coverage["missing"].clone(), coverage["extra"].clone()), (json!([ada_device]), json!([stranger])));
    assert!(uncovered.text().contains("\"missing\":["), "{}", uncovered.text());

    // The id of an epoch is only good in the one spelling browsers bind keys to.
    let id = Id::random();
    for spelled in [id.to_string().to_uppercase(), id.to_string().replace('-', ""), format!("{{{id}}}")] {
        let refused = post_epoch(maja, &conversation, &epoch_body(&spelled, None, &every)).await;
        assert_eq!(refused.status, 400, "{spelled}");
    }

    let first = Id::random().to_string();
    let created = post_epoch(maja, &conversation, &epoch_body(&first, None, &every)).await;
    assert_eq!(created.status, 201, "{}", created.text());
    assert_eq!(created.header("location"), Some(format!("/api/social/conversations/{conversation}/encryption/epochs").as_str()));
    let epoch: Value = created.json();
    assert_eq!((epoch["sequence"].clone(), epoch["previousEpochId"].clone()), (json!(1), Value::Null));
    assert_eq!(epoch["createdById"], crowd.maja.id.as_str());
    // The answer holds the envelopes of the keys of the person who made it, and no other.
    assert_eq!(envelope_ids(&epoch), own);
    assert_eq!(epoch["recipients"].as_array().unwrap().len(), 4);
    assert!(created.text().starts_with(&format!("{{\"id\":\"{first}\",\"sequence\":1,\"previousEpochId\":null,\"createdById\":\"")), "{}", created.text());
    // The person who made it gets the same answer again; another person does not.
    assert_eq!(post_epoch(maja, &conversation, &epoch_body(&first, None, &every)).await.status, 200);
    assert_eq!(refusal(&post_epoch(ada, &conversation, &epoch_body(&first, Some(&first), &every)).await), (409, Some("epoch-exists".into())));

    // Out of order: one that does not follow the current epoch is late.
    let late = post_epoch(ada, &conversation, &epoch_body(&Id::random().to_string(), None, &every)).await;
    assert_eq!(refusal(&late), (409, Some("epoch-stale".into())));
    assert_eq!(late.json::<Value>()["currentEpochId"], first.as_str());
    let second = Id::random().to_string();
    assert_eq!(post_epoch(ada, &conversation, &epoch_body(&second, Some(&first), &every)).await.status, 201);

    // The keys route hands the reader the envelopes of the current epoch for their own keys, and no others.
    let seen = crowd.keys(maja, &conversation).await;
    assert_eq!(seen["currentEpoch"]["recipients"].as_array().unwrap().len(), 4);
    assert_eq!(envelope_ids(&seen["currentEpoch"]), own);
    let ada_keys = crowd.keys(ada, &conversation).await;
    let mut ada_own = vec![ada_keys["self"]["accountKey"]["keyId"].as_str().unwrap().to_string(), ada_keys["currentDeviceKeyId"].as_str().unwrap().to_string()];
    ada_own.sort();
    assert_eq!(envelope_ids(&ada_keys["currentEpoch"]), ada_own);
    // Each reader gets the envelopes for their own account key and browser, and no others.
    let epochs = read(maja, &path(&conversation, "epochs")).await;
    assert_eq!(epochs["currentEpochId"], second.as_str());
    let ids: Vec<&str> = epochs["epochs"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [first.as_str(), second.as_str()]);
    for epoch in epochs["epochs"].as_array().unwrap() {
        assert_eq!(epoch["recipients"].as_array().unwrap().len(), 4);
        let mut got: Vec<String> = epoch["envelopes"].as_array().unwrap().iter().map(|e| e["recipientKeyId"].as_str().unwrap().to_string()).collect();
        got.sort();
        assert_eq!(got, own);
    }

    // Switching on again starts over from no epoch, and the old ones stay readable.
    assert_eq!(action(ada, &conversation, "disable").await.status, 200);
    assert_eq!(refusal(&post_epoch(maja, &conversation, &epoch_body(&Id::random().to_string(), Some(&second), &every)).await), (409, Some("encryption-off".into())));
    crowd.enable(&crowd.ada, &crowd.maja, &conversation).await;
    let third = Id::random().to_string();
    let restarted = post_epoch(maja, &conversation, &epoch_body(&third, None, &every)).await;
    assert_eq!(restarted.status, 201);
    assert_eq!(restarted.json::<Value>()["sequence"], 3);
    let later = read(ada, &format!("{}?after=1", path(&conversation, "epochs"))).await;
    let ids: Vec<&str> = later["epochs"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [second.as_str(), third.as_str()]);
    assert_eq!(later["hasMore"], false);
}

#[tokio::test]
async fn an_epoch_wrapped_for_a_key_of_nobody_in_the_conversation_is_refused_though_nothing_is_missing() {
    let crowd = Crowd::ready().await;
    let maja = &crowd.maja.client;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let mut every = crowd.recipients(maja, &conversation).await;
    let stranger = Id::random().to_string();
    every.push(stranger.clone());

    let refused = post_epoch(maja, &conversation, &epoch_body(&Id::random().to_string(), None, &every)).await;

    assert_eq!(refusal(&refused), (409, Some("epoch-coverage".into())));
    let body: Value = refused.json();
    assert_eq!((body["missing"].clone(), body["extra"].clone()), (json!([]), json!([stranger])));
}

#[tokio::test]
async fn what_an_epoch_says_of_the_keys_it_is_wrapped_for_is_checked_before_it_is_stored() {
    let crowd = Crowd::ready().await;
    let maja = &crowd.maja.client;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let every = crowd.recipients(maja, &conversation).await;
    let envelope = |key: &str, ephemeral: Value, iv: Value, ciphertext: Value| {
        json!({ "recipientKeyId": key, "ephemeralPublicKey": ephemeral, "iv": iv, "ciphertext": ciphertext })
    };
    let good = |key: &str| envelope(key, json!(text(&bytes(32, 7))), json!(text(&bytes(12, 8))), json!(text(&bytes(48, 9))));
    let body_of = |envelopes: Vec<Value>| json!({ "epochId": Id::random().to_string(), "previousEpochId": null, "envelopes": envelopes });
    let all_good = || every.iter().map(|key| good(key)).collect::<Vec<_>>();
    let with_first = |change: Value| {
        let mut envelopes = all_good();
        envelopes[0] = change;
        body_of(envelopes)
    };
    let status = |body: Value| {
        let (client, path) = (maja, path(&conversation, "epochs"));
        async move { post(client, &path, &body).await.status.as_u16() }
    };

    for (what, change) in [
        ("an iv one byte short", envelope(&every[0], json!(text(&bytes(32, 7))), json!(text(&bytes(11, 8))), json!(text(&bytes(48, 9))))),
        ("an iv one byte long", envelope(&every[0], json!(text(&bytes(32, 7))), json!(text(&bytes(13, 8))), json!(text(&bytes(48, 9))))),
        ("no iv", envelope(&every[0], json!(text(&bytes(32, 7))), Value::Null, json!(text(&bytes(48, 9))))),
        ("an empty iv", envelope(&every[0], json!(text(&bytes(32, 7))), json!(""), json!(text(&bytes(48, 9))))),
        ("a ciphertext that is only a tag", envelope(&every[0], json!(text(&bytes(32, 7))), json!(text(&bytes(12, 8))), json!(text(&bytes(16, 9))))),
        ("a ciphertext one byte too long", envelope(&every[0], json!(text(&bytes(32, 7))), json!(text(&bytes(12, 8))), json!(text(&bytes(257, 9))))),
        ("no ciphertext", envelope(&every[0], json!(text(&bytes(32, 7))), json!(text(&bytes(12, 8))), Value::Null)),
        ("no ephemeral key", envelope(&every[0], Value::Null, json!(text(&bytes(12, 8))), json!(text(&bytes(48, 9))))),
        ("an ephemeral key one byte short", envelope(&every[0], json!(text(&bytes(31, 7))), json!(text(&bytes(12, 8))), json!(text(&bytes(48, 9))))),
        ("an ephemeral key one byte long", envelope(&every[0], json!(text(&bytes(33, 7))), json!(text(&bytes(12, 8))), json!(text(&bytes(48, 9))))),
        ("an id spelled in capitals", good(&every[0].to_uppercase())),
        ("a key that is no id", good("not-an-id")),
        ("no key id", json!({ "ephemeralPublicKey": text(&bytes(32, 7)), "iv": text(&bytes(12, 8)), "ciphertext": text(&bytes(48, 9)) })),
        ("a hole in the list", Value::Null),
    ] {
        assert_eq!(status(with_first(change)).await, 400, "{what}");
    }
    // Exactly at the edges they are taken.
    let at_edges = |ciphertext: usize| {
        let mut envelopes = all_good();
        envelopes[0] = envelope(&every[0], json!(text(&bytes(32, 7))), json!(text(&bytes(12, 8))), json!(text(&bytes(ciphertext, 9))));
        body_of(envelopes)
    };
    let epoch = at_edges(17);
    assert_eq!(status(epoch).await, 201);
    let current = crowd.keys(maja, &conversation).await["encryption"]["currentEpochId"].as_str().unwrap().to_string();
    let mut longest = at_edges(256);
    longest["previousEpochId"] = json!(current);
    assert_eq!(status(longest).await, 201);

    // The same key twice, none, and too many.
    let mut twice = all_good();
    twice.push(good(&every[0]));
    assert_eq!(status(body_of(twice)).await, 400);
    assert_eq!(status(body_of(vec![])).await, 400);
    assert_eq!(status(json!({ "epochId": Id::random().to_string() })).await, 400);
    assert_eq!(status(json!({ "epochId": Id::random().to_string(), "envelopes": null })).await, 400);
    assert_eq!(status(json!({ "envelopes": all_good() })).await, 400);
    let crowded: Vec<Value> = (0..203).map(|_| good(&Id::random().to_string())).collect();
    assert_eq!(status(body_of(crowded)).await, 400);
    let crowded: Vec<Value> = (0..202).map(|_| good(&Id::random().to_string())).collect();
    assert_eq!(status(body_of(crowded)).await, 409, "202 are a coverage problem, not an invalid request");
    // A previous epoch that is not an id is not a body.
    let mut body = body_of(all_good());
    body["previousEpochId"] = json!("nope");
    let refused = post(maja, &path(&conversation, "epochs"), &body).await;
    assert_eq!((refused.status.as_u16(), refused.body.is_empty()), (400, true));
}

#[tokio::test]
async fn an_epoch_needs_every_key_to_be_the_kind_its_algorithm_takes() {
    let crowd = Crowd::ready().await;
    let maja = &crowd.maja.client;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let every = crowd.recipients(maja, &conversation).await;
    let mut body = epoch_body(&Id::random().to_string(), None, &every);

    body["envelopes"][2]["ephemeralPublicKey"] = json!(text(&bytes(33, 7)));

    assert_eq!(post_epoch(maja, &conversation, &body).await.status, 400);
}

#[tokio::test]
async fn the_epochs_are_read_a_page_at_a_time_after_a_sequence_number() {
    let crowd = Crowd::ready().await;
    let maja = &crowd.maja.client;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let every = crowd.recipients(maja, &conversation).await;
    // 201 epochs, as an earlier life of the conversation may have left.
    let connection = crowd.server.database();
    let id = Id::parse(&conversation).unwrap();
    let mut last = None;
    for sequence in 1..=201 {
        let epoch = Id::random();
        connection
            .execute(
                "INSERT INTO \"EncryptionEpochs\" (\"Id\", \"ConversationId\", \"Sequence\", \"PreviousEpochId\", \"CreatedById\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![epoch, id, sequence, last, Id::parse(&crowd.maja.id).unwrap(), Timestamp::now()],
            )
            .unwrap();
        last = Some(epoch);
    }
    connection
        .execute("UPDATE \"DirectConversations\" SET \"CurrentEpochId\" = ?2 WHERE \"Id\" = ?1", params![id, last])
        .unwrap();

    let page = read(maja, &path(&conversation, "epochs")).await;
    assert_eq!((page["epochs"].as_array().unwrap().len(), page["hasMore"].clone()), (200, json!(true)));
    let rest = read(maja, &format!("{}?after=200", path(&conversation, "epochs"))).await;
    assert_eq!((rest["epochs"].as_array().unwrap().len(), rest["hasMore"].clone()), (1, json!(false)));
    assert_eq!(rest["epochs"][0]["sequence"], 201);
    // The next one follows the highest.
    let current = last.unwrap().to_string();
    let next = post_epoch(maja, &conversation, &epoch_body(&Id::random().to_string(), Some(&current), &every)).await;
    assert_eq!(next.json::<Value>()["sequence"], 202);

    for (query, wanted) in [
        ("", 200),
        ("?after=202", 0),
        ("?AFTER=3", 200),
        ("?other=1", 200),
    ] {
        let found = read(maja, &format!("{}{query}", path(&conversation, "epochs"))).await;
        assert_eq!(found["epochs"].as_array().unwrap().len(), wanted, "{query}");
    }
    for query in ["?after=", "?after", "?after=%20", "?after=x", "?after=1.5", "?after=1&after=2", "?after=99999999999", "?after=-5", "?after=%2B3", "?after=+3", "?after=%203%20"] {
        let response = maja.get(&format!("{}{query}", path(&conversation, "epochs"))).await;
        assert_eq!(response.status.as_u16(), 400, "{query}");
        assert!(response.json::<Value>()["errors"]["after"].is_array(), "{query}");
    }
}

#[tokio::test]
async fn both_people_and_nobody_else_hear_about_switches_epochs_and_new_devices() {
    let crowd = Crowd::ready().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    let mut maja = listening(&crowd, &crowd.maja).await;
    let mut ada = listening(&crowd, &crowd.ada).await;
    let mut ilya = listening(&crowd, &crowd.ilya).await;

    assert_eq!(action(&crowd.maja.client, &conversation, "request").await.status, 200);
    let (to_maja, to_ada) = (told(&mut maja).await, told(&mut ada).await);
    assert_eq!(names(&to_ada), ["DirectMessageCreated", "DirectEncryptionChanged", "SocialChanged"]);
    assert_eq!(names(&to_maja), names(&to_ada));
    let line = &to_ada[0].1;
    assert_eq!(line["channelId"], conversation.as_str());
    assert_eq!(line["system"], json!({ "kind": "encryption_requested", "actorId": crowd.maja.id }));
    assert_eq!((line["content"].clone(), line["author"]["id"].clone(), line["author"]["presence"].clone()), (json!(""), json!(crowd.maja.id), json!("online")));
    assert_eq!(to_ada[1].1["encryption"]["state"], "requested");
    assert_eq!(to_ada[1].1["conversationId"], conversation.as_str());
    assert_eq!(to_ada[2].1, json!({}));

    assert_eq!(action(&crowd.ada.client, &conversation, "confirm").await.status, 200);
    assert_eq!(names(&told(&mut ada).await), ["DirectMessageCreated", "DirectEncryptionChanged", "SocialChanged"]);
    told(&mut maja).await;

    let epoch = rotate(&crowd, &crowd.maja.client, &conversation).await;
    let (to_maja, to_ada) = (told(&mut maja).await, told(&mut ada).await);
    for (events, who) in [(&to_maja, &crowd.maja), (&to_ada, &crowd.ada)] {
        assert_eq!(names(events), ["DirectEncryptionEpochCreated"]);
        let sent = &events[0].1;
        assert_eq!((sent["conversationId"].clone(), sent["epoch"]["id"].clone()), (json!(conversation), json!(epoch)));
        assert_eq!(sent["epoch"]["recipients"].as_array().unwrap().len(), 4);
        let mine = sent["epoch"]["envelopes"].as_array().unwrap();
        assert_eq!(mine.len(), 2, "the keys of {}", who.username);
    }

    let phone = crowd.server.sign_in(&crowd.maja).await;
    assert_eq!(register_device_with(&phone, &bytes(32, 70), "Phone").await.status, 201);
    let (to_maja, to_ada) = (told(&mut maja).await, told(&mut ada).await);
    assert_eq!(names(&to_ada), ["DirectMessageCreated", "DirectEncryptionKeysChanged", "SocialChanged"]);
    assert_eq!(names(&to_maja), ["DirectMessageCreated", "DirectEncryptionKeysChanged", "EncryptionAccountChanged", "SocialChanged"]);
    assert_eq!(to_ada[1].1, json!({ "conversationId": conversation, "userId": crowd.maja.id }));
    assert_eq!(to_ada[0].1["system"]["kind"], "device_added");

    // Registering the same key again is no news.
    assert_eq!(register_device_with(&phone, &bytes(32, 70), "Phone").await.status, 200);
    assert!(told(&mut maja).await.is_empty() && told(&mut ada).await.is_empty());

    // Removing its keys is told without a line.
    let device = read(&phone, ACCOUNT).await["device"]["id"].as_str().unwrap().to_string();
    assert_eq!(delete(&crowd.maja.client, &format!("{ACCOUNT}/devices/{device}")).await.status, 204);
    assert_eq!(names(&told(&mut ada).await), ["DirectEncryptionKeysChanged"]);
    assert_eq!(names(&told(&mut maja).await), ["DirectEncryptionKeysChanged", "EncryptionAccountChanged"]);

    assert!(told(&mut ilya).await.is_empty(), "somebody outside the conversation learns nothing of it");
}

#[tokio::test]
async fn the_account_is_told_of_its_own_changes_and_nobody_else_is() {
    let crowd = Crowd::new().await;
    let mut maja = listening(&crowd, &crowd.maja).await;
    let mut ada = listening(&crowd, &crowd.ada).await;

    let account = set_up(&crowd.maja.client).await;
    assert_eq!(names(&told(&mut maja).await), ["EncryptionAccountChanged"]);
    assert_eq!(told(&mut maja).await.len(), 0);
    register_device(&crowd.maja.client).await;
    assert_eq!(names(&told(&mut maja).await), ["EncryptionAccountChanged"]);
    let new_pin = json!({ "currentPassword": crate::support::PASSWORD, "accountKeyId": account.key_id, "backup": backup(&bytes(32, 1)) });
    assert_eq!(put(&crowd.maja.client, "/api/me/encryption/backup", &new_pin).await.status, 200);
    assert_eq!(names(&told(&mut maja).await), ["EncryptionAccountChanged"]);
    // A wrong PIN is not news.
    let wrong = post(&crowd.maja.client, "/api/me/encryption/backup/unlock", &json!({ "authKey": text(&bytes(32, 9)) })).await;
    assert_eq!(wrong.status, 403);
    assert!(told(&mut maja).await.is_empty());
    let reset = json!({ "currentPassword": crate::support::PASSWORD, "algorithm": "x25519", "publicKey": text(&bytes(32, 3)), "backup": backup(&bytes(32, 1)) });
    assert_eq!(post(&crowd.maja.client, "/api/me/encryption/account-key/reset", &reset).await.status, 200);
    // A reset says "look again" to the friends screen, always.
    assert_eq!(names(&told(&mut maja).await), ["EncryptionAccountChanged", "SocialChanged"]);
    assert!(told(&mut ada).await.is_empty());
}

#[tokio::test]
async fn a_new_sign_in_is_not_announced_across_a_block_but_stays_in_the_history() {
    let crowd = Crowd::ready().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let mut ada = listening(&crowd, &crowd.ada).await;
    block(&crowd, &crowd.ada, &crowd.maja);

    let laptop = crowd.server.sign_in(&crowd.maja).await;
    assert_eq!(register_device_with(&laptop, &bytes(32, 80), "Laptop").await.status, 201);
    let phone = crowd.server.sign_in(&crowd.ada).await;
    assert_eq!(register_device_with(&phone, &bytes(32, 81), "Phone").await.status, 201);

    // Ada's own change reaches her; what Maja's sign-in sent does not.
    assert_eq!(names(&told(&mut ada).await), ["EncryptionAccountChanged"]);
    assert_eq!(
        crowd.timeline(&conversation),
        ["encryption_requested:maja", "encryption_enabled:ada", "device_added:maja", "device_added:ada"]
    );
}
