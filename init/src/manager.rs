//! The manager itself: the [`Supervisor`] of [`core`](crate::core) on real
//! processes.
//!
//! One thread runs one loop. It waits in `poll` on a signalfd, the control
//! socket and the readiness checks under way, never longer than the next
//! deadline or a second, and nothing in it waits on anything else. A second
//! thread writes the manager's log, see [`crate::log`].

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::accounts;
use crate::control::Control;
use crate::core::{Io, Supervisor};
use crate::log::Log;
use crate::probe::Probe;
use crate::protocol::{ManagerLine, Request, ServiceLine, reply_line};
use crate::service::{Readiness, Service, Signal, Table, signal_name};
use crate::sys::{self, SignalFd};

/// The longest the loop sleeps when nothing is due, so that a child whose
/// SIGCHLD went astray is reaped within a second all the same.
const IDLE: Duration = Duration::from_secs(1);

/// SIGCHLD wakes the loop to reap; SIGTERM and SIGINT start the shutdown.
/// The rest are only logged: left alone they would end a manager that is
/// not PID 1.
const SIGNALS: [i32; 7] = [
    libc::SIGCHLD,
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGUSR1,
    libc::SIGUSR2,
];

pub struct Options {
    pub socket: PathBuf,
    pub shutdown_timeout: Duration,
}

/// Runs `services` until a shutdown ends, and returns the exit status: 0
/// after a shutdown that was asked for, 2 when the table cannot run, 1 when
/// the manager itself fails.
pub fn run(services: Vec<Service>, options: Options) -> i32 {
    // Before any thread starts, so that no thread can take these signals
    // instead of the signalfd.
    if let Err(e) = sys::block_signals(&SIGNALS) {
        eprintln!("stellarr-init: cannot block signals: {e}");
        return 1;
    }
    let log = match Log::start() {
        Ok(log) => log,
        Err(e) => {
            eprintln!("stellarr-init: cannot start the log thread: {e}");
            return 1;
        }
    };
    let code = match Table::new(services) {
        Ok(table) => match supervise(table, &options, &log) {
            Ok(code) => code,
            Err(e) => {
                log.line(&e);
                1
            }
        },
        Err(e) => {
            log.line(&format!("the service table cannot run: {e}"));
            2
        }
    };
    log.finish(Duration::from_millis(500));
    code
}

fn supervise(table: Table, options: &Options, log: &Log) -> Result<i32, String> {
    let signals = SignalFd::new(&SIGNALS).map_err(|e| format!("cannot create a signalfd: {e}"))?;
    let pid = std::process::id();
    if pid != 1 {
        sys::become_subreaper().map_err(|e| format!("cannot become a subreaper: {e}"))?;
        log.line(&format!(
            "running as PID {pid}, not 1; orphans come here all the same"
        ));
    }
    let mut control = Control::bind(&options.socket)
        .map_err(|e| format!("cannot listen on {}: {e}", options.socket.display()))?;
    log.line(&format!(
        "supervising {} services; control socket {}",
        table.services().len(),
        options.socket.display()
    ));

    let mut supervisor = Supervisor::new(table, options.shutdown_timeout);
    let mut world = World {
        log,
        probes: Vec::new(),
        answers: Vec::new(),
    };
    supervisor.tick(Instant::now(), &mut world);

    let mut fds: Vec<libc::pollfd> = Vec::new();
    loop {
        if let Some(code) = supervisor.exit_code() {
            return Ok(code);
        }

        let now = Instant::now();
        let deadline = [
            supervisor.next_deadline(),
            control.next_deadline(),
            world.probes.iter().map(|p| p.deadline).min(),
        ]
        .into_iter()
        .flatten()
        .min();
        let timeout = if world.answers.is_empty() {
            deadline.map_or(IDLE, |d| d.saturating_duration_since(now).min(IDLE))
        } else {
            Duration::ZERO
        };

        fds.clear();
        fds.push(libc::pollfd {
            fd: signals.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        control.interests(&mut fds);
        let probes_from = fds.len();
        fds.extend(world.probes.iter().map(Probe::interest));

        if let Err(e) = sys::poll(&mut fds, Some(timeout)) {
            abandon(&supervisor);
            return Err(format!("poll failed: {e}"));
        }
        let now = Instant::now();

        if fds[0].revents != 0 {
            loop {
                match signals.read() {
                    Ok(Some(signal)) => match signal {
                        libc::SIGCHLD => {}
                        libc::SIGTERM | libc::SIGINT => {
                            supervisor.shutdown(&signal_name(signal), now, &mut world);
                        }
                        other => log.line(&format!("{} ignored", signal_name(other))),
                    },
                    Ok(None) => break,
                    Err(e) => {
                        abandon(&supervisor);
                        return Err(format!("cannot read the signalfd: {e}"));
                    }
                }
            }
        }

        // Every round, not only on SIGCHLD: that costs one system call and
        // makes a lost signal harmless.
        loop {
            match sys::reap() {
                Ok(Some((pid, exit))) => {
                    supervisor.exited(pid, exit, now, &mut world);
                }
                Ok(None) => break,
                Err(e) => {
                    log.line(&format!("waitpid failed: {e}"));
                    break;
                }
            }
        }

        let revents: Vec<i16> = fds[probes_from..].iter().map(|f| f.revents).collect();
        world.collect_probes(&revents, now);
        for (index, run, ok) in std::mem::take(&mut world.answers) {
            supervisor.probed(index, run, ok, now, &mut world);
        }

        let revents: Vec<i16> = fds[1..probes_from].iter().map(|f| f.revents).collect();
        control.handle(&revents, now, |request| {
            answer(request, &mut supervisor, &mut world, now)
        });
        control.expire(now);

        supervisor.tick(now, &mut world);
    }
}

/// On the way out after a failure: kill every service rather than leave it
/// to whoever reaps orphans next.
fn abandon(supervisor: &Supervisor) {
    for status in supervisor.status() {
        if let Some(pid) = status.pid {
            let _ = sys::kill(pid, libc::SIGKILL, true);
        }
    }
}

fn answer(
    request: Result<Request, String>,
    supervisor: &mut Supervisor,
    world: &mut World,
    now: Instant,
) -> String {
    match request {
        Err(e) => reply_line(&Err(e)),
        Ok(Request::Health) => reply_line(&supervisor.health().map(|()| String::new())),
        Ok(Request::Restart(name)) => reply_line(&supervisor.restart(&name, now, world)),
        Ok(Request::Reload(name)) => reply_line(&supervisor.reload(&name, world)),
        Ok(Request::Status) => {
            let wall = SystemTime::now();
            let mut text = reply_line(&Ok(String::new()));
            text.push_str(
                &ManagerLine {
                    phase: supervisor.phase(),
                    pid: std::process::id(),
                }
                .format(),
            );
            for status in supervisor.status() {
                text.push_str(
                    &ServiceLine {
                        name: status.name,
                        state: status.state,
                        pid: status.pid,
                        restarts: status.restarts,
                        up_since: status.up_since.map(|at| unix_ms(at, now, wall)),
                        next_try: status.next_try.map(|at| unix_ms(at, now, wall)),
                        critical: status.critical,
                    }
                    .format(),
                );
            }
            text
        }
    }
}

/// `at` on the wall clock, in milliseconds since the Unix epoch.
fn unix_ms(at: Instant, now: Instant, wall: SystemTime) -> u64 {
    let wall = wall
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i128;
    let offset = if at >= now {
        (at - now).as_millis() as i128
    } else {
        -((now - at).as_millis() as i128)
    };
    (wall + offset).max(0) as u64
}

/// What the supervisor acts through.
struct World<'a> {
    log: &'a Log,
    probes: Vec<Probe>,
    /// Answers of readiness checks, for the supervisor to take next.
    answers: Vec<(usize, u64, bool)>,
}

impl World<'_> {
    /// Moves every check on and keeps the answers of those that ended,
    /// on their own or by running out of time.
    fn collect_probes(&mut self, revents: &[i16], now: Instant) {
        let mut k = 0;
        let answers = &mut self.answers;
        self.probes.retain_mut(|probe| {
            let events = revents.get(k).copied().unwrap_or(0);
            k += 1;
            let result = if events != 0 {
                probe.advance(events)
            } else {
                None
            };
            let result = result.or(if now >= probe.deadline {
                Some(false)
            } else {
                None
            });
            match result {
                Some(ok) => {
                    answers.push((probe.index, probe.run, ok));
                    false
                }
                None => true,
            }
        });
    }
}

impl Io for World<'_> {
    fn spawn(&mut self, service: &Service) -> Result<u32, String> {
        spawn(service)
    }

    fn signal(&mut self, pid: u32, signal: Signal, group: bool) {
        match sys::kill(pid, signal.number(), group) {
            Ok(()) => {}
            // The process, or every process of the group, is gone already.
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {}
            Err(e) => self.log.line(&format!(
                "cannot send {} to {}{pid}: {e}",
                signal.name(),
                if group { "the group of " } else { "" }
            )),
        }
    }

    fn probe(&mut self, index: usize, run: u64, check: &Readiness, timeout: Duration) {
        match Probe::start(index, run, check, Instant::now() + timeout) {
            Ok(probe) => self.probes.push(probe),
            Err(_) => self.answers.push((index, run, false)),
        }
    }

    fn log(&mut self, line: String) {
        self.log.line(&line);
    }
}

/// Starts a service in a process group of its own, with the manager's
/// stdout and stderr, nothing on stdin and no signal blocked. The standard
/// library drops the supplementary groups when it changes the user, and
/// every descriptor of the manager is close-on-exec.
pub fn spawn(service: &Service) -> Result<u32, String> {
    let program = &service.command[0];
    let mut command = Command::new(program);
    command
        .args(&service.command[1..])
        .stdin(Stdio::null())
        .process_group(0);
    sys::unblock_signals_on_exec(&mut command);
    if let Some(dir) = &service.dir {
        command.current_dir(dir);
    }
    for (name, value) in &service.env {
        command.env(name, value);
    }
    match (&service.user, &service.group) {
        (Some(user), group) => {
            let account = accounts::resolve(user, group.as_deref())?;
            command.uid(account.uid).gid(account.gid);
        }
        (None, Some(group)) => {
            let account = accounts::resolve("root", Some(group))?;
            command.gid(account.gid);
        }
        (None, None) => {}
    }
    let child = command
        .spawn()
        .map_err(|e| describe_spawn_error(program, &e))?;
    // The Child handle is dropped without waiting: the loop reaps every
    // child itself, with waitpid(-1).
    Ok(child.id())
}

fn describe_spawn_error(program: &str, error: &io::Error) -> String {
    format!("{program}: {error}")
}
