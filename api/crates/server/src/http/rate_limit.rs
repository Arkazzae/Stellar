//! The named limits a route opts into.
//!
//! Each policy counts requests in a fixed window of its own, per client
//! address, except uploads, which share one budget across the instance
//! because their cost is bytes, not callers. A request over its limit is
//! answered with 429 and no body, with no `Retry-After`.
//!
//! A fixed window starts at the first request of its partition and runs for
//! its whole length; the count starts again from zero when it is over.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::settings::RateLimitSettings;
use crate::values::Id;

/// The limits a route can ask for, by what they protect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateLimitPolicy {
    /// Signing in with a password.
    Login,
    /// The desktop client asking whether the browser approved its sign-in.
    LoginPoll,
    /// Account changes that send mail.
    AuthWrite,
    /// Changes to a signed-in account's own second factor. They send no mail
    /// and already run inside attempt budgets, so they count apart from
    /// `AuthWrite`.
    SecurityWrite,
    /// The anonymous look at an invite before joining. A page asks once per
    /// visit; the budget leaves room for a few people behind one address and
    /// none for walking through codes.
    InvitePreview,
    /// Tries of the encryption PIN, counted per address here and per account
    /// by [`AttemptLimiter`]: the limiter runs before authentication, so it
    /// cannot tell whose PIN is being tried.
    PinUnlock,
    /// Uploads that run at once, queued briefly, and uploads per minute, both
    /// counted for the whole instance.
    Uploads,
}

const MINUTE: Duration = Duration::from_secs(60);
const QUARTER_HOUR: Duration = Duration::from_secs(15 * 60);

/// The window of the one counted per address and the one counted for the instance.
const PIN_UNLOCK_WINDOW: Duration = QUARTER_HOUR;

/// A request that has been let through. Dropping it ends what it holds, which
/// for an upload is its place among those running.
pub struct Lease {
    _permit: Option<OwnedSemaphorePermit>,
}

pub struct RateLimiter {
    limits: RateLimitSettings,
    windows: Mutex<Windows<(RateLimitPolicy, String)>>,
    uploads: Uploads,
}

struct Window {
    started: Instant,
    used: u32,
}

/// Where the counts of a limit are kept, one window to each address or
/// account. A window that is over owes nothing, so expired ones are swept
/// every so many requests, and the table never holds more than
/// [`MOST_WINDOWS`]: when it is full of live ones the oldest go first.
struct Windows<K> {
    table: HashMap<K, Window>,
    since_sweep: u32,
}

impl<K> Default for Windows<K> {
    fn default() -> Windows<K> {
        Windows {
            table: HashMap::new(),
            since_sweep: 0,
        }
    }
}

const SWEEP_EVERY: u32 = 1024;
const MOST_WINDOWS: usize = 16 * 1024;

impl<K: Hash + Eq> Windows<K> {
    fn get(&mut self, key: K, now: Instant, length_of: impl Fn(&K) -> Duration) -> &mut Window {
        self.since_sweep += 1;
        if self.since_sweep >= SWEEP_EVERY {
            self.since_sweep = 0;
            self.table.retain(|key, window| {
                now.saturating_duration_since(window.started) < length_of(key)
            });
        }
        if self.table.len() >= MOST_WINDOWS && !self.table.contains_key(&key) {
            // Full of windows that are still running: make room by dropping
            // the oldest eighth, whose counts would soon have started again.
            let mut started: Vec<Instant> =
                self.table.values().map(|window| window.started).collect();
            started.sort_unstable();
            let cut = started[MOST_WINDOWS / 8];
            self.table.retain(|_, window| window.started > cut);
        }
        self.table.entry(key).or_insert(Window {
            started: now,
            used: 0,
        })
    }
}

struct Uploads {
    running: Arc<Semaphore>,
    waiting: AtomicUsize,
    queue_limit: usize,
    per_minute: Mutex<Window>,
}

impl RateLimiter {
    pub fn new(limits: &RateLimitSettings) -> RateLimiter {
        let concurrent = limits.concurrent_uploads as usize;
        RateLimiter {
            limits: limits.clone(),
            windows: Mutex::new(Windows::default()),
            uploads: Uploads {
                running: Arc::new(Semaphore::new(concurrent)),
                waiting: AtomicUsize::new(0),
                queue_limit: concurrent * 4,
                per_minute: Mutex::new(Window {
                    started: Instant::now(),
                    used: 0,
                }),
            },
        }
    }

    /// Counts one request of `address` against `policy`. None means it is over
    /// the limit.
    pub async fn acquire(&self, policy: RateLimitPolicy, address: &str) -> Option<Lease> {
        self.acquire_at(policy, address, Instant::now()).await
    }

    async fn acquire_at(
        &self,
        policy: RateLimitPolicy,
        address: &str,
        now: Instant,
    ) -> Option<Lease> {
        let (limit, length) = match policy {
            RateLimitPolicy::Login => (self.limits.login, MINUTE),
            RateLimitPolicy::LoginPoll => (self.limits.login_poll, MINUTE),
            RateLimitPolicy::AuthWrite => (self.limits.auth_write, QUARTER_HOUR),
            RateLimitPolicy::SecurityWrite => (self.limits.security_write, QUARTER_HOUR),
            RateLimitPolicy::InvitePreview => (self.limits.invite_preview, MINUTE),
            RateLimitPolicy::PinUnlock => (self.limits.pin_unlock, PIN_UNLOCK_WINDOW),
            RateLimitPolicy::Uploads => return self.acquire_upload(now).await,
        };
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let window = windows.get((policy, address.to_string()), now, |(policy, _)| {
            window_length(*policy)
        });
        take(window, limit, length, now).then_some(Lease { _permit: None })
    }

    async fn acquire_upload(&self, now: Instant) -> Option<Lease> {
        let uploads = &self.uploads;
        // The queue is as long as four times what runs at once; beyond it a
        // request is refused rather than made to wait.
        let permit = match Arc::clone(&uploads.running).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                if uploads.waiting.fetch_add(1, Ordering::SeqCst) >= uploads.queue_limit {
                    uploads.waiting.fetch_sub(1, Ordering::SeqCst);
                    return None;
                }
                let waited = Arc::clone(&uploads.running).acquire_owned().await;
                uploads.waiting.fetch_sub(1, Ordering::SeqCst);
                waited.ok()?
            }
        };
        let mut window = uploads
            .per_minute
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Past the window the permit goes back with the lease it was part of.
        take(&mut window, self.limits.uploads_per_minute, MINUTE, now).then_some(Lease {
            _permit: Some(permit),
        })
    }
}

fn window_length(policy: RateLimitPolicy) -> Duration {
    match policy {
        RateLimitPolicy::AuthWrite
        | RateLimitPolicy::SecurityWrite
        | RateLimitPolicy::PinUnlock => QUARTER_HOUR,
        _ => MINUTE,
    }
}

/// Counts one use in a window that renews itself every `length`.
fn take(window: &mut Window, limit: u32, length: Duration, now: Instant) -> bool {
    let elapsed = now.saturating_duration_since(window.started);
    if elapsed >= length {
        let whole = elapsed.as_nanos() / length.as_nanos();
        window.started += Duration::from_nanos((length.as_nanos() * whole) as u64);
        window.used = 0;
    }
    if window.used >= limit {
        return false;
    }
    window.used += 1;
    true
}

/// A window of attempts for each account, for routes that [`RateLimitPolicy`]
/// can only count per address.
pub struct AttemptLimiter {
    limit: u32,
    length: Duration,
    windows: Mutex<Windows<Id>>,
}

impl AttemptLimiter {
    pub fn new(limit: u32, length: Duration) -> AttemptLimiter {
        AttemptLimiter {
            limit,
            length,
            windows: Mutex::new(Windows::default()),
        }
    }

    /// The limiter for the encryption PIN: the same count and window as its
    /// address policy.
    pub fn for_pin_unlock(limits: &RateLimitSettings) -> AttemptLimiter {
        AttemptLimiter::new(limits.pin_unlock, PIN_UNLOCK_WINDOW)
    }

    /// Counts one attempt of `account`; false when it has used them all.
    pub fn try_acquire(&self, account: Id) -> bool {
        self.try_acquire_at(account, Instant::now())
    }

    fn try_acquire_at(&self, account: Id, now: Instant) -> bool {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let length = self.length;
        let window = windows.get(account, now, |_| length);
        take(window, self.limit, self.length, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> RateLimitSettings {
        RateLimitSettings {
            login: 3,
            login_poll: 120,
            auth_write: 5,
            security_write: 30,
            invite_preview: 30,
            uploads_per_minute: 2,
            pin_unlock: 5,
            concurrent_uploads: 2,
        }
    }

    #[tokio::test]
    async fn a_policy_lets_its_limit_through_per_address_and_no_more() {
        let limiter = RateLimiter::new(&limits());
        let now = Instant::now();

        for _ in 0..3 {
            assert!(
                limiter
                    .acquire_at(RateLimitPolicy::Login, "198.51.100.7", now)
                    .await
                    .is_some()
            );
        }
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Login, "198.51.100.7", now)
                .await
                .is_none()
        );
        // Another address has a count of its own, and another policy too.
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Login, "198.51.100.8", now)
                .await
                .is_some()
        );
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::LoginPoll, "198.51.100.7", now)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_count_starts_again_when_the_window_is_over() {
        let limiter = RateLimiter::new(&limits());
        let start = Instant::now();
        for _ in 0..3 {
            limiter
                .acquire_at(RateLimitPolicy::Login, "a", start)
                .await
                .unwrap();
        }

        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Login, "a", start + Duration::from_secs(59))
                .await
                .is_none()
        );
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Login, "a", start + Duration::from_secs(61))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_table_of_windows_is_swept_and_bounded() {
        let limiter = RateLimiter::new(&limits());
        let start = Instant::now();
        for index in 0..(MOST_WINDOWS as u32 * 2) {
            limiter
                .acquire_at(
                    RateLimitPolicy::Login,
                    &format!("10.{}.{}.{}", index >> 16, index >> 8 & 255, index & 255),
                    start,
                )
                .await;
        }
        let held = limiter.windows.lock().unwrap().table.len();
        assert!(held <= MOST_WINDOWS, "{held}");

        // Once they are all over, a sweep leaves only the one that is asked for.
        let later = start + Duration::from_secs(61);
        for _ in 0..SWEEP_EVERY {
            limiter
                .acquire_at(RateLimitPolicy::Login, "same", later)
                .await;
        }
        assert_eq!(limiter.windows.lock().unwrap().table.len(), 1);
    }

    #[tokio::test]
    async fn windows_follow_one_another_without_a_gap() {
        let limiter = RateLimiter::new(&limits());
        let start = Instant::now();
        // The first request of the partition opens the window at second 0.
        limiter
            .acquire_at(RateLimitPolicy::Login, "a", start)
            .await
            .unwrap();
        // A request at second 130 is in the third window, which began at 120,
        // so the third window ends at 180 and not at 190.
        for _ in 0..3 {
            limiter
                .acquire_at(
                    RateLimitPolicy::Login,
                    "a",
                    start + Duration::from_secs(130),
                )
                .await
                .unwrap();
        }
        assert!(
            limiter
                .acquire_at(
                    RateLimitPolicy::Login,
                    "a",
                    start + Duration::from_secs(179)
                )
                .await
                .is_none()
        );
        assert!(
            limiter
                .acquire_at(
                    RateLimitPolicy::Login,
                    "a",
                    start + Duration::from_secs(181)
                )
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn uploads_share_one_budget_and_hold_a_place_while_they_run() {
        let limiter = RateLimiter::new(&limits());
        let now = Instant::now();

        let first = limiter
            .acquire_at(RateLimitPolicy::Uploads, "a", now)
            .await
            .unwrap();
        let second = limiter
            .acquire_at(RateLimitPolicy::Uploads, "b", now)
            .await
            .unwrap();
        // The per-minute budget of two is spent, whoever asks.
        drop(first);
        drop(second);
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Uploads, "c", now)
                .await
                .is_none()
        );
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Uploads, "c", now + Duration::from_secs(61))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn uploads_beyond_those_running_wait_in_a_queue_of_four_times_as_many() {
        let mut wide = limits();
        wide.uploads_per_minute = 1000;
        let limiter = Arc::new(RateLimiter::new(&wide));
        let now = Instant::now();
        let running = [
            limiter
                .acquire_at(RateLimitPolicy::Uploads, "a", now)
                .await
                .unwrap(),
            limiter
                .acquire_at(RateLimitPolicy::Uploads, "a", now)
                .await
                .unwrap(),
        ];

        // Eight more may wait, the ninth is refused.
        let waiting: Vec<_> = (0..8)
            .map(|_| {
                let limiter = Arc::clone(&limiter);
                tokio::spawn(async move {
                    limiter
                        .acquire_at(RateLimitPolicy::Uploads, "a", Instant::now())
                        .await
                        .is_some()
                })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(waiting.iter().all(|task| !task.is_finished()));
        assert!(
            limiter
                .acquire_at(RateLimitPolicy::Uploads, "a", now)
                .await
                .is_none()
        );

        drop(running);
        for task in waiting {
            // Each one that gets a place keeps it only until it is dropped.
            assert!(task.await.unwrap());
        }
    }

    #[test]
    fn an_account_has_attempts_of_its_own() {
        let limiter = AttemptLimiter::new(2, Duration::from_secs(900));
        let (maya, jo) = (Id::random(), Id::random());
        let now = Instant::now();

        assert!(limiter.try_acquire_at(maya, now));
        assert!(limiter.try_acquire_at(maya, now));
        assert!(!limiter.try_acquire_at(maya, now));
        assert!(limiter.try_acquire_at(jo, now));
        assert!(limiter.try_acquire_at(maya, now + Duration::from_secs(901)));
    }
}
