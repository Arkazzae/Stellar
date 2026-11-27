//! A fault point for the tests. It makes the next read of untrusted input at
//! one place panic, as a bug in a parser would, and the tests below prove
//! that such a panic costs that input or its session, never the media task.
//! Each test fails with the injected panic when the guard around its read is
//! taken away.

use std::cell::Cell;

use str0m::ice::TransId;

use super::*;

thread_local! {
    /// The read that panics next.
    static ARMED: Cell<Option<Untrusted>> = const { Cell::new(None) };
}

/// Runs before every read of untrusted input.
pub(super) fn trip(at: Untrusted) {
    if ARMED.get() == Some(at) {
        ARMED.set(None);
        panic!("a fault injected while reading {at:?}");
    }
}

/// Makes the next read at `at` panic.
fn arm(at: Untrusted) {
    ARMED.set(Some(at));
}

/// Whether the armed read happened.
fn tripped() -> bool {
    ARMED.get().is_none()
}

const SERVER: &str = "203.0.113.7:49000";
const BROWSER: &str = "198.51.100.9:50000";
const ROOM: &str = "00000000-0000-4000-8000-00000000aaaa";
/// A real offer of Chromium for a microphone and a camera.
const CHROMIUM_OFFER: &str = include_str!("../../tests/fixtures/chromium-offer.sdp");

fn new_core() -> Core {
    let candidates = vec![SERVER.parse().unwrap()];
    Core::with_tcp(CoreConfig { candidates }, "e".into(), true)
}

/// A new session, and the connectivity check its browser sends first.
fn join(core: &mut Core, now: Instant, user: u64) -> (String, SessionKey, Vec<u8>) {
    let user = format!("00000000-0000-4000-8000-{user:012x}");
    let id = core.create_session(now, ROOM, &user).unwrap().session_id;
    let key = core.by_id[&id];
    let creds = core
        .sessions
        .get_mut(&key)
        .unwrap()
        .rtc
        .direct_api()
        .local_ice_credentials();
    let username = format!("{}:browser", creds.ufrag);
    let hmac = core.crypto.sha1_hmac_provider;
    let mut check = vec![0; 512];
    let len = StunMessage::binding_request(&username, TransId::new(), true, 1, 1, false)
        .to_bytes(Some(creds.pass.as_bytes()), &mut check, |key, data| {
            hmac.sha1_hmac(key, data)
        })
        .unwrap();
    check.truncate(len);
    (id, key, check)
}

/// The latest signal of `kind` for a session.
fn signal(core: &Core, session: &str, kind: SignalKind) -> Value {
    core.events(Some("e"), 0)
        .events
        .into_iter()
        .rev()
        .find_map(|event| match &event.body {
            EventBody::Signal {
                session_id,
                kind: k,
                payload,
            } if session_id == session && *k == kind => Some(payload.clone()),
            _ => None,
        })
        .expect("a signal")
}

fn closed_with_error(core: &Core, session: &str) -> bool {
    let closed = EventBody::Closed {
        session_id: session.into(),
        reason: CloseReason::Error,
    };
    core.events(Some("e"), 0)
        .events
        .iter()
        .any(|event| event.body == closed)
}

#[test]
fn a_panic_reading_a_packet_costs_only_that_packet() {
    // The read that panics, the transport, and whether the session already
    // owns the address. A check from an address its session owns fails the
    // address path and is still found by its credentials.
    let cases = [
        (Untrusted::Datagram, Protocol::Udp, false),
        (Untrusted::Ufrag, Protocol::Udp, false),
        (Untrusted::Accepts, Protocol::Udp, true),
        (Untrusted::Datagram, Protocol::Tcp, false),
        (Untrusted::TcpCheck, Protocol::Tcp, false),
    ];
    for (at, protocol, owned) in cases {
        let now = Instant::now();
        let mut core = new_core();
        let (id, key, check) = join(&mut core, now, 1);
        let from = Endpoint {
            protocol,
            remote: BROWSER.parse().unwrap(),
            local: SERVER.parse().unwrap(),
        };
        if owned {
            assert_eq!(core.handle_packet(now, from, &check, None), Some(key));
        }

        arm(at);
        // For a first TCP frame, `None` authorises nothing.
        let routed = core.handle_packet(now, from, &check, None);
        assert!(tripped(), "{at:?} was read");
        assert_eq!(routed, owned.then_some(key), "{at:?}");

        // The same check again is the session's.
        assert!(core.has_session(&id));
        let routed = core.handle_packet(now, from, &check, None);
        assert_eq!(routed, Some(key), "{at:?}");
    }
}

#[test]
fn a_panic_reading_a_signal_costs_at_most_its_session() {
    let now = Instant::now();
    let mut core = new_core();
    let offer = CHROMIUM_OFFER.replace("\r\n", "\n").replace('\n', "\r\n");

    // Before the `Rtc` has seen a candidate or an offer, a panic refuses it
    // like a malformed one and the session stays as it was.
    let (a, a_key, _) = join(&mut core, now, 1);
    let candidate = json!({
        "candidate": "candidate:1 1 udp 2130706431 10.1.0.1 5000 typ host",
        "sdpMid": "0",
        "sdpMLineIndex": 0,
    });
    arm(Untrusted::Candidate);
    assert!(matches!(
        core.candidate(now, &a, Some(&candidate)),
        Err(ApiError::InvalidCandidate(_))
    ));
    assert!(tripped());
    assert_eq!(core.sessions[&a_key].remote_candidates, 0);
    assert_eq!(core.candidate(now, &a, Some(&candidate)), Ok(()));
    assert_eq!(core.sessions[&a_key].remote_candidates, 1);

    let seq = core.last_seq();
    arm(Untrusted::BrowserOffer);
    assert!(matches!(
        core.offer(now, &a, &offer),
        Err(ApiError::InvalidSdp(_))
    ));
    assert!(tripped());
    assert_eq!(core.last_seq(), seq, "neither answered nor closed");
    assert_eq!(core.offer(now, &a, &offer), Ok(()));

    // The media server offers a second publisher what the first publishes,
    // and its offer keeps waiting for a good answer.
    let (b, _, _) = join(&mut core, now, 2);
    let mut browser = Rtc::builder().set_rtp_mode(true).build(now);
    browser.add_local_candidate(Candidate::host(BROWSER.parse().unwrap(), "udp").unwrap());
    let mut api = browser.sdp_api();
    api.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    let (own, pending) = api.apply().unwrap();
    core.offer(now, &b, &own.to_sdp_string()).unwrap();
    let answer = signal(&core, &b, SignalKind::Answer);
    let answer = SdpAnswer::from_sdp_string(answer["sdp"].as_str().unwrap()).unwrap();
    browser.sdp_api().accept_answer(pending, answer).unwrap();
    let offered = signal(&core, &b, SignalKind::Offer);
    let negotiation = offered["negotiationId"].as_u64().unwrap();
    let server_offer = SdpOffer::from_sdp_string(offered["sdp"].as_str().unwrap()).unwrap();
    let reply = browser.sdp_api().accept_offer(server_offer).unwrap();
    let reply = reply.to_sdp_string();
    arm(Untrusted::BrowserAnswer);
    assert!(matches!(
        core.answer(now, &b, &reply, negotiation),
        Err(ApiError::InvalidSdp(_))
    ));
    assert!(tripped());
    assert_eq!(core.answer(now, &b, &reply, negotiation), Ok(()));

    // Once the `Rtc` has taken an offer, a panic in writing the answer
    // closes that session and no other.
    let (c, _, _) = join(&mut core, now, 3);
    arm(Untrusted::ServerAnswer);
    assert!(matches!(
        core.offer(now, &c, &offer),
        Err(ApiError::InvalidSdp(_))
    ));
    assert!(tripped());
    assert!(!core.has_session(&c));
    assert!(closed_with_error(&core, &c));
    assert!(core.has_session(&a) && core.has_session(&b));
    let (c, _, _) = join(&mut core, now, 3);
    assert_eq!(core.offer(now, &c, &offer), Ok(()));

    // A session that publishes nothing is answered and then offered what
    // the room publishes. A panic in writing that offer closes it and no
    // other.
    let mut listener = Rtc::builder().set_rtp_mode(true).build(now);
    let mut api = listener.sdp_api();
    api.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
    let (listens, _) = api.apply().unwrap();
    let listens = listens.to_sdp_string();
    let (d, _, _) = join(&mut core, now, 4);
    arm(Untrusted::ServerOffer);
    assert_eq!(core.offer(now, &d, &listens), Ok(()));
    assert!(tripped());
    assert!(!core.has_session(&d));
    assert!(closed_with_error(&core, &d));
    assert!(core.has_session(&a) && core.has_session(&b) && core.has_session(&c));
    let (d, _, _) = join(&mut core, now, 4);
    assert_eq!(core.offer(now, &d, &listens), Ok(()));
    let offered = signal(&core, &d, SignalKind::Offer);
    assert!(!offered["tracks"].as_array().unwrap().is_empty());
}
