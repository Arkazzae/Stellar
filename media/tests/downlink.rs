//! Receivers on poor downlinks: a bottleneck of a few hundred kbit/s with a
//! bounded queue, delay, jitter and losses in bursts between the media server
//! and one client, while everybody else is on a clean link. What reaches the
//! capped receiver is measured from the packets themselves: which voice
//! packets arrived and how late, how many bytes and how many video frames.
//!
//! A voice here is 32 kbit/s of payload in a packet every 20 ms, about
//! 67 kbit/s on the wire with its headers. The capped receiver hears one, so
//! that it fits even the slowest link; two would take about 130 kbit/s, which
//! is what the last test is about.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{Duration, Instant};

use common::*;
use serde_json::Value;
use str0m::media::MediaKind;

const SECOND: Duration = Duration::from_secs(1);

/// The tags of the feeds of [`Call`].
const VOICE_A: u8 = 1;
const VOICE_B: u8 = 2;
const CAMERA_B: u8 = 3;
const VOICE_C: u8 = 4;
const CAMERA_C: u8 = 5;

/// What one receiver got from one window of a call.
#[derive(Debug, Default)]
struct Report {
    seconds: f64,
    /// Voice packets sent in the window and those of them that arrived, by tag.
    voice_sent: BTreeMap<u8, u32>,
    voice_received: BTreeMap<u8, u32>,
    /// Arrival minus sending time of every voice packet that arrived.
    voice_delays: Vec<Duration>,
    /// Video frames that arrived whole, by tag.
    frames: BTreeMap<u8, usize>,
    /// How long after it was sent the first of them arrived whole, by tag.
    first_frame: BTreeMap<u8, Duration>,
    /// RTP payload received, in kbit/s.
    payload_kbps: f64,
    /// Whole datagrams the downlink delivered, in kbit/s, when there is one.
    link_kbps: Option<f64>,
    link: Option<LinkStats>,
}

impl Report {
    fn voice_loss(&self, tag: u8) -> f64 {
        let sent = self.voice_sent.get(&tag).copied().unwrap_or(0);
        let got = self.voice_received.get(&tag).copied().unwrap_or(0);
        if sent == 0 {
            return 0.0;
        }
        1.0 - f64::from(got) / f64::from(sent)
    }

    fn delay_percentile(&self, p: f64) -> Duration {
        let mut d = self.voice_delays.clone();
        d.sort();
        d.get(((d.len() as f64 - 1.0) * p).round() as usize)
            .copied()
            .unwrap_or_default()
    }

    /// The share of the datagrams the link lost in bursts in the window.
    fn link_loss(&self) -> f64 {
        self.link
            .map_or(0.0, |l| l.burst_drops as f64 / l.offered.max(1) as f64)
    }

    /// The worst loss of any voice the receiver hears.
    fn worst_voice_loss(&self) -> f64 {
        self.voice_sent
            .keys()
            .map(|tag| self.voice_loss(*tag))
            .fold(0.0, f64::max)
    }

    fn fps(&self, tag: u8) -> f64 {
        self.frames.get(&tag).copied().unwrap_or(0) as f64 / self.seconds
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "voice loss")?;
        for tag in self.voice_sent.keys() {
            write!(f, " {} {:.1}%", name(*tag), self.voice_loss(*tag) * 100.0)?;
        }
        write!(
            f,
            ", voice delay p50 {} ms p95 {} ms max {} ms, \
             camera B {:.1} fps C {:.1} fps, payload {:.0} kbit/s",
            self.delay_percentile(0.5).as_millis(),
            self.delay_percentile(0.95).as_millis(),
            self.delay_percentile(1.0).as_millis(),
            self.fps(CAMERA_B),
            self.fps(CAMERA_C),
            self.payload_kbps,
        )?;
        if let (Some(kbps), Some(link)) = (self.link_kbps, self.link) {
            write!(
                f,
                ", link {kbps:.0} kbit/s, queue drops {}, burst drops {} ({:.1}%), \
                 max queueing {} ms",
                link.queue_drops,
                link.burst_drops,
                self.link_loss() * 100.0,
                link.max_queueing.as_millis()
            )?;
        }
        Ok(())
    }
}

fn name(tag: u8) -> &'static str {
    match tag {
        VOICE_A => "A",
        VOICE_B | CAMERA_B => "B",
        VOICE_C | CAMERA_C => "C",
        _ => "?",
    }
}

/// What arrived of one video frame: its parts, the part with the marker,
/// and when the last part arrived.
struct FrameParts {
    parts: BTreeSet<u16>,
    last: Option<u16>,
    at: Instant,
}

/// Where the measurement of a receiver starts.
struct Mark {
    at: Instant,
    received: usize,
    link: Option<LinkStats>,
    /// How many of the receiver's events the log held.
    events: usize,
}

/// A, B and C in one room. A publishes a voice and receives; B and C each
/// publish a voice and a camera. A's downlink can be capped.
struct Call {
    net: Net,
    a: usize,
    b: usize,
    c: usize,
    feeds: Vec<Feed>,
}

impl Call {
    /// B sends a voice and a camera, C a camera and, when `c_speaks`, a
    /// voice.
    fn new(downlink: Option<LinkShape>, camera_bps: u64) -> Call {
        Call::with(downlink, camera_bps, false)
    }

    fn with(downlink: Option<LinkShape>, camera_bps: u64, c_speaks: bool) -> Call {
        let mut net = Net::new();
        let a = net.join(1);
        let b = net.join(2);
        let c = net.join(3);
        net.clients[a].downlink = downlink.map(Link::new);
        let mic_a = net.publish(a, MediaKind::Audio, "a");
        let mic_b = net.publish(b, MediaKind::Audio, "b");
        let cam_b = net.publish(b, MediaKind::Video, "b");
        let mic_c = c_speaks.then(|| net.publish(c, MediaKind::Audio, "c"));
        let cam_c = net.publish(c, MediaKind::Video, "c");
        // A gets B's two tracks and C's; B gets A's and C's; C gets A's and B's.
        let c_tracks = usize::from(c_speaks) + 1;
        let sizes = [2 + c_tracks, 1 + c_tracks, 3];
        net.run_until(Duration::from_secs(10), |n| {
            n.clients.iter().all(|c| c.is_connected())
                && n.clients
                    .iter()
                    .zip(sizes)
                    .all(|(c, size)| c.directory.len() == size)
        });
        let mut feeds = vec![
            Feed::voice(a, mic_a, VOICE_A, 80),
            Feed::voice(b, mic_b, VOICE_B, 80),
            Feed::camera(b, cam_b, CAMERA_B, camera_bps, 30),
            // At another frame rate, so the two cameras never fall into step.
            Feed::camera(c, cam_c, CAMERA_C, camera_bps, 24),
        ];
        if let Some(mic_c) = mic_c {
            // Half a packet behind B's, as two people never speak in step.
            feeds.push(Feed::voice(c, mic_c, VOICE_C, 80).after(Duration::from_millis(10)));
        }
        Call {
            net,
            a,
            b,
            c,
            feeds,
        }
    }

    fn play(&mut self, d: Duration) {
        play(&mut self.net, &mut self.feeds, d);
    }

    fn mark(&self, i: usize) -> Mark {
        let c = self.net.client(i);
        Mark {
            at: self.net.now,
            received: c.received.len(),
            link: c.downlink.as_ref().map(|l| l.stats),
            events: self.net.events_of(i).len(),
        }
    }

    /// The cameras held back for receiver `i` by its latest directory, by
    /// publisher.
    fn held(&self, i: usize) -> Vec<&'static str> {
        let mut held: Vec<&'static str> = self
            .net
            .client(i)
            .directory
            .iter()
            .filter(|t| t["heldBack"] == true)
            .map(|t| self.publisher(t))
            .collect();
        held.sort();
        held
    }

    fn publisher(&self, track: &Value) -> &'static str {
        match track["userId"].as_str() {
            Some(u) if u == uuid(1) => "A",
            Some(u) if u == uuid(2) => "B",
            Some(u) if u == uuid(3) => "C",
            _ => "?",
        }
    }

    /// How often what is held back for receiver `i` changed since `mark`.
    fn held_changes(&self, i: usize, mark: &Mark) -> usize {
        let held = |e: &Value| -> BTreeSet<String> {
            let tracks = e["payload"]["tracks"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            tracks
                .iter()
                .filter(|t| t["heldBack"] == true)
                .map(|t| t["mid"].as_str().unwrap().to_string())
                .collect()
        };
        let directories: Vec<BTreeSet<String>> = self
            .net
            .events_of(i)
            .into_iter()
            .filter(|e| e["kind"] == "tracks" || e["kind"] == "offer")
            .map(held)
            .collect();
        let before = self
            .net
            .events_of(i)
            .into_iter()
            .take(mark.events)
            .filter(|e| e["kind"] == "tracks" || e["kind"] == "offer")
            .count();
        directories
            .windows(2)
            .skip(before.saturating_sub(1))
            .filter(|w| w[0] != w[1])
            .count()
    }

    /// The mids receiver `i` has for the voices and cameras of B and C.
    fn mid(&self, i: usize, user: u64, kind: &str) -> String {
        self.net
            .client(i)
            .directory
            .iter()
            .find(|t| t["userId"] == uuid(user) && t["kind"] == kind)
            .and_then(|t| t["mid"].as_str())
            .unwrap()
            .to_string()
    }

    /// What receiver `i` got since `mark`.
    fn report(&self, i: usize, mark: &Mark) -> Report {
        let c = self.net.client(i);
        let seconds = (self.net.now - mark.at).as_secs_f64();
        let mut report = Report {
            seconds,
            ..Report::default()
        };

        // Voice packets that were sent inside the window, and of them those
        // that arrived, whenever they did.
        let feed = |tag: u8| self.feeds.iter().find(|f| f.tag == tag);
        let sent_in_window = |tag: u8, n: u32| {
            feed(tag)
                .and_then(|f| f.sent_at(n))
                .is_some_and(|at| at >= mark.at && at + Duration::from_millis(500) <= self.net.now)
        };
        let own = [VOICE_A, VOICE_B, VOICE_C][i];
        for tag in [VOICE_A, VOICE_B, VOICE_C] {
            let Some(f) = feed(tag).filter(|_| tag != own) else {
                continue;
            };
            let sent = (0..f.sent).filter(|n| sent_in_window(tag, *n)).count() as u32;
            report.voice_sent.insert(tag, sent);
        }
        let mut voices: BTreeSet<(u8, u32)> = BTreeSet::new();
        let mut frames: BTreeMap<(u8, u32), FrameParts> = BTreeMap::new();
        let mut payload = 0usize;
        for r in &c.received {
            let Some((tag, n)) = read_payload(&r.packet.payload) else {
                continue;
            };
            let is_voice = feed(tag).is_some_and(|f| matches!(f.kind, FeedKind::Voice { .. }));
            if is_voice
                && tag != own
                && sent_in_window(tag, n)
                && voices.insert((tag, n))
                && let Some(at) = feed(tag).and_then(|f| f.sent_at(n))
            {
                report.voice_delays.push(r.at - at);
            }
        }
        for r in &c.received[mark.received..] {
            payload += r.packet.payload.len();
            let Some((tag, n)) = read_payload(&r.packet.payload) else {
                continue;
            };
            if feed(tag).is_some_and(|f| matches!(f.kind, FeedKind::Camera { .. })) {
                let part = u16::from_be_bytes([r.packet.payload[5], r.packet.payload[6]]);
                let frame = frames.entry((tag, n)).or_insert_with(|| FrameParts {
                    parts: BTreeSet::new(),
                    last: None,
                    at: r.at,
                });
                frame.parts.insert(part);
                frame.at = frame.at.max(r.at);
                if r.packet.header.marker {
                    frame.last = Some(part);
                }
            }
        }
        for (tag, _) in &voices {
            *report.voice_received.entry(*tag).or_default() += 1;
        }
        for ((tag, n), frame) in &frames {
            if frame
                .last
                .is_some_and(|last| frame.parts.len() == usize::from(last) + 1)
            {
                *report.frames.entry(*tag).or_default() += 1;
                if let Some(sent) = feed(*tag).and_then(|f| f.sent_at(*n)) {
                    report.first_frame.entry(*tag).or_insert(frame.at - sent);
                }
            }
        }
        report.payload_kbps = payload as f64 * 8.0 / seconds / 1000.0;
        if let (Some(link), Some(before)) = (&c.downlink, &mark.link) {
            let delivered = link.stats.delivered_bytes - before.delivered_bytes;
            report.link_kbps = Some(delivered as f64 * 8.0 / seconds / 1000.0);
            report.link = Some(LinkStats {
                offered: link.stats.offered - before.offered,
                delivered: link.stats.delivered - before.delivered,
                delivered_bytes: delivered,
                queue_drops: link.stats.queue_drops - before.queue_drops,
                burst_drops: link.stats.burst_drops - before.burst_drops,
                max_queueing: link.stats.max_queueing,
            });
        }
        report
    }
}

/// The poor links of the rig: a cap, a queue of 300 ms of the cap's own
/// rate, 40 ms of delay, up to 15 ms of jitter and 1% of the datagrams lost in
/// bursts of three on average.
fn poor(kbps: u64) -> LinkShape {
    LinkShape {
        jitter: Duration::from_millis(15),
        ..LinkShape::capped(kbps, 300)
    }
    .with_loss(BurstLoss::bursts(0.01, 3.0))
}

#[test]
fn the_link_serialises_at_its_rate_and_drops_what_its_queue_cannot_hold() {
    let start = Instant::now();
    let shape = LinkShape {
        jitter: Duration::ZERO,
        ..LinkShape::capped(256, 200)
    };
    let mut link = Link::new(shape);

    // A second of 1 Mbit/s offered in datagrams of 1,000 bytes.
    let mut arrivals = Vec::new();
    for n in 0..125u64 {
        let at = start + Duration::from_millis(n * 8);
        link.push(at, vec![0; 1000 - DATAGRAM_OVERHEAD]);
        arrivals.extend(link.due(at).into_iter().map(|_| at));
    }
    let mut t = start;
    while link.next_arrival().is_some() {
        t += Duration::from_millis(1);
        arrivals.extend(link.due(t).into_iter().map(|_| t));
    }

    let stats = link.stats;
    assert_eq!(stats.offered, 125);
    assert_eq!(stats.burst_drops, 0);
    assert!(stats.queue_drops > 80, "{stats:?}");
    assert!(
        stats.max_queueing <= Duration::from_millis(200),
        "{stats:?}"
    );
    // What was delivered took the bottleneck's time to send: 1,000 bytes
    // every 31.25 ms, after 40 ms of delay.
    let first = arrivals[0];
    let last = *arrivals.last().unwrap();
    let spread = last - first;
    let expected = Duration::from_micros(31_250) * (stats.delivered as u32 - 1);
    assert!(
        spread + Duration::from_millis(1) >= expected,
        "{spread:?} {expected:?}"
    );
    assert!(first >= start + Duration::from_millis(40 + 31));
}

#[test]
fn burst_losses_have_the_mean_and_the_length_they_are_made_with() {
    let start = Instant::now();
    let shape = LinkShape {
        rate_bps: 1_000_000_000,
        queue_bytes: usize::MAX / 2,
        delay: Duration::ZERO,
        jitter: Duration::from_millis(20),
        loss: BurstLoss::bursts(0.05, 4.0),
        seed: 11,
    };
    let run = || {
        let mut link = Link::new(shape);
        let mut got = Vec::new();
        for n in 0..100_000u32 {
            link.push(start, n.to_be_bytes().to_vec());
        }
        for d in link.due(start + SECOND) {
            got.push(u32::from_be_bytes([d[0], d[1], d[2], d[3]]));
        }
        (link.stats, got)
    };
    let (stats, got) = run();

    let loss = stats.burst_drops as f64 / stats.offered as f64;
    assert!((0.04..0.06).contains(&loss), "{loss}");
    let mut bursts = 0;
    let mut previous = None;
    for n in &got {
        if previous.is_some_and(|p: u32| *n != p + 1) {
            bursts += 1;
        }
        previous = Some(*n);
    }
    let mean_burst = stats.burst_drops as f64 / f64::from(bursts);
    assert!((3.5..4.5).contains(&mean_burst), "{mean_burst}");
    // Jitter never reorders, and the same seed gives the same run.
    assert!(got.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(run().1, got);
}

/// A camera of the kind the browser suite uses: 250 kbit/s.
const CAMERA_BPS: u64 = 250_000;

#[test]
fn a_capped_receiver_keeps_its_voices_and_gets_the_pictures_that_fit() {
    for (kbps, kept) in [(512, vec!["B"]), (256, vec![]), (128, vec![])] {
        let mut call = Call::new(Some(poor(kbps)), CAMERA_BPS);
        let (a, c) = (call.a, call.c);
        call.play(15 * SECOND);
        let at_a = call.mark(a);
        let at_c = call.mark(c);
        call.play(20 * SECOND);
        let capped = call.report(a, &at_a);
        let clean = call.report(c, &at_c);
        eprintln!("{kbps} kbit/s, A: {capped}; held {:?}", call.held(a));

        assert!(capped.link_kbps.unwrap() <= kbps as f64 * 1.01, "{capped}");
        // The voice loses no more than the link's own bursts, nothing is
        // dropped for want of room, and it waits no longer than the link's
        // delay, jitter and a little queueing.
        let link = capped.link.unwrap();
        assert_eq!(link.queue_drops, 0, "{kbps}: {capped}");
        assert!(
            capped.worst_voice_loss() <= capped.link_loss() * 1.5 + 0.005,
            "{kbps}: {capped}"
        );
        assert!(
            capped.delay_percentile(0.95) < Duration::from_millis(150),
            "{kbps}: {capped}"
        );
        // B's camera comes first in A's list: it is the one that fits at 512.
        for (camera, name) in [(CAMERA_B, "B"), (CAMERA_C, "C")] {
            if kept.contains(&name) {
                assert!(capped.fps(camera) > 28.0, "{kbps}: {capped}");
            } else {
                assert_eq!(capped.fps(camera), 0.0, "{kbps}: {capped}");
            }
        }
        let held: Vec<&str> = ["B", "C"]
            .into_iter()
            .filter(|n| !kept.contains(n))
            .collect();
        assert_eq!(call.held(a), held, "{kbps}");
        assert_eq!(
            call.held_changes(a, &at_a),
            0,
            "{kbps}: a steady link flapped"
        );

        // C, on a clean link, loses nothing and is held back nothing.
        assert_eq!(clean.worst_voice_loss(), 0.0, "{clean}");
        assert!(clean.fps(CAMERA_B) >= 29.0, "{clean}");
        assert!(call.held(c).is_empty());
        assert!(call.net.client(call.b).failure.is_none());
    }
}

#[test]
fn held_back_pictures_come_back_once_there_is_room_and_a_steady_link_never_flaps() {
    let mut call = Call::new(Some(poor(256)), CAMERA_BPS);
    let a = call.a;
    call.play(15 * SECOND);
    assert_eq!(call.held(a), ["B", "C"]);

    // The cap is lifted: both come back, each with a keyframe.
    let keyframes = |call: &Call| call.feeds.iter().map(|f| f.keyframes).sum::<u32>();
    let before = keyframes(&call);
    let lifted = call.net.now;
    call.net.clients[a]
        .downlink
        .as_mut()
        .unwrap()
        .reshape(LinkShape::capped(100_000, 300));
    let mut back = None;
    for _ in 0..60 {
        call.play(SECOND);
        if call.held(a).is_empty() {
            back = Some(call.net.now - lifted);
            break;
        }
    }
    let back = back.expect("the pictures came back");
    eprintln!("both cameras back {back:?} after the cap was lifted");
    assert!(keyframes(&call) >= before + 2);
    let steady = call.mark(a);
    call.play(30 * SECOND);
    let clean = call.report(a, &steady);
    assert!(
        clean.fps(CAMERA_B) > 29.0 && clean.fps(CAMERA_C) > 23.0,
        "{clean}"
    );
    assert_eq!(call.held_changes(a, &steady), 0);

    // Down to 512 kbit/s: C's camera, last in the list, is held back at once.
    call.net.clients[a]
        .downlink
        .as_mut()
        .unwrap()
        .reshape(poor(512));
    let lowered = call.net.now;
    while call.held(a) != ["C"] {
        call.play(Duration::from_millis(100));
        assert!(
            call.net.now - lowered < 5 * SECOND,
            "held {:?}",
            call.held(a)
        );
    }
    eprintln!(
        "C's camera held back {:?} after the cap",
        call.net.now - lowered
    );
    call.play(10 * SECOND);
    let steady = call.mark(a);
    call.play(60 * SECOND);
    let capped = call.report(a, &steady);
    eprintln!("a minute at 512 kbit/s: {capped}");
    assert_eq!(call.held_changes(a, &steady), 0, "a steady link flapped");
    assert!(capped.fps(CAMERA_B) > 28.0, "{capped}");
    assert_eq!(capped.link.unwrap().queue_drops, 0, "{capped}");
    assert!(
        capped.worst_voice_loss() <= capped.link_loss() * 1.5 + 0.005,
        "{capped}"
    );
}

#[test]
fn the_order_of_the_list_decides_which_picture_stays() {
    let mut call = Call::new(Some(poor(512)), CAMERA_BPS);
    let a = call.a;
    call.net.clients[a].subscribe_to_all = false;
    let list = [
        call.mid(a, 2, "audio"),
        call.mid(a, 3, "video"),
        call.mid(a, 2, "video"),
    ];
    let session = call.net.client(a).session_id.clone();
    call.net
        .core
        .set_subscriptions(call.net.now, &session, &list)
        .unwrap();
    call.play(15 * SECOND);
    let at = call.mark(a);
    call.play(10 * SECOND);
    let report = call.report(a, &at);
    assert_eq!(call.held(a), ["B"], "{report}");
    assert!(report.fps(CAMERA_C) > 22.0, "{report}");
    assert_eq!(report.fps(CAMERA_B), 0.0, "{report}");
}

#[test]
fn a_receiver_with_room_gets_every_picture_at_once_and_no_voice_waits() {
    let mut call = Call::new(None, 1_500_000);
    let a = call.a;
    let start = call.mark(a);
    call.play(2 * SECOND);
    let first = call.report(a, &start);
    call.play(8 * SECOND);
    let all = call.report(a, &start);
    eprintln!(
        "first two seconds: {first}; first frames whole after {:?}; ten seconds: {all}",
        first.first_frame
    );
    // The first keyframes are not held back in the pacer.
    for camera in [CAMERA_B, CAMERA_C] {
        assert!(
            first.first_frame[&camera] <= Duration::from_millis(50),
            "{:?}",
            first.first_frame
        );
    }
    // Every frame from the first one on, as if nothing were measured.
    assert!(
        first.fps(CAMERA_B) > 29.0 && first.fps(CAMERA_C) > 23.0,
        "{first}"
    );
    assert!(
        all.fps(CAMERA_B) > 29.5 && all.fps(CAMERA_C) > 23.5,
        "{all}"
    );
    // A voice leaves the media server in the millisecond it arrived.
    assert_eq!(all.delay_percentile(1.0), Duration::ZERO, "{all}");
    assert!(call.held(a).is_empty());
}

#[test]
fn the_directory_and_the_stats_say_what_is_held_back() {
    let mut call = Call::new(Some(poor(256)), CAMERA_BPS);
    let (a, c) = (call.a, call.c);
    call.play(15 * SECOND);

    // Every entry says whether it is held back; only A's cameras are.
    for (i, held) in [(a, 2), (c, 0)] {
        let directory = &call.net.client(i).directory;
        assert!(directory.iter().all(|t| t["heldBack"].is_boolean()));
        assert_eq!(
            directory.iter().filter(|t| t["heldBack"] == true).count(),
            held
        );
        assert!(
            directory
                .iter()
                .filter(|t| t["kind"] == "audio")
                .all(|t| t["heldBack"] == false)
        );
    }

    let stats = serde_json::to_value(call.net.core.stats(call.net.now)).unwrap();
    let sessions = stats["rooms"][0]["sessions"].as_array().unwrap();
    let of = |i: usize| {
        let id = &call.net.client(i).session_id;
        sessions.iter().find(|s| s["sessionId"] == *id).unwrap()["downlink"].clone()
    };
    let downlink = of(a);
    eprintln!("A's downlink: {downlink}");
    assert_eq!(downlink["measured"], true);
    let estimate = downlink["estimateBps"].as_u64().unwrap();
    assert!((40_000..256_000).contains(&estimate), "{downlink}");
    let held = downlink["heldBack"].as_array().unwrap();
    assert_eq!(held.len(), 2);
    for track in held {
        let cost = track["costBps"].as_u64().unwrap();
        assert!((200_000..320_000).contains(&cost), "{downlink}");
        assert!(track["userId"] == uuid(2) || track["userId"] == uuid(3));
    }
    let forwarded = downlink["forwardedBps"].as_u64().unwrap();
    assert!((28_000..40_000).contains(&forwarded), "{downlink}");
    assert!(downlink["desiredBps"].as_u64().unwrap() > forwarded);
    assert!(downlink["holds"].as_u64().unwrap() >= 2);
    assert_eq!(of(c)["heldBack"].as_array().unwrap().len(), 0);
}

#[test]
fn when_not_even_the_voices_fit_they_are_still_all_sent_and_nothing_else() {
    // Two voices take about 130 kbit/s on the wire.
    let mut call = Call::with(Some(poor(96)), CAMERA_BPS, true);
    let a = call.a;
    call.play(15 * SECOND);
    let at = call.mark(a);
    call.play(20 * SECOND);
    let report = call.report(a, &at);
    eprintln!("two voices at 96 kbit/s: {report}");
    assert_eq!(call.held(a), ["B", "C"]);
    // Both voices are still sent, and what the link cannot carry of them it
    // drops; no padding or picture adds to it.
    let received = report.voice_received.values().sum::<u32>();
    assert!(received > 0 && report.voice_received.len() == 2, "{report}");
    assert!(report.link.unwrap().queue_drops > 0, "{report}");
    let stats = serde_json::to_value(call.net.core.stats(call.net.now)).unwrap();
    let id = call.net.client(a).session_id.clone();
    let downlink = stats["rooms"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["sessionId"] == id)
        .unwrap()["downlink"]
        .clone();
    assert_eq!(downlink["paddingBps"], 0, "{downlink}");
    assert_eq!(report.fps(CAMERA_B) + report.fps(CAMERA_C), 0.0);
}
