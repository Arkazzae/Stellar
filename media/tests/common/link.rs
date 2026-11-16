//! A poor downlink between the media server and one client.
//!
//! Datagrams to the client queue at a bottleneck that sends them one after
//! another at a fixed rate, and a datagram that would make the queue longer
//! than its bound is dropped there, as a router's tail drop would. What
//! leaves the bottleneck is then delayed by a fixed amount plus jitter, keeps
//! its order, and may be lost in a burst on the way. Every random choice comes
//! from a seeded generator and every time from the harness clock, so a run is
//! the same each time it is made.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// UDP and IPv4 headers, which the bottleneck carries as well.
pub const DATAGRAM_OVERHEAD: usize = 28;

/// A small generator that is the same on every machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Losses in bursts: a two-state chain that loses everything while it is in
/// its bad state. `enter` is the chance per datagram of falling into it and
/// `leave` the chance of coming out, so the mean burst is `1 / leave`
/// datagrams and the mean loss `enter / (enter + leave)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BurstLoss {
    pub enter: f64,
    pub leave: f64,
}

impl BurstLoss {
    pub const NONE: BurstLoss = BurstLoss {
        enter: 0.0,
        leave: 1.0,
    };

    /// A mean loss of `mean` in bursts of `burst` datagrams on average.
    pub fn bursts(mean: f64, burst: f64) -> BurstLoss {
        let leave = 1.0 / burst;
        BurstLoss {
            enter: mean * leave / (1.0 - mean),
            leave,
        }
    }
}

/// How the link is made.
#[derive(Debug, Clone, Copy)]
pub struct LinkShape {
    /// The bottleneck's rate in bits per second of whole datagrams, UDP and
    /// IP headers included.
    pub rate_bps: u64,
    /// The most the bottleneck holds before it drops, in bytes.
    pub queue_bytes: usize,
    pub delay: Duration,
    /// Each datagram is delayed by up to this much more, uniformly.
    pub jitter: Duration,
    pub loss: BurstLoss,
    pub seed: u64,
}

impl LinkShape {
    /// A link of `kbps` with a queue of `queue_ms` of its own rate, 40 ms of
    /// delay and 10 ms of jitter, and no loss.
    pub fn capped(kbps: u64, queue_ms: u64) -> LinkShape {
        LinkShape {
            rate_bps: kbps * 1000,
            queue_bytes: (kbps * queue_ms / 8) as usize,
            delay: Duration::from_millis(40),
            jitter: Duration::from_millis(10),
            loss: BurstLoss::NONE,
            seed: 7,
        }
    }

    pub fn with_loss(mut self, loss: BurstLoss) -> LinkShape {
        self.loss = loss;
        self
    }
}

/// What happened to the datagrams offered to a link.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinkStats {
    pub offered: u64,
    /// Delivered to the client so far.
    pub delivered: u64,
    /// Bytes delivered, UDP and IP headers included.
    pub delivered_bytes: u64,
    /// Dropped because the bottleneck's queue was full.
    pub queue_drops: u64,
    /// Lost in a burst after the bottleneck.
    pub burst_drops: u64,
    /// The longest a delivered datagram waited at the bottleneck.
    pub max_queueing: Duration,
}

pub struct Link {
    pub shape: LinkShape,
    rng: Rng,
    bad: bool,
    /// When the bottleneck has sent everything it holds.
    free_at: Option<Instant>,
    /// When the last datagram reaches the client, which a later one never
    /// overtakes.
    last_arrival: Option<Instant>,
    in_flight: VecDeque<(Instant, Vec<u8>)>,
    pub stats: LinkStats,
}

impl Link {
    pub fn new(shape: LinkShape) -> Link {
        Link {
            rng: Rng::new(shape.seed),
            shape,
            bad: false,
            free_at: None,
            last_arrival: None,
            in_flight: VecDeque::new(),
            stats: LinkStats::default(),
        }
    }

    /// Changes the rate, the queue, the delay or the losses from now on.
    /// What is in flight keeps its times.
    pub fn reshape(&mut self, shape: LinkShape) {
        self.shape = LinkShape {
            seed: self.shape.seed,
            ..shape
        };
    }

    fn serialisation(&self, bytes: usize) -> Duration {
        Duration::from_nanos((bytes as u64 * 8 * 1_000_000_000) / self.shape.rate_bps.max(1))
    }

    /// A datagram reaches the bottleneck at `now`.
    pub fn push(&mut self, now: Instant, datagram: Vec<u8>) {
        self.stats.offered += 1;
        let size = datagram.len() + DATAGRAM_OVERHEAD;
        let free_at = self.free_at.filter(|t| *t > now).unwrap_or(now);
        let backlog = (free_at - now).as_nanos() as u64 * self.shape.rate_bps / 8 / 1_000_000_000;
        if backlog as usize + size > self.shape.queue_bytes {
            self.stats.queue_drops += 1;
            return;
        }
        let departure = free_at + self.serialisation(size);
        self.free_at = Some(departure);
        self.stats.max_queueing = self.stats.max_queueing.max(free_at - now);

        let loss = self.shape.loss;
        self.bad = if self.bad {
            self.rng.unit() >= loss.leave
        } else {
            self.rng.unit() < loss.enter
        };
        if self.bad {
            self.stats.burst_drops += 1;
            return;
        }

        let jitter = self.shape.jitter.mul_f64(self.rng.unit());
        let arrival = (departure + self.shape.delay + jitter).max(self.last_arrival.unwrap_or(now));
        self.last_arrival = Some(arrival);
        self.in_flight.push_back((arrival, datagram));
    }

    /// Every datagram that has reached the client by `now`, in order.
    pub fn due(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while self.in_flight.front().is_some_and(|(at, _)| *at <= now) {
            let datagram = self.in_flight.pop_front().expect("checked").1;
            self.stats.delivered += 1;
            self.stats.delivered_bytes += (datagram.len() + DATAGRAM_OVERHEAD) as u64;
            out.push(datagram);
        }
        out
    }

    pub fn next_arrival(&self) -> Option<Instant> {
        self.in_flight.front().map(|(at, _)| *at)
    }
}
