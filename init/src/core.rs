//! Every decision of the manager, free of processes, sockets and clocks.
//!
//! [`Supervisor`] holds the state of each service and decides what happens
//! next: what to start and when, what to probe, what to signal, when to stop
//! waiting. It never reads the clock, as every call takes `now`, and it never
//! touches a process: it asks an [`Io`] to. The manager drives it with real
//! processes; the tests drive it with a fake clock and an `Io` that records.
//!
//! A service moves through these states:
//!
//! - `waiting`: not running, and something it needs is not ready yet.
//! - `starting`: running, and not yet ready.
//! - `ready`: running and ready. It stays ready until the process exits.
//! - `backoff`: exited, and waiting to try again.
//! - `stopping`: asked to stop, by a restart or by the shutdown.
//! - `stopped`: stopped by the shutdown, for good.

use std::fmt;
use std::time::{Duration, Instant};

use crate::service::{Readiness, Service, Signal, Table, signal_name};

/// How long after a failed readiness check the next one starts.
pub const PROBE_INTERVAL: Duration = Duration::from_millis(500);

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exit::Code(code) => write!(f, "exited with status {code}"),
            Exit::Signal(signal) => write!(f, "killed by {}", signal_name(*signal)),
        }
    }
}

/// What the supervisor asks of the world.
pub trait Io {
    /// Starts the service's process in a new process group and returns its
    /// PID, which is also the group's ID.
    fn spawn(&mut self, service: &Service) -> Result<u32, String>;
    /// Sends `signal` to the process group `pid` leads, or to `pid` alone.
    fn signal(&mut self, pid: u32, signal: Signal, group: bool);
    /// Starts a readiness check of run `run` of service `index`. Its answer
    /// comes back through [`Supervisor::probed`], however it ends.
    fn probe(&mut self, index: usize, run: u64, check: &Readiness);
    /// One line of the manager's log.
    fn log(&mut self, line: String);
}

/// The state of a service as the control socket reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Waiting,
    Starting,
    Ready,
    Backoff,
    Stopping,
    Stopped,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Starting => "starting",
            State::Ready => "ready",
            State::Backoff => "backoff",
            State::Stopping => "stopping",
            State::Stopped => "stopped",
        }
    }

    pub fn parse(name: &str) -> Option<State> {
        [
            State::Waiting,
            State::Starting,
            State::Ready,
            State::Backoff,
            State::Stopping,
            State::Stopped,
        ]
        .into_iter()
        .find(|s| s.name() == name)
    }
}

/// One service, as `status` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub name: String,
    pub state: State,
    pub pid: Option<u32>,
    /// How many times the service was started again after its first start,
    /// whether it had exited or was restarted on request.
    pub restarts: u32,
    /// When the running process started.
    pub up_since: Option<Instant>,
    /// When the next try is due, while the service backs off.
    pub next_try: Option<Instant>,
    pub critical: bool,
}

/// Where the manager as a whole is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    /// Stopping every service, after SIGTERM or SIGINT.
    ShuttingDown,
    /// Everything is stopped; the manager exits with [`Supervisor::exit_code`].
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterStop {
    /// Start it again at once: a restart on request.
    Start,
    /// Leave it stopped: the shutdown.
    Stay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Waiting,
    Starting {
        pid: u32,
        since: Instant,
        /// When the next readiness check is due; `None` while one runs.
        probe_at: Option<Instant>,
    },
    Ready {
        pid: u32,
        since: Instant,
    },
    Backoff {
        until: Instant,
    },
    Stopping {
        pid: u32,
        since: Instant,
        asked: Instant,
        kill_at: Instant,
        killed: bool,
        then: AfterStop,
    },
    Stopped,
}

impl Run {
    fn pid(&self) -> Option<u32> {
        match *self {
            Run::Starting { pid, .. } | Run::Ready { pid, .. } | Run::Stopping { pid, .. } => {
                Some(pid)
            }
            _ => None,
        }
    }

    fn state(&self) -> State {
        match self {
            Run::Waiting => State::Waiting,
            Run::Starting { .. } => State::Starting,
            Run::Ready { .. } => State::Ready,
            Run::Backoff { .. } => State::Backoff,
            Run::Stopping { .. } => State::Stopping,
            Run::Stopped => State::Stopped,
        }
    }
}

#[derive(Debug, Clone)]
struct Slot {
    state: Run,
    /// Counts the spawns, so a readiness answer for an earlier run is known
    /// as such.
    run: u64,
    restarts: u32,
    /// Short runs in a row; they set how long the next wait is.
    failures: u32,
    /// Whether "waiting for ..." was logged for the current wait.
    told_waiting: bool,
}

impl Slot {
    /// Counts a run that ended after `uptime` and returns the wait before
    /// the next one.
    fn fail(&mut self, uptime: Duration, service: &Service) -> Duration {
        if uptime >= service.backoff.reset_after {
            self.failures = 0;
        }
        self.failures = self.failures.saturating_add(1);
        service.backoff.delay(self.failures)
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Running,
    ShuttingDown { deadline: Instant },
    Done,
}

/// The state of every service and the rules that move it.
pub struct Supervisor {
    table: Table,
    slots: Vec<Slot>,
    mode: Mode,
    shutdown_timeout: Duration,
}

impl Supervisor {
    /// Every service starts out waiting; the first [`tick`](Self::tick)
    /// starts those that need nothing. A shutdown stops each service in the
    /// reverse of the start order and ends `shutdown_timeout` after it began
    /// at the latest.
    pub fn new(table: Table, shutdown_timeout: Duration) -> Supervisor {
        let slots = table
            .services()
            .iter()
            .map(|_| Slot {
                state: Run::Waiting,
                run: 0,
                restarts: 0,
                failures: 0,
                told_waiting: false,
            })
            .collect();
        Supervisor {
            table,
            slots,
            mode: Mode::Running,
            shutdown_timeout,
        }
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn phase(&self) -> Phase {
        match self.mode {
            Mode::Running => Phase::Running,
            Mode::ShuttingDown { .. } => Phase::ShuttingDown,
            Mode::Done => Phase::Done,
        }
    }

    /// The manager's exit status, once everything is stopped. A shutdown
    /// that was asked for ends with 0, even when it had to kill.
    pub fn exit_code(&self) -> Option<i32> {
        match self.mode {
            Mode::Done => Some(0),
            _ => None,
        }
    }

    /// Acts on whatever is due at `now`.
    pub fn tick(&mut self, now: Instant, io: &mut impl Io) {
        self.advance(now, io);
    }

    /// The earliest time something is due, if anything is.
    pub fn next_deadline(&self) -> Option<Instant> {
        let running = matches!(self.mode, Mode::Running);
        let mut next: Option<Instant> = None;
        let mut consider = |at: Instant| {
            next = Some(next.map_or(at, |n| n.min(at)));
        };
        for (slot, service) in self.slots.iter().zip(self.table.services()) {
            match slot.state {
                Run::Starting {
                    since, probe_at, ..
                } => match service.ready {
                    Readiness::Running(after) => consider(since + after),
                    _ => {
                        if let (true, Some(at)) = (running, probe_at) {
                            consider(at);
                        }
                    }
                },
                Run::Stopping {
                    kill_at,
                    killed: false,
                    ..
                } => consider(kill_at),
                Run::Backoff { until } if running => consider(until),
                _ => {}
            }
        }
        if let Mode::ShuttingDown { deadline } = self.mode {
            consider(deadline);
        }
        next
    }

    /// A process ended. Returns whether it was a service's; an orphan that
    /// was reaped is not.
    pub fn exited(&mut self, pid: u32, exit: Exit, now: Instant, io: &mut impl Io) -> bool {
        let Some(i) = self.slots.iter().position(|s| s.state.pid() == Some(pid)) else {
            return false;
        };
        // Whatever the process left in its group goes with it, so that a
        // new run never finds the old one's port still held.
        io.signal(pid, Signal::Kill, true);

        let running = matches!(self.mode, Mode::Running);
        let service = &self.table.services()[i];
        let name = &service.name;
        let slot = &mut self.slots[i];
        match slot.state {
            Run::Starting { since, .. } | Run::Ready { since, .. } => {
                let uptime = now.saturating_duration_since(since);
                if running {
                    let delay = slot.fail(uptime, service);
                    slot.state = Run::Backoff { until: now + delay };
                    io.log(format!(
                        "{name}: {exit} after {}; next try in {}",
                        secs(uptime),
                        secs(delay)
                    ));
                } else {
                    slot.state = Run::Stopped;
                    io.log(format!("{name}: {exit} after {}; stopped", secs(uptime)));
                }
            }
            Run::Stopping { asked, then, .. } => {
                let took = now.saturating_duration_since(asked);
                if running && then == AfterStop::Start {
                    slot.failures = 0;
                    slot.state = Run::Waiting;
                    slot.told_waiting = false;
                    io.log(format!(
                        "{name}: {exit} {} after it was asked to stop; starting again",
                        secs(took)
                    ));
                } else {
                    slot.state = Run::Stopped;
                    io.log(format!(
                        "{name}: {exit} {} after it was asked to stop; stopped",
                        secs(took)
                    ));
                }
            }
            Run::Waiting | Run::Backoff { .. } | Run::Stopped => {
                unreachable!("a slot without a process matched a PID")
            }
        }
        self.advance(now, io);
        true
    }

    /// The answer to a readiness check started through [`Io::probe`].
    pub fn probed(&mut self, index: usize, run: u64, ok: bool, now: Instant, io: &mut impl Io) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        if slot.run != run {
            return;
        }
        let Run::Starting {
            pid,
            since,
            probe_at: None,
        } = slot.state
        else {
            return;
        };
        if ok {
            slot.state = Run::Ready { pid, since };
            io.log(format!(
                "{}: ready after {}",
                self.table.services()[index].name,
                secs(now.saturating_duration_since(since))
            ));
        } else {
            slot.state = Run::Starting {
                pid,
                since,
                probe_at: Some(now + PROBE_INTERVAL),
            };
        }
        self.advance(now, io);
    }

    /// Begins the shutdown: nothing starts any more, and every service is
    /// stopped in the reverse of the start order. `reason` names the signal.
    pub fn shutdown(&mut self, reason: &str, now: Instant, io: &mut impl Io) {
        if !matches!(self.mode, Mode::Running) {
            io.log(format!("{reason} while shutting down; carrying on"));
            return;
        }
        let deadline = now + self.shutdown_timeout;
        self.mode = Mode::ShuttingDown { deadline };
        io.log(format!(
            "shutting down on {reason}; everything is stopped within {}",
            secs(self.shutdown_timeout)
        ));
        for (slot, service) in self.slots.iter_mut().zip(self.table.services()) {
            match &mut slot.state {
                Run::Waiting | Run::Backoff { .. } => {
                    slot.state = Run::Stopped;
                    io.log(format!("{}: stopped", service.name));
                }
                Run::Stopping { kill_at, then, .. } => {
                    *then = AfterStop::Stay;
                    *kill_at = (*kill_at).min(deadline);
                }
                _ => {}
            }
        }
        self.advance(now, io);
    }

    /// Stops the service and starts it again at once, or starts it at once
    /// if it is waiting to try again. Returns what happens, or why not.
    pub fn restart(
        &mut self,
        name: &str,
        now: Instant,
        io: &mut impl Io,
    ) -> Result<String, String> {
        let i = self.find(name)?;
        let service = &self.table.services()[i];
        let slot = &mut self.slots[i];
        match &mut slot.state {
            Run::Waiting => {
                let missing = self
                    .table
                    .needs(i)
                    .iter()
                    .filter(|&&j| !matches!(self.slots[j].state, Run::Ready { .. }))
                    .map(|&j| self.table.services()[j].name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!("{name} has not started: it waits for {missing}"));
            }
            Run::Backoff { .. } => {
                slot.failures = 0;
                slot.state = Run::Waiting;
                slot.told_waiting = false;
                io.log(format!("{name}: trying again now, on request"));
            }
            Run::Starting { pid, since, .. } | Run::Ready { pid, since } => {
                let (pid, since) = (*pid, *since);
                io.signal(pid, service.stop_signal, true);
                slot.failures = 0;
                slot.state = Run::Stopping {
                    pid,
                    since,
                    asked: now,
                    kill_at: now + service.stop_timeout,
                    killed: false,
                    then: AfterStop::Start,
                };
                io.log(format!(
                    "{name}: restarting on request; {} sent, SIGKILL in {}",
                    service.stop_signal.name(),
                    secs(service.stop_timeout)
                ));
            }
            Run::Stopping { then, .. } => {
                *then = AfterStop::Start;
                slot.failures = 0;
            }
            Run::Stopped => return Err(format!("{name} is stopped")),
        }
        self.advance(now, io);
        Ok(format!("restarting {name}"))
    }

    /// Sends the service's reload signal to its main process.
    pub fn reload(&mut self, name: &str, io: &mut impl Io) -> Result<String, String> {
        let i = self.find(name)?;
        let service = &self.table.services()[i];
        let Some(signal) = service.reload else {
            return Err(format!("{name} cannot reload; restart it instead"));
        };
        // A process that is not ready may not handle the signal yet, and the
        // default action of SIGHUP is to end it.
        match self.slots[i].state {
            Run::Ready { pid, .. } => {
                io.signal(pid, signal, false);
                io.log(format!("{name}: reloading with {}", signal.name()));
                Ok(format!("reloading {name}"))
            }
            other => Err(format!(
                "{name} is {}, not ready; there is nothing to reload",
                other.state().name()
            )),
        }
    }

    /// Ok while the manager runs and every critical service is ready;
    /// otherwise, what is wrong.
    pub fn health(&self) -> Result<(), String> {
        if !matches!(self.mode, Mode::Running) {
            return Err("the manager is shutting down".into());
        }
        let failing: Vec<String> = self
            .slots
            .iter()
            .zip(self.table.services())
            .filter(|(slot, service)| service.critical && !matches!(slot.state, Run::Ready { .. }))
            .map(|(slot, service)| format!("{} is {}", service.name, slot.state.state().name()))
            .collect();
        if failing.is_empty() {
            Ok(())
        } else {
            Err(failing.join(", "))
        }
    }

    /// Every service, in table order.
    pub fn status(&self) -> Vec<Status> {
        self.slots
            .iter()
            .zip(self.table.services())
            .map(|(slot, service)| {
                let (up_since, next_try) = match slot.state {
                    Run::Starting { since, .. }
                    | Run::Ready { since, .. }
                    | Run::Stopping { since, .. } => (Some(since), None),
                    Run::Backoff { until } => (None, Some(until)),
                    Run::Waiting | Run::Stopped => (None, None),
                };
                Status {
                    name: service.name.clone(),
                    state: slot.state.state(),
                    pid: slot.state.pid(),
                    restarts: slot.restarts,
                    up_since,
                    next_try,
                    critical: service.critical,
                }
            })
            .collect()
    }

    fn find(&self, name: &str) -> Result<usize, String> {
        let i = self
            .table
            .index_of(name)
            .ok_or_else(|| format!("there is no service named {name}"))?;
        if !matches!(self.mode, Mode::Running) {
            return Err("the manager is shutting down".into());
        }
        Ok(i)
    }

    /// Applies every rule until nothing changes: timers that are due, starts
    /// that have become possible, and the next step of a shutdown.
    fn advance(&mut self, now: Instant, io: &mut impl Io) {
        loop {
            let before: Vec<(Run, u64)> = self.slots.iter().map(|s| (s.state, s.run)).collect();
            self.timers(now, io);
            match self.mode {
                Mode::Running => self.start_waiting(now, io),
                Mode::ShuttingDown { deadline } => self.stop_next(now, deadline, io),
                Mode::Done => {}
            }
            let after = self.slots.iter().map(|s| (s.state, s.run));
            if before.into_iter().eq(after) {
                return;
            }
        }
    }

    fn timers(&mut self, now: Instant, io: &mut impl Io) {
        let running = matches!(self.mode, Mode::Running);
        for (i, (slot, service)) in self.slots.iter_mut().zip(self.table.services()).enumerate() {
            match slot.state {
                Run::Starting {
                    pid,
                    since,
                    probe_at,
                } => match &service.ready {
                    Readiness::Running(after) => {
                        if now >= since + *after {
                            slot.state = Run::Ready { pid, since };
                            io.log(format!(
                                "{}: ready, running for {}",
                                service.name,
                                secs(now.saturating_duration_since(since))
                            ));
                        }
                    }
                    check => {
                        if let (true, Some(at)) = (running, probe_at)
                            && now >= at
                        {
                            slot.state = Run::Starting {
                                pid,
                                since,
                                probe_at: None,
                            };
                            io.probe(i, slot.run, check);
                        }
                    }
                },
                Run::Stopping {
                    pid,
                    since,
                    asked,
                    kill_at,
                    killed: false,
                    then,
                } if now >= kill_at => {
                    io.signal(pid, Signal::Kill, true);
                    slot.state = Run::Stopping {
                        pid,
                        since,
                        asked,
                        kill_at,
                        killed: true,
                        then,
                    };
                    io.log(format!(
                        "{}: still running {} after {}; SIGKILL sent",
                        service.name,
                        secs(now.saturating_duration_since(asked)),
                        service.stop_signal.name()
                    ));
                }
                Run::Backoff { until } if running && now >= until => {
                    slot.state = Run::Waiting;
                    slot.told_waiting = false;
                }
                _ => {}
            }
        }
    }

    fn start_waiting(&mut self, now: Instant, io: &mut impl Io) {
        for k in 0..self.table.order().len() {
            let i = self.table.order()[k];
            if !matches!(self.slots[i].state, Run::Waiting) {
                continue;
            }
            let missing: Vec<usize> = self
                .table
                .needs(i)
                .iter()
                .copied()
                .filter(|&j| !matches!(self.slots[j].state, Run::Ready { .. }))
                .collect();
            if missing.is_empty() {
                self.spawn(i, now, io);
            } else if !self.slots[i].told_waiting {
                self.slots[i].told_waiting = true;
                let names: Vec<&str> = missing
                    .iter()
                    .map(|&j| self.table.services()[j].name.as_str())
                    .collect();
                io.log(format!(
                    "{}: waiting for {}",
                    self.table.services()[i].name,
                    names.join(", ")
                ));
            }
        }
    }

    fn spawn(&mut self, i: usize, now: Instant, io: &mut impl Io) {
        assert!(
            matches!(self.mode, Mode::Running),
            "nothing starts once the shutdown has begun"
        );
        let service = &self.table.services()[i];
        let slot = &mut self.slots[i];
        if slot.run > 0 {
            slot.restarts = slot.restarts.saturating_add(1);
        }
        slot.run += 1;
        match io.spawn(service) {
            Ok(pid) => {
                slot.state = Run::Starting {
                    pid,
                    since: now,
                    probe_at: Some(now),
                };
                io.log(format!("{}: started, pid {pid}", service.name));
            }
            Err(error) => {
                let delay = slot.fail(Duration::ZERO, service);
                slot.state = Run::Backoff { until: now + delay };
                io.log(format!(
                    "{}: cannot start: {error}; next try in {}",
                    service.name,
                    secs(delay)
                ));
            }
        }
    }

    fn stop_next(&mut self, now: Instant, deadline: Instant, io: &mut impl Io) {
        if now >= deadline {
            let mut killed = Vec::new();
            for (slot, service) in self.slots.iter_mut().zip(self.table.services()) {
                if let Some(pid) = slot.state.pid() {
                    io.signal(pid, Signal::Kill, true);
                    killed.push(service.name.as_str());
                }
                slot.state = Run::Stopped;
            }
            io.log(format!(
                "the shutdown took {}; SIGKILL sent to {}; exiting",
                secs(self.shutdown_timeout),
                killed.join(", ")
            ));
            self.mode = Mode::Done;
            return;
        }
        for k in (0..self.table.order().len()).rev() {
            let i = self.table.order()[k];
            let service = &self.table.services()[i];
            let slot = &mut self.slots[i];
            match slot.state {
                Run::Stopped => {}
                Run::Waiting | Run::Backoff { .. } => {
                    slot.state = Run::Stopped;
                    io.log(format!("{}: stopped", service.name));
                }
                Run::Starting { pid, since, .. } | Run::Ready { pid, since } => {
                    io.signal(pid, service.stop_signal, true);
                    let kill_at = (now + service.stop_timeout).min(deadline);
                    slot.state = Run::Stopping {
                        pid,
                        since,
                        asked: now,
                        kill_at,
                        killed: false,
                        then: AfterStop::Stay,
                    };
                    io.log(format!(
                        "{}: stopping; {} sent, SIGKILL in {}",
                        service.name,
                        service.stop_signal.name(),
                        secs(kill_at.saturating_duration_since(now))
                    ));
                    return;
                }
                Run::Stopping { .. } => return,
            }
        }
        io.log("every service is stopped; exiting".into());
        self.mode = Mode::Done;
    }
}

/// A duration for the log: tenths of a second below ten seconds, whole
/// seconds above.
pub fn secs(d: Duration) -> String {
    if d < Duration::from_secs(10) {
        let tenths = (d.as_millis() + 50) / 100;
        if tenths.is_multiple_of(10) {
            format!("{}s", tenths / 10)
        } else {
            format!("{}.{}s", tenths / 10, tenths % 10)
        }
    } else {
        format!("{}s", (d.as_millis() + 500) / 1000)
    }
}
