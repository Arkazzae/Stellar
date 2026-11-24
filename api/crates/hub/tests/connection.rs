//! A connection from handshake to close, with the hub driven over an
//! in-memory WebSocket.

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use stellarr_hub::{Config, Hub};
use stellarr_websocket::frame::{OP_CLOSE, OP_TEXT};
use support::*;

fn quick() -> Config {
    Config {
        close_grace: Duration::from_millis(500),
        ..Config::default()
    }
}

#[tokio::test]
async fn the_handshake_is_answered_before_anything_the_application_sends() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    assert_eq!(
        r.client.json().await,
        json!({"type": 1, "target": "Hello", "arguments": ["ann"]})
    );
    assert_eq!(r.app.lines(), ["connected ann"]);
}

#[tokio::test]
async fn a_protocol_the_server_does_not_speak_is_refused_and_closed() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client
        .send(&format!(
            "{{\"protocol\":\"messagepack\",\"version\":1}}{RS}"
        ))
        .await;
    assert_eq!(
        r.client.json().await,
        json!({"error": "The protocol 'messagepack' is not supported."})
    );
    let (rest, code) = r.client.until_close().await;
    assert!(rest.is_empty());
    assert_eq!(code, Some(1000));
    r.task.await.unwrap();
    assert!(
        r.app.lines().is_empty(),
        "the application never heard of this connection"
    );
}

#[tokio::test]
async fn an_unreadable_handshake_gets_the_general_error() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.send(&format!("garbage{RS}")).await;
    assert_eq!(
        r.client.json().await,
        json!({"error": "An unexpected error occurred during connection handshake."})
    );
    assert_eq!(r.client.until_close().await.1, Some(1000));
}

#[tokio::test]
async fn a_client_that_never_sends_a_handshake_is_told_it_was_too_slow() {
    let config = Config {
        handshake_timeout: Duration::from_millis(100),
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    assert_eq!(
        r.client.json().await,
        json!({"error": "Handshake was canceled."})
    );
    assert_eq!(r.client.until_close().await.1, Some(1000));
}

#[tokio::test]
async fn a_handshake_over_the_size_limit_is_answered_as_a_timeout_is() {
    let config = Config {
        max_record_size: 100,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client
        .send(&format!(
            "{{\"protocol\":\"json\",\"version\":1,\"pad\":\"{}\"}}{RS}",
            "x".repeat(200)
        ))
        .await;
    assert_eq!(
        r.client.json().await,
        json!({"error": "Handshake was canceled."})
    );
}

#[tokio::test]
async fn the_handshake_and_a_call_may_arrive_in_one_message() {
    let mut r = start(TestApp::default(), quick(), "ann");
    let both = format!(
        "{{\"protocol\":\"json\",\"version\":1}}{RS}{{\"type\":1,\"invocationId\":\"1\",\"target\":\"echo\",\"arguments\":[5]}}{RS}"
    );
    r.client.send(&both).await;
    assert_eq!(r.client.record().await, "{}");
    assert_eq!(r.client.json().await["target"], "Hello");
    assert_eq!(
        r.client.json().await,
        json!({"type": 3, "invocationId": "1", "result": 5})
    );
}

#[tokio::test]
async fn nothing_the_client_sends_is_read_until_the_connect_hook_has_finished() {
    let app = TestApp {
        connect_delay: Duration::from_millis(150),
        ..Default::default()
    };
    let mut r = start(app, quick(), "ann");
    r.client
        .send(&format!("{{\"protocol\":\"json\",\"version\":1}}{RS}"))
        .await;
    assert_eq!(r.client.record().await, "{}");
    r.client.invoke("1", "whoami", json!([])).await;
    // Hello is queued by the hook, so it comes first, then the answer.
    assert_eq!(r.client.json().await["target"], "Hello");
    assert_eq!(r.client.json().await["result"], "ann");
    assert_eq!(r.app.lines(), ["connected ann"]);
}

#[tokio::test]
async fn a_call_with_a_result_a_void_call_and_the_ones_that_fail() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    let cases = [
        (
            "1",
            "echo",
            json!(["hi"]),
            json!({"type": 3, "invocationId": "1", "result": "hi"}),
        ),
        (
            "2",
            "void",
            json!([]),
            json!({"type": 3, "invocationId": "2", "result": null}),
        ),
        (
            "3",
            "fail",
            json!([]),
            json!({"type": 3, "invocationId": "3", "error": "An unexpected error occurred invoking 'fail' on the server. HubException: Not allowed here."}),
        ),
        (
            "4",
            "Nope",
            json!([]),
            json!({"type": 3, "invocationId": "4", "error": "Unknown method 'Nope'."}),
        ),
        (
            "5",
            "echo",
            json!([]),
            json!({"type": 3, "invocationId": "5", "error": "Failed to invoke 'echo' due to an error on the server."}),
        ),
        (
            "6",
            "echo",
            json!([1, 2]),
            json!({"type": 3, "invocationId": "6", "error": "Failed to invoke 'echo' due to an error on the server."}),
        ),
        (
            "7",
            "internal",
            json!([]),
            json!({"type": 3, "invocationId": "7", "error": "An unexpected error occurred invoking 'internal' on the server."}),
        ),
        (
            "8",
            "panic",
            json!([]),
            json!({"type": 3, "invocationId": "8", "error": "An unexpected error occurred invoking 'panic' on the server."}),
        ),
        (
            "9",
            "ECHO",
            json!([{"a": [1, null]}]),
            json!({"type": 3, "invocationId": "9", "result": {"a": [1, null]}}),
        ),
    ];
    for (id, target, arguments, expected) in cases {
        r.client.invoke(id, target, arguments).await;
        assert_eq!(r.client.json().await, expected, "{target}");
    }
    // The connection survived the panic.
    r.client.invoke("10", "whoami", json!([])).await;
    assert_eq!(r.client.json().await["result"], "ann");
}

#[tokio::test]
async fn a_send_without_an_id_gets_no_answer_even_when_it_fails() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    r.client
        .send_records(&[
            json!({"type": 1, "target": "fail", "arguments": []}),
            json!({"type": 1, "invocationId": "", "target": "nope", "arguments": []}),
            json!({"type": 1, "target": "echo", "arguments": [1]}),
            json!({"type": 1, "invocationId": "last", "target": "echo", "arguments": [2]}),
        ])
        .await;
    assert_eq!(
        r.client.json().await,
        json!({"type": 3, "invocationId": "last", "result": 2})
    );
}

#[tokio::test]
async fn a_streaming_invocation_is_answered_with_an_error() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    r.client
        .send_records(&[
            json!({"type": 4, "invocationId": "1", "target": "echo", "arguments": [1], "streamIds": []}),
            json!({"type": 4, "invocationId": "2", "target": "nope", "arguments": []}),
            json!({"type": 1, "invocationId": "3", "target": "echo", "arguments": [1], "streamIds": ["a"]}),
        ])
        .await;
    assert_eq!(
        r.client.json().await["error"],
        "The client attempted to invoke the non-streaming 'echo' method with a streaming invocation."
    );
    assert_eq!(
        r.client.json().await["error"],
        "Unknown method 'nope'."
    );
    assert_eq!(
        r.client.json().await["error"],
        "An unexpected error occurred invoking 'echo' on the server. HubException: Client sent 1 stream(s), Hub method expects 0."
    );
}

#[tokio::test]
async fn messages_the_server_has_no_use_for_are_skipped() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    r.client
        .send_records(&[
            json!({"type": 6}),
            json!({"type": 5, "invocationId": "1"}),
            json!({"type": 8, "sequenceId": 1}),
            json!({"type": 9, "sequenceId": 1}),
            json!({"type": 2, "invocationId": "1", "item": 1}),
            json!({"type": 3, "invocationId": "1", "result": 1}),
            json!({"type": 99}),
        ])
        .await;
    r.client.invoke("1", "whoami", json!([])).await;
    assert_eq!(r.client.json().await["result"], "ann");
}

#[tokio::test]
async fn a_call_may_arrive_in_pieces_and_pieces_may_hold_several_calls() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    let one = format!(
        "{{\"type\":1,\"invocationId\":\"1\",\"target\":\"echo\",\"arguments\":[\"a\"]}}{RS}"
    );
    let two = format!(
        "{{\"type\":1,\"invocationId\":\"2\",\"target\":\"echo\",\"arguments\":[\"b\"]}}{RS}"
    );
    let joined = format!("{one}{two}");
    let (first, rest) = joined.split_at(20);
    let (middle, last) = rest.split_at(one.len() + 5 - 20);
    for part in [first, middle, last] {
        r.client.send_frame(OP_TEXT, part.as_bytes()).await;
    }
    assert_eq!(r.client.json().await["result"], "a");
    assert_eq!(r.client.json().await["result"], "b");
}

#[tokio::test]
async fn binary_messages_are_read_like_text() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    let text =
        format!("{{\"type\":1,\"invocationId\":\"1\",\"target\":\"whoami\",\"arguments\":[]}}{RS}");
    r.client.send_frame(2, text.as_bytes()).await;
    assert_eq!(r.client.json().await["result"], "ann");
}

#[tokio::test]
async fn a_client_calls_run_one_at_a_time_in_the_order_sent() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    let calls: Vec<Value> = (0..5)
        .map(|n| json!({"type": 1, "invocationId": n.to_string(), "target": "slow", "arguments": [n]}))
        .collect();
    r.client.send_records(&calls).await;
    for n in 0..5 {
        assert_eq!(r.client.json().await["result"], n);
    }
    assert_eq!(r.app.max_running.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_malformed_record_closes_the_connection_with_an_error_the_client_may_retry_after() {
    for bad in [
        "{not json".to_owned(),
        "[1]".to_owned(),
        "{\"invocationId\":\"1\",\"target\":\"echo\",\"arguments\":[]}".to_owned(),
        "{\"type\":1,\"invocationId\":\"1\",\"arguments\":[]}".to_owned(),
        "{\"type\":1,\"invocationId\":\"1\",\"target\":\"echo\",\"arguments\":{}}".to_owned(),
        "{\"type\":1,\"invocationId\":7,\"target\":\"echo\",\"arguments\":[]}".to_owned(),
        String::new(),
    ] {
        let mut r = start(TestApp::default(), quick(), "ann");
        r.client.handshake().await;
        r.client.json().await;
        r.client.send(&format!("{bad}{RS}")).await;
        let (records, code) = r.client.until_close().await;
        assert_eq!(
            records,
            [
                json!({"type": 7, "error": "Connection closed with an error.", "allowReconnect": true})
            ],
            "{bad:?}"
        );
        assert_eq!(code, Some(1000));
        r.task.await.unwrap();
        assert!(
            r.app
                .lines()
                .last()
                .unwrap()
                .starts_with("disconnected ann ")
        );
    }
}

#[tokio::test]
async fn the_size_limit_counts_the_separator() {
    let limit = 400;
    let base_len = json!({"type": 1, "invocationId": "1", "target": "echo", "arguments": [""]})
        .to_string()
        .len();
    let record = |size: usize| {
        let pad = size - base_len;
        json!({"type": 1, "invocationId": "1", "target": "echo", "arguments": ["x".repeat(pad)]})
            .to_string()
    };
    // At the limit: a record of 399 bytes and its separator.
    let mut r = start(
        TestApp::default(),
        Config {
            max_record_size: limit,
            ..quick()
        },
        "ann",
    );
    r.client.handshake().await;
    r.client.json().await;
    let fits = record(limit - 1);
    assert_eq!(fits.len() + 1, limit);
    r.client.send(&format!("{fits}{RS}")).await;
    assert_eq!(
        r.client.json().await["result"].as_str().unwrap().len(),
        limit - 1 - base_len
    );
    // One byte more closes the connection.
    let over = record(limit);
    r.client.send(&format!("{over}{RS}")).await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(records[0]["allowReconnect"], true);
    assert_eq!(code, Some(1000));
}

#[tokio::test]
async fn a_record_that_never_ends_is_cut_off_at_the_limit() {
    let mut r = start(
        TestApp::default(),
        Config {
            max_record_size: 1000,
            ..quick()
        },
        "ann",
    );
    r.client.handshake().await;
    r.client.json().await;
    for _ in 0..20 {
        r.client.send(&"x".repeat(100)).await;
    }
    let (records, code) = r.client.until_close().await;
    assert_eq!(records[0]["type"], 7);
    assert_eq!(code, Some(1000));
}

#[tokio::test]
async fn a_close_from_the_client_is_answered_with_a_close_and_ends_the_connection() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    r.client.send_records(&[json!({"type": 7})]).await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(records, [json!({"type": 7})]);
    assert_eq!(code, Some(1000));
    r.task.await.unwrap();
    assert_eq!(r.app.lines(), ["connected ann", "disconnected ann -"]);
    assert_eq!(r.hub.connection_count(), 0);
}

#[tokio::test]
async fn a_websocket_close_from_the_client_is_an_orderly_disconnect() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
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
    client.handshake().await;
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
    r.client.handshake().await;
    r.client.json().await;
    assert!(r.hub.abort("conn-ann"));
    let (records, code) = r.client.until_close().await;
    assert_eq!(records, [json!({"type": 7})]);
    assert_eq!(code, Some(1000));
    r.task.await.unwrap();
    assert_eq!(r.app.lines(), ["connected ann", "disconnected ann -"]);
    assert!(!r.hub.abort("conn-ann"), "it is gone");
}

#[tokio::test]
async fn a_method_may_end_its_own_connection() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    r.client.invoke("1", "abortme", json!([])).await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(
        records,
        [json!({"type": 7})],
        "the answer is not sent to a connection that is ending"
    );
    assert_eq!(code, Some(1000));
}

#[tokio::test]
async fn the_application_can_close_with_a_reason_and_leave_reconnecting_to_the_client() {
    let mut r = start(TestApp::default(), quick(), "ann");
    r.client.handshake().await;
    r.client.json().await;
    r.hub.close(
        "conn-ann",
        stellarr_hub::CloseRequest {
            error: Some("The server is restarting.".into()),
            allow_reconnect: true,
        },
    );
    let (records, _) = r.client.until_close().await;
    assert_eq!(
        records,
        [json!({"type": 7, "error": "The server is restarting.", "allowReconnect": true})]
    );
}

#[tokio::test]
async fn a_connect_hook_that_fails_closes_the_connection_and_the_disconnect_hook_runs() {
    let app = TestApp {
        fail_connect: true,
        ..Default::default()
    };
    let mut r = start(app, quick(), "ann");
    r.client.handshake().await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(
        records,
        [json!({"type": 7, "error": "Connection closed with an error."})]
    );
    assert_eq!(code, Some(1000));
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
    r.client.handshake().await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(
        records,
        [json!({"type": 7, "error": "Connection closed with an error."})]
    );
    assert_eq!(code, Some(1000));
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
        client.handshake().await;
        assert_eq!(client.json().await["target"], "Hello");
    }
    ann.client
        .invoke("1", "say", json!(["all", "hello all"]))
        .await;
    assert_eq!(
        ann.client.json().await,
        json!({"type": 1, "target": "Said", "arguments": ["hello all"]})
    );
    assert_eq!(ann.client.json().await["result"], Value::Null);
    assert_eq!(
        bob.client.json().await,
        json!({"type": 1, "target": "Said", "arguments": ["hello all"]})
    );

    ann.client
        .invoke("2", "say", json!(["user:bob", "just bob"]))
        .await;
    assert_eq!(
        ann.client.json().await["invocationId"],
        "2",
        "ann gets only her answer"
    );
    assert_eq!(bob.client.json().await["arguments"][0], "just bob");

    ann.client.invoke("3", "others", json!([])).await;
    assert_eq!(ann.client.json().await["invocationId"], "3");
    assert_eq!(bob.client.json().await["target"], "Others");
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
    r.client.handshake().await;
    r.client.json().await;
    r.client.invoke("1", "big", json!([])).await;
    let answer = r.client.json().await;
    assert_eq!(answer["result"].as_str().unwrap().len(), 200_000);
}

#[tokio::test]
async fn the_server_pings_a_quiet_client_at_both_levels() {
    let config = Config {
        keep_alive_interval: Duration::from_millis(60),
        transport_ping_interval: Duration::from_millis(80),
        client_timeout: None,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.handshake().await;
    r.client.json().await;
    assert_eq!(r.client.record().await, "{\"type\":6}");
    assert_eq!(r.client.record().await, "{\"type\":6}");
    // The transport ping was answered by `record`, which a browser does too.
    r.client.invoke("1", "whoami", json!([])).await;
    assert_eq!(
        loop {
            let record = r.client.json().await;
            if record["type"] == 3 {
                break record;
            }
        }["result"],
        "ann"
    );
}

#[tokio::test]
async fn pings_come_on_their_own_beat_whatever_else_is_sent() {
    let config = Config {
        keep_alive_interval: Duration::from_millis(200),
        client_timeout: None,
        ..quick()
    };
    let mut r = start(TestApp::default(), config, "ann");
    r.client.handshake().await;
    r.client.json().await;
    let started = std::time::Instant::now();
    for n in 0..3 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        r.hub
            .send_to_connection("conn-ann", &stellarr_hub::Outbound::bare(&format!("E{n}")));
    }
    let mut events = 0;
    loop {
        let record = r.client.json().await;
        if record["type"] == 6 {
            break;
        }
        events += 1;
    }
    assert_eq!(events, 3);
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(150) && waited < Duration::from_millis(600),
        "{waited:?}"
    );
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
    r.client.handshake().await;
    r.client.json().await;
    let (records, code) = r.client.until_close().await;
    assert_eq!(records, [json!({"type": 7, "allowReconnect": true})]);
    assert_eq!(code, Some(1000));
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
    r.client.handshake().await;
    r.client.json().await;
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        r.client.send_frame(10, b"").await;
    }
    r.client.invoke("1", "whoami", json!([])).await;
    assert_eq!(r.client.json().await["result"], "ann");
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
    client.handshake().await;
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
    client.handshake().await;
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
    fast.client.handshake().await;
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
    stuck.handshake().await;
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
        assert_eq!(fast.client.json().await["target"], "Tick");
        seen += 1;
    }
}
