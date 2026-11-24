//! A connection from startup to close, with the hub driven over an
//! in-memory WebSocket.

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use stellarr_hub::{Config, Hub};
use stellarr_websocket::frame::{OP_CLOSE, OP_CONTINUATION, OP_PING, OP_PONG, OP_TEXT};
use support::*;

fn quick() -> Config {
    Config {
        close_grace: Duration::from_millis(500),
        ..Config::default()
    }
}

#[tokio::test]
async fn ready_arrives_before_anything_the_application_sends() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    assert_eq!(
        r.client.json().await,
        json!({"type":"event","name":"Hello","data":"ann"})
    );
    assert_eq!(r.app.lines(), ["connected ann"]);
}

#[tokio::test]
async fn a_connect_hook_that_exceeds_its_deadline_closes_before_ready() {
    let config = Config {
        startup_timeout: Duration::from_millis(50),
        ..quick()
    };
    let app = TestApp {
        connect_delay: Duration::from_secs(1),
        ..Default::default()
    };
    let mut r = start(app, config, "ann");
    let (messages, code) = r.client.until_close().await;
    assert!(messages.is_empty());
    assert_eq!(code, Some(1011));
    r.task.await.unwrap();
    assert_eq!(r.hub.connection_count(), 0);
    assert!(r.app.lines()[0].starts_with("disconnected ann "));
}

#[tokio::test]
async fn nothing_the_client_sends_is_read_until_the_connect_hook_has_finished() {
    let app = TestApp {
        connect_delay: Duration::from_millis(150),
        ..Default::default()
    };
    let mut r = start(app, quick(), "ann");
    r.client.invoke(1, "whoami", json!([])).await;
    r.client.ready().await;
    assert_eq!(r.client.json().await["name"], "Hello");
    assert_eq!(
        r.client.json().await,
        json!({"type":"result","id":1,"value":"ann"})
    );
    assert_eq!(r.app.lines(), ["connected ann"]);
}

#[tokio::test]
async fn a_call_with_a_result_a_void_call_and_the_ones_that_fail() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    let cases = [
        (
            1,
            "echo",
            json!(["hi"]),
            json!({"type": "result", "id": 1, "value": "hi"}),
        ),
        (
            2,
            "void",
            json!([]),
            json!({"type": "result", "id": 2, "value": null}),
        ),
        (
            3,
            "fail",
            json!([]),
            json!({"type":"error","id":3,"code":"visible","message":"Not allowed here."}),
        ),
        (
            4,
            "Nope",
            json!([]),
            json!({"type":"error","id":4,"code":"unknown_method"}),
        ),
        (
            5,
            "echo",
            json!([]),
            json!({"type":"error","id":5,"code":"invalid_arguments"}),
        ),
        (
            6,
            "echo",
            json!([1, 2]),
            json!({"type":"error","id":6,"code":"invalid_arguments"}),
        ),
        (
            7,
            "internal",
            json!([]),
            json!({"type":"error","id":7,"code":"internal"}),
        ),
        (
            8,
            "panic",
            json!([]),
            json!({"type":"error","id":8,"code":"internal"}),
        ),
        (
            9,
            "ECHO",
            json!([{"a": [1, null]}]),
            json!({"type": "result", "id": 9, "value": {"a": [1, null]}}),
        ),
    ];
    for (id, target, arguments, expected) in cases {
        r.client.invoke(id, target, arguments).await;
        assert_eq!(r.client.json().await, expected, "{target}");
    }
    // The connection survived the panic.
    r.client.invoke(10, "whoami", json!([])).await;
    assert_eq!(r.client.json().await["value"], "ann");
}

#[tokio::test]
async fn a_send_has_no_answer_even_when_it_fails() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    for method in ["fail", "nope", "void"] {
        r.client
            .send_json(&json!({"type":"send","method":method,"args":[]}))
            .await;
    }
    r.client.invoke(1, "echo", json!([2])).await;
    assert_eq!(
        r.client.json().await,
        json!({"type":"result","id":1,"value":2})
    );
}

#[tokio::test]
async fn a_fragmented_call_keeps_its_order_with_the_next_message() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    let one = br#"{"type":"call","id":1,"method":"echo","args":["a"]}"#;
    r.client.fragment(false, OP_TEXT, &one[..20]).await;
    r.client.fragment(true, OP_CONTINUATION, &one[20..]).await;
    r.client.invoke(2, "echo", json!(["b"])).await;
    assert_eq!(
        r.client.json().await,
        json!({"type":"result","id":1,"value":"a"})
    );
    assert_eq!(
        r.client.json().await,
        json!({"type":"result","id":2,"value":"b"})
    );
}

#[tokio::test]
async fn a_client_calls_run_in_order_even_when_the_invocation_queue_fills() {
    let config = Config {
        invocation_queue: 1,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.ready().await;
    r.client.json().await;
    for id in 1..=5 {
        r.client.invoke(id, "slow", json!([id])).await;
    }
    for id in 1..=5 {
        assert_eq!(
            r.client.json().await,
            json!({"type":"result","id":id,"value":id})
        );
    }
    assert_eq!(r.app.max_running.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn malformed_messages_end_the_connection_without_an_application_reply() {
    for bad in [
        "{not json",
        "[1]",
        r#"{"id":1,"method":"echo","args":[]}"#,
        r#"{"type":"call","id":1,"args":[]}"#,
        r#"{"type":"call","id":1,"method":"echo","args":{}}"#,
        r#"{"type":"call","id":"7","method":"echo","args":[]}"#,
        "",
    ] {
        let mut r = start(TestApp::default(), quick(), "ann");
        r.client.ready().await;
        r.client.json().await;
        r.client.send(bad).await;
        let (messages, code) = r.client.until_close().await;
        assert!(messages.is_empty(), "{bad:?}");
        assert_eq!(code, Some(1002), "{bad:?}");
        r.task.await.unwrap();
        assert!(
            r.app
                .lines()
                .last()
                .unwrap()
                .starts_with("disconnected ann ")
        );
        assert_eq!(r.hub.connection_count(), 0);
    }
}

#[tokio::test]
async fn the_message_size_limit_includes_every_byte_of_the_json() {
    let limit = 400;
    let base_len = json!({"type":"call","id":1,"method":"echo","args":[""]})
        .to_string()
        .len();
    let message = |size| {
        json!({
            "type":"call","id":1,"method":"echo","args":["x".repeat(size - base_len)]
        })
        .to_string()
    };
    let mut r = start(
        TestApp::default(),
        Config {
            max_message_size: limit,
            ..quick()
        },
        "ann",
    );
    r.client.ready().await;
    r.client.json().await;
    let fits = message(limit);
    assert_eq!(fits.len(), limit);
    r.client.send(&fits).await;
    assert_eq!(
        r.client.json().await["value"].as_str().unwrap().len(),
        limit - base_len
    );
    r.client.send(&message(limit + 1)).await;
    assert_eq!(r.client.until_close().await, (vec![], Some(1009)));
}

#[tokio::test]
async fn an_unfinished_fragmented_message_is_cut_off_at_the_limit() {
    let mut r = start(
        TestApp::default(),
        Config {
            max_message_size: 1000,
            ..quick()
        },
        "ann",
    );
    r.client.ready().await;
    r.client.json().await;
    r.client.fragment(false, OP_TEXT, &[b' '; 500]).await;
    r.client
        .fragment(false, OP_CONTINUATION, &[b' '; 500])
        .await;
    r.client.fragment(false, OP_CONTINUATION, b"x").await;
    assert_eq!(r.client.until_close().await, (vec![], Some(1009)));
}

#[tokio::test]
async fn a_websocket_close_from_the_client_is_an_orderly_disconnect() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    r.client.send_frame(OP_CLOSE, &1000u16.to_be_bytes()).await;
    let (frame_op, _) = r.client.frame().await.unwrap();
    assert_eq!(frame_op, OP_CLOSE);
    r.task.await.unwrap();
    assert_eq!(r.app.lines(), ["connected ann", "disconnected ann -"]);
}

#[tokio::test]
async fn a_client_that_drops_without_a_word_is_reported_as_an_error_disconnect() {
    let r = start(TestApp::default(), quick(), "ann");
    let Running {
        mut client,
        app,
        hub,
        task,
    } = r;
    client.ready().await;
    client.json().await;
    drop(client);
    task.await.unwrap();
    let lines = app.lines();
    assert_eq!(lines[0], "connected ann");
    assert!(
        lines[1].starts_with("disconnected ann ") && !lines[1].ends_with(" -"),
        "{lines:?}"
    );
    assert_eq!(hub.connection_count(), 0);
}

#[tokio::test]
async fn the_application_can_end_a_connection_and_the_client_is_told_not_to_return() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    assert!(r.hub.abort("conn-ann"));
    let (records, code) = r.client.until_close().await;
    assert!(records.is_empty());
    assert_eq!(code, Some(1000));
    r.task.await.unwrap();
    assert_eq!(r.app.lines(), ["connected ann", "disconnected ann -"]);
    assert!(!r.hub.abort("conn-ann"), "it is gone");
}

#[tokio::test]
async fn a_method_may_end_its_own_connection() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    r.client.invoke(1, "abortme", json!([])).await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(
        records,
        Vec::<Value>::new(),
        "the answer is not sent to a connection that is ending"
    );
    assert_eq!(code, Some(1000));
}

#[tokio::test]
async fn restarting_and_ending_a_session_send_distinct_close_codes() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    r.hub.close_all();
    assert_eq!(r.client.until_close().await, (vec![], Some(1012)));
    r.task.await.unwrap();

    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    assert!(r.hub.end_session("conn-ann"));
    assert_eq!(r.client.until_close().await, (vec![], Some(4401)));
    r.task.await.unwrap();
}

#[tokio::test]
async fn a_connect_hook_that_fails_closes_the_connection_and_the_disconnect_hook_runs() {
    let app = TestApp {
        fail_connect: true,
        ..Default::default()
    };
    let mut r = start(app, quick(), "ann");
    let (messages, code) = r.client.until_close().await;
    assert!(messages.is_empty());
    assert_eq!(code, Some(1011));
    r.task.await.unwrap();
    assert_eq!(r.app.lines().len(), 2);
    assert!(r.app.lines()[1].starts_with("disconnected ann"));
}

#[tokio::test]
async fn a_connect_hook_that_panics_still_leaves_nothing_behind() {
    let app = TestApp {
        panic_connect: true,
        ..Default::default()
    };
    let mut r = start(app, quick(), "ann");
    let (messages, code) = r.client.until_close().await;
    assert!(messages.is_empty());
    assert_eq!(code, Some(1011));
    r.task.await.unwrap();
    assert_eq!(r.hub.connection_count(), 0);
    assert!(
        r.app
            .lines()
            .last()
            .unwrap()
            .starts_with("disconnected ann")
    );
}

#[tokio::test]
async fn events_reach_the_group_and_the_caller_is_left_out_of_others() {
    let config = quick();
    let hub = Hub::new(&config);
    let app = std::sync::Arc::new(TestApp::default());
    let mut ann = start_on(
        app.clone(),
        hub.clone(),
        config.clone(),
        "ann",
        "conn-ann".into(),
    );
    let mut bob = start_on(
        app.clone(),
        hub.clone(),
        config.clone(),
        "bob",
        "conn-bob".into(),
    );
    for client in [&mut ann.client, &mut bob.client] {
        client.ready().await;
        assert_eq!(client.json().await["name"], "Hello");
    }
    ann.client
        .invoke(1, "say", json!(["all", "hello all"]))
        .await;
    assert_eq!(
        ann.client.json().await,
        json!({"type":"event","name":"Said","data":"hello all"})
    );
    assert_eq!(ann.client.json().await["value"], Value::Null);
    assert_eq!(
        bob.client.json().await,
        json!({"type":"event","name":"Said","data":"hello all"})
    );

    ann.client
        .invoke(2, "say", json!(["user:bob", "just bob"]))
        .await;
    assert_eq!(ann.client.json().await["id"], 2, "ann gets only her answer");
    assert_eq!(bob.client.json().await["data"], "just bob");

    ann.client.invoke(3, "others", json!([])).await;
    assert_eq!(ann.client.json().await["id"], 3);
    assert_eq!(bob.client.json().await["name"], "Others");
    assert_eq!(hub.group_members("all").len(), 2);
    assert_eq!(hub.connection_count(), 2);

    // A connection that leaves takes its group memberships with it.
    drop(bob.client);
    bob.task.await.unwrap();
    assert_eq!(hub.group_members("all"), ["conn-ann"]);
    assert!(hub.group_members("user:bob").is_empty());
}

#[tokio::test]
async fn a_very_large_answer_is_written_whole() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.ready().await;
    r.client.json().await;
    r.client.invoke(1, "big", json!([])).await;
    let answer = r.client.json().await;
    assert_eq!(answer["value"].as_str().unwrap().len(), 200_000);
}

#[tokio::test]
async fn the_server_pings_a_quiet_client_at_both_levels() {
    let config = Config {
        keep_alive_interval: Duration::from_millis(60),
        transport_ping_interval: Duration::from_millis(100),
        client_timeout: None,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.ready().await;
    r.client.json().await;
    assert_eq!(r.client.json().await, json!({"type":"ping"}));
    let (op, ping) = r.client.frame().await.unwrap();
    assert_eq!(op, OP_PING);
    r.client.send_frame(OP_PONG, &ping).await;
    r.client.invoke(1, "whoami", json!([])).await;
    loop {
        let message = r.client.json().await;
        if message["type"] == "result" {
            assert_eq!(message, json!({"type":"result","id":1,"value":"ann"}));
            break;
        }
        assert_eq!(message, json!({"type":"ping"}));
    }
}

#[tokio::test]
async fn outgoing_events_postpone_the_idle_heartbeat_but_not_transport_pings() {
    let config = Config {
        keep_alive_interval: Duration::from_millis(400),
        transport_ping_interval: Duration::from_millis(100),
        client_timeout: None,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.ready().await;
    r.client.json().await;
    for n in 0..4 {
        let (op, ping) = r.client.frame().await.unwrap();
        assert_eq!(op, OP_PING);
        r.client.send_frame(OP_PONG, &ping).await;
        let event = format!("E{n}");
        assert!(
            r.hub
                .send_to_connection("conn-ann", &stellarr_hub::Outbound::bare(&event))
        );
        assert_eq!(
            r.client.json().await,
            json!({"type":"event","name":event,"data":null})
        );
    }
    assert_eq!(r.client.json().await, json!({"type":"ping"}));
}

#[tokio::test]
async fn a_client_that_says_nothing_at_all_is_closed_after_the_timeout() {
    let config = Config {
        client_timeout: Some(Duration::from_millis(150)),
        keep_alive_interval: Duration::from_secs(60),
        transport_ping_interval: Duration::from_secs(60),
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.ready().await;
    r.client.json().await;
    let (records, code) = r.client.until_close().await;
    assert!(records.is_empty());
    assert_eq!(code, Some(1001));
    r.task.await.unwrap();
}

#[tokio::test]
async fn any_frame_from_the_client_keeps_it_from_timing_out() {
    let config = Config {
        client_timeout: Some(Duration::from_millis(200)),
        keep_alive_interval: Duration::from_secs(60),
        transport_ping_interval: Duration::from_secs(60),
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.ready().await;
    r.client.json().await;
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        r.client.send_frame(OP_PONG, b"").await;
    }
    r.client.invoke(1, "whoami", json!([])).await;
    assert_eq!(r.client.json().await["value"], "ann");
}

#[tokio::test]
async fn an_outbound_queue_overflow_closes_with_a_retry_code() {
    let config = Config {
        max_queued_messages: 2,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.ready().await;
    r.client.json().await;

    let event = stellarr_hub::Outbound::bare("Burst");
    assert!(r.hub.send_to_connection("conn-ann", &event));
    assert!(r.hub.send_to_connection("conn-ann", &event));
    assert!(!r.hub.send_to_connection("conn-ann", &event));
    assert_eq!(r.client.until_close().await, (vec![], Some(1013)));
    r.task.await.unwrap();
    assert_eq!(r.hub.connection_count(), 0);
}

#[tokio::test]
async fn a_client_that_does_not_read_is_closed_when_its_queue_is_full() {
    // A one-kilobyte pipe the client never reads from: the writer blocks,
    // the queue fills, and the connection is given up on.
    let config = Config {
        max_queued_messages: 8,
        write_timeout: Duration::from_millis(200),
        close_grace: Duration::from_millis(200),
        ..Config::default()
    };
    let hub = Hub::new(&config);
    let app = std::sync::Arc::new(TestApp::default());
    let (client_io, server_io) = tokio::io::duplex(1024);
    let (sender, receiver) = stellarr_websocket::split(
        server_io,
        stellarr_websocket::Config {
            max_message_size: usize::MAX,
            ..Default::default()
        },
    );
    let task = tokio::spawn(stellarr_hub::serve(
        app.clone(),
        hub.clone(),
        std::sync::Arc::new(config),
        std::sync::Arc::new("ann".to_owned()),
        "slow".into(),
        sender,
        receiver,
    ));
    let mut client = Client::wrap(client_io);
    client.ready().await;
    // Never read again. Everything sent to it is queued, then refused.
    let big = stellarr_hub::Outbound::single("Fill", &"x".repeat(2000)).unwrap();
    let mut accepted = 0;
    for _ in 0..200 {
        if hub.send_to_connection("slow", &big) {
            accepted += 1;
        }
        tokio::task::yield_now().await;
    }
    assert!(accepted < 200, "the queue is bounded");
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the connection was given up on")
        .unwrap();
    assert_eq!(hub.connection_count(), 0);
    assert!(app.lines().last().unwrap().starts_with("disconnected ann"));
}

#[tokio::test]
async fn a_write_that_cannot_finish_ends_the_connection_before_any_queue_fills() {
    // A tiny pipe that is never read, roomy queues, a short write timeout:
    // the only way out is the timeout.
    let config = Config {
        max_queued_messages: 100_000,
        max_queued_bytes: 1 << 30,
        write_timeout: Duration::from_millis(200),
        close_grace: Duration::from_millis(200),
        keep_alive_interval: Duration::from_secs(60),
        client_timeout: None,
        ..Config::default()
    };
    let hub = Hub::new(&config);
    let app = std::sync::Arc::new(TestApp::default());
    let (client_io, server_io) = tokio::io::duplex(512);
    let (sender, receiver) = stellarr_websocket::split(
        server_io,
        stellarr_websocket::Config {
            max_message_size: usize::MAX,
            ..Default::default()
        },
    );
    let task = tokio::spawn(stellarr_hub::serve(
        app.clone(),
        hub.clone(),
        std::sync::Arc::new(config),
        std::sync::Arc::new("ann".to_owned()),
        "stuck".into(),
        sender,
        receiver,
    ));
    let mut client = Client::wrap(client_io);
    client.ready().await;
    let big = stellarr_hub::Outbound::single("Fill", &"x".repeat(2000)).unwrap();
    for _ in 0..5 {
        assert!(
            hub.send_to_connection("stuck", &big),
            "nowhere near a queue limit"
        );
    }
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the stuck write ended the connection")
        .unwrap();
    assert_eq!(hub.connection_count(), 0);
    assert!(app.lines().last().unwrap().starts_with("disconnected ann"));
}

#[tokio::test]
async fn a_slow_client_does_not_hold_up_the_others() {
    let config = Config {
        max_queued_messages: 8,
        write_timeout: Duration::from_secs(30),
        ..quick()
    };
    let hub = Hub::new(&config);
    let app = std::sync::Arc::new(TestApp::default());
    let mut fast = start_on(
        app.clone(),
        hub.clone(),
        config.clone(),
        "fast",
        "fast".into(),
    );
    fast.client.ready().await;
    fast.client.json().await;
    let (stuck_io, server_io) = tokio::io::duplex(256);
    let (sender, receiver) = stellarr_websocket::split(
        server_io,
        stellarr_websocket::Config {
            max_message_size: usize::MAX,
            ..Default::default()
        },
    );
    tokio::spawn(stellarr_hub::serve(
        app.clone(),
        hub.clone(),
        std::sync::Arc::new(config),
        std::sync::Arc::new("stuck".to_owned()),
        "stuck".into(),
        sender,
        receiver,
    ));
    let mut stuck = Client::wrap(stuck_io);
    stuck.ready().await;
    for g in ["fast", "stuck"] {
        hub.add_to_group(g, "everyone");
    }
    let started = std::time::Instant::now();
    for n in 0..100 {
        hub.send_to_group(
            "everyone",
            &stellarr_hub::Outbound::single("Tick", &n).unwrap(),
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "sending never waits for a slow reader"
    );
    let mut seen = 0;
    while seen < 100 {
        assert_eq!(fast.client.json().await["name"], "Tick");
        seen += 1;
    }
}
