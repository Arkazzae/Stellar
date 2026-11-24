//! A cooldown per person and scope that every hub connection shares. Whoever
//! asks first inside the window gets the slot, also when several connections
//! of the same account ask at once.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::values::Id;

/// Expired entries are swept once in this many calls, so the table stays as
/// small as the people who played recently.
const SWEEP_INTERVAL: u64 = 256;

#[derive(Default)]
struct Table {
    ready_at: HashMap<(Id, Id), Instant>,
    calls: u64,
}

/// The cooldowns of the soundboard and of arrival sounds.
#[derive(Default)]
pub struct PlaybackGate {
    table: Mutex<Table>,
}

impl PlaybackGate {
    /// Takes the slot of `user` in `scope` if it is free, and holds it for
    /// `cooldown`.
    pub fn try_acquire(&self, user: Id, scope: Id, cooldown: Duration) -> bool {
        self.try_acquire_at(user, scope, cooldown, Instant::now())
    }

    fn try_acquire_at(&self, user: Id, scope: Id, cooldown: Duration, now: Instant) -> bool {
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        table.calls += 1;
        if table.calls.is_multiple_of(SWEEP_INTERVAL) {
            table.ready_at.retain(|_, ready| *ready > now);
        }
        match table.ready_at.get(&(user, scope)) {
            Some(ready) if *ready > now => false,
            _ => {
                table.ready_at.insert((user, scope), now + cooldown);
                true
            }
        }
    }

    #[cfg(test)]
    fn size(&self) -> usize {
        self.table.lock().unwrap().ready_at.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(5);

    #[test]
    fn the_first_to_ask_inside_the_window_gets_the_slot() {
        let gate = PlaybackGate::default();
        let (maja, ada, server) = (Id::random(), Id::random(), Id::random());
        let now = Instant::now();

        assert!(gate.try_acquire_at(maja, server, WINDOW, now));
        assert!(!gate.try_acquire_at(maja, server, WINDOW, now + Duration::from_secs(4)));
        assert!(gate.try_acquire_at(ada, server, WINDOW, now));
        assert!(gate.try_acquire_at(maja, Id::random(), WINDOW, now));
        assert!(gate.try_acquire_at(maja, server, WINDOW, now + WINDOW));
    }

    #[test]
    fn the_slot_goes_to_exactly_one_of_many_who_ask_at_once() {
        let gate = PlaybackGate::default();
        let (maja, server) = (Id::random(), Id::random());

        let accepted = std::thread::scope(|scope| {
            let askers: Vec<_> = (0..32)
                .map(|_| scope.spawn(|| gate.try_acquire(maja, server, Duration::from_secs(60))))
                .collect();
            askers
                .into_iter()
                .map(|asker| asker.join().unwrap())
                .filter(|accepted| *accepted)
                .count()
        });

        assert_eq!(accepted, 1);
    }

    #[test]
    fn a_refusal_does_not_extend_the_window() {
        let gate = PlaybackGate::default();
        let (maja, server) = (Id::random(), Id::random());
        let now = Instant::now();

        assert!(gate.try_acquire_at(maja, server, WINDOW, now));
        assert!(!gate.try_acquire_at(maja, server, WINDOW, now + Duration::from_secs(3)));

        assert!(gate.try_acquire_at(maja, server, WINDOW, now + Duration::from_secs(5)));
    }

    #[test]
    fn expired_entries_are_swept() {
        let gate = PlaybackGate::default();
        let now = Instant::now();
        for _ in 0..255 {
            gate.try_acquire_at(Id::random(), Id::random(), WINDOW, now);
        }
        assert_eq!(gate.size(), 255);

        gate.try_acquire_at(Id::random(), Id::random(), WINDOW, now + WINDOW * 2);

        assert_eq!(gate.size(), 1);
    }
}
