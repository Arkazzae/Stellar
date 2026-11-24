//! What Discord said about its rate limits, kept so the next request waits
//! instead of being refused.
//!
//! Discord groups routes into buckets it names in `X-RateLimit-Bucket`, and
//! counts each bucket separately for every top-level resource (channel or
//! server) in the path. A route's bucket is learnt from its first answer;
//! until then the route counts as its own bucket. The global limit covers
//! every route at once. Times are kept as moments of the given clock, from
//! `X-RateLimit-Reset-After`, a duration, rather than the epoch in
//! `X-RateLimit-Reset`, which would trust this machine's clock to agree with
//! Discord's. The reader is meant to be used one request at a time, but a
//! lock keeps the record whole should two tasks share it.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use http::HeaderMap;

use crate::time::parse_http_date;

/// No answer Discord gives means a longer wait than this.
const LONGEST: Duration = Duration::from_secs(86_400);

/// One request's address, relative to the API's base, and what its rate limit
/// is counted by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Route {
    /// The path and query, such as `channels/1/messages?limit=100`.
    pub path: String,
    /// The route with placeholders for its ids, such as
    /// `channels/{channel}/messages`: Discord's bucket is learnt per shape.
    pub shape: String,
    /// The channel or server id Discord counts the route's limit separately for.
    pub major: String,
}

impl Route {
    pub fn new(
        path: impl Into<String>,
        shape: impl Into<String>,
        major: impl Into<String>,
    ) -> Route {
        Route {
            path: path.into(),
            shape: shape.into(),
            major: major.into(),
        }
    }
}

/// The method and path without the query, as errors and logs name the route.
impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.split('?').next().unwrap_or("");
        write!(f, "GET {path}")
    }
}

#[derive(Default)]
struct State {
    bucket_of_route: HashMap<String, String>,
    exhausted_until: HashMap<String, Duration>,
    global_until: Duration,
}

impl State {
    fn key(&self, route: &Route) -> String {
        let bucket = self
            .bucket_of_route
            .get(&route.shape)
            .map_or(route.shape.as_str(), String::as_str);
        format!("{bucket}|{}", route.major)
    }
}

#[derive(Default)]
pub(crate) struct RateLimits {
    state: Mutex<State>,
}

impl RateLimits {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // The record holds plain numbers; a task that panicked mid-update
        // cannot have left it in a state worse than a wait that is too short.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// How long to wait before sending a request on `route`.
    pub fn delay_before(&self, route: &Route, now: Duration) -> Duration {
        let state = self.state();
        let mut until = state.global_until;
        if let Some(&bucket_until) = state.exhausted_until.get(&state.key(route))
            && bucket_until > until
        {
            until = bucket_until;
        }
        until.saturating_sub(now)
    }

    /// Reads the headers of any answer: which bucket the route is in, and
    /// whether it has run out.
    pub fn observe(&self, route: &Route, headers: &HeaderMap, now: Duration) {
        let bucket = header(headers, "x-ratelimit-bucket");
        let remaining = header(headers, "x-ratelimit-remaining");
        let reset = reset_after(headers, now);
        let mut state = self.state();
        if let Some(bucket) = bucket.filter(|bucket| !bucket.is_empty()) {
            state.bucket_of_route.insert(route.shape.clone(), bucket);
        }

        let key = state.key(route);
        if let Some(remaining) = remaining.and_then(|text| text.parse::<i32>().ok()) {
            if remaining > 0 {
                state.exhausted_until.remove(&key);
            } else if let Some(after) = reset {
                state.exhausted_until.insert(key, now.saturating_add(after));
            }
        }
    }

    /// Records a 429: the bucket, or with `global` every route, waits
    /// `retry_after`.
    pub fn limited(&self, route: &Route, retry_after: Duration, global: bool, now: Duration) {
        let until = now.saturating_add(retry_after);
        let mut state = self.state();
        if !global {
            let key = state.key(route);
            state.exhausted_until.insert(key, until);
        } else if until > state.global_until {
            state.global_until = until;
        }
    }
}

/// The wait a 429 asks for: its body, then `Retry-After`, then the bucket's
/// reset.
pub(crate) fn retry_after(
    body_seconds: Option<f64>,
    headers: &HeaderMap,
    now: Duration,
) -> Option<Duration> {
    if let Some(seconds) = body_seconds
        && seconds.is_finite()
        && seconds >= 0.0
    {
        return Some(seconds_capped(seconds));
    }

    if let Some(text) = header(headers, "retry-after") {
        // Seconds, or a date; a value that is neither says nothing.
        if let Some(seconds) = whole_seconds(&text) {
            return Some(Duration::from_secs(seconds));
        }
        if let Some(date) = parse_http_date(&text) {
            return Some(date.saturating_sub(now));
        }
    }

    reset_after(headers, now)
}

pub(crate) fn is_global(headers: &HeaderMap) -> bool {
    header(headers, "x-ratelimit-global").is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || header(headers, "x-ratelimit-scope")
            .is_some_and(|value| value.eq_ignore_ascii_case("global"))
}

/// A header's first value, trimmed. Text that is not ASCII reads as absent.
fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_owned())
}

fn whole_seconds(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Header values that do not fit a 32-bit number are not read.
    text.parse::<i32>().ok().map(|seconds| seconds as u64)
}

fn reset_after(headers: &HeaderMap, now: Duration) -> Option<Duration> {
    if let Some(seconds) = header(headers, "x-ratelimit-reset-after")
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    {
        return Some(seconds_capped(seconds));
    }

    let epoch = header(headers, "x-ratelimit-reset")
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|epoch| epoch.is_finite())?;
    // The wait to a moment on Discord's clock, which is as far from now as
    // this machine's clock believes: at most a day, and none for the past.
    let at = Duration::from_secs_f64(epoch.clamp(0.0, 253_402_300_799.0));
    Some(at.saturating_sub(now).min(LONGEST))
}

/// Seconds as a duration, capped at a day.
fn seconds_capped(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds.min(LONGEST.as_secs_f64()))
}
