use super::*;
use serde_json::{Value, json};

fn config() -> Config {
    Config {
        max_queued_messages: 4,
        max_queued_bytes: 1000,
        ..Config::default()
    }
}

fn connect(hub: &Hub, id: &str) -> Arc<Shared> {
    hub.register(id.into())
}

fn queued(shared: &Shared) -> Vec<String> {
    let state = lock(&shared.outbox.state);
    state
        .queue
        .iter()
        .map(|record| String::from_utf8(record.to_vec()).unwrap())
        .collect()
}

fn event(name: &str) -> Outbound {
    Outbound::bare(name)
}

fn names(shared: &Shared) -> Vec<String> {
    queued(shared)
        .iter()
        .map(|text| {
            let value: Value = serde_json::from_str(text).unwrap();
            value["name"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn a_bare_event_carries_null() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    assert!(hub.send_to_connection("a", &event("Ping")));
    assert_eq!(
        serde_json::from_str::<Value>(&queued(&a)[0]).unwrap(),
        json!({"type":"event","name":"Ping","data":null})
    );
}

#[test]
fn sending_to_one_connection_and_to_several() {
    let hub = Hub::new(&config());
    let (a, b, c) = (connect(&hub, "a"), connect(&hub, "b"), connect(&hub, "c"));
    assert!(hub.send_to_connection("b", &event("one")));
    assert!(!hub.send_to_connection("nobody", &event("lost")));
    assert_eq!(
        hub.send_to_connections(["a", "c", "nobody", "a"], &event("two")),
        2,
        "each once, the missing skipped"
    );
    assert_eq!(names(&a), ["two"]);
    assert_eq!(names(&b), ["one"]);
    assert_eq!(names(&c), ["two"]);
}

#[test]
fn group_sends_reach_members_only() {
    let hub = Hub::new(&config());
    let (a, b, c) = (connect(&hub, "a"), connect(&hub, "b"), connect(&hub, "c"));
    assert!(hub.add_to_group("a", "room"));
    assert!(hub.add_to_group("b", "room"));
    assert!(hub.add_to_group("b", "room"), "joining twice is harmless");
    assert_eq!(hub.send_to_group("room", &event("hi")), 2);
    assert_eq!(hub.send_to_group("empty", &event("nobody")), 0);
    assert_eq!(names(&a), ["hi"]);
    assert_eq!(names(&b), ["hi"]);
    assert!(names(&c).is_empty());
}

#[test]
fn several_groups_deliver_once_to_a_connection_in_more_than_one() {
    let hub = Hub::new(&config());
    let (a, b) = (connect(&hub, "a"), connect(&hub, "b"));
    for (id, group) in [("a", "g1"), ("a", "g2"), ("b", "g2")] {
        hub.add_to_group(id, group);
    }
    assert_eq!(hub.send_to_groups(["g1", "g2", "g3"], &event("x")), 2);
    assert_eq!((names(&a).len(), names(&b).len()), (1, 1));
}

#[test]
fn group_except_leaves_the_named_connections_out() {
    let hub = Hub::new(&config());
    let (a, b, c) = (connect(&hub, "a"), connect(&hub, "b"), connect(&hub, "c"));
    for id in ["a", "b", "c"] {
        hub.add_to_group(id, "room");
    }
    assert_eq!(
        hub.send_to_group_except("room", &["b"], &event("others")),
        2
    );
    assert_eq!(
        hub.send_to_group_except("room", &["a", "b", "c", "zzz"], &event("nobody")),
        0
    );
    assert_eq!(
        (names(&a).len(), names(&b).len(), names(&c).len()),
        (1, 0, 1)
    );
}

#[test]
fn send_to_all_reaches_every_connection() {
    let hub = Hub::new(&config());
    let all: Vec<_> = ["a", "b", "c"].iter().map(|id| connect(&hub, id)).collect();
    assert_eq!(hub.send_to_all(&event("everyone")), 3);
    assert!(all.iter().all(|shared| names(shared) == ["everyone"]));
    assert_eq!(hub.connection_count(), 3);
}

#[test]
fn leaving_a_group_stops_its_messages() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    hub.add_to_group("a", "room");
    assert!(hub.remove_from_group("a", "room"));
    assert!(!hub.remove_from_group("a", "room"), "already out");
    assert!(!hub.remove_from_group("nobody", "room"));
    assert_eq!(hub.send_to_group("room", &event("late")), 0);
    assert!(names(&a).is_empty());
    assert!(
        hub.group_members("room").is_empty(),
        "an empty group is forgotten"
    );
}

#[test]
fn a_disconnected_connection_leaves_every_group_and_gets_nothing() {
    let hub = Hub::new(&config());
    let _a = connect(&hub, "a");
    let b = connect(&hub, "b");
    for group in ["g1", "g2"] {
        hub.add_to_group("a", group);
        hub.add_to_group("b", group);
    }
    hub.unregister("a");
    assert!(!hub.is_connected("a"));
    assert!(
        !hub.add_to_group("a", "g3"),
        "a gone connection cannot join"
    );
    assert_eq!(hub.group_members("g1"), ["b"]);
    assert_eq!(hub.send_to_groups(["g1", "g2"], &event("x")), 1);
    assert_eq!(names(&b), ["x"]);
    hub.unregister("a");
    hub.unregister("b");
    assert_eq!(hub.connection_count(), 0);
    assert!(hub.group_members("g1").is_empty());
}

#[test]
fn messages_to_a_connection_keep_the_order_they_were_sent_in() {
    let hub = Hub::new(&Config::default());
    let a = connect(&hub, "a");
    for n in 0..50 {
        hub.send_to_connection("a", &event(&format!("m{n}")));
    }
    let expected: Vec<String> = (0..50).map(|n| format!("m{n}")).collect();
    assert_eq!(names(&a), expected);
}

#[test]
fn a_connection_that_falls_behind_by_count_is_closed_and_may_reconnect() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    let b = connect(&hub, "b");
    for n in 0..4 {
        assert!(hub.send_to_connection("a", &event(&format!("m{n}"))));
    }
    assert!(a.close_signal().is_none());
    assert!(!hub.send_to_connection("a", &event("one too many")));
    let request = a.close_signal().unwrap();
    assert_eq!(request.code, 1013);
    assert!(request.error.is_some());
    // What was queued stays; nothing more is added; others are unaffected.
    assert_eq!(names(&a).len(), 4);
    assert!(!hub.send_to_connection("a", &event("still refused")));
    assert!(hub.send_to_connection("b", &event("fine")));
    assert!(b.close_signal().is_none());
}

#[test]
fn a_connection_that_falls_behind_by_bytes_is_closed() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    let big = Outbound::single("Big", &"x".repeat(400)).unwrap();
    assert!(hub.send_to_connection("a", &big));
    assert!(hub.send_to_connection("a", &big));
    assert!(
        !hub.send_to_connection("a", &big),
        "a third would pass 1000 bytes"
    );
    assert!(a.close_signal().is_some());
}

#[test]
fn one_message_bigger_than_the_allowance_is_still_delivered_to_an_idle_client() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    let huge = Outbound::single("Huge", &"x".repeat(5000)).unwrap();
    assert!(hub.send_to_connection("a", &huge));
    assert!(a.close_signal().is_none());
}

#[test]
fn closing_is_decided_once() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    assert!(hub.abort("a"));
    assert!(hub.end_session("a"));
    assert_eq!(a.close_signal().unwrap().code, 1000);
    assert!(!hub.abort("nobody"));
    assert!(
        !hub.send_to_connection("a", &event("after")),
        "a closing connection takes nothing more"
    );
}

#[test]
fn closing_everyone_asks_every_client_to_start_over() {
    let hub = Hub::new(&config());
    let all: Vec<_> = ["a", "b"].iter().map(|id| connect(&hub, id)).collect();
    hub.close_all();
    for shared in all {
        assert_eq!(shared.close_signal().unwrap().code, 1012);
    }
}

#[tokio::test]
async fn take_waits_for_a_message() {
    let hub = Hub::new(&Config::default());
    let a = connect(&hub, "a");
    let waiting = {
        let a = a.clone();
        tokio::spawn(async move { a.take().await })
    };
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    let first = event("first");
    hub.send_to_connection("a", &first);
    assert_eq!(waiting.await.unwrap(), first.bytes);
}

#[tokio::test]
async fn a_message_sent_while_the_writer_is_busy_is_not_forgotten() {
    let hub = Hub::new(&Config::default());
    let a = connect(&hub, "a");
    let first = event("one");
    let second = event("two");
    hub.send_to_connection("a", &first);
    hub.send_to_connection("a", &second);
    assert_eq!(a.take().await, first.bytes);
    let sent = tokio::time::timeout(std::time::Duration::from_secs(1), a.take())
        .await
        .unwrap();
    assert_eq!(sent, second.bytes);
}

#[tokio::test]
async fn native_presence_coalesces_only_consecutive_snapshots_for_the_same_user() {
    let hub = Hub::new(&Config {
        max_queued_messages: 5,
        ..config()
    });
    let native = connect(&hub, "native");
    let snapshot = |user: &str, presence: &str| {
        Outbound::single(
            "PresenceChanged",
            &serde_json::json!({"userId": user, "presence": presence}),
        )
        .unwrap()
    };

    let first = snapshot("a", "idle");
    let replacement = snapshot("a", "online");
    for message in [&first, &replacement] {
        assert_eq!(hub.send_to_all(message), 1);
    }
    let after_other_user = snapshot("a", "offline");
    for message in [&snapshot("b", "idle"), &after_other_user, &event("Barrier")] {
        assert!(hub.send_to_connection("native", message));
    }
    assert!(hub.send_to_connection("native", &snapshot("a", "dnd")));
    assert!(hub.send_to_connection("native", &snapshot("a", "online")));
    assert_eq!(queued(&native).len(), 5);
    {
        let state = lock(&native.outbox.state);
        assert_eq!(
            state.bytes,
            state.queue.iter().map(Bytes::len).sum::<usize>()
        );
    }
    let mut taken = Vec::new();
    for _ in 0..5 {
        taken.push(native.take().await);
    }
    assert_eq!(taken[0], replacement.bytes);
    assert_eq!(taken[2], after_other_user.bytes);
    assert_eq!(taken[4], replacement.bytes);

    // Taking the last snapshot clears its key for the now empty queue.
    assert!(hub.send_to_connection("native", &first));
    assert_eq!(queued(&native).len(), 1);
    let partial = Outbound::single("PresenceChanged", &serde_json::json!({"userId": "a"})).unwrap();
    assert!(hub.send_to_connection("native", &partial));
    assert!(hub.send_to_connection("native", &replacement));
    assert_eq!(queued(&native).len(), 3, "partial snapshots are barriers");
}

#[test]
fn outbound_keeps_the_application_payload_member_order() {
    let raw =
        RawValue::from_string(r#"{"z":123456789012345678901234567890,"a":2}"#.to_owned()).unwrap();
    let message = Outbound::single("Update", &raw).unwrap();
    assert!(String::from_utf8_lossy(message.as_bytes()).contains(raw.get()));
}
