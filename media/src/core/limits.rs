//! What one session may send the media server: token buckets on the core's
//! own clock, for the datagrams it sends and for the packets of each track it
//! publishes, and the count of what they turned away.

use std::time::Instant;

use str0m::media::MediaKind;

use super::{
    DATAGRAMS_PER_SECOND, Dropped, LIMIT_LOG_INTERVAL, PICTURE_BYTES_PER_SECOND,
    PICTURE_PACKETS_PER_SECOND, SOUND_BYTES_PER_SECOND, SOUND_PACKETS_PER_SECOND, STUN_PER_SECOND,
};

/// A bucket that refills at `rate` a second and holds one second of it. It
/// starts full, on first use, so building one needs no clock.
#[derive(Debug)]
struct Bucket {
    rate: f64,
    tokens: f64,
    last: Option<Instant>,
}

impl Bucket {
    fn per_second(rate: u64) -> Bucket {
        Bucket {
            rate: rate as f64,
            tokens: rate as f64,
            last: None,
        }
    }

    fn refill(&mut self, now: Instant) {
        if let Some(last) = self.last {
            let elapsed = now.saturating_duration_since(last).as_secs_f64();
            self.tokens = (self.tokens + elapsed * self.rate).min(self.rate);
        }
        self.last = Some(now);
    }
}

/// The two buckets of one published track: its packets and its payload bytes.
#[derive(Debug)]
pub(crate) struct TrackLimit {
    packets: Bucket,
    bytes: Bucket,
}

impl TrackLimit {
    pub fn new(kind: MediaKind) -> TrackLimit {
        let (packets, bytes) = match kind {
            MediaKind::Audio => (SOUND_PACKETS_PER_SECOND, SOUND_BYTES_PER_SECOND),
            MediaKind::Video => (PICTURE_PACKETS_PER_SECOND, PICTURE_BYTES_PER_SECOND),
        };
        TrackLimit {
            packets: Bucket::per_second(packets),
            bytes: Bucket::per_second(bytes),
        }
    }

    /// Whether a packet of `payload` bytes is within both limits. A packet
    /// over one of them takes nothing from the other.
    pub fn admit(&mut self, now: Instant, payload: usize) -> bool {
        self.packets.refill(now);
        self.bytes.refill(now);
        let bytes = payload as f64;
        if self.packets.tokens < 1.0 || self.bytes.tokens < bytes {
            return false;
        }
        self.packets.tokens -= 1.0;
        self.bytes.tokens -= bytes;
        true
    }
}

/// Counts what one kind of limit turned away, and says when a log line about
/// it is due: at the first drop, and then once `LIMIT_LOG_INTERVAL` has
/// passed.
#[derive(Debug, Default)]
struct Overflow {
    total: u64,
    unreported: u64,
    reported_at: Option<Instant>,
}

impl Overflow {
    /// Counts one drop. Returns how many were dropped since the last line
    /// when a line is due.
    fn drop_one(&mut self, now: Instant) -> Option<u64> {
        self.total += 1;
        self.unreported += 1;
        if self
            .reported_at
            .is_some_and(|at| now.saturating_duration_since(at) < LIMIT_LOG_INTERVAL)
        {
            return None;
        }
        self.reported_at = Some(now);
        Some(std::mem::take(&mut self.unreported))
    }
}

/// The limits that belong to a session as a whole, and what the limits of
/// its tracks turned away.
#[derive(Debug)]
pub(crate) struct Limits {
    datagrams: Bucket,
    stun: Bucket,
    dropped_datagrams: Overflow,
    dropped_packets: Overflow,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            datagrams: Bucket::per_second(DATAGRAMS_PER_SECOND),
            stun: Bucket::per_second(STUN_PER_SECOND),
            dropped_datagrams: Overflow::default(),
            dropped_packets: Overflow::default(),
        }
    }
}

impl Limits {
    /// Whether a datagram of the session is handed to its connection. STUN
    /// has a bucket of its own, so media over the limit cannot keep the
    /// connectivity checks out.
    pub fn admit_datagram(&mut self, now: Instant, stun: bool) -> bool {
        let bucket = if stun {
            &mut self.stun
        } else {
            &mut self.datagrams
        };
        bucket.refill(now);
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    /// Counts a datagram the session's limit turned away; see
    /// [`Overflow::drop_one`].
    pub fn datagram_dropped(&mut self, now: Instant) -> Option<u64> {
        self.dropped_datagrams.drop_one(now)
    }

    /// Counts a packet the limit of one of the session's tracks turned
    /// away; see [`Overflow::drop_one`].
    pub fn packet_dropped(&mut self, now: Instant) -> Option<u64> {
        self.dropped_packets.drop_one(now)
    }

    pub fn dropped(&self) -> Dropped {
        Dropped {
            datagrams: self.dropped_datagrams.total,
            packets: self.dropped_packets.total,
        }
    }
}
