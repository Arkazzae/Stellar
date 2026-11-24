//! How the process is doing: one sample of it every half minute, kept for the
//! last hour, and the figures the runtime page shows beside them.
//!
//! The samples live in memory and a restart starts the history afresh. The
//! process's own figures come from `/proc` where there is one; elsewhere they
//! read as zero, since nothing else may be asked without `unsafe`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::background::Worker;
use crate::state::AppState;
use crate::values::Timestamp;

/// How often the process is sampled.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// One hour of samples at [`INTERVAL`].
const CAPACITY: usize = 120;

/// A database or storage check that takes longer than this counts as down.
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// What the process looked like over one interval. Request counts are the
/// responses finished during it, not totals.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub at: Timestamp,
    pub working_set_bytes: i64,
    pub gc_heap_bytes: i64,
    pub threads: i64,
    #[serde(serialize_with = "super::plain_number")]
    pub cpu_percent: f64,
    pub realtime_connections: i64,
    pub requests: i64,
    pub client_errors: i64,
    pub server_errors: i64,
    pub database_up: bool,
    #[serde(serialize_with = "super::plain_number")]
    pub database_latency_ms: f64,
    pub storage_up: Option<bool>,
}

/// The figures of the process itself.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Process {
    /// Resident memory.
    pub working_set_bytes: i64,
    /// Memory the process has mapped for its own data: the nearest thing to a
    /// managed heap a native process has.
    pub heap_bytes: i64,
    pub threads: i64,
    /// Processor time used since the process started.
    pub cpu_seconds: f64,
}

impl Process {
    /// The figures now; zeros where the system does not say.
    pub fn now() -> Process {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
        Process::read(&status, &stat)
    }

    /// Reads the text of `/proc/self/status` and `/proc/self/stat`.
    pub fn read(status: &str, stat: &str) -> Process {
        let kilobytes = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                .and_then(|rest| rest.trim().strip_suffix("kB")?.trim().parse::<i64>().ok())
                .map_or(0, |kilobytes| kilobytes.saturating_mul(1024))
        };
        let threads = status
            .lines()
            .find_map(|line| line.strip_prefix("Threads:"))
            .and_then(|rest| rest.trim().parse().ok())
            .unwrap_or(0);
        // The command name in the second field may hold spaces and
        // parentheses, so the numbers are counted from its closing one:
        // utime and stime are the 14th and 15th fields of the line.
        let ticks = stat
            .rsplit_once(')')
            .map(|(_, rest)| rest.split_whitespace().collect::<Vec<_>>())
            .and_then(|fields| {
                let user: f64 = fields.get(11)?.parse().ok()?;
                let system: f64 = fields.get(12)?.parse().ok()?;
                Some(user + system)
            })
            .unwrap_or(0.0);
        Process {
            working_set_bytes: kilobytes("VmRSS"),
            heap_bytes: kilobytes("VmData"),
            threads,
            // The kernel counts in hundredths of a second on every system
            // this runs on.
            cpu_seconds: ticks / 100.0,
        }
    }
}

struct Taken {
    samples: VecDeque<Sample>,
    sampled_requests: u64,
    sampled_client_errors: u64,
    sampled_server_errors: u64,
    sampled_cpu: f64,
    sampled_at: Instant,
}

/// The samples kept, and what the next one is measured from.
pub struct History {
    started: Instant,
    started_at: Timestamp,
    taken: Mutex<Taken>,
}

impl Default for History {
    fn default() -> History {
        let started = Instant::now();
        History {
            started,
            started_at: Timestamp::now(),
            taken: Mutex::new(Taken {
                samples: VecDeque::new(),
                sampled_requests: 0,
                sampled_client_errors: 0,
                sampled_server_errors: 0,
                sampled_cpu: 0.0,
                sampled_at: started,
            }),
        }
    }
}

/// What a sample is made of besides the process's own figures.
pub struct Observed {
    pub realtime_connections: i64,
    pub requests: u64,
    pub client_errors: u64,
    pub server_errors: u64,
    pub database_up: bool,
    pub database_latency_ms: f64,
    pub storage_up: Option<bool>,
}

impl History {
    /// When the process started, as far as this server can tell: when its
    /// state was built.
    pub fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub fn uptime_seconds(&self) -> i64 {
        i64::try_from(self.started.elapsed().as_secs()).unwrap_or(i64::MAX)
    }

    /// Takes a sample and keeps it, dropping the oldest once an hour is kept.
    pub fn take(&self, process: Process, observed: Observed, processors: usize) -> Sample {
        let mut taken = self.taken.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let wall = taken.sampled_at.elapsed().as_secs_f64() * processors.max(1) as f64;
        let used = process.cpu_seconds - taken.sampled_cpu;
        let cpu = if wall > 0.0 { (used / wall * 100.0).clamp(0.0, 100.0) } else { 0.0 };
        let count = |now: u64, before: u64| i64::try_from(now.saturating_sub(before)).unwrap_or(i64::MAX);
        let sample = Sample {
            at: Timestamp::now(),
            working_set_bytes: process.working_set_bytes,
            gc_heap_bytes: process.heap_bytes,
            threads: process.threads,
            cpu_percent: (cpu * 10.0).round() / 10.0,
            realtime_connections: observed.realtime_connections,
            requests: count(observed.requests, taken.sampled_requests),
            client_errors: count(observed.client_errors, taken.sampled_client_errors),
            server_errors: count(observed.server_errors, taken.sampled_server_errors),
            database_up: observed.database_up,
            database_latency_ms: (observed.database_latency_ms * 100.0).round() / 100.0,
            storage_up: observed.storage_up,
        };
        taken.sampled_at = Instant::now();
        taken.sampled_cpu = process.cpu_seconds;
        taken.sampled_requests = observed.requests;
        taken.sampled_client_errors = observed.client_errors;
        taken.sampled_server_errors = observed.server_errors;
        taken.samples.push_back(sample.clone());
        while taken.samples.len() > CAPACITY {
            taken.samples.pop_front();
        }
        sample
    }

    /// The kept samples, oldest first.
    pub fn samples(&self) -> Vec<Sample> {
        let taken = self.taken.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        taken.samples.iter().cloned().collect()
    }
}

/// Asks the database whether it answers, and how long that took.
pub async fn check_database(state: &AppState) -> (bool, f64) {
    let started = Instant::now();
    let asking = state.database().read(|db| -> Result<(), crate::database::DbError> {
        db.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
        Ok(())
    });
    let up = matches!(tokio::time::timeout(CHECK_TIMEOUT, asking).await, Ok(Ok(())));
    (up, started.elapsed().as_secs_f64() * 1000.0)
}

/// Asks the object store whether it answers; none where it is not used.
pub async fn check_storage(state: &AppState) -> Option<bool> {
    if state.settings().storage.skip_initialization {
        return None;
    }
    Some(matches!(
        tokio::time::timeout(CHECK_TIMEOUT, state.storage().probe()).await,
        Ok(Ok(()))
    ))
}

/// How many connections the hub holds open.
pub fn realtime_connections(state: &AppState) -> i64 {
    state
        .realtime()
        .map_or(0, |hub| i64::try_from(hub.registry().count()).unwrap_or(i64::MAX))
}

async fn sample(state: &AppState) {
    let (database_up, database_latency_ms) = check_database(state).await;
    let storage_up = check_storage(state).await;
    let metrics = state.metrics();
    let processors = std::thread::available_parallelism().map_or(1, usize::from);
    state.cockpit().history().take(
        Process::now(),
        Observed {
            realtime_connections: realtime_connections(state),
            requests: metrics.requests(),
            client_errors: metrics.client_errors(),
            server_errors: metrics.server_errors(),
            database_up,
            database_latency_ms,
            storage_up,
        },
        processors,
    );
}

/// Samples the process at start and every [`INTERVAL`] after.
pub fn workers() -> Vec<Worker> {
    vec![Worker::new("cockpit-runtime", |state, stop| async move {
        sample(&state).await;
        while stop.sleep(INTERVAL).await {
            sample(&state).await;
        }
    })]
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\tstellarr-api\nVmData:\t  204800 kB\nVmRSS:\t   51200 kB\nThreads:\t17\n";

    #[test]
    fn the_figures_are_read_from_the_systems_text() {
        let stat = "123 (stellarr (api) x) S 1 1 1 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 17 0 1 2 3";

        let process = Process::read(STATUS, stat);

        assert_eq!(process.working_set_bytes, 51_200 * 1024);
        assert_eq!(process.heap_bytes, 204_800 * 1024);
        assert_eq!(process.threads, 17);
        assert!((process.cpu_seconds - 3.0).abs() < 1e-9);
        assert_eq!(Process::read("", ""), Process::default());
        assert_eq!(Process::read("VmRSS: lots kB\nThreads: many", "no parenthesis"), Process::default());
    }

    fn observed(requests: u64) -> Observed {
        Observed {
            realtime_connections: 2,
            requests,
            client_errors: 1,
            server_errors: 0,
            database_up: true,
            database_latency_ms: 1.234_5,
            storage_up: None,
        }
    }

    #[test]
    fn a_sample_counts_what_happened_since_the_last_one() {
        let history = History::default();

        let first = history.take(Process::default(), observed(10), 1);
        let second = history.take(Process::default(), observed(15), 1);

        assert_eq!((first.requests, second.requests), (10, 5));
        assert_eq!((first.client_errors, second.client_errors), (1, 0));
        assert_eq!(second.database_latency_ms, 1.23);
        assert_eq!(history.samples().len(), 2);
    }

    #[test]
    fn only_an_hour_of_samples_is_kept() {
        let history = History::default();
        for number in 0..CAPACITY as u64 + 7 {
            history.take(Process::default(), observed(number), 1);
        }

        let kept = history.samples();

        assert_eq!(kept.len(), CAPACITY);
        assert_eq!(kept[0].requests, 1);
    }
}
