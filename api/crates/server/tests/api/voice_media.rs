//! The hub leg of the media server: opening a media session for a seat,
//! signalling the media server through the hub, the rule that a session lives
//! exactly as long as its seat, and the pump that hands the media server's
//! events to the browsers. The media server is [`FakeMedia`] behind the binary.

use std::time::{Duration, Instant};

use http::Method;
use serde_json::{Value, json};
use stellarr_api::values::Id;

use crate::support::{Canned, FakeMedia, SECRET, until};
use crate::voice::{NOT_ALLOWED, Peer, Rig, in_room, occupants_of, signal};

const OFFER: &str = "{\"type\":\"offer\",\"sdp\":\"v=0 browser\"}";
const ANSWER: &str = "{\"type\":\"answer\",\"sdp\":\"v=0 answer\",\"negotiationId\":7}";
const CANDIDATE: &str =
    "{\"candidate\":\"candidate:1 1 udp 2122260223 192.0.2.1 54321 typ host\",\"sdpMid\":\"0\",\"sdpMLineIndex\":0}";
const REFUSED: &str = "Invalid media signal.";
const UNAVAILABLE: &str = "Media server is unavailable.";
const NOT_SEATED: &str = "Not seated in this voice room.";
const MAX_SIGNAL_BYTES: usize = 128 * 1024;

fn media_signal(room: Id, session: Option<&str>, kind: Option<&str>, payload: Option<&str>, version: i64) -> Value {
    json!({"protocolVersion": version, "channelId": room, "sessionId": session, "kind": kind, "payload": payload})
}

impl Peer {
    pub async fn open_media(&mut self, room: Id) -> String {
        self.ok("OpenMediaSession", json!([room])).await["sessionId"]
            .as_str()
            .expect("a session id")
            .to_string()
    }

    pub async fn media(&mut self, room: Id, session: &str, kind: &str, payload: &str) {
        self.ok("SendMediaSignal", json!([media_signal(room, Some(session), Some(kind), Some(payload), 1)]))
            .await;
    }

    pub async fn media_refused(&mut self, room: Id, session: &str, kind: &str, payload: &str, text: &str) {
        self.refused("SendMediaSignal", json!([media_signal(room, Some(session), Some(kind), Some(payload), 1)]), text)
            .await;
    }
}

/// An offer whose payload is exactly this many bytes long.
fn offer_of_length(bytes: usize) -> String {
    let head = "{\"type\":\"offer\",\"sdp\":\"";
    let tail = "\"}";
    format!("{head}{}{tail}", "x".repeat(bytes - head.len() - tail.len()))
}

/// A media-server offer as an event payload.
fn media_offer(negotiation: u32) -> String {
    format!("{{\"type\":\"offer\",\"sdp\":\"v=0 media {negotiation}\",\"negotiationId\":{negotiation},\"tracks\":[]}}")
}

fn created_sessions(media: &FakeMedia) -> Vec<crate::support::Recorded> {
    media
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path == "/v1/sessions")
        .collect()
}

#[tokio::test]
async fn a_session_needs_the_callers_seat_in_that_room() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let call = rig.conversation(&maja, &ada);
    let mut peer = rig.connect(&maja).await;

    peer.refused("OpenMediaSession", json!([rig.salon]), NOT_SEATED).await;
    peer.refused("OpenMediaSession", json!([call]), NOT_SEATED).await;
    peer.refused("OpenMediaSession", json!(["not-a-guid"]), "Invalid channelId.").await;
    peer.refused("OpenMediaSession", json!([null]), "Invalid channelId.").await;
    peer.join(rig.salon).await;
    peer.refused("OpenMediaSession", json!([rig.night]), NOT_SEATED).await;
    assert_eq!(rig.media.count("POST", "/v1/sessions"), 0);

    let session = peer.open_media(rig.salon).await;

    let created = created_sessions(&rig.media);
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].json(), json!({"roomId": rig.salon, "userId": maja.id}));
    assert_eq!(created[0].body, format!("{{\"roomId\":\"{}\",\"userId\":\"{}\"}}", rig.salon, maja.id));
    assert_eq!(created[0].authorization.as_deref(), Some(format!("Bearer {SECRET}").as_str()));
    assert_eq!(rig.media.session_of(&session), Some((rig.salon.to_string(), maja.id.clone())));
}

#[tokio::test]
async fn opening_again_closes_the_earlier_session_silently() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;

    let first = peer.open_media(rig.salon).await;
    let second = peer.open_media(rig.salon).await;

    assert_ne!(first, second);
    rig.media.delete_of(&first).await;
    peer.media_refused(rig.salon, &first, "offer", OFFER, REFUSED).await;
    peer.media(rig.salon, &second, "offer", OFFER).await;
    assert_eq!(rig.media.count("POST", &format!("/v1/sessions/{second}/offer")), 1);
    assert!(peer.drain("MediaSessionClosed").await.is_empty());
}

#[tokio::test]
async fn a_media_server_that_is_down_refuses_the_session_but_never_the_seat() {
    let rig = Rig::start().await;
    let (jonas, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let mut observer = rig.connect(&jonas).await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;

    rig.media.set_down(true);

    peer.media_refused(rig.salon, &session, "offer", OFFER, UNAVAILABLE).await;
    peer.refused("OpenMediaSession", json!([rig.salon]), UNAVAILABLE).await;
    // Voice itself, presence and the occupancy others see carry on.
    peer.state(rig.salon, "{\"muted\":true}", None).await;
    observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && s["participants"][0]["muted"] == true)
        .await;
    peer.leave().await;
    observer
        .next_where("VoiceParticipantChanged", |s| in_room(rig.salon)(s) && occupants_of(s).is_empty())
        .await;
    peer.join(rig.salon).await;

    // The close the seat asked for waits for the media server and arrives.
    rig.media.set_down(false);
    until(|| !rig.media.sessions().contains(&session)).await;
    assert_ne!(peer.open_media(rig.salon).await, session);
}

#[tokio::test]
async fn a_slow_media_server_holds_up_only_the_media_calls_and_only_for_their_limit() {
    let rig = Rig::start_with(&[("Testing__MediaCommandTimeoutMs", "500")]).await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);
    first.join(rig.salon).await;
    let session = first.open_media(rig.salon).await;

    rig.media.set_slowness(Duration::from_secs(5));
    let clock = Instant::now();

    first.media_refused(rig.salon, &session, "offer", OFFER, UNAVAILABLE).await;
    first.refused("OpenMediaSession", json!([rig.salon]), UNAVAILABLE).await;
    // Presence and voice seats do not wait for the media server.
    second.ok("SetPresence", json!(["dnd"])).await;
    second.join(rig.salon).await;
    second.leave().await;
    first.leave().await;
    rig.occupants_become(rig.salon, &[]).await;

    // Two commands that gave up after their limit, and nothing that waited for
    // the media server's five seconds.
    assert!(clock.elapsed() < Duration::from_secs(4), "took {:?}", clock.elapsed());
    rig.media.set_slowness(Duration::ZERO);
    first.join(rig.salon).await;
    first.open_media(rig.salon).await;
}

#[tokio::test]
async fn a_seat_that_ends_while_its_session_is_being_created_gets_none() {
    let rig = Rig::start().await;
    let jonas = rig.person("jonas").await;
    let mut peer = rig.connect(&jonas).await;
    peer.join(rig.salon).await;
    rig.media.hold_creates();

    let opening = tokio::spawn(async move {
        let answer = peer.call("OpenMediaSession", json!([rig.salon])).await;
        (peer, answer)
    });
    // The room is only moved by the owner, who is not in it.
    let kicked_room = rig.salon;
    rig.media.count_reaches("POST", "/v1/sessions", 1).await;
    let kicked = rig
        .call(&rig.owner, Method::DELETE, &format!("/api/admin/channels/{kicked_room}/members/{}", jonas.id), None)
        .await;
    assert_eq!(kicked.status, 204);
    rig.media.release_creates();

    let (_peer, answer) = opening.await.unwrap();
    assert!(answer["error"].as_str().unwrap().ends_with(&format!("HubException: {NOT_SEATED}")), "{answer}");
    // The session the media server made for the seat that is gone is closed again.
    let created = created_sessions(&rig.media);
    assert_eq!(created.len(), 1);
    rig.media.next_request(|r| r.method == "DELETE").await;
    until(|| rig.media.sessions().is_empty()).await;
    assert_eq!(created[0].json()["userId"], jonas.id.as_str());
}

#[tokio::test]
async fn every_check_of_the_envelope_refuses_the_signal_before_the_media_server_sees_it() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut peer, mut elsewhere, mut other) =
        (rig.connect(&maja).await, rig.connect(&maja).await, rig.connect(&ada).await);
    peer.join(rig.salon).await;
    other.join(rig.salon).await;
    let own = peer.open_media(rig.salon).await;
    let others = other.open_media(rig.salon).await;
    let signal_of = |kind: Option<&str>, payload: Option<&str>, version: i64| {
        json!([media_signal(rig.salon, Some(&own), kind, payload, version)])
    };

    peer.refused("SendMediaSignal", signal_of(Some("offer"), Some(OFFER), 2), REFUSED).await;
    peer.refused("SendMediaSignal", signal_of(Some("offer"), Some(OFFER), 0), REFUSED).await;
    for kind in ["state", "tracks", "hello", "OFFER"] {
        peer.refused("SendMediaSignal", signal_of(Some(kind), Some("{}"), 1), REFUSED).await;
    }
    peer.refused("SendMediaSignal", signal_of(None, Some(OFFER), 1), REFUSED).await;
    peer.refused("SendMediaSignal", signal_of(Some("offer"), None, 1), REFUSED).await;
    peer.refused("SendMediaSignal", signal_of(Some("offer"), Some(&offer_of_length(MAX_SIGNAL_BYTES + 1)), 1), REFUSED).await;
    peer.refused("SendMediaSignal", json!([media_signal(rig.night, Some(&own), Some("offer"), Some(OFFER), 1)]), REFUSED).await;
    peer.refused("SendMediaSignal", json!([media_signal(rig.salon, Some(&others), Some("offer"), Some(OFFER), 1)]), REFUSED).await;
    peer.refused("SendMediaSignal", json!([media_signal(rig.salon, None, Some("offer"), Some(OFFER), 1)]), REFUSED).await;
    peer.refused("SendMediaSignal", json!([media_signal(rig.salon, Some("session-unknown"), Some("offer"), Some(OFFER), 1)]), REFUSED).await;
    elsewhere.refused("SendMediaSignal", json!([media_signal(rig.salon, Some(&own), Some("offer"), Some(OFFER), 1)]), REFUSED).await;
    peer.refused(
        "SendMediaSignal",
        json!([{"protocolVersion": 1, "channelId": "not-a-guid", "sessionId": own, "kind": "offer", "payload": OFFER}]),
        "Invalid ChannelId.",
    )
    .await;
    peer.refused("SendMediaSignal", json!([null]), REFUSED).await;
    peer.does_not_fit("SendMediaSignal", json!([])).await;
    peer.does_not_fit("SendMediaSignal", json!(["x"])).await;
    peer.does_not_fit("SendMediaSignal", json!([{"protocolVersion": 1, "channelId": rig.salon, "sessionId": 5, "kind": "offer", "payload": OFFER}])).await;

    // A payload has to be what its kind promises.
    for (kind, payload) in [
        ("offer", "{not json"),
        ("offer", "null"),
        ("offer", "{\"type\":\"answer\",\"sdp\":\"x\"}"),
        ("offer", "{\"type\":\"offer\"}"),
        ("offer", "{\"type\":\"offer\",\"sdp\":7}"),
        ("answer", "{\"type\":\"answer\",\"sdp\":\"x\"}"),
        ("answer", "{\"type\":\"answer\",\"sdp\":\"x\",\"negotiationId\":\"7\"}"),
        ("answer", "{\"type\":\"answer\",\"sdp\":\"x\",\"negotiationId\":1.5}"),
        ("answer", "{\"type\":\"offer\",\"sdp\":\"x\",\"negotiationId\":7}"),
        ("candidate", "\"candidate:1\""),
        ("candidate", "[]"),
    ] {
        peer.media_refused(rig.salon, &own, kind, payload, REFUSED).await;
    }

    assert!(rig.media.requests().iter().all(|r| !r.path.starts_with("/v1/sessions/")), "{:?}", rig.media.requests());

    // The refusals left the session as it was, and the limit itself passes.
    peer.media(rig.salon, &own, "offer", &offer_of_length(MAX_SIGNAL_BYTES)).await;
    assert_eq!(rig.media.count("POST", &format!("/v1/sessions/{own}/offer")), 1);
}

#[tokio::test]
async fn signals_reach_the_media_server_in_the_form_of_its_control_api_and_in_order() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;

    peer.media(rig.salon, &session, "offer", OFFER).await;
    peer.media(rig.salon, &session, "candidate", CANDIDATE).await;
    peer.media(rig.salon, &session, "candidate", "null").await;
    peer.media(rig.salon, &session, "answer", ANSWER).await;
    // Whitespace and escapes are written the way the first server wrote them.
    peer.media(rig.salon, &session, "offer", "{ \"type\" : \"offer\",\"sdp\":\"a+b <c> \\u00e9\\r\\n\" }").await;
    peer.media(rig.salon, &session, "candidate", " { \"candidate\" : \"a+b\" , \"n\": 1.50, \"t\": true } ").await;

    let prefix = format!("/v1/sessions/{session}/");
    let signals: Vec<_> = rig.media.requests().into_iter().filter(|r| r.path.starts_with(&prefix)).collect();
    let kinds: Vec<&str> = signals.iter().map(|r| r.path.rsplit('/').next().unwrap()).collect();
    assert_eq!(kinds, ["offer", "candidate", "candidate", "answer", "offer", "candidate"]);
    assert!(signals.iter().all(|r| r.method == "POST"));
    assert!(signals.iter().all(|r| r.authorization.as_deref() == Some(format!("Bearer {SECRET}").as_str())));
    assert_eq!(signals[0].body, "{\"sdp\":\"v=0 browser\"}");
    assert_eq!(signals[1].body, format!("{{\"candidate\":{CANDIDATE}}}"));
    assert_eq!(signals[2].body, "{\"candidate\":null}");
    assert_eq!(signals[3].body, "{\"sdp\":\"v=0 answer\",\"negotiationId\":7}");
    assert_eq!(signals[4].body, "{\"sdp\":\"a\\u002Bb \\u003Cc\\u003E \\u00E9\\r\\n\"}");
    assert_eq!(signals[5].body, "{\"candidate\":{\"candidate\":\"a\\u002Bb\",\"n\":1.50,\"t\":true}}");
}

#[tokio::test]
async fn voice_signal_refuses_signals_that_would_connect_browsers_directly() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut sender, mut receiver) = (rig.connect(&maja).await, rig.connect(&ada).await);
    sender.join(rig.salon).await;
    receiver.join(rig.salon).await;

    for (kind, payload) in [("offer", OFFER), ("answer", ANSWER), ("candidate", CANDIDATE)] {
        sender.refused("SendVoiceSignal", json!([signal(rig.salon, kind, payload, None)]), "Invalid voice signal.").await;
        sender
            .refused("SendVoiceSignal", json!([signal(rig.salon, kind, payload, Some(&ada.id))]), "Invalid voice signal.")
            .await;
    }
    assert!(receiver.drain("VoiceSignal").await.is_empty());

    let state = "{\"muted\":false,\"screenStreamId\":\"screen-1\",\"cameraStreamId\":null}";
    sender.state(rig.salon, state, None).await;
    assert_eq!(receiver.next("VoiceSignal").await["payload"], state);
}

#[tokio::test]
async fn a_stale_answer_passes_quietly_while_a_refused_or_failed_signal_is_reported() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;

    rig.media.answer_signals(&session, 409);
    peer.media(rig.salon, &session, "answer", ANSWER).await;
    rig.media.answer_signals(&session, 422);
    peer.media_refused(rig.salon, &session, "offer", OFFER, REFUSED).await;
    rig.media.answer_signals(&session, 413);
    peer.media_refused(rig.salon, &session, "offer", OFFER, REFUSED).await;
    rig.media.answer_signals(&session, 500);
    peer.media_refused(rig.salon, &session, "offer", OFFER, UNAVAILABLE).await;
    rig.media.answer_signals(&session, 401);
    peer.media_refused(rig.salon, &session, "offer", OFFER, UNAVAILABLE).await;

    // None of these ends the session.
    rig.media.answer_signals(&session, 204);
    peer.media(rig.salon, &session, "offer", OFFER).await;
    assert!(peer.drain("MediaSessionClosed").await.is_empty());
}

#[tokio::test]
async fn a_session_the_media_server_lost_is_forgotten_and_the_browser_told_to_replace_it() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    rig.media.drop_session(&session);

    peer.media(rig.salon, &session, "candidate", CANDIDATE).await;

    let closed = peer.next("MediaSessionClosed").await;
    assert_eq!(closed, json!({"channelId": rig.salon, "sessionId": session, "reason": "media-restarted"}));
    peer.media_refused(rig.salon, &session, "candidate", CANDIDATE, REFUSED).await;
    // The seat is kept; only the session went.
    peer.ok("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", None)])).await;
    let replacement = peer.open_media(rig.salon).await;
    peer.media(rig.salon, &replacement, "offer", OFFER).await;
    assert!(peer.drain("MediaSessionClosed").await.is_empty());
}

#[tokio::test]
async fn every_change_of_the_seat_closes_its_media_session_without_an_event() {
    for change in ["leave", "disconnect", "newer tab", "another room", "voice kick", "move", "server kick", "ban"] {
        let rig = Rig::start().await;
        let jonas = rig.person("jonas").await;
        let mut peer = rig.connect(&jonas).await;
        peer.join(rig.salon).await;
        let session = peer.open_media(rig.salon).await;
        let room_path = format!("/api/admin/channels/{}/members/{}", rig.salon, jonas.id);
        let mut tab = None;
        peer.drain("VoiceParticipantChanged").await;

        match change {
            "leave" => peer.leave().await,
            "disconnect" => {
                let Peer { hub, .. } = peer;
                hub.hang_up().await;
                rig.media.delete_of(&session).await;
                until(|| !rig.media.sessions().contains(&session)).await;
                continue;
            }
            "newer tab" => {
                let mut newer = rig.connect(&jonas).await;
                newer.join(rig.salon).await;
                peer.next("VoiceSessionReplaced").await;
                tab = Some(newer);
            }
            "another room" => peer.join(rig.night).await,
            "voice kick" => {
                assert_eq!(rig.call(&rig.owner, Method::DELETE, &room_path, None).await.status, 204);
                peer.next("VoiceChannelKicked").await;
            }
            "move" => {
                let moved = rig.call(&rig.owner, Method::PATCH, &room_path, Some(json!({"targetChannelId": rig.night}))).await;
                assert_eq!(moved.status, 204);
                peer.next("VoiceChannelMoved").await;
            }
            "server kick" => {
                let answer = rig.call(&rig.owner, Method::DELETE, &format!("/api/admin/members/{}", jonas.id), None).await;
                assert_eq!(answer.status, 204, "{}", answer.text());
            }
            "ban" => {
                let answer = rig.call(&rig.owner, Method::POST, &format!("/api/admin/members/{}/ban", jonas.id), Some(json!({"reason": null}))).await;
                assert_eq!(answer.status, 201, "{}", answer.text());
            }
            _ => unreachable!(),
        }
        if change == "server kick" || change == "ban" {
            // The room is told, and the person is not among those who hear it.
            rig.occupants_become(rig.salon, &[]).await;
            assert!(peer.drain("VoiceParticipantChanged").await.is_empty(), "{change}");
        }

        let closed = rig.media.delete_of(&session).await;
        assert_eq!(closed.authorization.as_deref(), Some(format!("Bearer {SECRET}").as_str()), "{change}");
        until(|| !rig.media.sessions().contains(&session)).await;
        assert!(peer.drain("MediaSessionClosed").await.is_empty(), "{change}");
        peer.media_refused(rig.salon, &session, "offer", OFFER, REFUSED).await;
        if change == "move" {
            // The client opens a session in the room it was moved to.
            peer.join(rig.night).await;
            let moved = peer.open_media(rig.night).await;
            peer.media(rig.night, &moved, "offer", OFFER).await;
        }
        drop(tab);
    }
}

#[tokio::test]
async fn ending_a_direct_call_closes_the_media_sessions_of_both_people() {
    for ending in ["decline", "block"] {
        let rig = Rig::start().await;
        let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
        let room = rig.conversation(&maja, &ada);
        let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);
        first.join(room).await;
        second.join(room).await;
        let first_session = first.open_media(room).await;
        let second_session = second.open_media(room).await;

        if ending == "decline" {
            second.ok("DeclineDirectCall", json!([room])).await;
        } else {
            let blocked = rig.call(&ada, Method::POST, &format!("/api/social/blocks/{}", maja.id), None).await;
            assert_eq!(blocked.status, 204, "{}", blocked.text());
        }

        rig.media.delete_of(&first_session).await;
        rig.media.delete_of(&second_session).await;
        for peer in [&mut first, &mut second] {
            peer.next("DirectCallDeclined").await;
            assert!(peer.drain("MediaSessionClosed").await.is_empty(), "{ending}");
        }
    }
}

#[tokio::test]
async fn joining_the_room_the_connection_sits_in_keeps_its_session() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;

    peer.join(rig.salon).await;

    peer.media(rig.salon, &session, "offer", OFFER).await;
    assert_eq!(rig.media.count("DELETE", &format!("/v1/sessions/{session}")), 0);
}

#[tokio::test]
async fn a_member_removed_from_connect_still_has_the_media_of_the_seat_they_hold() {
    let rig = Rig::start().await;
    let jonas = rig.person("jonas").await;
    let mut peer = rig.connect(&jonas).await;
    peer.join(rig.salon).await;
    rig.base_role_loses(1 << 12);

    let session = peer.open_media(rig.salon).await;
    peer.media(rig.salon, &session, "offer", OFFER).await;
    peer.leave().await;
    peer.refused("JoinVoice", json!([rig.salon]), NOT_ALLOWED).await;
    peer.refused("OpenMediaSession", json!([rig.salon]), NOT_SEATED).await;
}

// Receive subscriptions: the complete, ordered allowlist of what the media
// server forwards to a session.

fn mids_body(mids: &[String]) -> String {
    serde_json::to_string(&json!({ "mids": mids })).unwrap()
}

#[tokio::test]
async fn subscriptions_forward_the_complete_mid_allowlist_including_empty_and_unknown_mids() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let path = format!("/v1/sessions/{session}/subscriptions");

    peer.media(rig.salon, &session, "subscriptions", "{\"mids\":[]}").await;
    let mids: Vec<String> = ["0", "screen-1", "!~", "abcdefghijklmnop", "A", "a"].iter().map(|s| s.to_string()).collect();
    peer.media(rig.salon, &session, "subscriptions", &mids_body(&mids)).await;

    let requests: Vec<_> = rig.media.requests().into_iter().filter(|r| r.path == path).collect();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.method == "POST"));
    assert!(requests.iter().all(|r| r.authorization.as_deref() == Some(format!("Bearer {SECRET}").as_str())));
    assert_eq!(requests[0].body, "{\"mids\":[]}");
    assert_eq!(requests[1].json(), json!({"mids": mids}));
    assert_eq!(requests[1].body, "{\"mids\":[\"0\",\"screen-1\",\"!~\",\"abcdefghijklmnop\",\"A\",\"a\"]}");
}

#[tokio::test]
async fn the_order_of_the_list_reaches_the_media_server_as_sent_since_it_is_the_priority() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let path = format!("/v1/sessions/{session}/subscriptions");
    let list = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let voices_first = list(&["a1", "a2", "share", "share-sound", "cam-1", "cam-2"]);
    let cameras_first = list(&["cam-2", "cam-1", "a2", "a1", "share-sound", "share"]);

    peer.media(rig.salon, &session, "subscriptions", &mids_body(&voices_first)).await;
    peer.media(rig.salon, &session, "subscriptions", &mids_body(&cameras_first)).await;

    let requests: Vec<_> = rig.media.requests().into_iter().filter(|r| r.path == path).collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].json()["mids"], json!(voices_first));
    assert_eq!(requests[1].json()["mids"], json!(cameras_first));
}

#[tokio::test]
async fn malformed_subscription_allowlists_never_reach_the_media_server() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let too_many: Vec<String> = (0..4097).map(|n| format!("{n:016}")).collect();

    let mut bad: Vec<String> = [
        "{not json", "null", "[]", "{}", "{\"mids\":null}", "{\"mids\":\"0\"}", "{\"mids\":{}}", "{\"mids\":[null]}",
        "{\"mids\":[0]}", "{\"mids\":[true]}", "{\"mids\":[{}]}", "{\"mids\":[[]]}", "{\"mids\":[\"\"]}",
        "{\"mids\":[\"abcdefghijklmnopq\"]}", "{\"mids\":[\"a b\"]}", "{\"mids\":[\"a\\tb\"]}", "{\"mids\":[\"a\\nb\"]}",
        "{\"mids\":[\"\\u0000\"]}", "{\"mids\":[\"\\u007f\"]}", "{\"mids\":[\"é\"]}", "{\"mids\":[\"0\",\"0\"]}",
        "{\"mids\":[\"0\",\"\\u0030\"]}", "{\"mids\":[],\"mids\":[\"0\"]}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    bad.push(mids_body(&too_many));
    for payload in bad {
        peer.media_refused(rig.salon, &session, "subscriptions", &payload, REFUSED).await;
    }

    assert_eq!(rig.media.count("POST", &format!("/v1/sessions/{session}/subscriptions")), 0);
}

#[tokio::test]
async fn subscription_count_and_envelope_limits_are_inclusive() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let path = format!("/v1/sessions/{session}/subscriptions");
    let mids: Vec<String> = (0..4096).map(|n| format!("{n:016}")).collect();

    peer.media(rig.salon, &session, "subscriptions", &mids_body(&mids)).await;
    let largest: Vec<_> = rig.media.requests().into_iter().filter(|r| r.path == path).collect();
    assert_eq!(largest.len(), 1);
    assert_eq!(largest[0].json()["mids"], json!(mids));

    let empty = "{\"mids\":[]}";
    let at_limit = format!("{empty}{}", " ".repeat(MAX_SIGNAL_BYTES - empty.len()));
    peer.media_refused(rig.salon, &session, "subscriptions", &format!("{at_limit} "), REFUSED).await;
    assert_eq!(rig.media.count("POST", &path), 1);
    peer.media(rig.salon, &session, "subscriptions", &at_limit).await;
    assert_eq!(rig.media.count("POST", &path), 2);
}

#[tokio::test]
async fn subscriptions_require_the_callers_current_session_and_seat() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut peer, mut elsewhere, mut other) =
        (rig.connect(&maja).await, rig.connect(&maja).await, rig.connect(&ada).await);
    peer.join(rig.salon).await;
    other.join(rig.salon).await;
    let own = peer.open_media(rig.salon).await;
    let others = other.open_media(rig.salon).await;
    let payload = "{\"mids\":[\"0\"]}";

    let envelope = |room: Id, session: Option<&str>, version: i64| json!([media_signal(room, session, Some("subscriptions"), Some(payload), version)]);
    peer.refused("SendMediaSignal", envelope(rig.salon, Some(&own), 2), REFUSED).await;
    peer.refused("SendMediaSignal", envelope(rig.night, Some(&own), 1), REFUSED).await;
    peer.refused("SendMediaSignal", envelope(rig.salon, Some(&others), 1), REFUSED).await;
    elsewhere.refused("SendMediaSignal", envelope(rig.salon, Some(&own), 1), REFUSED).await;
    peer.refused("SendMediaSignal", envelope(rig.salon, None, 1), REFUSED).await;
    peer.refused("SendMediaSignal", envelope(rig.salon, Some("session-unknown"), 1), REFUSED).await;
    let replacement = peer.open_media(rig.salon).await;
    peer.refused("SendMediaSignal", envelope(rig.salon, Some(&own), 1), REFUSED).await;
    peer.leave().await;
    peer.refused("SendMediaSignal", envelope(rig.salon, Some(&replacement), 1), REFUSED).await;

    assert!(rig.media.requests().iter().all(|r| !r.path.ends_with("/subscriptions")));
}

#[tokio::test]
async fn subscription_failures_use_the_existing_media_signal_outcomes() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let payload = "{\"mids\":[]}";

    rig.media.answer_signals(&session, 422);
    peer.media_refused(rig.salon, &session, "subscriptions", payload, REFUSED).await;
    rig.media.answer_signals(&session, 500);
    peer.media_refused(rig.salon, &session, "subscriptions", payload, UNAVAILABLE).await;

    rig.media.answer_signals(&session, 404);
    rig.media.drop_session(&session);
    peer.media(rig.salon, &session, "subscriptions", payload).await;
    let closed = peer.next("MediaSessionClosed").await;
    assert_eq!(closed["sessionId"], session.as_str());
    assert_eq!(closed["reason"], "media-restarted");
    peer.media_refused(rig.salon, &session, "subscriptions", payload, REFUSED).await;
}

// The media event pump: what the media server says reaches the browser whose
// seat holds the session, in order, and a media server that restarts, loses
// its events or goes away is handled as the contract says.

#[tokio::test]
async fn signals_of_the_media_server_reach_their_session_in_order() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);
    first.join(rig.salon).await;
    second.join(rig.salon).await;
    let first_session = first.open_media(rig.salon).await;
    let second_session = second.open_media(rig.salon).await;

    let count = 60;
    for i in 1..=count {
        rig.media.signal(if i % 2 == 0 { &first_session } else { &second_session }, "offer", &media_offer(i));
    }
    // A directory as the media server writes it, which the hub relays as it is.
    let tracks = format!(
        "{{\"tracks\":[{{\"heldBack\":false,\"kind\":\"audio\",\"mid\":\"4\",\"streamId\":\"mic\",\"userId\":\"{}\"}},{{\"heldBack\":true,\"kind\":\"video\",\"mid\":\"5\",\"streamId\":\"mic\",\"userId\":\"{}\"}}]}}",
        ada.id, ada.id
    );
    rig.media.signal(&first_session, "tracks", &tracks);

    for (peer, session, start) in [(&mut first, &first_session, 2), (&mut second, &second_session, 1)] {
        for i in (start..=count).step_by(2) {
            let signal = peer.next("MediaSignal").await;
            assert_eq!(
                signal,
                json!({"protocolVersion": 1, "channelId": rig.salon, "sessionId": session, "kind": "offer", "payload": media_offer(i)})
            );
        }
    }
    let directory = first.next("MediaSignal").await;
    assert_eq!(directory["kind"], "tracks");
    assert_eq!(directory["payload"], tracks.as_str());
    assert!(second.drain("MediaSignal").await.is_empty());
}

#[tokio::test]
async fn an_event_issued_while_the_session_is_being_created_still_reaches_the_browser() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    rig.media.signal_on_create(Some(("offer", &media_offer(1))));

    let session = peer.open_media(rig.salon).await;

    // The media server issued the offer and the pump fetched it before the seat
    // held the session; it is handed out once it does.
    let offer = peer.next("MediaSignal").await;
    assert_eq!(offer["sessionId"], session.as_str());
    assert_eq!(offer["payload"], media_offer(1).as_str());
}

#[tokio::test]
async fn an_event_for_a_session_no_seat_holds_reaches_nobody() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let earlier = peer.open_media(rig.salon).await;
    let current = peer.open_media(rig.salon).await;

    rig.media.signal(&earlier, "offer", &media_offer(1));
    rig.media.closed(&earlier, "error");
    rig.media.signal("session-of-nobody", "offer", &media_offer(2));
    rig.media.signal(&current, "offer", &media_offer(3));

    // Events are handed out in order, so this one comes after the pump went past
    // everything before it.
    let only = peer.next("MediaSignal").await;
    assert_eq!(only["sessionId"], current.as_str());
    assert_eq!(only["payload"], media_offer(3).as_str());
    assert!(peer.drain("MediaSessionClosed").await.is_empty());
    peer.media(rig.salon, &current, "offer", OFFER).await;
}

#[tokio::test]
async fn a_session_the_media_plane_closed_is_reported_with_its_reason() {
    for reason in ["transport-timeout", "error", "something-else"] {
        let rig = Rig::start().await;
        let maja = rig.person("maja").await;
        let mut peer = rig.connect(&maja).await;
        peer.join(rig.salon).await;
        let session = peer.open_media(rig.salon).await;

        rig.media.signal(&session, "answer", "{\"type\":\"answer\",\"sdp\":\"v=0 media\"}");
        rig.media.closed(&session, reason);

        assert_eq!(peer.next("MediaSignal").await["kind"], "answer");
        let closed = peer.next("MediaSessionClosed").await;
        let told = if reason == "transport-timeout" { reason } else { "error" };
        assert_eq!(closed, json!({"channelId": rig.salon, "sessionId": session, "reason": told}));
        peer.media_refused(rig.salon, &session, "offer", OFFER, REFUSED).await;

        // The media server already let go of it; closing it again is not needed.
        assert_eq!(rig.media.count("DELETE", &format!("/v1/sessions/{session}")), 0);
        let replacement = peer.open_media(rig.salon).await;
        peer.media(rig.salon, &replacement, "offer", OFFER).await;
    }
}

#[tokio::test]
async fn events_the_pump_cannot_use_are_ignored_and_do_not_stop_the_ones_after_them() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;

    rig.media.signal(&session, "candidate", "{\"candidate\":\"x\"}");
    rig.media.signal(&session, "offer", "[]");
    rig.media.signal(&session, "offer", "null");
    rig.media.raw_event(|seq| format!("{{\"seq\":{seq},\"type\":\"hello\",\"sessionId\":\"{session}\"}}"));
    rig.media.raw_event(|seq| format!("{{\"seq\":{seq},\"type\":\"signal\",\"sessionId\":\"{session}\",\"payload\":{{}}}}"));
    rig.media.signal(&session, "offer", &media_offer(9));

    let only = peer.next("MediaSignal").await;
    assert_eq!(only["payload"], media_offer(9).as_str());
}

#[tokio::test]
async fn a_restart_of_the_media_server_ends_only_sessions_of_earlier_epochs() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);
    first.join(rig.salon).await;
    second.join(rig.salon).await;
    let before = first.open_media(rig.salon).await;

    // The pump is kept from noticing the restart until a session has been opened
    // in the new epoch.
    let polls = rig.media.count("GET", "/v1/events");
    rig.media.hold_polls();
    rig.media.count_reaches("GET", "/v1/events", polls + 1).await;
    rig.media.restart("epoch-2");
    let after = second.open_media(rig.salon).await;
    rig.media.release_polls();

    let closed = first.next("MediaSessionClosed").await;
    assert_eq!(closed["sessionId"], before.as_str());
    assert_eq!(closed["reason"], "media-restarted");
    first.media_refused(rig.salon, &before, "offer", OFFER, REFUSED).await;

    // The session of the new epoch lives on, and the pump follows that epoch
    // from its start.
    rig.media.signal(&after, "offer", &media_offer(1));
    assert_eq!(second.next("MediaSignal").await["payload"], media_offer(1).as_str());
    assert!(rig.media.requests().iter().any(|r| r.path == "/v1/events" && r.query.contains("epoch=epoch-2&after=0")));
    assert!(second.drain("MediaSessionClosed").await.is_empty());
    second.media(rig.salon, &after, "offer", OFFER).await;
    assert_eq!(rig.media.count("POST", "/v1/reset"), 1);
}

#[tokio::test]
async fn losing_the_window_of_events_resets_the_media_server_and_ends_every_session() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut first, mut second) = (rig.connect(&maja).await, rig.connect(&ada).await);
    first.join(rig.salon).await;
    second.join(rig.salon).await;
    let first_session = first.open_media(rig.salon).await;
    let second_session = second.open_media(rig.salon).await;
    rig.media.signal(&first_session, "offer", &media_offer(1));
    first.next("MediaSignal").await;

    rig.media.lose_window();

    for (peer, session) in [(&mut first, &first_session), (&mut second, &second_session)] {
        let closed = peer.next("MediaSessionClosed").await;
        assert_eq!(closed["sessionId"], session.as_str());
        assert_eq!(closed["reason"], "media-restarted");
    }
    assert_eq!(rig.media.count("POST", "/v1/reset"), 2);

    // After the reset the pump follows on from where the media server is.
    rig.media.count_reaches("GET", "/v1/events", 3).await;
    let reopened = first.open_media(rig.salon).await;
    rig.media.signal(&reopened, "offer", &media_offer(2));
    assert_eq!(first.next("MediaSignal").await["payload"], media_offer(2).as_str());
}

#[tokio::test]
async fn the_event_pump_waits_for_a_media_server_that_is_down_and_carries_on_when_it_is_back() {
    let media = FakeMedia::start().await;
    media.set_down(true);
    let rig = Rig::start_behind(media, &[], false).await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;

    // Down at start: the reset is tried again and again, and no session opens
    // meanwhile.
    rig.media.count_reaches("POST", "/v1/reset", 3).await;
    peer.refused("OpenMediaSession", json!([rig.salon]), UNAVAILABLE).await;
    rig.media.set_down(false);
    rig.media.count_reaches("GET", "/v1/events", 1).await;
    let resets = rig.media.count("POST", "/v1/reset");
    let session = peer.open_media(rig.salon).await;

    // Down for a while in the same run: nothing is lost and the session stays.
    rig.media.set_down(true);
    let polls = rig.media.count("GET", "/v1/events");
    rig.media.count_reaches("GET", "/v1/events", polls + 3).await;
    rig.media.signal(&session, "offer", &media_offer(1));
    rig.media.set_down(false);
    assert_eq!(peer.next("MediaSignal").await["payload"], media_offer(1).as_str());
    peer.media(rig.salon, &session, "answer", ANSWER).await;
    assert!(peer.drain("MediaSessionClosed").await.is_empty());

    // Down and restarted meanwhile: the first answer after the outage tells.
    rig.media.set_down(true);
    let polls = rig.media.count("GET", "/v1/events");
    rig.media.count_reaches("GET", "/v1/events", polls + 2).await;
    rig.media.restart("epoch-2");
    rig.media.set_down(false);
    let closed = peer.next("MediaSessionClosed").await;
    assert_eq!(closed["sessionId"], session.as_str());
    assert_eq!(closed["reason"], "media-restarted");
    // A new epoch needs no reset of its own: the new run has no sessions.
    assert_eq!(rig.media.count("POST", "/v1/reset"), resets);
    peer.open_media(rig.salon).await;
}

// A media server that answers nonsense, or too slowly, never holds a hub call
// for longer than its limit and never takes the API down.

#[tokio::test]
async fn a_media_server_that_answers_nonsense_is_unavailable_and_nothing_else() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let canned = |status: u16, content_type: Option<&str>, body: &[u8]| {
        Some(Canned { status, content_type: content_type.map(str::to_string), body: body.to_vec() })
    };

    for status in [302, 400, 401, 500] {
        rig.media.answer_everything_with(canned(status, Some("application/json"), b"nonsense"));
        peer.media_refused(rig.salon, &session, "offer", OFFER, UNAVAILABLE).await;
    }


    // A signal the media server answered with a success is taken whatever it says.
    rig.media.answer_everything_with(canned(200, Some("text/plain"), b"nonsense"));
    peer.media(rig.salon, &session, "offer", OFFER).await;

    for (status, content_type, body) in [
        (201, Some("application/json"), &b"not json"[..]),
        (201, Some("text/plain"), &b"{\"sessionId\":\"x\",\"epoch\":\"e\"}"[..]),
        (201, Some("application/json"), &b"{}"[..]),
        (201, Some("application/json"), &b"{\"sessionId\":\"\",\"epoch\":\"e\"}"[..]),
        (200, Some("application/json"), &b"{\"sessionId\":\"x\",\"epoch\":\"e\"}"[..]),
        (500, Some("application/json"), &b"{\"error\":\"boom\"}"[..]),
        (503, None, &b""[..]),
        (302, None, &b""[..]),
    ] {
        rig.media.answer_everything_with(canned(status, content_type, body));
        // A connection of its own for each: opening sessions is counted.
        let mut fresh = rig.connect(&maja).await;
        fresh.join(rig.salon).await;
        fresh.refused("OpenMediaSession", json!([rig.salon]), UNAVAILABLE).await;
    }
    peer.join(rig.salon).await;
    // The seat and the room are untouched, and the media server coming back to
    // its senses is all it takes.
    peer.ok("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", None)])).await;
    rig.media.answer_everything_with(None);
    let again = peer.open_media(rig.salon).await;
    peer.media(rig.salon, &again, "offer", OFFER).await;
}

#[tokio::test]
async fn a_media_server_that_answers_the_poll_with_nonsense_is_waited_for_and_the_api_stays_up() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let canned = |status: u16, body: &str| Some(Canned { status, content_type: Some("application/json".into()), body: body.as_bytes().to_vec() });

    for (status, body) in [
        (200, "not json"),
        (200, "{}"),
        (200, "{\"epoch\":\"\",\"reset\":false,\"events\":[]}"),
        (200, "{\"epoch\":\"epoch-1\",\"reset\":false,\"events\":[null]}"),
        (200, "{\"epoch\":\"epoch-1\",\"reset\":false,\"events\":[{\"seq\":1}]}"),
        (500, "{}"),
    ] {
        let polls = rig.media.count("GET", "/v1/events");
        rig.media.answer_polls_with(canned(status, body));
        rig.media.count_reaches("GET", "/v1/events", polls + 2).await;
    }
    rig.media.answer_polls_with(None);

    // What was said while it did not make sense is not lost: the pump went on
    // from the same place.
    rig.media.signal(&session, "offer", &media_offer(1));
    assert_eq!(peer.next("MediaSignal").await["payload"], media_offer(1).as_str());
    peer.ok("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", None)])).await;
}

#[tokio::test]
async fn a_poll_answered_with_more_than_a_megabyte_is_read() {
    let rig = Rig::start().await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    let sdp = "v".repeat(100 * 1024);

    // Forty events of a hundred kilobytes each come in one answer.
    for number in 1..=40u32 {
        rig.media.signal(&session, "offer", &format!("{{\"type\":\"offer\",\"sdp\":\"{sdp}\",\"negotiationId\":{number}}}"));
    }

    for number in 1..=40u32 {
        let signal = peer.next("MediaSignal").await;
        let payload: Value = serde_json::from_str(signal["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["negotiationId"], number);
    }
}

#[tokio::test]
async fn a_media_server_that_never_answers_holds_a_call_for_its_limit_and_no_longer() {
    let rig = Rig::start_with(&[("Testing__MediaCommandTimeoutMs", "400")]).await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;
    let session = peer.open_media(rig.salon).await;
    rig.media.set_slowness(Duration::from_secs(30));

    let clock = Instant::now();
    peer.media_refused(rig.salon, &session, "offer", OFFER, UNAVAILABLE).await;
    peer.refused("OpenMediaSession", json!([rig.salon]), UNAVAILABLE).await;

    assert!(clock.elapsed() < Duration::from_secs(5), "took {:?}", clock.elapsed());
    peer.ok("GetRtcClock", json!([])).await;
}

#[tokio::test]
async fn the_media_server_is_asked_with_the_secret_and_a_wrong_one_is_refused() {
    let rig = Rig::start_behind(FakeMedia::start().await, &[("Rtc__MediaControlSecret", "another-secret")], false).await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;

    // The fake wants the test secret; the pump is refused and voice waits.
    until(|| rig.media.count("POST", "/v1/reset") >= 2).await;
    peer.refused("OpenMediaSession", json!([rig.salon]), UNAVAILABLE).await;
    assert!(rig.media.requests().iter().all(|r| r.authorization.as_deref() == Some("Bearer another-secret")));
}

#[tokio::test]
async fn without_a_secret_no_credentials_are_sent() {
    let media = FakeMedia::start().await;
    media.set_bearer(None);
    let rig = Rig::start_behind(media, &[("Rtc__MediaControlSecret", "")], true).await;
    let maja = rig.person("maja").await;
    let mut peer = rig.connect(&maja).await;
    peer.join(rig.salon).await;

    peer.open_media(rig.salon).await;

    assert!(rig.media.requests().iter().all(|r| r.authorization.is_none()));
}

#[tokio::test]
async fn signing_out_closes_its_hub_and_its_media_but_leaves_another_account_active() {
    let rig = Rig::start().await;
    let (maja, ada) = (rig.person("maja").await, rig.person("ada").await);
    let (mut leaving, mut staying) = (rig.connect(&maja).await, rig.connect(&ada).await);
    leaving.join(rig.salon).await;
    staying.join(rig.salon).await;
    let leaving_session = leaving.open_media(rig.salon).await;
    let staying_session = staying.open_media(rig.salon).await;
    until(|| rig.media.sessions().contains(&leaving_session) && rig.media.sessions().contains(&staying_session)).await;

    let signed_out = maja.client.post_json("/api/auth/logout", &json!({})).await;
    assert_eq!(signed_out.status, 204, "{}", signed_out.text());

    let (_, _) = leaving.hub.until_close().await;
    until(|| !rig.media.sessions().contains(&leaving_session)).await;
    rig.media.delete_of(&leaving_session).await;
    assert!(staying.ok("GetRtcClock", json!([])).await.as_i64().unwrap() > 0);
    assert!(rig.media.sessions().contains(&staying_session));
    rig.occupants_become(rig.salon, &[&ada.id]).await;
}

#[tokio::test]
async fn a_seat_moved_to_another_room_while_its_session_is_being_created_gets_none() {
    let rig = Rig::start().await;
    let jonas = rig.person("jonas").await;
    let mut peer = rig.connect(&jonas).await;
    peer.join(rig.salon).await;
    rig.media.hold_creates();

    let room = rig.salon;
    let opening = tokio::spawn(async move {
        let answer = peer.call("OpenMediaSession", json!([room])).await;
        (peer, answer)
    });
    rig.media.count_reaches("POST", "/v1/sessions", 1).await;
    // A moderator moves the member while the media server is still making the session.
    let path = format!("/api/admin/channels/{}/members/{}", rig.salon, jonas.id);
    let moved = rig.call(&rig.owner, Method::PATCH, &path, Some(json!({"targetChannelId": rig.night}))).await;
    assert_eq!(moved.status, 204, "{}", moved.text());
    rig.media.release_creates();

    let (mut peer, answer) = opening.await.unwrap();
    // The seat is in the other room now: the session is not the new seat's.
    assert!(answer["error"].as_str().unwrap().ends_with(&format!("HubException: {NOT_SEATED}")), "{answer}");
    let created = created_sessions(&rig.media);
    assert_eq!(created.len(), 1);
    rig.media.next_request(|r| r.method == "DELETE").await;
    until(|| rig.media.sessions().is_empty()).await;
    // The seat in the new room has no session of that made, and may open its own.
    peer.media_refused(rig.night, "session-1", "offer", OFFER, REFUSED).await;
    peer.join(rig.night).await;
    let own = peer.open_media(rig.night).await;
    assert_ne!(own, "session-1");
}

#[tokio::test]
async fn a_member_whose_membership_row_is_removed_under_a_seat_may_neither_signal_nor_play() {
    let rig = Rig::start().await;
    let (jonas, maja) = (rig.person("jonas").await, rig.person("maja").await);
    let (mut peer, mut other) = (rig.connect(&jonas).await, rig.connect(&maja).await);
    peer.join(rig.salon).await;
    other.join(rig.salon).await;
    // A sound of the server, so that the refusal is about the member and not the clip.
    let sound = crate::voice_moderation::add_sound_for_tests(&rig);
    peer.ok("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", None)])).await;
    peer.ok("PlaySoundboard", json!([rig.salon, sound])).await;
    other.drain("VoiceSignal").await;

    // Removed behind the application's back: the seat is still there.
    rig.db()
        .execute(
            "DELETE FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
            rusqlite::params![rig.server_id, Id::parse(&jonas.id).unwrap()],
        )
        .unwrap();

    peer.refused("SendVoiceSignal", json!([signal(rig.salon, "state", "{}", None)]), "Invalid voice signal.").await;
    peer.refused("PlaySoundboard", json!([rig.salon, sound]), "Unknown voice channel.").await;
    assert!(other.drain("VoiceSignal").await.is_empty());
}
