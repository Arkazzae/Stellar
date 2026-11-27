//! What one receiver's downlink can carry, and what of the tracks it wants is
//! sent within it.
//!
//! Every receiving session estimates its downlink from the transport-wide
//! feedback of its browser (str0m's send-side estimator). Every published
//! track measures its own recent bitrate as it arrives. From the two, each
//! receiver gets an allocation: every sound it wants is forwarded, and its
//! pictures are taken in the order of its list, each that still fits in
//! what is left. A picture that does not fit is held back for that receiver
//! alone; it stays negotiated and named, and comes back once there has been
//! room for it for a while.
//!
//! Rates are of RTP payload, as str0m's estimate is.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use str0m::media::{MediaKind, Mid};

/// The estimate a connection starts with. Nothing is held back on it, and
/// nothing is paced against it: until feedback has moved it and it has
/// settled, a receiver is sent everything it asks for as it comes. It sizes
/// the first two probes, at three and six times it but at most twice what
/// the receiver wants.
pub const INITIAL_ESTIMATE_BPS: u64 = 1_000_000;

/// How long the estimate is left to settle after feedback first moves it,
/// before anything is held back on it. The first probes report within this.
const SETTLE: Duration = Duration::from_secs(1);

/// How long a track's bitrate is averaged over, in buckets of a quarter.
const METER_WINDOW: Duration = Duration::from_secs(2);
const METER_BUCKETS: u32 = 8;
/// A track measured for less than this is taken as if it had been measured
/// for this long, so its first keyframe does not count for a whole second.
const METER_MIN_SPAN: Duration = Duration::from_millis(500);

/// A forwarded picture is held back when what it needs, with what is sent
/// before it, has been above the smoothed estimate by more than
/// `KEEP_TOLERANCE` for this long. The smoothed estimate rides out the dip
/// every congestion signal brings, which would otherwise take away the
/// picture that has only just fitted.
const OVER_GRACE: Duration = Duration::from_secs(2);
/// str0m paces what it sends at 1.1 times the estimate, so a sent picture
/// within that is one the link is given no faster than the pacer allows.
const KEEP_TOLERANCE: f64 = 1.1;
/// It is held back sooner when the estimate itself has dropped so far that
/// it carries less than this share of what the picture needs...
const DEEP_DROP: f64 = 1.0 / 1.5;
/// ...for this long.
const DEEP_GRACE: Duration = Duration::from_millis(300);
/// A held picture comes back when what it needs, with what is sent before
/// it, is at most this share of the estimate...
const HEADROOM: f64 = 0.85;
/// ...and has been for this long the first time it is held back.
const RESUME_AFTER: Duration = Duration::from_secs(4);
/// A held picture that never had room has no evidence the link carries it,
/// and str0m's estimate can stand well above the link while little is sent.
/// It comes back only while padding toward it sends what it needs, once the
/// estimate has stayed at that for this long.
const PROVEN_AFTER: Duration = Duration::from_secs(1);
/// Each time a picture is held back again within `FLAP_WINDOW` of coming
/// back, it waits twice as long before the next return, up to this.
const RESUME_AFTER_MAX: Duration = Duration::from_secs(64);
/// A picture that stays forwarded this long is steady again, and waits the
/// shortest time when it is next held back.
const FLAP_WINDOW: Duration = Duration::from_secs(30);

/// While a picture is held back and needs no more than this many times the
/// estimate, padding keeps the rate it needs up for `PAD_TRIAL`, capped by
/// the estimate, so the estimate can grow to it. A trial that does not bring
/// it back is followed by a rest without padding, twice as long each time
/// from `PAD_REST` up to `PAD_REST_MAX`, so a link that cannot carry it is
/// not kept full in front of the voices.
const PAD_REACH: f64 = 1.5;
const PAD_TRIAL: Duration = Duration::from_secs(6);
const PAD_REST: Duration = Duration::from_secs(15);
const PAD_REST_MAX: Duration = Duration::from_secs(60);
/// Padding fills the link up to the estimate, and the estimate dips when
/// it does. While it runs and for this long after, a sent picture is held
/// back only on a deep drop, never on a dip of the media server's making.
const PAD_SETTLE: Duration = Duration::from_secs(5);

/// str0m does not probe while it takes the rate to be limited by loss, and
/// with nothing but voices acked it may stay so on a link that has since
/// cleared. When a picture is held, no padding trial runs and the estimate
/// has not moved for `LOOK_AFTER`, the estimator is started again at its
/// current value, which clears that state and sends its two first probes. A
/// look that finds nothing waits twice as long before the next, from
/// `LOOK_REST` up to `LOOK_REST_MAX`.
const LOOK_AFTER: Duration = Duration::from_secs(5);
const LOOK_REST: Duration = Duration::from_secs(10);
const LOOK_REST_MAX: Duration = Duration::from_secs(120);
/// The estimate has moved when it changes by more than this share.
const MOVED: f64 = 0.05;

/// How often an allocation is looked at again for the costs of its tracks
/// and for its clocks.
pub const RECHECK: Duration = Duration::from_millis(500);
/// A track's cost has changed materially when it moves by this share since
/// the allocation last ran.
const COST_CHANGE: f64 = 0.2;

/// The recent bitrate of a published track, from the payload that arrives.
#[derive(Debug, Default)]
pub(crate) struct RateMeter {
    /// The start of the current bucket and the bytes of every bucket, the
    /// current one last.
    current: Option<Instant>,
    buckets: Vec<u64>,
    first: Option<Instant>,
}

impl RateMeter {
    fn bucket() -> Duration {
        METER_WINDOW / METER_BUCKETS
    }

    pub fn record(&mut self, now: Instant, bytes: usize) {
        self.first.get_or_insert(now);
        self.roll(now);
        if let Some(last) = self.buckets.last_mut() {
            *last += bytes as u64;
        }
    }

    /// Moves the buckets on to `now`, forgetting what fell out of the window.
    fn roll(&mut self, now: Instant) {
        let bucket = Self::bucket();
        let start = *self.current.get_or_insert(now);
        if self.buckets.is_empty() {
            self.buckets.push(0);
        }
        let elapsed = now.saturating_duration_since(start);
        let steps = (elapsed.as_nanos() / bucket.as_nanos()) as u64;
        if steps == 0 {
            return;
        }
        for _ in 0..steps.min(u64::from(METER_BUCKETS)) {
            self.buckets.push(0);
        }
        let keep = METER_BUCKETS as usize;
        if self.buckets.len() > keep {
            self.buckets.drain(..self.buckets.len() - keep);
        }
        self.current = Some(start + bucket * steps as u32);
    }

    pub fn has_full_window(&self, now: Instant) -> bool {
        self.first
            .is_some_and(|first| now.saturating_duration_since(first) >= METER_WINDOW)
    }

    /// Bits per second over the buckets that still fall in the window, or
    /// over the time since the first packet when that is shorter.
    pub fn bps(&self, now: Instant) -> u64 {
        let (Some(first), Some(current)) = (self.first, self.current) else {
            return 0;
        };
        let bucket = Self::bucket();
        // Buckets that ended before the window began no longer count.
        let behind = (now.saturating_duration_since(current).as_nanos() / bucket.as_nanos()) as u32;
        let keep = METER_BUCKETS.saturating_sub(behind);
        if keep == 0 {
            return 0;
        }
        let bytes: u64 = self.buckets.iter().rev().take(keep as usize).sum();
        let oldest = current
            .checked_sub(bucket * (keep - 1))
            .map_or(first, |t| t.max(first));
        let span = now
            .saturating_duration_since(oldest)
            .clamp(METER_MIN_SPAN, METER_WINDOW);
        (bytes as f64 * 8.0 / span.as_secs_f64()) as u64
    }
}

/// A section the receiver asks for, in the order it asks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Wanted {
    pub mid: Mid,
    pub kind: MediaKind,
    /// The track's recent bitrate.
    pub cost: u64,
}

/// How one picture of one receiver stands with its hysteresis.
#[derive(Debug, Clone, Default)]
struct Clock {
    /// Since when a forwarded picture has not fitted, and since when the
    /// estimate has dropped far below what it needs.
    over_since: Option<Instant>,
    deep_since: Option<Instant>,
    /// Since when a held picture has fitted with headroom.
    fits_since: Option<Instant>,
    /// How long the headroom has to last before it comes back.
    resume_after: Duration,
    resumed_at: Option<Instant>,
    /// It has been forwarded to this receiver with room for it. One that
    /// never was comes as soon as it fits, so a picture that starts on a
    /// good link waits only for the estimator to find room for it.
    forwarded: bool,
}

/// What the allocation decided.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Allocation {
    /// The rate of everything forwarded.
    pub forwarded: u64,
    /// What the estimator should look for: what is forwarded and the first
    /// picture held back.
    pub desired: u64,
    /// The rate to keep up with padding while a picture is held back, so
    /// the estimate can grow to what it needs: the same as `desired`, and
    /// nothing while every picture is sent.
    pub minimum: u64,
}

/// Padding toward the first held picture, in trials and rests.
#[derive(Debug, Clone, Default)]
struct Padding {
    /// The picture being padded toward, and until when.
    trial: Option<(Mid, Instant)>,
    /// No padding until then.
    rest_until: Option<Instant>,
    /// When the last trial ended, and the estimate then.
    ended_at: Option<Instant>,
    ended_estimate: f64,
    /// How long the next rest lasts.
    rest: Duration,
}

/// A rest ends early once the estimate has risen this much since the trial
/// before it: room may have been found that is worth a trial now.
const PAD_RISE: f64 = 1.25;

/// Fresh looks of the estimator while a held picture waits and no trial runs.
#[derive(Debug, Clone, Default)]
struct Looks {
    /// The estimate, and since when it has been there.
    still: Option<(f64, Instant)>,
    /// No look before then.
    rest_until: Option<Instant>,
    rest: Duration,
}

/// The downlink of one receiving session.
#[derive(Debug)]
pub(crate) struct Downlink {
    /// The estimate as str0m last computed it, from the connection's
    /// statistics, and as it last reported it smoothed.
    pub raw: Option<u64>,
    pub smoothed: Option<u64>,
    pub can_probe: bool,
    /// When feedback first moved the estimate off its initial value.
    pub moved_at: Option<Instant>,
    /// The estimate has settled since, and the allocation goes by it.
    pub measured: bool,
    /// The pictures held back for this receiver now, and those sent, by its
    /// mids.
    pub held: HashSet<Mid>,
    sent: HashSet<Mid>,
    clocks: HashMap<Mid, Clock>,
    padding: Padding,
    looks: Looks,
    /// The estimator is to start again at its current value.
    pub look: bool,
    pub allocation: Allocation,
    /// The costs the allocation last ran with.
    costs: HashMap<Mid, u64>,
    /// Something it depends on changed: the estimate, the list or the
    /// directory.
    pub dirty: bool,
    pub next_check: Option<Instant>,
    /// What was last given to the estimator, to give it only changes.
    pub bitrate_set: Option<(u64, u64)>,
    /// How often a picture was held back and came back, for the stats.
    pub holds: u64,
    pub resumes: u64,
}

impl Default for Downlink {
    fn default() -> Downlink {
        Downlink {
            raw: None,
            smoothed: None,
            can_probe: false,
            moved_at: None,
            measured: false,
            held: HashSet::new(),
            sent: HashSet::new(),
            clocks: HashMap::new(),
            padding: Padding::default(),
            looks: Looks::default(),
            look: false,
            allocation: Allocation::default(),
            costs: HashMap::new(),
            dirty: true,
            next_check: None,
            bitrate_set: None,
            holds: 0,
            resumes: 0,
        }
    }
}

impl Downlink {
    /// The estimate the allocation works with: the lower of the two, so a
    /// drop counts at once and a rise only once it has lasted. `None` until
    /// feedback has moved it and it has settled, since the initial value is
    /// a guess and the first move may come before the first probe's report.
    pub fn estimate(&self) -> Option<u64> {
        if !self.measured {
            return None;
        }
        match (self.raw, self.smoothed) {
            (Some(r), Some(s)) => Some(r.min(s)),
            (r, s) => r.or(s),
        }
    }

    fn moved(&mut self, now: Instant, bps: u64) {
        if bps != INITIAL_ESTIMATE_BPS && self.moved_at.is_none() {
            self.moved_at = Some(now);
        }
    }

    /// A new raw estimate from the connection's statistics.
    pub fn raw_estimate(&mut self, now: Instant, bps: u64) {
        self.moved(now, bps);
        if self.raw != Some(bps) {
            self.raw = Some(bps);
            self.dirty = true;
        }
    }

    pub fn smoothed_estimate(&mut self, now: Instant, bps: u64, can_probe: bool) {
        self.moved(now, bps);
        self.smoothed = Some(bps);
        self.can_probe = can_probe;
        self.dirty = true;
    }

    /// Whether anything the allocation depends on changed materially since
    /// it last ran, or one of its clocks may have run out.
    pub fn needs_allocation(&self, wanted: &[Wanted]) -> bool {
        if self.dirty || (self.moved_at.is_some() && !self.measured) {
            return true;
        }
        let clocks_running =
            self.clocks.values().any(|c| {
                c.over_since.is_some() || c.deep_since.is_some() || c.fits_since.is_some()
            }) || self.padding.trial.is_some()
                || self.padding.rest_until.is_some()
                || (self.looks.still.is_some() && !self.held.is_empty());
        if clocks_running {
            return true;
        }
        if wanted.len() != self.costs.len() {
            return true;
        }
        wanted.iter().any(|w| {
            let before = self.costs.get(&w.mid).copied().unwrap_or(0);
            let (low, high) = (before.min(w.cost) as f64, before.max(w.cost) as f64);
            high > low * (1.0 + COST_CHANGE) && high - low > 8_000.0
        })
    }

    /// Forgets the clocks of sections that are gone.
    pub fn retain(&mut self, live: impl Fn(Mid) -> bool) {
        self.clocks.retain(|mid, _| live(*mid));
        self.held.retain(|mid| live(*mid));
        self.sent.retain(|mid| live(*mid));
    }

    /// Decides what of `wanted`, in the receiver's order, is sent now.
    /// Returns whether the held pictures changed.
    pub fn allocate(&mut self, now: Instant, wanted: &[Wanted]) -> bool {
        self.dirty = false;
        if self
            .moved_at
            .is_some_and(|at| now.saturating_duration_since(at) >= SETTLE)
        {
            self.measured = true;
        }
        self.costs = wanted.iter().map(|w| (w.mid, w.cost)).collect();
        // The lower of the two lets a picture come back only on room that
        // has lasted; the smoothed one keeps a picture through a passing dip.
        let estimates = self.estimate().map(|low| {
            let raw = self.raw.unwrap_or(low) as f64;
            let smoothed = self.smoothed.unwrap_or(low) as f64;
            (low as f64, raw, smoothed)
        });

        // Every sound the receiver wants is sent, whatever the estimate.
        let audio: u64 = wanted
            .iter()
            .filter(|w| w.kind == MediaKind::Audio)
            .map(|w| w.cost)
            .sum();

        let mut used = audio;
        let mut held = HashSet::new();
        let mut sent = HashSet::new();
        let mut next_held: Option<(Mid, u64)> = None;
        // A held picture within reach keeps those after it from being taken,
        // so a later one never takes the room an earlier one waits for.
        let mut blocked = false;
        let padding = self.padding.trial.is_some()
            || self
                .padding
                .ended_at
                .is_some_and(|at| now.saturating_duration_since(at) < PAD_SETTLE);
        for w in wanted.iter().filter(|w| w.kind == MediaKind::Video) {
            let clock = self.clocks.entry(w.mid).or_default();
            let was_held = self.held.contains(&w.mid);
            let was_sent = self.sent.contains(&w.mid);
            let need = used + w.cost;
            let keep = match estimates {
                // Until the estimate is measured, everything is sent.
                None => true,
                Some((low, raw, smoothed)) => {
                    let need = need as f64;
                    if !was_held && !was_sent {
                        // Newly asked for: taken if it fits now.
                        !blocked && need <= low
                    } else if was_held {
                        if blocked {
                            clock.fits_since = None;
                            false
                        } else if !clock.forwarded {
                            // It never had room: padding toward it has to
                            // show the room first.
                            let proving =
                                matches!(self.padding.trial, Some((mid, _)) if mid == w.mid);
                            if proving && need <= low {
                                let since = *clock.fits_since.get_or_insert(now);
                                now.saturating_duration_since(since) >= PROVEN_AFTER
                            } else {
                                clock.fits_since = None;
                                false
                            }
                        } else if need <= low * HEADROOM {
                            let since = *clock.fits_since.get_or_insert(now);
                            now.saturating_duration_since(since) >= clock.resume_after
                        } else {
                            clock.fits_since = None;
                            false
                        }
                    } else {
                        // Room the estimate has shown is judged by the
                        // smoothed estimate. What was sent only on the
                        // initial guess is judged by the lower one, since the
                        // guess still inflates the smoothing.
                        let base = if clock.forwarded { smoothed } else { low };
                        let deep = need * DEEP_DROP > raw;
                        let over = need > base * KEEP_TOLERANCE && !padding;
                        if !deep {
                            clock.deep_since = None;
                        }
                        if !over {
                            clock.over_since = None;
                        }
                        let deep_for = deep.then(|| now - *clock.deep_since.get_or_insert(now));
                        let over_for = over.then(|| now - *clock.over_since.get_or_insert(now));
                        !(deep_for.is_some_and(|d| d >= DEEP_GRACE)
                            || over_for.is_some_and(|d| d >= OVER_GRACE))
                    }
                }
            };

            if keep {
                used = need;
                sent.insert(w.mid);
                // Only room the estimate showed counts: what was sent on the
                // initial guess, or through the grace of a picture that no
                // longer fitted, does not make a later return wait.
                clock.forwarded |=
                    estimates.is_some() && clock.over_since.is_none() && clock.deep_since.is_none();
                if was_held {
                    clock.fits_since = None;
                    clock.resumed_at = Some(now);
                    self.resumes += 1;
                }
                // Steady for long enough: the next hold waits the least.
                if clock
                    .resumed_at
                    .is_some_and(|at| now.saturating_duration_since(at) >= FLAP_WINDOW)
                {
                    clock.resumed_at = None;
                    clock.resume_after = RESUME_AFTER;
                }
            } else {
                if !was_held {
                    clock.over_since = None;
                    clock.deep_since = None;
                    clock.resume_after = match clock.resumed_at {
                        _ if !clock.forwarded => Duration::ZERO,
                        // Back so recently that this is a flap: wait longer.
                        Some(at) if now.saturating_duration_since(at) < FLAP_WINDOW => {
                            (clock.resume_after * 2).clamp(RESUME_AFTER, RESUME_AFTER_MAX)
                        }
                        _ => RESUME_AFTER,
                    };
                    clock.resumed_at = None;
                    self.holds += 1;
                }
                held.insert(w.mid);
                next_held.get_or_insert((w.mid, w.cost));
                let low = estimates.map_or(0.0, |(low, _, _)| low);
                blocked |= need as f64 <= low * PAD_REACH;
            }
        }

        let desired = used + next_held.map_or(0, |(_, cost)| cost);
        let low = estimates.map_or(0.0, |(low, _, _)| low);
        let in_reach = desired as f64 <= low * PAD_REACH;
        let pad = self.pad(now, next_held.map(|(mid, _)| mid), in_reach, low);
        self.look = self.look_due(now, next_held.is_some() && !pad, low);
        self.allocation = Allocation {
            forwarded: used,
            desired,
            minimum: if pad { desired } else { 0 },
        };
        let changed = held != self.held;
        self.held = held;
        self.sent = sent;
        changed
    }

    /// Whether the estimator should take a fresh look now: a picture is held,
    /// nothing pads toward it, and the estimate has stood still long enough.
    fn look_due(&mut self, now: Instant, waiting: bool, estimate: f64) -> bool {
        let l = &mut self.looks;
        if !waiting {
            *l = Looks::default();
            return false;
        }
        if l.rest.is_zero() {
            l.rest = LOOK_REST;
        }
        let (value, since) = match l.still {
            Some((value, since)) if (estimate - value).abs() <= value * MOVED => (value, since),
            _ => (estimate, now),
        };
        l.still = Some((value, since));
        if now.saturating_duration_since(since) < LOOK_AFTER
            || l.rest_until.is_some_and(|until| now < until)
        {
            return false;
        }
        l.rest_until = Some(now + l.rest);
        l.rest = (l.rest * 2).min(LOOK_REST_MAX);
        l.still = Some((value, now));
        true
    }

    /// Whether to pad toward the first held picture now, given whether it is
    /// within reach of the estimate.
    fn pad(
        &mut self,
        now: Instant,
        first_held: Option<Mid>,
        in_reach: bool,
        estimate: f64,
    ) -> bool {
        let p = &mut self.padding;
        let Some(target) = first_held else {
            let ended_at = p.trial.map(|_| now).or(p.ended_at);
            *p = Padding {
                ended_at,
                ..Padding::default()
            };
            return false;
        };
        if p.rest.is_zero() {
            p.rest = PAD_REST;
        }
        match p.trial {
            // Another picture is first now: it gets a trial of its own.
            Some((mid, _)) if mid != target => {
                p.trial = None;
                p.rest_until = None;
                p.rest = PAD_REST;
            }
            Some((_, until)) if now >= until => {
                p.trial = None;
                p.ended_at = Some(now);
                p.ended_estimate = estimate;
                p.rest_until = Some(now + p.rest);
                p.rest = (p.rest * 2).min(PAD_REST_MAX);
            }
            _ => {}
        }
        let risen = estimate > p.ended_estimate * PAD_RISE;
        if p.rest_until.is_some_and(|until| now < until) && !risen {
            return false;
        }
        p.rest_until = None;
        if p.trial.is_none() {
            if !in_reach {
                return false;
            }
            p.trial = Some((target, now + PAD_TRIAL));
        }
        true
    }

    /// When the allocation should be looked at again.
    pub fn schedule(&mut self, now: Instant, has_video: bool) {
        self.next_check = has_video.then_some(now + RECHECK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KBPS: u64 = 1000;

    fn audio(mid: &str, kbps: u64) -> Wanted {
        Wanted {
            mid: mid.into(),
            kind: MediaKind::Audio,
            cost: kbps * KBPS,
        }
    }

    fn video(mid: &str, kbps: u64) -> Wanted {
        Wanted {
            mid: mid.into(),
            kind: MediaKind::Video,
            cost: kbps * KBPS,
        }
    }

    /// Both estimates at `kbps`.
    fn set(d: &mut Downlink, now: Instant, kbps: u64) {
        d.raw_estimate(now, kbps * KBPS);
        d.smoothed_estimate(now, kbps * KBPS, true);
    }

    /// A downlink whose estimate moved long enough ago to count.
    fn measured(kbps: u64) -> Downlink {
        let mut d = Downlink::default();
        set(&mut d, Instant::now() - Duration::from_secs(10), kbps);
        d
    }

    fn held(d: &Downlink) -> Vec<String> {
        let mut v: Vec<String> = d.held.iter().map(|m| m.to_string()).collect();
        v.sort();
        v
    }

    fn ms(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn the_meter_measures_the_recent_rate() {
        let start = Instant::now();
        let mut m = RateMeter::default();
        assert_eq!(m.bps(start), 0);
        // 100 kbit/s for three seconds in packets every 20 ms.
        for n in 0..150u32 {
            m.record(start + Duration::from_millis(20) * n, 250);
        }
        let now = start + Duration::from_secs(3);
        let bps = m.bps(now);
        assert!((90_000..=110_000).contains(&bps), "{bps}");
        // Silence empties the window.
        assert_eq!(m.bps(now + Duration::from_secs(3)), 0);
        // A first burst counts over half a second at least.
        let mut first = RateMeter::default();
        first.record(start, 10_000);
        assert_eq!(first.bps(start), 160_000);
    }

    #[test]
    fn nothing_is_held_back_before_the_estimate_has_moved_and_settled() {
        let now = Instant::now();
        let mut d = Downlink::default();
        d.raw_estimate(now, INITIAL_ESTIMATE_BPS);
        let wanted = [audio("a", 64), video("v", 5_000)];
        d.allocate(now, &wanted);
        assert!(d.held.is_empty());
        assert_eq!(d.estimate(), None);

        // Feedback moves it: it settles for a second before it counts.
        d.raw_estimate(now, 300 * KBPS);
        d.allocate(ms(now, 900), &wanted);
        assert!(d.held.is_empty());
        assert!(d.needs_allocation(&wanted));
        d.allocate(ms(now, 1_000), &wanted);
        d.allocate(ms(now, 1_300), &wanted);
        assert_eq!(held(&d), ["v"]);
        // Sent only on the guess, it never had room: it is back once padding
        // toward it has shown the room for a second.
        set(&mut d, now, 6_000);
        d.allocate(ms(now, 1_400), &wanted);
        assert!(d.allocation.minimum > 0);
        d.allocate(ms(now, 1_500), &wanted);
        d.allocate(ms(now, 2_400), &wanted);
        assert_eq!(held(&d), ["v"]);
        d.allocate(ms(now, 2_500), &wanted);
        assert!(d.held.is_empty());
    }

    #[test]
    fn voices_come_first_and_pictures_in_order_each_that_fits() {
        let now = Instant::now();
        let mut d = measured(500);
        let wanted = [
            audio("a1", 64),
            audio("a2", 64),
            video("share", 1_500),
            video("cam1", 250),
            video("cam2", 250),
        ];
        assert!(d.allocate(now, &wanted));
        // The share does not fit; the first camera does, the second does not.
        assert_eq!(held(&d), ["cam2", "share"]);
        assert_eq!(d.allocation.forwarded, 378 * KBPS);
        // The estimator looks for the first picture held back. It is out of
        // the estimate's reach, so nothing is padded toward it.
        assert_eq!(d.allocation.desired, (378 + 1_500) * KBPS);
        assert_eq!(d.allocation.minimum, 0);

        // Not even the sounds fit: they are still all sent.
        let mut d = measured(100);
        d.allocate(now, &wanted);
        assert_eq!(held(&d), ["cam1", "cam2", "share"]);
        assert_eq!(d.allocation.forwarded, 128 * KBPS);

        // With everything sent, nothing is padded.
        let mut d = measured(5_000);
        d.allocate(now, &wanted);
        assert!(d.held.is_empty());
        assert_eq!(d.allocation.desired, 2_128 * KBPS);
        assert_eq!(d.allocation.minimum, 0);
    }

    #[test]
    fn a_sent_picture_rides_out_a_dip_and_goes_on_a_drop() {
        let start = Instant::now();
        let wanted = [audio("a", 64), video("cam", 800)];

        // A dip of the raw estimate alone takes nothing away.
        let mut d = measured(2_000);
        d.allocate(start, &wanted);
        d.raw_estimate(start, 700 * KBPS);
        d.allocate(ms(start, 100), &wanted);
        d.allocate(ms(start, 5_000), &wanted);
        assert!(d.held.is_empty());

        // The smoothed estimate under what it needs by more than a tenth:
        // held back after two seconds.
        set(&mut d, start, 770);
        d.allocate(ms(start, 5_100), &wanted);
        d.allocate(ms(start, 7_000), &wanted);
        assert!(d.held.is_empty());
        assert!(d.allocate(ms(start, 7_100), &wanted));
        assert_eq!(held(&d), ["cam"]);

        // A drop of the raw estimate far below it: held back in 300 ms.
        let mut d = measured(2_000);
        d.allocate(start, &wanted);
        d.raw_estimate(start, 500 * KBPS);
        d.allocate(ms(start, 10), &wanted);
        assert!(d.held.is_empty());
        assert!(d.allocate(ms(start, 310), &wanted));
        assert_eq!(held(&d), ["cam"]);
    }

    #[test]
    fn a_held_picture_comes_back_after_lasting_headroom_and_flaps_are_bounded() {
        let start = Instant::now();
        let at = |t: u64| ms(start, t);
        let wanted = [audio("a", 64), video("cam", 800)];
        let mut d = measured(2_000);
        d.allocate(at(0), &wanted);
        set(&mut d, start, 500);
        d.allocate(at(10), &wanted);
        d.allocate(at(400), &wanted);
        assert_eq!(held(&d), ["cam"]);

        // Room for it, but only just: never enough headroom.
        set(&mut d, start, 900);
        d.allocate(at(1_000), &wanted);
        d.allocate(at(20_000), &wanted);
        assert_eq!(held(&d), ["cam"]);

        // Headroom that lasts four seconds brings it back.
        set(&mut d, start, 1_100);
        d.allocate(at(21_000), &wanted);
        d.allocate(at(24_900), &wanted);
        assert_eq!(held(&d), ["cam"]);
        assert!(d.allocate(at(25_000), &wanted));
        assert!(d.held.is_empty());

        // Held back again soon after: every return waits twice as long, up
        // to a bound.
        let mut t = 25_000;
        let mut waits = Vec::new();
        for _ in 0..7 {
            set(&mut d, start, 500);
            t += 1_000;
            d.allocate(at(t), &wanted);
            t += 300;
            d.allocate(at(t), &wanted);
            assert_eq!(held(&d), ["cam"]);
            set(&mut d, start, 1_100);
            let from = t;
            while !d.held.is_empty() {
                t += 100;
                d.allocate(at(t), &wanted);
            }
            waits.push((t - from) / 1000);
        }
        assert_eq!(waits, [8, 16, 32, 64, 64, 64, 64]);

        // Steady for half a minute, the next hold waits the least again.
        t += 31_000;
        d.allocate(at(t), &wanted);
        set(&mut d, start, 500);
        d.allocate(at(t + 100), &wanted);
        d.allocate(at(t + 400), &wanted);
        assert_eq!(held(&d), ["cam"]);
        set(&mut d, start, 1_100);
        d.allocate(at(t + 500), &wanted);
        d.allocate(at(t + 4_400), &wanted);
        assert_eq!(held(&d), ["cam"]);
        d.allocate(at(t + 4_500), &wanted);
        assert!(d.held.is_empty());
    }

    #[test]
    fn a_picture_that_never_had_room_comes_once_padding_shows_the_room() {
        let start = Instant::now();
        // A new picture that fits is taken at once.
        let wanted = [audio("a", 64), video("cam", 800)];
        let mut d = measured(1_800);
        d.allocate(start, &wanted);
        assert!(d.held.is_empty());

        // One that does not is padded toward at once, being within reach,
        // and comes a second after the estimate holds what it needs.
        let wanted = [audio("a", 64), video("share", 2_500)];
        let mut d = measured(1_800);
        d.allocate(start, &wanted);
        assert_eq!(held(&d), ["share"]);
        assert_eq!(d.allocation.minimum, 2_564 * KBPS);
        d.raw_estimate(start, 5_000 * KBPS);
        d.smoothed_estimate(start, 4_000 * KBPS, true);
        d.allocate(ms(start, 300), &wanted);
        assert_eq!(held(&d), ["share"]);
        assert!(d.allocate(ms(start, 1_300), &wanted));
        assert!(d.held.is_empty());

        // Without padding toward it, an estimate alone is no evidence: the
        // estimate may stand above the link while little is sent.
        let wanted = [audio("a", 32), video("cam", 272)];
        let mut d = measured(171);
        d.allocate(start, &wanted);
        assert_eq!(held(&d), ["cam"]);
        assert_eq!(d.allocation.minimum, 0, "out of reach");
        set(&mut d, start, 310);
        d.allocate(ms(start, 100), &wanted);
        assert_eq!(held(&d), ["cam"]);
        assert!(d.allocation.minimum > 0, "within reach now");
        d.allocate(ms(start, 200), &wanted);
        d.allocate(ms(start, 1_100), &wanted);
        assert_eq!(held(&d), ["cam"]);
        assert!(d.allocate(ms(start, 1_200), &wanted));
        assert!(d.held.is_empty());
    }

    #[test]
    fn padding_toward_a_held_picture_comes_in_trials_with_growing_rests() {
        let start = Instant::now();
        let wanted = [audio("a", 64), video("cam", 250)];
        // Within reach: 314 kbit/s needed on an estimate of 250.
        let mut d = measured(250);
        const { assert!(314.0 <= 250.0 * PAD_REACH) };
        let mut padded = Vec::new();
        for s in 0..120u64 {
            d.allocate(ms(start, s * 1000), &wanted);
            if d.allocation.minimum > 0 {
                assert_eq!(d.allocation.minimum, 314 * KBPS);
                padded.push(s);
            }
        }
        // Six seconds of padding, then fifteen without, six with, thirty
        // without, and from then on a minute between trials.
        let trials: Vec<(u64, u64)> = padded
            .chunk_by(|a, b| b == &(a + 1))
            .map(|run| (run[0], run.len() as u64))
            .collect();
        assert_eq!(trials, [(0, 6), (21, 6), (57, 6)]);

        // Out of reach, nothing is padded.
        let mut d = measured(200);
        d.allocate(start, &wanted);
        assert_eq!(held(&d), ["cam"]);
        assert_eq!(d.allocation.minimum, 0);

        // A rest ends as soon as the estimate has risen by a quarter.
        let mut d = measured(250);
        for s in 0..8u64 {
            d.allocate(ms(start, s * 1000), &wanted);
        }
        assert_eq!(d.allocation.minimum, 0, "resting");
        set(&mut d, start, 320);
        d.allocate(ms(start, 9_000), &wanted);
        assert_eq!(d.allocation.minimum, 314 * KBPS, "a new trial");
    }

    #[test]
    fn a_later_picture_never_takes_the_room_an_earlier_one_waits_for() {
        let start = Instant::now();
        let wanted = [audio("a", 64), video("b", 250), video("c", 250)];
        let mut d = measured(450);
        d.allocate(start, &wanted);
        assert_eq!(held(&d), ["c"]);
        // b goes on a drop; c, which never had room, stays where it is.
        d.raw_estimate(start, 200 * KBPS);
        d.allocate(ms(start, 100), &wanted);
        d.allocate(ms(start, 400), &wanted);
        assert_eq!(held(&d), ["b", "c"]);
        set(&mut d, start, 380);
        for t in 1..30 {
            d.allocate(ms(start, 400 + t * 100), &wanted);
            assert_eq!(held(&d), ["b", "c"]);
        }
        // b comes back in its turn.
        d.allocate(ms(start, 4_500), &wanted);
        assert_eq!(held(&d), ["c"]);

        // An earlier picture far out of reach does not keep a later one out.
        let wanted = [audio("a", 64), video("share", 2_500), video("cam", 250)];
        let mut d = measured(500);
        d.allocate(start, &wanted);
        assert_eq!(held(&d), ["share"]);
    }

    #[test]
    fn padding_of_the_media_server_s_own_making_takes_nothing_away() {
        let start = Instant::now();
        let wanted = [audio("a", 64), video("b", 250), video("c", 250)];
        let mut d = measured(430);
        d.allocate(start, &wanted);
        assert_eq!(held(&d), ["c"]);
        assert!(d.allocation.minimum > 0, "a trial toward c");
        // The trial dips the estimate under what b needs, not far under.
        set(&mut d, start, 300);
        for t in 1..=90 {
            d.allocate(ms(start, t * 100), &wanted);
        }
        assert_eq!(held(&d), ["c"]);
        // A deep drop still takes b at once.
        d.raw_estimate(start, 150 * KBPS);
        d.allocate(ms(start, 9_100), &wanted);
        d.allocate(ms(start, 9_400), &wanted);
        assert_eq!(held(&d), ["b", "c"]);
    }

    #[test]
    fn an_estimate_that_stands_still_under_a_held_picture_takes_fresh_looks() {
        let start = Instant::now();
        let wanted = [audio("a", 32), video("cam", 250)];
        // Out of reach on an estimate of 100.
        let mut d = measured(100);
        let mut looks = Vec::new();
        for s in 0..200u64 {
            d.allocate(ms(start, s * 1000), &wanted);
            if d.look {
                looks.push(s);
            }
        }
        // Five seconds still, then looks that wait 10, 20, 40, 80 s and at
        // most two minutes.
        assert_eq!(looks, [5, 15, 35, 75, 155]);

        // An estimate that keeps moving needs no look.
        let mut d = measured(100);
        for s in 0..60u64 {
            set(&mut d, start, 100 + (s % 2) * 20);
            d.allocate(ms(start, s * 1000), &wanted);
            assert!(!d.look);
        }

        // Within reach, never while padding runs, only in its rests.
        let mut d = measured(250);
        let mut looks = Vec::new();
        for s in 0..60u64 {
            d.allocate(ms(start, s * 1000), &wanted);
            assert!(!(d.look && d.allocation.minimum > 0));
            if d.look {
                looks.push(s);
            }
        }
        // After the first trial, five seconds still; the clock starts again
        // after every trial.
        assert_eq!(looks, [11, 32, 42]);
    }

    #[test]
    fn a_cost_that_moves_materially_asks_for_an_allocation() {
        let now = Instant::now();
        let mut d = measured(2_000);
        let wanted = [audio("a", 64), video("cam", 800)];
        d.allocate(now, &wanted);
        assert!(!d.needs_allocation(&wanted));
        assert!(!d.needs_allocation(&[audio("a", 66), video("cam", 900)]));
        assert!(d.needs_allocation(&[audio("a", 64), video("cam", 1_000)]));
        assert!(d.needs_allocation(&[audio("a", 64)]));
    }
}
