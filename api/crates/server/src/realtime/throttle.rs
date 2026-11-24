//! A token bucket for each connection and each kind of call, which every hub
//! connection shares.
//!
//! A hub method reaches a room and the database rather than one client, so a
//! busy or broken connection is capped by the server instead of trusted:
//! every bucket holds at least the burst a real negotiation makes and refills
//! faster than any real client ever needs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What a connection may send quickly, and what it may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Policy {
    /// Voice signalling: SDP and many ICE candidates at once.
    Signalling,
    /// Signals forwarded to the media server for one session.
    MediaSignalling,
    /// Typing indicators, which live six seconds and say nothing new sooner.
    Typing,
    /// Opening media sessions: joins, rejoins and a few replacements.
    MediaSession,
}

impl Policy {
    /// The burst, and the refill per second. Nine people in a mesh restarting
    /// ICE at once send around a hundred and twenty candidates within a few
    /// seconds; no real client sustains even a small fraction of the
    /// signalling rate afterwards. A media session's signals are its
    /// negotiations and the lists of what it receives, which a browser sends
    /// at most four times a second however fast the others change their state.
    /// Typing arrives at human speed at worst. A media session is opened per
    /// seat, rejoin and replacement, the replacements of one outage being
    /// three at most.
    fn limit(self) -> (f64, f64) {
        match self {
            Policy::Signalling => (200.0, 40.0),
            Policy::MediaSignalling => (60.0, 10.0),
            Policy::Typing => (10.0, 1.0),
            Policy::MediaSession => (6.0, 6.0 / 60.0),
        }
    }
}

/// Buckets idle long enough to have refilled are swept once in this many
/// calls, so the table stays as small as the connections sending fast.
const SWEEP_INTERVAL: u64 = 256;

struct Bucket {
    tokens: f64,
    last_refill: Duration,
}

impl Bucket {
    fn refilled(&self, policy: Policy, now: Duration) -> f64 {
        let (burst, per_second) = policy.limit();
        let idle = now.saturating_sub(self.last_refill).as_secs_f64();
        (self.tokens + idle * per_second).min(burst)
    }
}

#[derive(Default)]
struct Table {
    buckets: HashMap<(String, Policy), Bucket>,
    calls: u64,
}

pub struct ThrottleGate {
    clock: Arc<dyn Fn() -> Duration + Send + Sync>,
    table: Mutex<Table>,
}

impl Default for ThrottleGate {
    fn default() -> ThrottleGate {
        let origin = Instant::now();
        ThrottleGate::with_clock(move || origin.elapsed())
    }
}

impl ThrottleGate {
    /// A gate that reads time from `clock`, the time since some fixed moment.
    pub fn with_clock(clock: impl Fn() -> Duration + Send + Sync + 'static) -> ThrottleGate {
        ThrottleGate {
            clock: Arc::new(clock),
            table: Mutex::new(Table::default()),
        }
    }

    /// Takes one call from the connection's bucket for `policy`; false when
    /// it is empty. A connection's first call finds the bucket full, whatever
    /// the process uptime.
    pub fn try_acquire(&self, connection_id: &str, policy: Policy) -> bool {
        let now = (self.clock)();
        let (burst, _) = policy.limit();
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = table
            .buckets
            .entry((connection_id.to_string(), policy))
            .or_insert(Bucket {
                tokens: burst,
                last_refill: now,
            });
        let tokens = bucket.refilled(policy, now);
        bucket.last_refill = now;
        let acquired = tokens >= 1.0;
        bucket.tokens = if acquired { tokens - 1.0 } else { tokens };

        table.calls += 1;
        if table.calls.is_multiple_of(SWEEP_INTERVAL) {
            // A bucket idle long enough to have refilled is the same as a new
            // one, so forgetting it changes nothing.
            table.buckets.retain(|(_, policy), bucket| {
                let (burst, _) = policy.limit();
                bucket.refilled(*policy, now) < burst
            });
        }
        acquired
    }

    /// Forgets what a connection that has ended was allowed.
    pub fn forget(&self, connection_id: &str) {
        self.table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .buckets
            .retain(|(id, _), _| id != connection_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A gate on a clock the test moves by hand.
    fn gate() -> (ThrottleGate, Arc<AtomicU64>) {
        let millis = Arc::new(AtomicU64::new(0));
        let seen = Arc::clone(&millis);
        let gate =
            ThrottleGate::with_clock(move || Duration::from_millis(seen.load(Ordering::SeqCst)));
        (gate, millis)
    }

    fn advance(clock: &AtomicU64, by: Duration) {
        clock.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }

    #[test]
    fn a_new_connection_starts_with_the_whole_burst() {
        let (gate, clock) = gate();
        // A connection arriving long after the process booted must not pay
        // for the time since start: a fresh bucket is full, not refilling.
        advance(&clock, Duration::from_secs(5000));

        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        assert!(!gate.try_acquire("c-1", Policy::Typing));
    }

    #[test]
    fn a_bucket_refills_at_its_policys_rate() {
        let (gate, clock) = gate();
        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        advance(&clock, Duration::from_secs(3));
        for _ in 0..3 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        assert!(!gate.try_acquire("c-1", Policy::Typing));
    }

    #[test]
    fn a_refill_stops_at_the_burst() {
        let (gate, clock) = gate();
        assert!(gate.try_acquire("c-1", Policy::Typing));
        advance(&clock, Duration::from_secs(30 * 60));

        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        assert!(!gate.try_acquire("c-1", Policy::Typing));
    }

    #[test]
    fn buckets_are_independent_per_connection_and_policy() {
        let (gate, _) = gate();
        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        // The exhausted bucket of one connection does not touch the typing of
        // another, nor the far larger signalling budget of the same one.
        assert!(gate.try_acquire("c-2", Policy::Typing));
        for _ in 0..200 {
            assert!(gate.try_acquire("c-1", Policy::Signalling));
        }

        assert!(!gate.try_acquire("c-1", Policy::Signalling));
    }

    #[test]
    fn swept_buckets_return_to_full() {
        let (gate, clock) = gate();
        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        // Sweeping runs once in its interval of calls, and only forgets
        // buckets idle long enough to have refilled anyway.
        advance(&clock, Duration::from_secs(60));
        for _ in 0..256 {
            gate.try_acquire("c-sweeper", Policy::MediaSession);
        }

        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }
    }

    #[test]
    fn a_sweep_keeps_the_buckets_that_are_not_full_yet() {
        let (gate, clock) = gate();
        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }
        advance(&clock, Duration::from_secs(2));

        for _ in 0..256 {
            gate.try_acquire("c-sweeper", Policy::MediaSession);
        }

        // Two tokens came back and nothing more: the bucket was not forgotten.
        assert!(gate.try_acquire("c-1", Policy::Typing));
        assert!(gate.try_acquire("c-1", Policy::Typing));
        assert!(!gate.try_acquire("c-1", Policy::Typing));
    }

    #[test]
    fn a_connection_that_ended_is_forgotten() {
        let (gate, _) = gate();
        for _ in 0..10 {
            assert!(gate.try_acquire("c-1", Policy::Typing));
        }

        gate.forget("c-1");

        assert!(gate.try_acquire("c-1", Policy::Typing));
    }
}
