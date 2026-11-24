//! The control API on an ephemeral port, and one conversation over a real UDP
//! socket through the whole process.

use std::net::{SocketAddr, SocketAddrV4, UdpSocket as StdUdpSocket};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stellarr_media::config::Config;
use stellarr_media::server::{self, Server, ServerError};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, MediaKind, Mid};
use str0m::net::{Protocol, Receive, TcpType, Transmit};
use str0m::rtp::RtpWrite;
use str0m::{Candidate, Event, Input, Output, Rtc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

const SECRET: &str = "a-test-secret";
const ROOM: &str = "00000000-0000-4000-8000-00000000aaaa";

fn uuid(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn config(secret: &str) -> Config {
    Config {
        media_bind: "127.0.0.1:0".parse().unwrap(),
        public_ip: None,
        public_port: None,
        advertise: None,
        control_bind: "127.0.0.1:0".parse().unwrap(),
        control_secret: secret.into(),
        log: "info".into(),
    }
}

async fn start() -> Server {
    server::start(config(SECRET))
        .await
        .expect("the server starts")
}

struct Reply {
    status: u16,
    body: Value,
}

async fn request(
    server: &Server,
    method: &str,
    path: &str,
    secret: Option<&str>,
    body: Option<&str>,
) -> Reply {
    raw_request(
        server.control_addr(),
        method,
        path,
        secret,
        body.map(|b| b.as_bytes().to_vec()),
    )
    .await
}

/// A minimal HTTP/1.1 client: one request per connection.
async fn raw_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    secret: Option<&str>,
    body: Option<Vec<u8>>,
) -> Reply {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body = body.unwrap_or_default();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: media\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(secret) = secret {
        head.push_str(&format!("Authorization: Bearer {secret}\r\n"));
    }
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str("\r\n");

    // A server that refuses a body early may close before it is all sent.
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;

    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text.get(9..12).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_string()))
    };
    Reply { status, body }
}

async fn post(server: &Server, path: &str, body: Value) -> Reply {
    request(server, "POST", path, Some(SECRET), Some(&body.to_string())).await
}

async fn get(server: &Server, path: &str) -> Reply {
    request(server, "GET", path, Some(SECRET), None).await
}

async fn create(server: &Server, user: u64) -> String {
    let reply = post(
        server,
        "/v1/sessions",
        json!({ "roomId": ROOM, "userId": uuid(user) }),
    )
    .await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    reply.body["sessionId"].as_str().unwrap().to_string()
}

/// A browser stand-in that only makes descriptions.
fn browser() -> Rtc {
    let mut rtc = Rtc::builder().set_rtp_mode(true).build(Instant::now());
    rtc.add_local_candidate(Candidate::host("10.0.0.1:5000".parse().unwrap(), "udp").unwrap());
    rtc
}

fn publish_offer(rtc: &mut Rtc, tracks: usize) -> (String, SdpPendingOffer) {
    let mut api = rtc.sdp_api();
    for n in 0..tracks {
        api.add_media(
            MediaKind::Audio,
            Direction::SendOnly,
            Some(format!("s{n}")),
            None,
            None,
        );
    }
    let (offer, pending) = api.apply().unwrap();
    (offer.to_sdp_string(), pending)
}

async fn all_events(server: &Server, epoch: &str) -> Vec<Value> {
    let reply = get(server, &format!("/v1/events?epoch={epoch}&after=0")).await;
    assert_eq!(reply.status, 200);
    reply.body["events"].as_array().unwrap().clone()
}

#[tokio::test]
async fn health_needs_no_secret() {
    let server = start().await;

    let reply = request(&server, "GET", "/v1/health", None, None).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["status"], "ok");
    assert_eq!(reply.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(reply.body["epoch"], server.epoch());
    assert_eq!(reply.body["rooms"], 0);
    assert_eq!(reply.body["sessions"], 0);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_other_route_needs_the_secret() {
    let server = start().await;
    let routes = [
        ("POST", "/v1/reset"),
        ("POST", "/v1/sessions"),
        ("DELETE", "/v1/sessions/x"),
        ("POST", "/v1/sessions/x/offer"),
        ("POST", "/v1/sessions/x/answer"),
        ("POST", "/v1/sessions/x/candidate"),
        ("POST", "/v1/sessions/x/subscriptions"),
        ("GET", "/v1/events?epoch=x&after=0"),
        ("GET", "/v1/stats"),
    ];

    for (method, path) in routes {
        for secret in [
            None,
            Some("wrong"),
            Some("a-test-secre"),
            Some("a-test-secret-"),
        ] {
            let reply = request(&server, method, path, secret, Some("{}")).await;
            assert_eq!(reply.status, 401, "{method} {path} with {secret:?}");
            assert_eq!(reply.body["error"], "unauthorized");
            assert!(reply.body["detail"].is_string());
        }
    }
    // A request without the scheme is refused too.
    let reply = raw_request(server.control_addr(), "GET", "/v1/stats", None, None).await;
    assert_eq!(reply.status, 401);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_empty_secret_on_loopback_asks_for_nothing() {
    let server = server::start(config("")).await.unwrap();

    let reply = request(&server, "GET", "/v1/stats", None, None).await;
    assert_eq!(reply.status, 200);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn refuses_to_start_with_an_empty_secret_off_loopback() {
    let mut c = config("");
    c.control_bind = "0.0.0.0:0".parse().unwrap();

    let refused = server::start(c).await;
    assert!(
        matches!(refused, Err(ServerError::Config(_))),
        "{:?}",
        refused.err()
    );
}

#[tokio::test]
async fn refuses_to_start_when_a_socket_is_taken() {
    let taken = StdUdpSocket::bind("127.0.0.1:0").unwrap();
    let mut c = config(SECRET);
    c.media_bind = match taken.local_addr().unwrap() {
        SocketAddr::V4(v4) => v4,
        SocketAddr::V6(_) => unreachable!(),
    };

    let refused = server::start(c).await;
    assert!(
        matches!(refused, Err(ServerError::Bind { .. })),
        "{:?}",
        refused.err()
    );
}

#[tokio::test]
async fn refuses_to_start_when_the_tcp_media_port_is_taken() {
    let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let SocketAddr::V4(address) = taken.local_addr().unwrap() else {
        unreachable!()
    };
    let mut config = config(SECRET);
    config.media_bind = address;
    let refused = server::start(config).await;
    assert!(matches!(
        refused,
        Err(ServerError::Bind {
            what: "TCP media socket",
            ..
        })
    ));
    // A failed start releases the UDP socket it reserved first.
    UdpSocket::bind(address).await.unwrap();
}

#[tokio::test]
async fn sessions_are_created_and_deleted() {
    let server = start().await;

    let reply = post(
        &server,
        "/v1/sessions",
        json!({ "roomId": ROOM, "userId": uuid(1) }),
    )
    .await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.body["epoch"], server.epoch());
    let id = reply.body["sessionId"].as_str().unwrap().to_string();

    for body in [
        "not json",
        "{}",
        r#"{ "roomId": "nope", "userId": "00000000-0000-4000-8000-000000000001" }"#,
        r#"{ "roomId": "00000000-0000-4000-8000-00000000aaaa", "userId": 5 }"#,
    ] {
        let reply = request(&server, "POST", "/v1/sessions", Some(SECRET), Some(body)).await;
        assert_eq!(reply.status, 422, "{body}");
        assert_eq!(reply.body["error"], "invalid-request");
    }

    let health = request(&server, "GET", "/v1/health", None, None).await;
    assert_eq!(health.body["sessions"], 1);

    for path in [
        format!("/v1/sessions/{id}"),
        format!("/v1/sessions/{id}"),
        "/v1/sessions/unknown".into(),
    ] {
        let reply = request(&server, "DELETE", &path, Some(SECRET), None).await;
        assert_eq!(reply.status, 204);
        assert_eq!(reply.body, Value::Null);
    }
    let health = request(&server, "GET", "/v1/health", None, None).await;
    assert_eq!(health.body["sessions"], 0);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn offers_answers_and_candidates_use_the_contract_codes() {
    let server = start().await;
    let a = create(&server, 1).await;
    let b = create(&server, 2).await;
    let mut browser_a = browser();
    let mut browser_b = browser();

    // Unknown sessions.
    for route in ["offer", "answer", "candidate", "subscriptions"] {
        let reply = post(
            &server,
            &format!("/v1/sessions/nobody/{route}"),
            json!({ "sdp": "v=0", "negotiationId": 1, "candidate": null }),
        )
        .await;
        assert_eq!(reply.status, 404, "{route}");
        assert_eq!(reply.body["error"], "unknown-session");
        // Even when the body is malformed.
        let reply = request(
            &server,
            "POST",
            &format!("/v1/sessions/nobody/{route}"),
            Some(SECRET),
            Some("{"),
        )
        .await;
        assert_eq!(reply.status, 404, "{route}");
    }

    // Offers.
    for body in [json!({ "sdp": "garbage" }), json!({}), json!({ "sdp": 5 })] {
        let reply = post(&server, &format!("/v1/sessions/{a}/offer"), body.clone()).await;
        assert_eq!(reply.status, 422, "{body}");
        assert_eq!(reply.body["error"], "invalid-sdp");
    }
    let reply = request(
        &server,
        "POST",
        &format!("/v1/sessions/{a}/offer"),
        Some(SECRET),
        Some("["),
    )
    .await;
    assert_eq!(
        (reply.status, reply.body["error"].as_str()),
        (422, Some("invalid-sdp"))
    );

    let (nine, _) = publish_offer(&mut browser(), 9);
    let reply = post(
        &server,
        &format!("/v1/sessions/{a}/offer"),
        json!({ "sdp": nine }),
    )
    .await;
    assert_eq!(reply.status, 422);
    assert_eq!(reply.body["error"], "limit-exceeded");

    let (offer_a, pending_a) = publish_offer(&mut browser_a, 1);
    let reply = post(
        &server,
        &format!("/v1/sessions/{a}/offer"),
        json!({ "sdp": offer_a }),
    )
    .await;
    assert_eq!(reply.status, 204, "{}", reply.body);
    assert_eq!(reply.body, Value::Null, "the answer travels as an event");

    let (offer_b, pending_b) = publish_offer(&mut browser_b, 1);
    let reply = post(
        &server,
        &format!("/v1/sessions/{b}/offer"),
        json!({ "sdp": offer_b }),
    )
    .await;
    assert_eq!(reply.status, 204);

    let events = all_events(&server, server.epoch()).await;
    let answer_to = |session: &str| {
        events
            .iter()
            .find(|e| e["sessionId"] == session && e["kind"] == "answer")
            .unwrap()["payload"]["sdp"]
            .as_str()
            .unwrap()
            .to_string()
    };
    browser_a
        .sdp_api()
        .accept_answer(
            pending_a,
            SdpAnswer::from_sdp_string(&answer_to(&a)).unwrap(),
        )
        .unwrap();
    browser_b
        .sdp_api()
        .accept_answer(
            pending_b,
            SdpAnswer::from_sdp_string(&answer_to(&b)).unwrap(),
        )
        .unwrap();

    // Answers: B has an offer of A's microphone pending.
    let offer = events
        .iter()
        .find(|e| e["sessionId"] == b && e["kind"] == "offer")
        .unwrap();
    let id = offer["payload"]["negotiationId"].as_u64().unwrap();
    let server_offer =
        SdpOffer::from_sdp_string(offer["payload"]["sdp"].as_str().unwrap()).unwrap();
    let answer = browser_b
        .sdp_api()
        .accept_offer(server_offer)
        .unwrap()
        .to_sdp_string();

    let reply = post(
        &server,
        &format!("/v1/sessions/{b}/answer"),
        json!({ "sdp": answer, "negotiationId": id + 5 }),
    )
    .await;
    assert_eq!(reply.status, 409);
    assert_eq!(reply.body["error"], "stale-negotiation");
    for body in [
        json!({ "sdp": "garbage", "negotiationId": id }),
        json!({ "sdp": answer }),
        json!({ "negotiationId": id }),
    ] {
        let reply = post(&server, &format!("/v1/sessions/{b}/answer"), body).await;
        assert_eq!(reply.status, 422);
        assert_eq!(reply.body["error"], "invalid-sdp");
    }
    let reply = post(
        &server,
        &format!("/v1/sessions/{b}/answer"),
        json!({ "sdp": answer, "negotiationId": id }),
    )
    .await;
    assert_eq!(reply.status, 204, "{}", reply.body);
    let reply = post(
        &server,
        &format!("/v1/sessions/{b}/answer"),
        json!({ "sdp": answer, "negotiationId": id }),
    )
    .await;
    assert_eq!(reply.status, 409);

    // Candidates.
    let candidate = |c: Value| json!({ "candidate": c });
    for ok in [
        json!({ "candidate": null }),
        candidate(
            json!({ "candidate": "candidate:1 1 udp 2130706431 10.0.0.9 5000 typ host", "sdpMid": "0", "sdpMLineIndex": 0 }),
        ),
        candidate(
            json!({ "candidate": "candidate:2 1 udp 2130706431 0e1c-4a.local 5000 typ host", "sdpMid": "0" }),
        ),
        candidate(json!({ "candidate": "" })),
    ] {
        let reply = post(&server, &format!("/v1/sessions/{a}/candidate"), ok.clone()).await;
        assert_eq!(reply.status, 204, "{ok}");
    }
    for bad in [
        json!({}),
        json!({ "candidate": "text" }),
        candidate(json!({ "candidate": "candidate:nonsense" })),
    ] {
        let reply = post(&server, &format!("/v1/sessions/{a}/candidate"), bad.clone()).await;
        assert_eq!(reply.status, 422, "{bad}");
        assert_eq!(reply.body["error"], "invalid-candidate");
    }

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn subscriptions_replace_a_validated_receiver_mid_allowlist() {
    let server = start().await;
    let session = create(&server, 1).await;
    let path = format!("/v1/sessions/{session}/subscriptions");
    for mids in [
        Vec::<String>::new(),
        vec!["future-mid".into(), "0123456789abcdef".into()],
        (0..4096).map(|i| format!("m{i}")).collect(),
    ] {
        let reply = post(&server, &path, json!({ "mids": mids })).await;
        assert_eq!(reply.status, 204, "{}", reply.body);
        assert_eq!(reply.body, Value::Null);
    }

    for body in [
        json!({}),
        json!([]),
        json!({ "mids": null }),
        json!({ "mids": "mid" }),
        json!({ "mids": [1] }),
        json!({ "mids": [""] }),
        json!({ "mids": ["same", "same"] }),
        json!({ "mids": ["0123456789abcdefg"] }),
        json!({ "mids": ["has space"] }),
        json!({ "mids": ["has\ttab"] }),
        json!({ "mids": ["has\nnewline"] }),
        json!({ "mids": ["control\u{7f}"] }),
        json!({ "mids": ["żółć"] }),
        json!({ "mids": (0..4097).map(|i| format!("m{i}")).collect::<Vec<_>>() }),
    ] {
        let reply = post(&server, &path, body.clone()).await;
        assert_eq!(reply.status, 422, "{body}");
        assert_eq!(reply.body["error"], "invalid-request", "{body}");
    }
    let reply = request(&server, "POST", &path, Some(SECRET), Some("{")).await;
    assert_eq!(reply.status, 422);
    assert_eq!(reply.body["error"], "invalid-request");

    let big = json!({ "mids": ["x".repeat(300 * 1024)] }).to_string();
    let reply = request(&server, "POST", &path, Some(SECRET), Some(&big)).await;
    assert_eq!(reply.status, 413);
    assert_eq!(reply.body["error"], "payload-too-large");
    assert!(all_events(&server, server.epoch()).await.is_empty());
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn bodies_over_256_kib_are_refused() {
    let server = start().await;
    let a = create(&server, 1).await;

    let big = format!(r#"{{ "sdp": "{}" }}"#, "x".repeat(300 * 1024));
    let reply = raw_request(
        server.control_addr(),
        "POST",
        &format!("/v1/sessions/{a}/offer"),
        Some(SECRET),
        Some(big.into_bytes()),
    )
    .await;
    assert_eq!(reply.status, 413);
    assert_eq!(reply.body["error"], "payload-too-large");

    // Between the SDP limit and the body limit the SDP is what is refused.
    let sdp = format!("v=0\r\n{}", "a=x\r\n".repeat(30_000));
    let reply = post(
        &server,
        &format!("/v1/sessions/{a}/offer"),
        json!({ "sdp": sdp }),
    )
    .await;
    assert_eq!(reply.status, 422);
    assert_eq!(reply.body["error"], "invalid-sdp");

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_routes_are_404() {
    let server = start().await;

    for (method, path) in [
        ("GET", "/"),
        ("GET", "/v2/health"),
        ("POST", "/v1/sessions/a/b/c"),
    ] {
        let reply = request(&server, method, path, Some(SECRET), None).await;
        assert_eq!(reply.status, 404, "{method} {path}");
        assert_eq!(reply.body["error"], "not-found");
    }
    let reply = request(&server, "PUT", "/v1/health", None, None).await;
    assert_eq!(reply.status, 405);
    assert_eq!(reply.body["error"], "method-not-allowed");

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_event_poll_resets_another_epoch_and_validates_its_query() {
    let server = start().await;

    for path in [
        "/v1/events?epoch=old&after=0&waitMs=5000",
        "/v1/events?after=0",
        "/v1/events",
    ] {
        let started = Instant::now();
        let reply = get(&server, path).await;
        assert_eq!(reply.status, 200);
        assert_eq!(
            reply.body,
            json!({ "epoch": server.epoch(), "reset": true, "events": [] })
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a reset is immediate"
        );
    }

    // An `after` ahead of the log means the poller is confused: reset.
    let reply = get(
        &server,
        &format!("/v1/events?epoch={}&after=99", server.epoch()),
    )
    .await;
    assert_eq!(reply.body["reset"], true);

    for path in [
        "/v1/events?after=-1",
        "/v1/events?waitMs=soon",
        "/v1/events?after=1.5",
    ] {
        let reply = get(&server, path).await;
        assert_eq!(reply.status, 422, "{path}");
        assert_eq!(reply.body["error"], "invalid-request");
    }

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_event_poll_returns_early_and_times_out() {
    let server = start().await;
    let epoch = server.epoch().to_string();
    let a = create(&server, 1).await;

    // Nothing to read: the poll waits for as long as it was asked to.
    let started = Instant::now();
    let reply = get(
        &server,
        &format!("/v1/events?epoch={epoch}&after=0&waitMs=300"),
    )
    .await;
    let waited = started.elapsed();
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.body,
        json!({ "epoch": epoch, "reset": false, "events": [] })
    );
    assert!(waited >= Duration::from_millis(290), "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");

    // An event that arrives during the wait ends it at once.
    let addr = server.control_addr();
    let path = format!("/v1/events?epoch={epoch}&after=0&waitMs=10000");
    let started = Instant::now();
    let poll =
        tokio::spawn(async move { raw_request(addr, "GET", &path, Some(SECRET), None).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (offer, _) = publish_offer(&mut browser(), 1);
    assert_eq!(
        post(
            &server,
            &format!("/v1/sessions/{a}/offer"),
            json!({ "sdp": offer })
        )
        .await
        .status,
        204
    );

    let reply = poll.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    let events = reply.body["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["seq"], 1);
    assert_eq!(events[0]["type"], "signal");
    assert_eq!(events[0]["kind"], "answer");
    assert_eq!(events[0]["sessionId"], a);
    assert_eq!(events[0]["payload"]["type"], "answer");
    assert!(
        events[0]["payload"]["sdp"]
            .as_str()
            .unwrap()
            .starts_with("v=0")
    );

    // Events already there come back without waiting.
    let started = Instant::now();
    let reply = get(
        &server,
        &format!("/v1/events?epoch={epoch}&after=0&waitMs=10000"),
    )
    .await;
    assert_eq!(reply.body["events"].as_array().unwrap().len(), 1);
    assert!(started.elapsed() < Duration::from_secs(2));

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_and_stats_report_what_the_contract_says() {
    let server = start().await;
    let a = create(&server, 1).await;
    let (offer, _) = publish_offer(&mut browser(), 2);
    post(
        &server,
        &format!("/v1/sessions/{a}/offer"),
        json!({ "sdp": offer }),
    )
    .await;

    let stats = get(&server, "/v1/stats").await;
    assert_eq!(stats.status, 200);
    assert_eq!(stats.body["epoch"], server.epoch());
    let session = &stats.body["rooms"][0]["sessions"][0];
    assert_eq!(stats.body["rooms"][0]["roomId"], ROOM);
    assert_eq!(session["sessionId"], a);
    assert_eq!(session["userId"], uuid(1));
    assert_eq!(session["state"], "connecting");
    assert_eq!(session["published"], 2);
    assert_eq!(session["forwarded"], 0);
    assert!(session["ageMs"].is_u64());
    assert_eq!(session["ingress"], json!({ "packets": 0, "bytes": 0 }));
    assert_eq!(session["egress"], json!({ "packets": 0, "bytes": 0 }));
    // Nothing has been estimated or held back of a session that is not up.
    assert_eq!(
        session["downlink"],
        json!({
            "estimateBps": null,
            "rawEstimateBps": null,
            "smoothedEstimateBps": null,
            "measured": false,
            "paddingBps": 0,
            "forwardedBps": 0,
            "desiredBps": 0,
            "heldBack": [],
            "holds": 0,
            "resumes": 0,
        })
    );

    let reset = post(&server, "/v1/reset", json!({})).await;
    assert_eq!(reset.status, 200);
    assert_eq!(reset.body, json!({ "epoch": server.epoch(), "seq": 1 }));

    let events = all_events(&server, server.epoch()).await;
    assert_eq!(events.len(), 1, "a reset emits nothing");
    let stats = get(&server, "/v1/stats").await;
    assert_eq!(stats.body["rooms"], json!([]));

    server.shutdown().await.unwrap();
}

// ------------------------------------------------------- over real UDP and TCP

enum MediaWire {
    Udp(UdpSocket),
    Tcp {
        stream: TcpStream,
        local: SocketAddr,
        remote: SocketAddr,
        buffered: Vec<u8>,
    },
}

impl MediaWire {
    async fn send(&mut self, packet: Transmit) {
        match self {
            Self::Udp(socket) => {
                assert_eq!(packet.proto, Protocol::Udp);
                socket
                    .send_to(&packet.contents, packet.destination)
                    .await
                    .unwrap();
            }
            Self::Tcp { stream, remote, .. } => {
                assert_eq!(packet.proto, Protocol::Tcp);
                assert_eq!(packet.destination, *remote);
                stream
                    .write_u16(packet.contents.len().try_into().unwrap())
                    .await
                    .unwrap();
                stream.write_all(&packet.contents).await.unwrap();
            }
        }
    }

    async fn receive(&mut self) -> Option<(Protocol, SocketAddr, SocketAddr, Vec<u8>)> {
        match self {
            Self::Udp(socket) => {
                let mut buf = vec![0; 2048];
                let (n, source) =
                    tokio::time::timeout(Duration::from_millis(1), socket.recv_from(&mut buf))
                        .await
                        .ok()?
                        .unwrap();
                buf.truncate(n);
                Some((Protocol::Udp, source, socket.local_addr().unwrap(), buf))
            }
            Self::Tcp {
                stream,
                local,
                remote,
                buffered,
            } => loop {
                if let Some(header) = buffered.get(..2) {
                    let len = u16::from_be_bytes(header.try_into().unwrap()) as usize;
                    if buffered.len() >= 2 + len {
                        let packet = buffered[2..2 + len].to_vec();
                        buffered.drain(..2 + len);
                        return Some((Protocol::Tcp, *remote, *local, packet));
                    }
                }
                let n = tokio::time::timeout(Duration::from_millis(1), stream.read_buf(buffered))
                    .await
                    .ok()?
                    .unwrap();
                assert_ne!(n, 0, "the media server closed a live TCP transport");
            },
        }
    }
}

struct NetworkClient {
    rtc: Rtc,
    wire: MediaWire,
    session: String,
    pending: Option<SdpPendingOffer>,
    received: Vec<(Mid, Vec<u8>)>,
}

impl NetworkClient {
    async fn new(server: &Server, user: u64, tcp: bool) -> Self {
        let mut rtc = Rtc::builder().set_rtp_mode(true).build(Instant::now());
        let wire = if tcp {
            let remote = server.candidates()[0];
            assert_eq!(remote.port(), server.media_addr().port());
            let stream = TcpStream::connect(remote).await.unwrap();
            let local = SocketAddr::new(stream.local_addr().unwrap().ip(), 9);
            rtc.add_local_candidate(
                Candidate::builder()
                    .tcp()
                    .host(local)
                    .tcptype(TcpType::Active)
                    .build()
                    .unwrap(),
            );
            MediaWire::Tcp {
                stream,
                local,
                remote,
                buffered: Vec::new(),
            }
        } else {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            rtc.add_local_candidate(Candidate::host(socket.local_addr().unwrap(), "udp").unwrap());
            MediaWire::Udp(socket)
        };
        Self {
            rtc,
            wire,
            session: create(server, user).await,
            pending: None,
            received: Vec::new(),
        }
    }

    async fn drain(&mut self) {
        loop {
            match self.rtc.poll_output().unwrap() {
                Output::Timeout(_) => return,
                Output::Transmit(t) => {
                    self.wire.send(t).await;
                }
                Output::Event(Event::RtpPacket(p)) => {
                    let mid = self
                        .rtc
                        .direct_api()
                        .stream_rx(&p.header.ssrc)
                        .unwrap()
                        .mid();
                    self.received.push((mid, p.payload.to_vec()));
                }
                Output::Event(_) => {}
            }
        }
    }

    async fn pump(&mut self) {
        while let Some((protocol, source, local, packet)) = self.wire.receive().await {
            let receive = Receive::new(protocol, source, local, &packet).unwrap();
            self.rtc
                .handle_input(Input::Receive(Instant::now(), receive))
                .unwrap();
            self.drain().await;
        }
        self.rtc
            .handle_input(Input::Timeout(Instant::now()))
            .unwrap();
        self.drain().await;
    }
}

async fn relay(server: &Server, clients: &mut [&mut NetworkClient], seen: &mut u64) {
    let reply = get(
        server,
        &format!("/v1/events?epoch={}&after={seen}", server.epoch()),
    )
    .await;
    for event in reply.body["events"].as_array().unwrap() {
        *seen = event["seq"].as_u64().unwrap();
        let Some(c) = clients
            .iter_mut()
            .find(|c| c.session == event["sessionId"].as_str().unwrap())
        else {
            continue;
        };
        // A browser asks for the tracks it is told of; nothing is sent to it
        // otherwise.
        if let Some(tracks) = event["payload"]["tracks"].as_array() {
            let mids: Vec<&str> = tracks.iter().filter_map(|t| t["mid"].as_str()).collect();
            let path = format!("/v1/sessions/{}/subscriptions", c.session);
            assert_eq!(
                post(server, &path, json!({ "mids": mids })).await.status,
                204
            );
        }
        let sdp = event["payload"]["sdp"].as_str().unwrap_or_default();
        match event["kind"].as_str() {
            Some("answer") => {
                let pending = c.pending.take().unwrap();
                c.rtc
                    .sdp_api()
                    .accept_answer(pending, SdpAnswer::from_sdp_string(sdp).unwrap())
                    .unwrap();
            }
            Some("offer") => {
                let answer = c
                    .rtc
                    .sdp_api()
                    .accept_offer(SdpOffer::from_sdp_string(sdp).unwrap())
                    .unwrap();
                let id = event["payload"]["negotiationId"].as_u64().unwrap();
                let body = json!({ "sdp": answer.to_sdp_string(), "negotiationId": id });
                assert_eq!(
                    post(server, &format!("/v1/sessions/{}/answer", c.session), body)
                        .await
                        .status,
                    204
                );
            }
            _ => {}
        }
        c.drain().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_clients_talk_over_real_udp_through_the_process() {
    conversation_over_media(false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clients_forward_media_in_both_directions_between_tcp_and_udp() {
    conversation_over_media(true, false).await;
    conversation_over_media(false, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_clients_talk_over_tcp_without_a_udp_route() {
    conversation_over_media(true, true).await;
}

async fn conversation_over_media(tcp_a: bool, tcp_b: bool) {
    // The socket is bound to every address, and the one address advertised
    // is not the bind address.
    let mut c = config(SECRET);
    if !(tcp_a && tcp_b) {
        let port = StdUdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        c.media_bind = SocketAddrV4::new([0, 0, 0, 0].into(), port);
        c.advertise = Some(vec![SocketAddrV4::new([127, 0, 0, 1].into(), port)]);
    }
    let server = server::start(c).await.unwrap();
    assert_eq!(
        server.candidates(),
        [SocketAddr::from((
            [127, 0, 0, 1],
            server.media_addr().port()
        ))]
    );

    let mut a = NetworkClient::new(&server, 1, tcp_a).await;
    let mut b = NetworkClient::new(&server, 2, tcp_b).await;
    let mut mids = Vec::new();
    for client in [&mut a, &mut b] {
        let mut api = client.rtc.sdp_api();
        mids.push(api.add_media(
            MediaKind::Audio,
            Direction::SendOnly,
            Some("mic".into()),
            None,
            None,
        ));
        let (offer, pending) = api.apply().unwrap();
        client.pending = Some(pending);
        let body = json!({ "sdp": offer.to_sdp_string() });
        assert_eq!(
            post(
                &server,
                &format!("/v1/sessions/{}/offer", client.session),
                body
            )
            .await
            .status,
            204
        );
        client.drain().await;
    }

    let mut seen = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(a.rtc.is_connected() && b.rtc.is_connected()) {
        assert!(Instant::now() < deadline, "the clients did not connect");
        relay(&server, &mut [&mut a, &mut b], &mut seen).await;
        a.pump().await;
        b.pump().await;
    }
    // Let the last negotiation settle.
    for _ in 0..50 {
        relay(&server, &mut [&mut a, &mut b], &mut seen).await;
        a.pump().await;
        b.pump().await;
    }

    let pt = a
        .rtc
        .codec_config()
        .iter()
        .find(|p| p.spec().codec == Codec::Opus)
        .unwrap()
        .pt();
    for n in 0..10u8 {
        let now = Instant::now();
        let write = RtpWrite::new(pt, (n as u64 + 1).into(), n as u32 * 960, now, vec![n, 42]);
        a.rtc
            .direct_api()
            .stream_tx_by_mid(mids[0], None)
            .unwrap()
            .write_rtp(write);
        a.drain().await;
        a.rtc.handle_input(Input::Timeout(now)).unwrap();
        a.drain().await;
        b.pump().await;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while b.received.len() < 10 {
        assert!(
            Instant::now() < deadline,
            "only {} packets arrived",
            b.received.len()
        );
        a.pump().await;
        b.pump().await;
    }
    let payloads: Vec<Vec<u8>> = b.received.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(payloads, (0..10u8).map(|n| vec![n, 42]).collect::<Vec<_>>());

    let stats = get(&server, "/v1/stats").await;
    let sessions = stats.body["rooms"][0]["sessions"].as_array().unwrap();
    assert!(
        sessions.iter().all(|s| s["state"] == "connected"),
        "{sessions:?}"
    );

    if let MediaWire::Tcp { stream, .. } = &mut a.wire {
        let deleted = request(
            &server,
            "DELETE",
            &format!("/v1/sessions/{}", a.session),
            Some(SECRET),
            None,
        )
        .await;
        assert_eq!(deleted.status, 204);
        tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut Vec::new()))
            .await
            .expect("deleting the session promptly closes its TCP stream")
            .unwrap();
        let stats = get(&server, "/v1/stats").await;
        let remaining = stats.body["rooms"][0]["sessions"].as_array().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["state"], "connected");
    }

    server.shutdown().await.unwrap();
}
