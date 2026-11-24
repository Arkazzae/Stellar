//! How far a running backup has got, for the panel to show while it runs.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use super::parts;

pub struct Progress {
    phase: Mutex<&'static str>,
    total: AtomicU32,
    done: AtomicU32,
}

impl Progress {
    pub fn new(objects_total: usize) -> Progress {
        Progress {
            phase: Mutex::new(parts::DATABASE),
            total: AtomicU32::new(u32::try_from(objects_total).unwrap_or(u32::MAX)),
            done: AtomicU32::new(0),
        }
    }

    pub fn set_phase(&self, phase: &'static str) {
        *self.phase.lock().unwrap_or_else(|p| p.into_inner()) = phase;
    }

    pub fn phase(&self) -> &'static str {
        *self.phase.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn object_done(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn objects_done(&self) -> u32 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn objects_total(&self) -> u32 {
        self.total.load(Ordering::Relaxed)
    }

    /// Takes the total of the listing, once it is known.
    pub fn set_total(&self, total: usize) {
        self.total
            .store(u32::try_from(total).unwrap_or(u32::MAX), Ordering::Relaxed);
    }
}
