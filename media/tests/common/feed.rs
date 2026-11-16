//! Synthetic media that publishers send as the harness clock moves: voices
//! of one packet every 20 ms, and cameras of a fixed bitrate whose frames are
//! split into packets, with a larger frame whenever a keyframe was asked for.
//!
//! Every packet says which feed it belongs to and its number, so a receiver
//! can count exactly what reached it and when it was sent.

use std::time::{Duration, Instant};

use str0m::Event;
use str0m::format::Codec;
use str0m::media::Mid;

use super::Net;

/// The largest payload of one video packet.
pub const VIDEO_PACKET: usize = 1100;
/// How many times larger than an ordinary frame a keyframe is.
pub const KEYFRAME_FACTOR: usize = 5;
pub const VOICE_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    Voice { bytes: usize },
    Camera { bps: u64, fps: u32 },
}

/// One track a client publishes.
#[derive(Debug, Clone)]
pub struct Feed {
    pub client: usize,
    pub mid: Mid,
    /// Written into every payload, so a receiver knows whose packet it is.
    pub tag: u8,
    pub kind: FeedKind,
    pub started: Option<Instant>,
    next: Option<Instant>,
    /// Packets of a voice, frames of a camera.
    pub sent: u32,
    pub keyframes: u32,
    requests_seen: usize,
}

impl Feed {
    pub fn voice(client: usize, mid: Mid, tag: u8, bytes: usize) -> Feed {
        Feed::new(client, mid, tag, FeedKind::Voice { bytes })
    }

    pub fn camera(client: usize, mid: Mid, tag: u8, bps: u64, fps: u32) -> Feed {
        Feed::new(client, mid, tag, FeedKind::Camera { bps, fps })
    }

    fn new(client: usize, mid: Mid, tag: u8, kind: FeedKind) -> Feed {
        Feed {
            client,
            mid,
            tag,
            kind,
            started: None,
            next: None,
            sent: 0,
            keyframes: 0,
            requests_seen: 0,
        }
    }

    fn interval(&self) -> Duration {
        match self.kind {
            FeedKind::Voice { .. } => VOICE_INTERVAL,
            FeedKind::Camera { fps, .. } => Duration::from_secs(1) / fps,
        }
    }

    /// When packet or frame `n` of this feed was sent.
    pub fn sent_at(&self, n: u32) -> Option<Instant> {
        Some(self.started? + self.interval() * n)
    }

    /// Sends whatever is due by `net.now`.
    fn play(&mut self, net: &mut Net) {
        let now = net.now;
        let next = *self.next.get_or_insert(now);
        if self.started.is_none() {
            self.started = Some(now);
        }
        if next > now {
            return;
        }
        self.next = Some(next + self.interval());

        match self.kind {
            FeedKind::Voice { bytes } => {
                let pt = net.client(self.client).pt_of(Codec::Opus);
                let payload = payload(self.tag, self.sent, 0, bytes);
                net.send_rtp(self.client, self.mid, pt, &payload, true, self.sent * 960);
            }
            FeedKind::Camera { bps, fps } => {
                // A keyframe follows every request the publisher received.
                let requests = net
                    .client(self.client)
                    .events
                    .iter()
                    .filter(|e| matches!(e, Event::KeyframeRequest(r) if r.mid == self.mid))
                    .count();
                let keyframe = requests > self.requests_seen || self.sent == 0;
                self.requests_seen = requests;
                let mut bytes = (bps / 8 / u64::from(fps)) as usize;
                if keyframe {
                    bytes *= KEYFRAME_FACTOR;
                    self.keyframes += 1;
                }
                let pt = net.client(self.client).pt_of(Codec::Vp8);
                let packets = bytes.div_ceil(VIDEO_PACKET).max(1);
                for p in 0..packets {
                    let size = (bytes - p * VIDEO_PACKET).clamp(8, VIDEO_PACKET);
                    let payload = payload(self.tag, self.sent, p as u16, size);
                    let ts = self.sent * (90_000 / fps);
                    net.send_rtp(self.client, self.mid, pt, &payload, p + 1 == packets, ts);
                }
            }
        }
        self.sent += 1;
    }
}

/// `[tag, number (4 bytes), part (2 bytes), 0...]`, at least 8 bytes.
fn payload(tag: u8, number: u32, part: u16, size: usize) -> Vec<u8> {
    let mut p = vec![0u8; size.max(8)];
    p[0] = tag;
    p[1..5].copy_from_slice(&number.to_be_bytes());
    p[5..7].copy_from_slice(&part.to_be_bytes());
    p
}

/// The tag and the number a payload of a feed carries.
pub fn read_payload(payload: &[u8]) -> Option<(u8, u32)> {
    (payload.len() >= 8).then(|| {
        (
            payload[0],
            u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]]),
        )
    })
}

/// Plays every feed for `d`, one millisecond at a time.
pub fn play(net: &mut Net, feeds: &mut [Feed], d: Duration) {
    let end = net.now + d;
    while net.now < end {
        for feed in feeds.iter_mut() {
            feed.play(net);
        }
        net.step(Duration::from_millis(1));
    }
}
