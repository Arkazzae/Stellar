//! Receivers on poor downlinks: a bottleneck of a few hundred kbit/s with a
//! bounded queue, delay, jitter and losses in bursts between the media server
//! and one client, while everybody else is on a clean link. What reaches the
//! capped receiver is measured from the packets themselves: which voice
//! packets arrived and how late, how many bytes and how many video frames.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{Duration, Instant};

use common::*;
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
                ", link {kbps:.0} kbit/s, queue drops {}, burst drops {}, max queueing {} ms",
                link.queue_drops,
                link.burst_drops,
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

/// Where the measurement of a receiver starts.
struct Mark {
    at: Instant,
    received: usize,
    link: Option<LinkStats>,
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
    fn new(downlink: Option<LinkShape>, camera_bps: u64) -> Call {
        let mut net = Net::new();
        let a = net.join(1);
        let b = net.join(2);
        let c = net.join(3);
        net.clients[a].downlink = downlink.map(Link::new);
        let mic_a = net.publish(a, MediaKind::Audio, "a");
        let mic_b = net.publish(b, MediaKind::Audio, "b");
        let cam_b = net.publish(b, MediaKind::Video, "b");
        let mic_c = net.publish(c, MediaKind::Audio, "c");
        let cam_c = net.publish(c, MediaKind::Video, "c");
        net.run_until(Duration::from_secs(10), |n| {
            n.clients.iter().all(|c| c.is_connected())
                && n.clients
                    .iter()
                    .all(|c| c.directory.len() == if c.user_id == uuid(1) { 4 } else { 3 })
        });
        let feeds = vec![
            Feed::voice(a, mic_a, VOICE_A, 80),
            Feed::voice(b, mic_b, VOICE_B, 80),
            Feed::camera(b, cam_b, CAMERA_B, camera_bps, 30),
            Feed::voice(c, mic_c, VOICE_C, 80),
            // At another frame rate, so the two cameras never fall into step.
            Feed::camera(c, cam_c, CAMERA_C, camera_bps, 24),
        ];
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
        }
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
        // The parts of every frame that arrived, and the part with the marker.
        let mut frames: BTreeMap<(u8, u32), (BTreeSet<u16>, Option<u16>)> = BTreeMap::new();
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
                let frame = frames.entry((tag, n)).or_default();
                frame.0.insert(part);
                if r.packet.header.marker {
                    frame.1 = Some(part);
                }
            }
        }
        for (tag, _) in &voices {
            *report.voice_received.entry(*tag).or_default() += 1;
        }
        for ((tag, _), (parts, last)) in &frames {
            if last.is_some_and(|last| parts.len() == usize::from(last) + 1) {
                *report.frames.entry(*tag).or_default() += 1;
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

#[test]
fn a_capped_downlink_never_carries_more_than_its_rate_and_spares_the_others() {
    let mut call = Call::new(Some(poor(256)), 500_000);
    call.play(5 * SECOND);
    let (a, c) = (call.a, call.c);
    let at_a = call.mark(a);
    let at_c = call.mark(c);
    call.play(10 * SECOND);
    let capped = call.report(a, &at_a);
    let clean = call.report(c, &at_c);
    eprintln!("capped A at 256 kbit/s: {capped}");
    eprintln!("clean C: {clean}");

    assert!(capped.link_kbps.unwrap() <= 256.0 * 1.01, "{capped}");
    // C, on a clean link, loses nothing and gets every frame of B's camera.
    assert_eq!(clean.worst_voice_loss(), 0.0, "{clean}");
    assert!(clean.fps(CAMERA_B) >= 29.0, "{clean}");
    assert!(call.net.client(call.b).failure.is_none());
}
