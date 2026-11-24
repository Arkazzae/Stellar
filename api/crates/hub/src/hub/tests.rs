use super::*;

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
            let value: Value = serde_json::from_str(text.trim_end_matches('\u{1e}')).unwrap();
            value["target"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn a_message_is_a_server_invocation_record() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    assert!(hub.send_to_connection("a", &event("Ping")));
    assert_eq!(
        queued(&a),
        ["{\"type\":1,\"target\":\"Ping\",\"arguments\":[]}\u{1e}"]
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
    assert!(a.close_request().is_none());
    assert!(!hub.send_to_connection("a", &event("one too many")));
    let request = a.close_request().unwrap();
    assert!(request.allow_reconnect);
    assert!(request.error.is_some());
    // What was queued stays; nothing more is added; others are unaffected.
    assert_eq!(names(&a).len(), 4);
    assert!(!hub.send_to_connection("a", &event("still refused")));
    assert!(hub.send_to_connection("b", &event("fine")));
    assert!(b.close_request().is_none());
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
    assert!(a.close_request().is_some());
}

#[test]
fn one_message_bigger_than_the_allowance_is_still_delivered_to_an_idle_client() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    let huge = Outbound::single("Huge", &"x".repeat(5000)).unwrap();
    assert!(hub.send_to_connection("a", &huge));
    assert!(a.close_request().is_none());
}

#[test]
fn closing_is_decided_once() {
    let hub = Hub::new(&config());
    let a = connect(&hub, "a");
    assert!(hub.abort("a"));
    assert!(hub.close(
        "a",
        CloseRequest {
            error: Some("later".into()),
            allow_reconnect: true
        }
    ));
    assert_eq!(
        a.close_request(),
        Some(CloseRequest {
            error: None,
            allow_reconnect: false
        })
    );
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
        assert_eq!(
            shared.close_request(),
            Some(CloseRequest {
                error: None,
                allow_reconnect: true
            })
        );
    }
}

#[tokio::test]
async fn take_waits_for_a_message_and_batches_what_fits() {
    let hub = Hub::new(&Config::default());
    let a = connect(&hub, "a");
    let waiting = {
        let a = a.clone();
        tokio::spawn(async move {
            let mut out = Vec::new();
            a.take(&mut out, 1 << 16).await;
            out
        })
    };
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    hub.send_to_connection("a", &event("first"));
    let out = waiting.await.unwrap();
    assert_eq!(out.len(), 1);
    for n in 0..5 {
        hub.send_to_connection("a", &event(&format!("m{n}")));
    }
    let one = out[0].len();
    let mut out = Vec::new();
    a.take(&mut out, one * 3).await;
    assert_eq!(out.len(), 3, "only what fits in the limit");
    a.take(&mut out, 1).await;
    assert_eq!(out.len(), 4, "at least one, however small the limit");
}

#[tokio::test]
async fn a_message_sent_while_the_writer_is_busy_is_not_forgotten() {
    let hub = Hub::new(&Config::default());
    let a = connect(&hub, "a");
    hub.send_to_connection("a", &event("one"));
    hub.send_to_connection("a", &event("two"));
    let mut out = Vec::new();
    a.take(&mut out, 1).await;
    assert_eq!(out.len(), 1);
    // The second must still wake the next waiter even though no new push came.
    tokio::time::timeout(std::time::Duration::from_secs(1), a.take(&mut out, 1))
        .await
        .unwrap();
    assert_eq!(out.len(), 2);
}
