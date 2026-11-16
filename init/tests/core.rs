//! The decisions on their own: the supervisor on a fake clock, with an `Io`
//! that records what it was asked to do and starts no process.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use stellarr_init::core::{Exit, Io, PROBE_INTERVAL, PROBE_TIMEOUT, Phase, State, Supervisor};
use stellarr_init::service::{Backoff, Readiness, Service, Signal, Table, Watch};

const MS: Duration = Duration::from_millis(1);
const SEC: Duration = Duration::from_secs(1);

fn http(port: u16) -> Readiness {
    Readiness::http(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        "/health",
    )
}

fn svc(name: &str) -> Service {
    Service::new(name, &["/bin/true"]).ready(http(1))
}

#[derive(Default)]
struct Fake {
    next_pid: u32,
    /// Spawns in order: the service and the PID it got.
    spawns: Vec<(String, u32)>,
    /// Services whose spawn fails.
    broken: HashSet<String>,
    signals: Vec<(u32, Signal, bool)>,
    /// Probes asked for and not answered yet: service index and run.
    probes: Vec<(usize, u64)>,
    /// The time each probe was given, in the order they were asked for.
    timeouts: Vec<Duration>,
    logs: Vec<String>,
}

impl Io for Fake {
    fn spawn(&mut self, service: &Service) -> Result<u32, String> {
        if self.broken.contains(&service.name) {
            return Err("no such file".into());
        }
        self.next_pid += 1;
        let pid = 100 + self.next_pid;
        self.spawns.push((service.name.clone(), pid));
        Ok(pid)
    }

    fn signal(&mut self, pid: u32, signal: Signal, group: bool) {
        self.signals.push((pid, signal, group));
    }

    fn probe(&mut self, index: usize, run: u64, _check: &Readiness, timeout: Duration) {
        self.probes.push((index, run));
        self.timeouts.push(timeout);
    }

    fn log(&mut self, line: String) {
        self.logs.push(line);
    }
}

struct Sim {
    sup: Supervisor,
    io: Fake,
    t0: Instant,
    now: Instant,
}

impl Sim {
    fn new(services: Vec<Service>) -> Sim {
        Sim::with_timeout(services, Duration::from_secs(60))
    }

    fn with_timeout(services: Vec<Service>, shutdown: Duration) -> Sim {
        let table = Table::new(services).expect("a valid table");
        let now = Instant::now();
        Sim {
            sup: Supervisor::new(table, shutdown),
            io: Fake::default(),
            t0: now,
            now,
        }
    }

    fn tick(&mut self) {
        self.sup.tick(self.now, &mut self.io);
    }

    /// Moves the clock on and acts on what is due.
    fn wait(&mut self, d: Duration) {
        self.now += d;
        self.tick();
    }

    fn index(&self, name: &str) -> usize {
        self.sup.table().index_of(name).unwrap()
    }

    fn status(&self, name: &str) -> stellarr_init::core::Status {
        self.sup
            .status()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    fn state(&self, name: &str) -> State {
        self.status(name).state
    }

    fn pid(&self, name: &str) -> u32 {
        self.status(name).pid.expect("a running service")
    }

    fn spawned(&self) -> Vec<&str> {
        self.io.spawns.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Answers the pending probe of `name`.
    fn probe(&mut self, name: &str, ok: bool) {
        let i = self.index(name);
        // The newest: an older one belongs to an earlier run.
        let k = self
            .io
            .probes
            .iter()
            .rposition(|(j, _)| *j == i)
            .expect("a pending probe");
        let (_, run) = self.io.probes.remove(k);
        self.sup.probed(i, run, ok, self.now, &mut self.io);
    }

    fn ready(&mut self, name: &str) {
        self.probe(name, true);
        assert_eq!(self.state(name), State::Ready);
    }

    fn exit(&mut self, name: &str, exit: Exit) {
        let pid = self.pid(name);
        assert!(self.sup.exited(pid, exit, self.now, &mut self.io));
    }

    fn next_try_in(&self, name: &str) -> Duration {
        self.status(name)
            .next_try
            .expect("backing off")
            .duration_since(self.now)
    }

    fn last_signal(&self) -> (u32, Signal, bool) {
        *self.io.signals.last().expect("a signal")
    }
}

#[test]
fn the_wait_doubles_up_to_the_cap() {
    let b = Backoff {
        initial: Duration::from_millis(500),
        max: Duration::from_secs(10),
        reset_after: Duration::from_secs(60),
    };
    let delays: Vec<u64> = (1..=8).map(|n| b.delay(n).as_millis() as u64).collect();
    assert_eq!(delays, [500, 1000, 2000, 4000, 8000, 10000, 10000, 10000]);
    // Zero counts as one, and a count past any shift stays at the cap.
    assert_eq!(b.delay(0), b.initial);
    assert_eq!(b.delay(u32::MAX), b.max);
    assert_eq!(Backoff::DEFAULT.delay(1), SEC);
    assert_eq!(Backoff::DEFAULT.delay(40), Duration::from_secs(30));
}

#[test]
fn a_service_that_exits_at_once_backs_off_further_each_time() {
    let mut sim = Sim::new(vec![svc("a").critical()]);
    sim.tick();
    let mut waits = Vec::new();
    for _ in 0..8 {
        sim.exit("a", Exit::Code(1));
        assert_eq!(sim.state("a"), State::Backoff);
        let wait = sim.next_try_in("a");
        waits.push(wait.as_secs());
        assert_eq!(sim.sup.next_deadline(), Some(sim.now + wait));
        // Nothing happens a moment before the time.
        sim.wait(wait - MS);
        assert_eq!(sim.state("a"), State::Backoff);
        sim.wait(MS);
        assert_eq!(sim.state("a"), State::Starting);
    }
    assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(sim.status("a").restarts, 8);
    assert_eq!(sim.spawned().len(), 9);
    assert!(sim.sup.health().is_err());
}

#[test]
fn a_long_run_starts_the_count_again() {
    let mut sim = Sim::new(vec![svc("a")]);
    sim.tick();
    sim.exit("a", Exit::Code(1));
    sim.wait(SEC);
    sim.exit("a", Exit::Code(1));
    assert_eq!(sim.next_try_in("a"), 2 * SEC);
    sim.wait(2 * SEC);
    sim.ready("a");

    // Up for exactly the reset time: the next wait is the first one again.
    sim.wait(Duration::from_secs(60));
    sim.exit("a", Exit::Signal(libc::SIGSEGV));
    assert_eq!(sim.next_try_in("a"), SEC);
    assert!(
        sim.io
            .logs
            .last()
            .unwrap()
            .contains("killed by SIGSEGV after 60s")
    );

    // Shorter runs keep doubling.
    for expected in [2, 4, 8] {
        sim.wait(sim.next_try_in("a"));
        sim.wait(Duration::from_secs(59));
        sim.exit("a", Exit::Code(0));
        assert_eq!(sim.next_try_in("a"), expected * SEC);
    }
}

#[test]
fn every_exit_takes_the_rest_of_the_group_with_it() {
    let mut sim = Sim::new(vec![svc("a")]);
    sim.tick();
    let pid = sim.pid("a");
    sim.exit("a", Exit::Code(0));
    assert_eq!(sim.last_signal(), (pid, Signal::Kill, true));
    // Exit 0 is an exit like any other.
    assert_eq!(sim.state("a"), State::Backoff);
    assert!(sim.io.logs.last().unwrap().contains("exited with status 0"));
}

#[test]
fn an_orphan_is_not_a_service() {
    let mut sim = Sim::new(vec![svc("a")]);
    sim.tick();
    let before = sim.io.signals.len();
    assert!(!sim.sup.exited(4242, Exit::Code(0), sim.now, &mut sim.io));
    assert_eq!(sim.io.signals.len(), before);
    assert_eq!(sim.state("a"), State::Starting);
}

#[test]
fn a_spawn_that_fails_backs_off_like_an_exit() {
    let mut sim = Sim::new(vec![svc("a").critical(), svc("b")]);
    sim.io.broken.insert("a".into());
    sim.tick();
    assert_eq!(sim.state("a"), State::Backoff);
    assert_eq!(sim.state("b"), State::Starting);
    assert_eq!(sim.next_try_in("a"), SEC);
    sim.wait(SEC);
    assert_eq!(sim.next_try_in("a"), 2 * SEC);
    assert_eq!(sim.status("a").restarts, 1);
    assert!(
        sim.io
            .logs
            .iter()
            .any(|l| l.contains("a: cannot start: no such file"))
    );

    sim.io.broken.clear();
    sim.wait(2 * SEC);
    assert_eq!(sim.state("a"), State::Starting);
    sim.ready("a");
    sim.ready("b");
    assert_eq!(sim.sup.health(), Ok(()));
}

#[test]
fn the_start_order_follows_the_needs_then_the_table() {
    let table = Table::new(vec![
        svc("web").needs(&["api"]),
        svc("api").needs(&["db", "cache"]),
        svc("cache"),
        svc("db"),
        svc("tool"),
    ])
    .unwrap();
    let names: Vec<&str> = table
        .order()
        .iter()
        .map(|&i| table.services()[i].name.as_str())
        .collect();
    assert_eq!(names, ["cache", "db", "api", "web", "tool"]);
}

#[test]
fn a_cycle_is_an_error_that_names_it() {
    let error = Table::new(vec![
        svc("a").needs(&["b"]),
        svc("b").needs(&["c"]),
        svc("c").needs(&["a"]),
        svc("d").needs(&["a"]),
    ])
    .unwrap_err();
    assert_eq!(
        error.0,
        "the services need each other in a cycle: a needs b needs c needs a"
    );

    let error = Table::new(vec![
        svc("x"),
        svc("a").needs(&["b"]),
        svc("b").needs(&["a"]),
    ])
    .unwrap_err();
    assert_eq!(
        error.0,
        "the services need each other in a cycle: a needs b needs a"
    );
}

#[test]
fn a_bad_table_is_refused() {
    let lo = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80);
    let cases: Vec<(Vec<Service>, &str)> = vec![
        (vec![], "there are no services"),
        (vec![svc("a"), svc("a")], "two services are named a"),
        (vec![svc("a").needs(&["a"])], "a needs itself"),
        (
            vec![svc("a").needs(&["b"])],
            "a needs b, which is not a service",
        ),
        (
            vec![svc("b"), svc("a").needs(&["b", "b"])],
            "a names b twice",
        ),
        (vec![svc("API")], "is not a service name"),
        (vec![svc("1a")], "is not a service name"),
        (vec![svc("a b")], "is not a service name"),
        (vec![svc(&"a".repeat(33))], "is not a service name"),
        (vec![Service::new("a", &[])], "a has no command"),
        (vec![Service::new("a", &["true"])], "not an absolute path"),
        (vec![svc("a").dir("tmp")], "not an absolute path"),
        (
            vec![svc("a").ready(Readiness::Tcp(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                80,
            )))],
            "not a loopback address",
        ),
        (
            vec![svc("a").stop(Signal::Term, Duration::ZERO)],
            "no time to stop",
        ),
        (
            vec![svc("a").ready(Readiness::Http {
                addr: lo,
                path: "/".into(),
                headers: vec![("X-Evil".into(), "a\r\nHost: b".into())],
            })],
            "cannot go in a request",
        ),
        (
            vec![svc("a").ready(Readiness::http(lo, "health"))],
            "which is not a path",
        ),
        (
            vec![svc("a").watch(Watch {
                misses: 0,
                ..Watch::DEFAULT
            })],
            "none of them can be zero",
        ),
        (
            vec![svc("a").backoff(Backoff {
                initial: Duration::ZERO,
                max: SEC,
                reset_after: SEC,
            })],
            "must be above zero",
        ),
        (
            vec![svc("a").backoff(Backoff {
                initial: 2 * SEC,
                max: SEC,
                reset_after: SEC,
            })],
            "not above the second",
        ),
    ];
    for (services, expected) in cases {
        let error = Table::new(services).unwrap_err();
        assert!(error.0.contains(expected), "{error} lacks {expected:?}");
    }
    assert!(Table::new(vec![svc("a").ready(Readiness::Tcp(lo))]).is_ok());
}

#[test]
fn a_service_starts_once_what_it_needs_is_ready() {
    let mut sim = Sim::new(vec![svc("api").needs(&["db"]), svc("db"), svc("other")]);
    sim.tick();
    assert_eq!(sim.spawned(), ["db", "other"]);
    assert_eq!(sim.state("api"), State::Waiting);
    assert!(sim.io.logs.iter().any(|l| l == "api: waiting for db"));

    // Probes until db answers; api starts in the same step.
    sim.probe("db", false);
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + PROBE_INTERVAL));
    sim.wait(PROBE_INTERVAL);
    sim.ready("db");
    assert_eq!(sim.spawned(), ["db", "other", "api"]);

    // db going down does not stop api, but api's next start waits for db.
    sim.exit("db", Exit::Code(1));
    assert_eq!(sim.state("api"), State::Starting);
    sim.ready("api");
    sim.exit("api", Exit::Code(1));
    sim.wait(SEC);
    assert_eq!(sim.state("api"), State::Waiting);
    assert_eq!(sim.state("db"), State::Starting);
    sim.ready("db");
    assert_eq!(sim.state("api"), State::Starting);
}

#[test]
fn a_service_with_nothing_to_ask_is_ready_once_it_has_run_long_enough() {
    let mut sim = Sim::new(vec![
        svc("watch").ready(Readiness::Running(2 * SEC)),
        svc("after").needs(&["watch"]),
    ]);
    sim.tick();
    assert!(sim.io.probes.is_empty());
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + 2 * SEC));
    sim.wait(2 * SEC - MS);
    assert_eq!(sim.state("watch"), State::Starting);
    sim.wait(MS);
    assert_eq!(sim.state("watch"), State::Ready);
    assert_eq!(sim.state("after"), State::Starting);
    // Nothing to ask later either.
    sim.ready("after");
    sim.wait(Duration::from_secs(3600));
    assert!(sim.io.probes.iter().all(|&(i, _)| i != sim.index("watch")));
}

#[test]
fn a_ready_service_is_checked_again_at_every_interval() {
    let interval = Watch::DEFAULT.interval;
    let mut sim = Sim::new(vec![svc("a").critical()]);
    sim.tick();
    assert_eq!(sim.io.timeouts, [PROBE_TIMEOUT]);
    sim.ready("a");
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + interval));
    sim.wait(interval - MS);
    assert!(sim.io.probes.is_empty());
    sim.wait(MS);
    assert_eq!(sim.io.probes.len(), 1);
    assert_eq!(sim.io.timeouts.last(), Some(&Watch::DEFAULT.timeout));
    // While a check runs, the next one is not due.
    assert_eq!(sim.sup.next_deadline(), None);
    sim.wait(Watch::DEFAULT.timeout);
    sim.probe("a", true);
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + interval));
    assert_eq!(sim.state("a"), State::Ready);
}

#[test]
fn a_service_fails_after_misses_in_a_row_and_is_left_running() {
    let watch = Watch {
        interval: 10 * SEC,
        timeout: 5 * SEC,
        misses: 3,
    };
    let mut sim = Sim::new(vec![svc("a").critical().watch(watch).reload(Signal::Hup)]);
    sim.tick();
    sim.ready("a");
    let pid = sim.pid("a");

    // Two misses and a pass leave it ready, and the count starts over.
    for ok in [false, false, true, false, false] {
        sim.wait(watch.interval);
        sim.probe("a", ok);
        assert_eq!(sim.state("a"), State::Ready);
        assert_eq!(sim.sup.health(), Ok(()));
    }
    let signals = sim.io.signals.len();
    sim.wait(watch.interval);
    sim.probe("a", false);
    assert_eq!(sim.state("a"), State::Failing);
    assert_eq!(sim.sup.health(), Err("a is failing".into()));
    assert_eq!(
        sim.io.logs.last().unwrap(),
        "a: failed its check 3 times in a row; failing, and left running"
    );
    // Not restarted, nor signalled at all, and still checked.
    assert_eq!(sim.io.signals.len(), signals);
    assert_eq!(sim.pid("a"), pid);
    sim.wait(watch.interval);
    sim.probe("a", false);
    assert_eq!(sim.state("a"), State::Failing);
    // A reload is still taken: it may be what the service needs.
    assert!(sim.sup.reload("a", &mut sim.io).is_ok());

    sim.wait(watch.interval);
    sim.probe("a", true);
    assert_eq!(sim.state("a"), State::Ready);
    assert_eq!(sim.sup.health(), Ok(()));
    assert_eq!(
        sim.io.logs.last().unwrap(),
        "a: answers its check again; ready"
    );
    assert_eq!(sim.status("a").restarts, 0);
}

#[test]
fn a_failing_service_holds_back_what_needs_it() {
    let mut sim = Sim::new(vec![svc("db"), svc("api").needs(&["db"])]);
    sim.tick();
    sim.ready("db");
    sim.ready("api");
    for _ in 0..Watch::DEFAULT.misses {
        sim.wait(Watch::DEFAULT.interval);
        sim.probe("db", false);
    }
    assert_eq!(sim.state("db"), State::Failing);
    // api keeps running, but its next start waits for db to pass again.
    assert_eq!(sim.state("api"), State::Ready);
    sim.exit("api", Exit::Code(1));
    sim.wait(SEC);
    assert_eq!(sim.state("api"), State::Waiting);
    assert_eq!(
        sim.sup.restart("api", sim.now, &mut sim.io),
        Err("api has not started: it waits for db".into())
    );
    sim.wait(Watch::DEFAULT.interval);
    sim.probe("db", true);
    assert_eq!(sim.state("api"), State::Starting);
}

#[test]
fn a_failing_service_restarts_on_request_and_starts_clean() {
    let mut sim = Sim::new(vec![svc("a").critical()]);
    sim.tick();
    sim.ready("a");
    for _ in 0..Watch::DEFAULT.misses {
        sim.wait(Watch::DEFAULT.interval);
        sim.probe("a", false);
    }
    assert_eq!(sim.state("a"), State::Failing);
    sim.sup.restart("a", sim.now, &mut sim.io).unwrap();
    sim.exit("a", Exit::Signal(libc::SIGTERM));
    sim.ready("a");
    assert_eq!(sim.state("a"), State::Ready);
    // One miss of the new run does not make it fail.
    sim.wait(Watch::DEFAULT.interval);
    sim.probe("a", false);
    assert_eq!(sim.state("a"), State::Ready);
}

#[test]
fn no_check_is_asked_for_once_the_shutdown_has_begun() {
    let mut sim = Sim::new(vec![svc("a").stop(Signal::Term, 50 * SEC), svc("b")]);
    sim.tick();
    sim.ready("a");
    sim.ready("b");
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    let asked = sim.io.probes.len();
    // b is stopping; a waits its turn, still ready, for a long time.
    sim.wait(40 * SEC);
    assert_eq!(sim.state("a"), State::Ready);
    assert_eq!(sim.io.probes.len(), asked);
}

#[test]
fn a_late_answer_for_an_earlier_run_is_ignored() {
    let mut sim = Sim::new(vec![svc("a")]);
    sim.tick();
    let (i, old_run) = sim.io.probes.remove(0);
    sim.exit("a", Exit::Code(1));
    sim.wait(SEC);
    assert_eq!(sim.state("a"), State::Starting);
    sim.sup.probed(i, old_run, true, sim.now, &mut sim.io);
    assert_eq!(sim.state("a"), State::Starting);
    // An answer nobody asked for is ignored too.
    sim.sup.probed(i, 99, true, sim.now, &mut sim.io);
    sim.sup.probed(42, 1, true, sim.now, &mut sim.io);
    assert_eq!(sim.state("a"), State::Starting);
    sim.ready("a");
}

#[test]
fn a_service_that_never_becomes_ready_keeps_being_asked() {
    let mut sim = Sim::new(vec![svc("slow").critical(), svc("after").needs(&["slow"])]);
    sim.tick();
    for _ in 0..100 {
        sim.probe("slow", false);
        sim.wait(PROBE_INTERVAL);
    }
    assert_eq!(sim.state("slow"), State::Starting);
    assert_eq!(sim.state("after"), State::Waiting);
    assert_eq!(sim.sup.health(), Err("slow is starting".into()));
    assert_eq!(sim.spawned(), ["slow"]);
}

#[test]
fn a_restart_stops_the_group_and_starts_again_at_once() {
    let mut sim = Sim::new(vec![svc("a").stop(Signal::Quit, 5 * SEC)]);
    sim.tick();
    sim.ready("a");
    let pid = sim.pid("a");
    assert_eq!(
        sim.sup.restart("a", sim.now, &mut sim.io),
        Ok("restarting a".into())
    );
    assert_eq!(sim.last_signal(), (pid, Signal::Quit, true));
    assert_eq!(sim.state("a"), State::Stopping);
    assert!(sim.sup.health().is_ok(), "a is not critical");

    sim.wait(SEC);
    sim.exit("a", Exit::Signal(libc::SIGQUIT));
    assert_eq!(sim.state("a"), State::Starting);
    assert_ne!(sim.pid("a"), pid);
    assert_eq!(sim.status("a").restarts, 1);
}

#[test]
fn a_restart_kills_what_does_not_stop_in_time() {
    let mut sim = Sim::new(vec![svc("a").stop(Signal::Term, 5 * SEC)]);
    sim.tick();
    sim.ready("a");
    let pid = sim.pid("a");
    sim.sup.restart("a", sim.now, &mut sim.io).unwrap();
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + 5 * SEC));
    // A second request while it stops changes nothing.
    sim.sup.restart("a", sim.now, &mut sim.io).unwrap();
    sim.wait(5 * SEC - MS);
    assert_eq!(sim.last_signal(), (pid, Signal::Term, true));
    sim.wait(MS);
    assert_eq!(sim.last_signal(), (pid, Signal::Kill, true));
    assert_eq!(sim.sup.next_deadline(), None);
    sim.exit("a", Exit::Signal(libc::SIGKILL));
    assert_eq!(sim.state("a"), State::Starting);
}

#[test]
fn a_restart_during_a_backoff_starts_at_once_and_forgets_the_failures() {
    let mut sim = Sim::new(vec![svc("a")]);
    sim.tick();
    for _ in 0..4 {
        sim.exit("a", Exit::Code(1));
        sim.wait(sim.next_try_in("a"));
    }
    sim.exit("a", Exit::Code(1));
    assert_eq!(sim.next_try_in("a"), 16 * SEC);
    sim.sup.restart("a", sim.now, &mut sim.io).unwrap();
    assert_eq!(sim.state("a"), State::Starting);
    sim.exit("a", Exit::Code(1));
    assert_eq!(sim.next_try_in("a"), SEC);
}

#[test]
fn restart_and_reload_refuse_what_they_cannot_do() {
    let mut sim = Sim::new(vec![
        svc("db"),
        svc("api").needs(&["db"]),
        svc("nginx").reload(Signal::Hup),
    ]);
    sim.tick();
    let now = sim.now;
    let io = &mut sim.io;
    assert_eq!(
        sim.sup.restart("nope", now, io),
        Err("there is no service named nope".into())
    );
    assert_eq!(
        sim.sup.restart("api", now, io),
        Err("api has not started: it waits for db".into())
    );
    assert_eq!(
        sim.sup.reload("db", io),
        Err("db cannot reload; restart it instead".into())
    );
    assert_eq!(
        sim.sup.reload("nginx", io),
        Err("nginx is starting, not ready; there is nothing to reload".into())
    );
    sim.ready("nginx");
    let pid = sim.pid("nginx");
    assert_eq!(
        sim.sup.reload("nginx", &mut sim.io),
        Ok("reloading nginx".into())
    );
    // To the main process only, and the state stays.
    assert_eq!(sim.last_signal(), (pid, Signal::Hup, false));
    assert_eq!(sim.state("nginx"), State::Ready);
}

#[test]
fn health_needs_every_critical_service_ready() {
    let mut sim = Sim::new(vec![svc("a").critical(), svc("b").critical(), svc("c")]);
    sim.tick();
    assert_eq!(sim.sup.health(), Err("a is starting, b is starting".into()));
    sim.ready("a");
    assert_eq!(sim.sup.health(), Err("b is starting".into()));
    sim.ready("b");
    assert_eq!(sim.sup.health(), Ok(()));
    sim.exit("b", Exit::Code(3));
    assert_eq!(sim.sup.health(), Err("b is backoff".into()));
    sim.wait(SEC);
    sim.ready("b");
    assert_eq!(sim.sup.health(), Ok(()));
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    assert_eq!(sim.sup.health(), Err("the manager is shutting down".into()));
}

#[test]
fn the_status_says_what_each_service_is_doing() {
    let mut sim = Sim::new(vec![svc("a").critical(), svc("b").needs(&["a"]), svc("c")]);
    sim.tick();
    sim.ready("a");
    sim.wait(3 * SEC);
    sim.exit("c", Exit::Code(1));
    let status = sim.sup.status();
    assert_eq!(status[0].state, State::Ready);
    assert_eq!(status[0].up_since, Some(sim.t0));
    assert!(status[0].pid.is_some() && status[0].critical);
    assert_eq!(status[1].state, State::Starting);
    assert_eq!(status[1].up_since, Some(sim.t0));
    assert_eq!(status[2].state, State::Backoff);
    assert_eq!(status[2].pid, None);
    assert_eq!(status[2].up_since, None);
    assert_eq!(status[2].next_try, Some(sim.now + SEC));
    assert_eq!(sim.sup.phase(), Phase::Running);
}

#[test]
fn the_shutdown_stops_one_service_at_a_time_in_reverse_order() {
    let mut sim = Sim::with_timeout(
        vec![
            svc("media").stop(Signal::Term, 5 * SEC),
            svc("api").needs(&["media"]).stop(Signal::Int, 30 * SEC),
            svc("nginx").stop(Signal::Quit, 10 * SEC),
            svc("crashy"),
        ],
        Duration::from_secs(60),
    );
    sim.tick();
    sim.ready("media");
    sim.ready("api");
    sim.ready("nginx");
    sim.exit("crashy", Exit::Code(1));
    let (media, api, nginx) = (sim.pid("media"), sim.pid("api"), sim.pid("nginx"));

    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    assert_eq!(sim.sup.phase(), Phase::ShuttingDown);
    // Start order: media, api, nginx, crashy. Nothing to stop in crashy.
    assert_eq!(sim.state("crashy"), State::Stopped);
    assert_eq!(sim.last_signal(), (nginx, Signal::Quit, true));
    assert_eq!(sim.state("api"), State::Ready);
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + 10 * SEC));

    sim.wait(2 * SEC);
    sim.exit("nginx", Exit::Code(0));
    assert_eq!(sim.state("nginx"), State::Stopped);
    assert_eq!(sim.last_signal(), (api, Signal::Int, true));

    // api takes its whole deadline and is killed.
    sim.wait(30 * SEC);
    assert_eq!(sim.last_signal(), (api, Signal::Kill, true));
    assert_eq!(sim.state("media"), State::Ready);
    sim.exit("api", Exit::Signal(libc::SIGKILL));
    assert_eq!(sim.last_signal(), (media, Signal::Term, true));
    sim.exit("media", Exit::Signal(libc::SIGTERM));

    assert_eq!(sim.sup.phase(), Phase::Done);
    assert_eq!(sim.sup.exit_code(), Some(0));
    assert_eq!(sim.spawned(), ["media", "nginx", "crashy", "api"]);
}

#[test]
fn nothing_starts_once_the_shutdown_has_begun() {
    let mut sim = Sim::new(vec![svc("db"), svc("api").needs(&["db"]), svc("crashy")]);
    sim.tick();
    sim.exit("crashy", Exit::Code(1));
    sim.sup.shutdown("SIGINT", sim.now, &mut sim.io);
    // db is being stopped; its probe still answers and crashy's wait ends.
    sim.probe("db", true);
    sim.wait(5 * SEC);
    assert_eq!(sim.spawned(), ["db", "crashy"]);
    assert_eq!(sim.state("api"), State::Stopped);
    assert_eq!(sim.state("crashy"), State::Stopped);
    assert!(sim.sup.restart("db", sim.now, &mut sim.io).is_err());
    assert!(sim.sup.reload("db", &mut sim.io).is_err());
    // No probe is asked for while shutting down.
    let probes = sim.io.probes.len();
    sim.wait(5 * SEC);
    assert_eq!(sim.io.probes.len(), probes);
    sim.exit("db", Exit::Code(0));
    assert_eq!(sim.sup.exit_code(), Some(0));
}

#[test]
fn the_shutdown_kills_everything_left_at_its_deadline() {
    let mut sim = Sim::with_timeout(
        vec![
            svc("a").stop(Signal::Term, 10 * SEC),
            svc("b").stop(Signal::Term, 10 * SEC),
        ],
        Duration::from_secs(4),
    );
    sim.tick();
    sim.ready("a");
    let (a, b) = (sim.pid("a"), sim.pid("b"));
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    assert_eq!(sim.last_signal(), (b, Signal::Term, true));
    // b's own deadline is cut to the total one.
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + 4 * SEC));
    sim.wait(4 * SEC);
    let kills: Vec<_> = sim.io.signals.iter().rev().take(2).copied().collect();
    assert!(kills.contains(&(a, Signal::Kill, true)));
    assert!(kills.contains(&(b, Signal::Kill, true)));
    assert_eq!(sim.sup.exit_code(), Some(0));
    assert_eq!(sim.sup.next_deadline(), None);
    // Their exits come after the end and are nobody's.
    assert!(!sim.sup.exited(a, Exit::Signal(9), sim.now, &mut sim.io));
}

#[test]
fn a_shutdown_during_a_restart_leaves_the_service_stopped() {
    let mut sim = Sim::new(vec![svc("a").stop(Signal::Term, 20 * SEC), svc("b")]);
    sim.tick();
    sim.ready("a");
    sim.ready("b");
    sim.sup.restart("a", sim.now, &mut sim.io).unwrap();
    sim.wait(SEC);
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    let b = sim.pid("b");
    assert_eq!(sim.last_signal(), (b, Signal::Term, true));
    sim.exit("b", Exit::Code(0));
    sim.exit("a", Exit::Code(0));
    assert_eq!(sim.state("a"), State::Stopped);
    assert_eq!(sim.spawned(), ["a", "b"]);
    assert_eq!(sim.sup.exit_code(), Some(0));
}

#[test]
fn a_restart_already_stopping_is_cut_to_the_shutdown_deadline() {
    let mut sim = Sim::with_timeout(
        vec![svc("a").stop(Signal::Term, 50 * SEC)],
        Duration::from_secs(10),
    );
    sim.tick();
    sim.ready("a");
    sim.sup.restart("a", sim.now, &mut sim.io).unwrap();
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    assert_eq!(sim.sup.next_deadline(), Some(sim.now + 10 * SEC));
}

#[test]
fn a_second_signal_does_not_start_the_shutdown_over() {
    let mut sim = Sim::with_timeout(vec![svc("a")], Duration::from_secs(10));
    sim.tick();
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    sim.wait(9 * SEC);
    sim.sup.shutdown("SIGINT", sim.now, &mut sim.io);
    assert_eq!(
        sim.io.logs.last().unwrap(),
        "SIGINT while shutting down; carrying on"
    );
    sim.wait(SEC);
    assert_eq!(sim.sup.exit_code(), Some(0));
}

#[test]
fn a_shutdown_with_nothing_running_ends_at_once() {
    let mut sim = Sim::new(vec![svc("a")]);
    sim.io.broken.insert("a".into());
    sim.tick();
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    assert_eq!(sim.sup.exit_code(), Some(0));
    assert_eq!(
        sim.io.logs.last().unwrap(),
        "every service is stopped; exiting"
    );
}

#[test]
fn every_state_change_is_one_log_line() {
    let mut sim = Sim::new(vec![svc("a"), svc("b").needs(&["a"])]);
    sim.tick();
    sim.ready("a");
    sim.exit("a", Exit::Code(2));
    sim.wait(SEC);
    sim.sup.restart("b", sim.now, &mut sim.io).unwrap();
    sim.exit("b", Exit::Signal(libc::SIGTERM));
    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    sim.exit("a", Exit::Signal(libc::SIGTERM));
    let expected = [
        "a: started, pid 101",
        "b: waiting for a",
        "a: ready after 0s",
        "b: started, pid 102",
        "a: exited with status 2 after 0s; next try in 1s",
        "a: started, pid 103",
        "b: restarting on request; SIGTERM sent, SIGKILL in 10s",
        "b: killed by SIGTERM 0s after it was asked to stop; starting again",
        "b: waiting for a",
        "shutting down on SIGTERM; everything is stopped within 60s",
        "b: stopped",
        "a: stopping; SIGTERM sent, SIGKILL in 10s",
        "a: killed by SIGTERM 0s after it was asked to stop; stopped",
        "every service is stopped; exiting",
    ];
    assert_eq!(sim.io.logs, expected);
}

/// A small generator, so that a failing sequence can be run again from its
/// seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn random_events_keep_the_rules() {
    for seed in 1..=300u64 {
        run_random(seed);
    }
}

/// Feeds the supervisor random exits, probe answers, requests, waits and one
/// shutdown, with processes that may or may not obey a stop signal, and
/// checks after every step what must always hold.
fn run_random(seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let services = vec![
        svc("a").critical().stop(Signal::Term, 3 * SEC),
        svc("b").needs(&["a"]).stop(Signal::Int, 2 * SEC),
        svc("c").ready(Readiness::Running(SEC)).reload(Signal::Hup),
        svc("d").needs(&["a", "c"]).critical(),
    ];
    let names = ["a", "b", "c", "d"];
    let mut sim = Sim::with_timeout(services, Duration::from_secs(8));
    let mut obeys: HashMap<u32, bool> = HashMap::new();
    let mut live: BTreeSet<u32> = BTreeSet::new();
    let mut dying: Vec<u32> = Vec::new();
    let mut shutdown_at: Option<Instant> = None;
    let mut spawns_before_shutdown = 0;
    let mut seen_spawns = 0;
    let mut seen_signals = 0;

    sim.tick();
    for step in 0..400 {
        let context = format!("seed {seed}, step {step}");
        match rng.below(10) {
            0..=2 => {
                let ms = rng.below(2500);
                sim.wait(Duration::from_millis(ms));
            }
            3 => {
                // A process ends on its own.
                if let Some(&pid) = live
                    .iter()
                    .nth(rng.below(live.len().max(1) as u64) as usize)
                {
                    let code = rng.below(3) as i32;
                    live.remove(&pid);
                    assert!(
                        sim.sup.exited(pid, Exit::Code(code), sim.now, &mut sim.io),
                        "{context}"
                    );
                }
            }
            4 => {
                // A process that was told to stop does.
                if let Some(pid) = dying.pop()
                    && live.remove(&pid)
                {
                    sim.sup
                        .exited(pid, Exit::Signal(libc::SIGTERM), sim.now, &mut sim.io);
                }
            }
            5 | 6 => {
                if !sim.io.probes.is_empty() {
                    let k = rng.below(sim.io.probes.len() as u64) as usize;
                    let (i, run) = sim.io.probes.remove(k);
                    let ok = rng.below(3) > 0;
                    sim.sup.probed(i, run, ok, sim.now, &mut sim.io);
                }
            }
            7 => {
                let name = names[rng.below(4) as usize];
                let _ = sim.sup.restart(name, sim.now, &mut sim.io);
            }
            8 => {
                let name = names[rng.below(4) as usize];
                let _ = sim.sup.reload(name, &mut sim.io);
            }
            _ => {
                if shutdown_at.is_none() && rng.below(4) == 0 {
                    shutdown_at = Some(sim.now);
                    spawns_before_shutdown = sim.io.spawns.len();
                    sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
                }
            }
        }

        // What the supervisor did in this step.
        if shutdown_at.is_some() {
            assert_eq!(
                sim.io.spawns.len(),
                spawns_before_shutdown,
                "{context}: a spawn during the shutdown"
            );
        }
        for (name, pid) in &sim.io.spawns[seen_spawns..] {
            let others_live = sim
                .io
                .spawns
                .iter()
                .filter(|(n, p)| n == name && p != pid && live.contains(p))
                .count();
            assert_eq!(others_live, 0, "{context}: two runs of {name} at once");
            live.insert(*pid);
            obeys.insert(*pid, rng.below(3) > 0);
        }
        seen_spawns = sim.io.spawns.len();
        for &(pid, signal, group) in &sim.io.signals[seen_signals..] {
            match signal {
                Signal::Kill => {
                    if live.contains(&pid) {
                        dying.push(pid);
                    }
                }
                Signal::Hup => {
                    assert!(live.contains(&pid) && !group, "{context}: reload to {pid}");
                }
                _ => {
                    assert!(live.contains(&pid) && group, "{context}: stop to {pid}");
                    if obeys[&pid] {
                        dying.push(pid);
                    }
                }
            }
        }
        seen_signals = sim.io.signals.len();

        // What must always hold.
        if let Some(next) = sim.sup.next_deadline() {
            assert!(next > sim.now, "{context}: something due is left undone");
        }
        for status in sim.sup.status() {
            let running = matches!(
                status.state,
                State::Starting | State::Ready | State::Failing | State::Stopping
            );
            assert_eq!(running, status.pid.is_some(), "{context}: {status:?}");
            if let Some(pid) = status.pid {
                assert!(
                    live.contains(&pid),
                    "{context}: {} has a dead PID",
                    status.name
                );
            }
        }
        if sim.sup.health().is_ok() {
            assert_eq!(sim.sup.phase(), Phase::Running);
            for status in sim.sup.status() {
                assert!(
                    !status.critical || status.state == State::Ready,
                    "{context}"
                );
            }
        }
        if let Some(at) = shutdown_at {
            if sim.now >= at + Duration::from_secs(8) {
                assert_eq!(sim.sup.phase(), Phase::Done, "{context}: past the deadline");
            }
            if sim.sup.phase() == Phase::Done {
                assert_eq!(sim.sup.exit_code(), Some(0));
                return;
            }
        }
    }
    // Every sequence ends with a shutdown that completes on time.
    if shutdown_at.is_none() {
        sim.sup.shutdown("SIGTERM", sim.now, &mut sim.io);
    }
    sim.wait(Duration::from_secs(8));
    assert_eq!(sim.sup.exit_code(), Some(0), "seed {seed}");
}
