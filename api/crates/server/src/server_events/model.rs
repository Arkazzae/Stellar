//! What follows from an event's rule and the clock alone: when each
//! occurrence of a series starts, and which state an occurrence is in.

use std::time::Duration;

use stellarr_timezone::TimeZone;

use crate::values::Timestamp;

/// How often an event comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    None,
    Weekly,
    Biweekly,
    Monthly,
}

impl Repeat {
    pub fn parse(name: &str) -> Option<Repeat> {
        Some(match name {
            "none" => Repeat::None,
            "weekly" => Repeat::Weekly,
            "biweekly" => Repeat::Biweekly,
            "monthly" => Repeat::Monthly,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Repeat::None => "none",
            Repeat::Weekly => "weekly",
            Repeat::Biweekly => "biweekly",
            Repeat::Monthly => "monthly",
        }
    }

    /// The shortest time between two occurrences, which an occurrence must
    /// be shorter than; none for an event that does not come back.
    pub fn shortest_gap(self) -> Option<Duration> {
        const DAY: u64 = 86_400;
        match self {
            Repeat::None => None,
            Repeat::Weekly => Some(Duration::from_secs(7 * DAY)),
            Repeat::Biweekly => Some(Duration::from_secs(14 * DAY)),
            Repeat::Monthly => Some(Duration::from_secs(28 * DAY)),
        }
    }

    /// When occurrence `sequence` starts, counting the first as 0: at the
    /// clock time of the first start in `zone`, that many weeks, fortnights
    /// or months on. Each is worked out from the first, so that the 31st of
    /// a month stays the 31st where a month has one.
    pub fn start_of(self, zone: &TimeZone, first: Timestamp, sequence: i64) -> Timestamp {
        let seconds = first.unix_seconds();
        let moved = match self {
            Repeat::None => return first,
            Repeat::Weekly => zone.shifted(seconds, 0, 7 * sequence),
            Repeat::Biweekly => zone.shifted(seconds, 0, 14 * sequence),
            Repeat::Monthly => zone.shifted(seconds, sequence, 0),
        };
        Timestamp::from_unix_seconds(moved).unwrap_or(first)
    }
}

/// Where an occurrence stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Upcoming,
    Live,
    Ended,
    Cancelled,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Upcoming => "upcoming",
            State::Live => "live",
            State::Ended => "ended",
            State::Cancelled => "cancelled",
        }
    }

    /// Whether it is still to come or running.
    pub fn is_active(self) -> bool {
        matches!(self, State::Upcoming | State::Live)
    }

    /// The state at `now`, by one rule for what is shown and what is
    /// allowed: an occurrence with an end has ended once the end has passed,
    /// whether or not the worker has marked it; one without an end has ended
    /// only once the worker said so.
    pub fn at(
        cancelled: bool,
        starts_at: Timestamp,
        ends_at: Option<Timestamp>,
        ended_at: Option<Timestamp>,
        now: Timestamp,
    ) -> State {
        if cancelled {
            State::Cancelled
        } else if ended_at.is_some() || ends_at.is_some_and(|end| end <= now) {
            State::Ended
        } else if starts_at <= now {
            State::Live
        } else {
            State::Upcoming
        }
    }
}
