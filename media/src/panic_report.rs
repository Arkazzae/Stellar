//! What a panic writes to the log.
//!
//! The default hook prints every panic to stderr, caught ones included, and
//! the core catches the panics of one session or one datagram so that the
//! process lives on. Whoever can make a panic happen once per datagram could
//! then fill the pipe to the process manager without bound. A panic inside
//! the core's `guard` is caught, so those are reported in full a few times a
//! minute and only counted past that. A panic outside every guard is always
//! reported in full, since in the media task it ends the process, and also
//! goes to the hook that was there before, which prints the backtrace that
//! `RUST_BACKTRACE` asks for.

use std::cell::Cell;
use std::panic::PanicHookInfo;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tracing::{error, warn};

/// How many caught panics are reported in full in one window.
const REPORTS_PER_WINDOW: u32 = 5;
const WINDOW: Duration = Duration::from_secs(60);
/// The longest message a report carries, in bytes.
const MAX_MESSAGE: usize = 512;

thread_local! {
    /// How many guards the current thread is inside of.
    static GUARDS: Cell<u32> = const { Cell::new(0) };
}

/// Marks the thread as inside a guard until it is dropped, unwinding
/// included.
pub(crate) struct Guarded(());

pub(crate) fn enter() -> Guarded {
    GUARDS.set(GUARDS.get() + 1);
    Guarded(())
}

impl Drop for Guarded {
    fn drop(&mut self) {
        GUARDS.set(GUARDS.get() - 1);
    }
}

#[derive(Default)]
struct Limiter {
    window: Option<Instant>,
    reported: u32,
    quiet: u64,
}

impl Limiter {
    /// Whether a panic at `now` is reported in full: `Some` with how many
    /// caught panics were kept quiet before it, or `None` to count it.
    fn admit(&mut self, now: Instant, caught: bool) -> Option<u64> {
        if caught {
            let new_window = match self.window {
                Some(start) => now.saturating_duration_since(start) >= WINDOW,
                None => true,
            };
            if new_window {
                self.window = Some(now);
                self.reported = 0;
            }
            if self.reported == REPORTS_PER_WINDOW {
                self.quiet += 1;
                return None;
            }
            self.reported += 1;
        }
        Some(std::mem::take(&mut self.quiet))
    }
}

/// The text of a panic, cut to what a log line should carry.
fn message_of<'a>(info: &'a PanicHookInfo<'_>) -> &'a str {
    let payload = info.payload();
    let text = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message");
    let mut end = text.len().min(MAX_MESSAGE);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Puts a panic hook in front of the current one that logs through `tracing`
/// and keeps what caught panics write bounded.
pub fn install() {
    let limiter = Mutex::new(Limiter::default());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let caught = GUARDS.get() > 0;
        // Nothing under the lock can panic, so a poisoned one is still sound.
        let admitted = limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(Instant::now(), caught);
        let Some(quiet) = admitted else {
            return;
        };
        if quiet > 0 {
            warn!(
                count = quiet,
                "caught panics that were not reported one by one"
            );
        }
        let location = info.location().map(ToString::to_string);
        error!(
            caught,
            location = location.as_deref().unwrap_or("unknown"),
            "a panic: {}",
            message_of(info)
        );
        if !caught {
            previous(info);
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caught_panics_are_reported_a_few_times_a_minute_and_counted_past_that() {
        let start = Instant::now();
        let later = start + Duration::from_secs(30);
        let mut limiter = Limiter::default();
        for _ in 0..REPORTS_PER_WINDOW {
            assert_eq!(limiter.admit(start, true), Some(0));
        }
        for _ in 0..1000 {
            assert_eq!(limiter.admit(later, true), None);
        }
        // One that nothing catches is never held back, and says how many
        // were kept quiet before it.
        assert_eq!(limiter.admit(later, false), Some(1000));
        for _ in 0..3 {
            assert_eq!(limiter.admit(later, true), None);
        }

        // The next window reports again, the first time with the count.
        let next = start + WINDOW;
        assert_eq!(limiter.admit(next, true), Some(3));
        for _ in 1..REPORTS_PER_WINDOW {
            assert_eq!(limiter.admit(next, true), Some(0));
        }
        assert_eq!(limiter.admit(next, true), None);
    }
}
