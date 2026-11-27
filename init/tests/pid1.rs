//! The manager as PID 1, with real processes.
//!
//! Each scenario starts the manager under `unshare --pid --fork --mount-proc`,
//! so that it is PID 1 of a PID namespace of its own and every orphan of the
//! scenario comes to it, as in the container. The manager is this same test
//! binary, started again with `PID1_MANAGER` naming the scenario: it builds
//! the scenario's services and calls `manager::run`, as the real binary does
//! with the compiled-in table. The services are the `misbehave` helper.
//!
//! The scenarios need root and the right to create namespaces, and run one at
//! a time. In a Linux container:
//!
//! ```text
//! docker run --rm --privileged -v "$PWD":/src -w /src rust:1-slim-bookworm cargo test
//! ```
//!
//! `cargo test --test pid1 -- NAME` runs the scenarios whose name contains
//! NAME; `PID1_VERBOSE=1` echoes the manager's log.

use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use stellarr_init::client;
use stellarr_init::core::{Phase, State};
use stellarr_init::manager::{self, Options};
use stellarr_init::protocol::{ServiceLine, StatusReply};
use stellarr_init::service::{Backoff, Readiness, Service, Signal, Watch};
use stellarr_init::sys;

const HELPER: &str = env!("CARGO_BIN_EXE_misbehave");
const CLI: &str = env!("CARGO_BIN_EXE_stellarr-init");
const ROOT: &str = "/tmp/stellarr-init-pid1";
const DEFAULT_SHUTDOWN: Duration = Duration::from_secs(10);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn lo(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn path(dir: &Path, name: &str) -> String {
    dir.join(name).display().to_string()
}

fn fast() -> Backoff {
    Backoff {
        initial: ms(200),
        max: ms(1600),
        reset_after: secs(2),
    }
}

/// The helper as service `name`, tagged with that name on its command line.
fn helper(name: &str, args: &[&str]) -> Service {
    let mut command = vec![HELPER, "--tag", name];
    command.extend_from_slice(args);
    Service::new(name, &command)
        .ready(Readiness::Running(ms(200)))
        .stop(Signal::Term, secs(2))
        .backoff(fast())
}

struct Scenario {
    name: &'static str,
    services: fn(&Path) -> Vec<Service>,
    shutdown: Duration,
    /// Where the manager listens, when not at `init.sock` in the folder.
    socket: Option<fn(&Path) -> PathBuf>,
    /// Runs before the manager starts.
    prepare: fn(&Path),
    /// Starts the manager as a plain child of the test, not as PID 1.
    plain: bool,
    check: fn(&mut Pid1),
}

fn scenario(
    name: &'static str,
    services: fn(&Path) -> Vec<Service>,
    check: fn(&mut Pid1),
) -> Scenario {
    Scenario {
        name,
        services,
        shutdown: DEFAULT_SHUTDOWN,
        socket: None,
        prepare: |_| {},
        plain: false,
        check,
    }
}

fn scenarios() -> Vec<Scenario> {
    vec![
        scenario(
            "spawn_environment",
            spawn_environment,
            check_spawn_environment,
        ),
        scenario(
            "exits_at_once_every_time",
            exits_at_once,
            check_exits_at_once,
        ),
        scenario(
            "exits_three_times_then_runs",
            exits_three_times,
            check_exits_three_times,
        ),
        scenario(
            "crashes_after_a_while",
            crashes_after_a_while,
            check_crashes_after_a_while,
        ),
        scenario(
            "slow_to_become_ready",
            slow_to_become_ready,
            check_slow_to_become_ready,
        ),
        scenario("never_ready", never_ready, check_never_ready),
        scenario("ignores_sigterm", ignores_sigterm, check_ignores_sigterm),
        scenario("ignores_sigterm_with_children", family, check_family),
        scenario("orphaned_grandchildren", orphans, check_orphans),
        scenario("floods_stdout_and_stderr", flood, check_flood),
        scenario("exits_zero_on_its_own", exits_zero, check_exits_zero),
        scenario("critical_service_cannot_run", critical, check_critical),
        scenario(
            "restart_and_reload",
            restart_and_reload,
            check_restart_and_reload,
        ),
        scenario("socket_clients", socket_clients, check_socket_clients),
        scenario(
            "sigterm_during_startup",
            startup,
            check_sigterm_during_startup,
        ),
        scenario(
            "sigterm_during_backoff",
            backoff,
            check_sigterm_during_backoff,
        ),
        scenario(
            "sigterm_during_restart",
            during_restart,
            check_sigterm_during_restart,
        ),
        Scenario {
            shutdown: ms(1500),
            ..scenario("shutdown_deadline", deadline, check_shutdown_deadline)
        },
        scenario("cycle_in_the_table", cycle, check_cycle),
        Scenario {
            socket: Some(|dir| dir.join("file/init.sock")),
            prepare: |dir| fs::write(dir.join("file"), "not a folder").unwrap(),
            ..scenario("cannot_listen", one_service, check_cannot_listen)
        },
        scenario("another_manager", one_service, check_another_manager),
        scenario(
            "health_without_an_answer",
            one_service,
            check_health_without_an_answer,
        ),
        scenario("health_against_a_trickle", one_service, check_trickle),
        scenario("stun_readiness", stun, check_stun),
        scenario("stops_answering_while_alive", stalls, check_stalls),
        scenario("restarted_for_stalling", restarted, check_restarted),
        Scenario {
            plain: true,
            ..scenario("not_pid1", orphan_outside, check_not_pid1)
        },
    ]
}

fn main() {
    if let Ok(name) = std::env::var("PID1_MANAGER") {
        run_manager(&name);
    }

    let filter: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    require_namespaces();

    let mut failed = Vec::new();
    let mut count = 0;
    for scenario in scenarios() {
        if !filter.is_empty() && !filter.iter().any(|f| scenario.name.contains(f.as_str())) {
            continue;
        }
        count += 1;
        let started = Instant::now();
        eprint!("scenario {} ... ", scenario.name);
        let outcome = catch_unwind(AssertUnwindSafe(|| run_scenario(&scenario)));
        let took = started.elapsed().as_secs_f64();
        match outcome {
            Ok(()) => eprintln!("ok ({took:.1}s)"),
            Err(_) => {
                eprintln!("FAILED ({took:.1}s)");
                failed.push(scenario.name);
            }
        }
    }
    eprintln!(
        "\n{} scenarios: {} passed, {} failed",
        count,
        count - failed.len(),
        failed.len()
    );
    if !failed.is_empty() {
        eprintln!("failed: {}", failed.join(", "));
        std::process::exit(1);
    }
}

/// The scenarios fail rather than skip when they cannot run as meant.
fn require_namespaces() {
    let ok = sys::effective_uid() == 0
        && Command::new("unshare")
            .args(["--pid", "--fork", "--mount-proc", "true"])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    if !ok {
        eprintln!(
            "tests/pid1.rs runs as root where it may create PID namespaces, such as a \
             Linux container started with --privileged; see the top of the file"
        );
        std::process::exit(1);
    }
}

/// The manager side of a scenario: this binary again, as PID 1.
fn run_manager(name: &str) -> ! {
    let dir = PathBuf::from(std::env::var("PID1_DIR").expect("PID1_DIR"));
    let scenario = scenarios()
        .into_iter()
        .find(|s| s.name == name)
        .expect("a known scenario");
    let socket = scenario
        .socket
        .map_or_else(|| dir.join("init.sock"), |f| f(&dir));
    let code = manager::run(
        (scenario.services)(&dir),
        Options {
            socket,
            shutdown_timeout: scenario.shutdown,
        },
    );
    std::process::exit(code);
}

fn run_scenario(scenario: &Scenario) {
    let dir = PathBuf::from(ROOT).join(scenario.name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Services that run as nobody write their reports here too.
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
    (scenario.prepare)(&dir);
    let mut pid1 = Pid1::launch(scenario, &dir);
    let outcome = catch_unwind(AssertUnwindSafe(|| (scenario.check)(&mut pid1)));
    if let Err(panic) = outcome {
        eprintln!("\n--- the manager's log ---\n{}--- end ---", pid1.log());
        std::panic::resume_unwind(panic);
    }
}

/// A process in a scenario's PID namespace.
#[derive(Debug)]
struct Proc {
    /// Its PID outside the namespace.
    pid: u32,
    state: char,
    args: Vec<String>,
}

impl Proc {
    fn tag(&self) -> Option<&str> {
        let i = self.args.iter().position(|a| a == "--tag")?;
        self.args.get(i + 1).map(String::as_str)
    }
}

fn processes_in(ns: &str) -> Vec<Proc> {
    processes_where(|dir| {
        fs::read_link(dir.join("ns/pid")).is_ok_and(|link| link.to_string_lossy() == ns)
    })
}

/// Every process whose /proc folder passes `keep`.
fn processes_where(keep: impl Fn(&Path) -> bool) -> Vec<Proc> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if !keep(&entry.path()) {
            continue;
        }
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let state = stat[stat.rfind(')').unwrap() + 2..].chars().next().unwrap();
        let args = fs::read(entry.path().join("cmdline"))
            .unwrap_or_default()
            .split(|&b| b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        found.push(Proc { pid, state, args });
    }
    found
}

fn parent_of(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat[stat.rfind(')')? + 2..].split(' ').nth(1)?.parse().ok()
}

/// The output of one run of the `stellarr-init` binary.
#[derive(Debug)]
struct Cli {
    code: i32,
    stdout: String,
    stderr: String,
    took: Duration,
}

/// The manager of one scenario, running.
struct Pid1 {
    dir: PathBuf,
    socket: PathBuf,
    /// unshare, or the manager itself when the scenario is plain.
    process: Child,
    /// The manager's PID outside its namespace; `None` if it exited before
    /// it was found.
    pid: Option<u32>,
    ns: String,
    log: Arc<Mutex<String>>,
    paused: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    exited: bool,
}

impl Pid1 {
    fn launch(scenario: &Scenario, dir: &Path) -> Pid1 {
        let socket = scenario
            .socket
            .map_or_else(|| dir.join("init.sock"), |f| f(dir));
        let (reader, writer) = std::io::pipe().unwrap();
        let mut command = if scenario.plain {
            Command::new(std::env::current_exe().unwrap())
        } else {
            let mut unshare = Command::new("unshare");
            unshare
                .args(["--pid", "--fork", "--kill-child", "--mount-proc"])
                .arg(std::env::current_exe().unwrap());
            unshare
        };
        let process = command
            .env("PID1_MANAGER", scenario.name)
            .env("PID1_DIR", dir)
            .stdin(Stdio::null())
            .stdout(writer.try_clone().unwrap())
            .stderr(writer)
            .spawn()
            .unwrap();

        let log = Arc::new(Mutex::new(String::new()));
        let paused = Arc::new(AtomicBool::new(false));
        let reader = {
            let log = Arc::clone(&log);
            let paused = Arc::clone(&paused);
            thread::spawn(move || read_log(reader, &log, &paused))
        };

        let mut pid1 = Pid1 {
            dir: dir.to_path_buf(),
            socket,
            process,
            pid: None,
            ns: String::new(),
            log,
            paused,
            reader: Some(reader),
            exited: false,
        };
        let outer = pid1.process.id();
        let deadline = Instant::now() + secs(3);
        if scenario.plain {
            pid1.pid = Some(outer);
        }
        while !scenario.plain && Instant::now() < deadline {
            let child = fs::read_dir("/proc").unwrap().flatten().find_map(|e| {
                let pid: u32 = e.file_name().to_string_lossy().parse().ok()?;
                (parent_of(pid) == Some(outer)).then_some(pid)
            });
            if let Some(pid) = child
                && let Ok(ns) = fs::read_link(format!("/proc/{pid}/ns/pid"))
            {
                pid1.pid = Some(pid);
                pid1.ns = ns.to_string_lossy().into_owned();
                break;
            }
            if pid1.process.try_wait().unwrap().is_some() {
                break;
            }
            thread::sleep(ms(5));
        }
        // Ready for requests once it listens, unless it gave up before. The
        // socket file appears when it is bound, a moment before it listens,
        // and a connection in between is refused; so wait for a connection.
        // A manager that is already listening, as in another_manager, takes
        // this one and closes it.
        while Instant::now() < deadline
            && pid1.pid.is_some()
            && UnixStream::connect(&pid1.socket).is_err()
            && pid1.process.try_wait().unwrap().is_none()
        {
            thread::sleep(ms(5));
        }
        pid1
    }

    /// Another manager on the same folder and socket.
    fn launch_again(&self, scenario_name: &str) -> Pid1 {
        let scenario = scenarios()
            .into_iter()
            .find(|s| s.name == scenario_name)
            .unwrap();
        Pid1::launch(&scenario, &self.dir)
    }

    fn pid(&self) -> u32 {
        self.pid.expect("the manager is running")
    }

    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }

    fn log_lines(&self, prefix: &str) -> Vec<String> {
        self.log()
            .lines()
            .filter(|l| l.starts_with(prefix))
            .map(str::to_string)
            .collect()
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn assert_pid1(&self) {
        let status = fs::read_to_string(format!("/proc/{}/status", self.pid())).unwrap();
        let nspid = status
            .lines()
            .find_map(|l| l.strip_prefix("NSpid:"))
            .unwrap();
        assert_eq!(nspid.split_whitespace().last(), Some("1"), "NSpid {nspid}");
    }

    fn processes(&self) -> Vec<Proc> {
        processes_in(&self.ns)
    }

    fn tagged(&self, tag: &str) -> Vec<Proc> {
        self.processes()
            .into_iter()
            .filter(|p| p.tag() == Some(tag))
            .collect()
    }

    /// Zombies may exist for the moment between an exit and the reap that
    /// follows it; none may last.
    fn assert_no_zombies(&self) {
        let deadline = Instant::now() + secs(1);
        loop {
            let zombies: Vec<Proc> = self
                .processes()
                .into_iter()
                .filter(|p| p.state == 'Z')
                .collect();
            if zombies.is_empty() {
                return;
            }
            assert!(Instant::now() < deadline, "zombies left: {zombies:?}");
            thread::sleep(ms(20));
        }
    }

    fn status(&self) -> StatusReply {
        let asked = Instant::now();
        client::status(&self.socket).unwrap_or_else(|e| {
            panic!(
                "no status after {:?}: {e}\n{}",
                asked.elapsed(),
                self.diagnose()
            );
        })
    }

    /// What the manager's threads are doing, and whether the container ran
    /// out of memory: for a failure that does not say so itself.
    fn diagnose(&self) -> String {
        let mut out = String::new();
        let Some(pid) = self.pid else {
            return out;
        };
        let tasks = fs::read_dir(format!("/proc/{pid}/task"))
            .map(|d| d.flatten().collect::<Vec<_>>())
            .unwrap_or_default();
        for task in tasks {
            let read = |name: &str| {
                fs::read_to_string(task.path().join(name))
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            let stat = read("stat");
            let state = stat
                .get(stat.rfind(')').map_or(0, |i| i + 2)..)
                .unwrap_or("");
            out.push_str(&format!(
                "task {} {}: state {}, wchan {}, syscall {}\n",
                task.file_name().to_string_lossy(),
                read("comm"),
                state.split(' ').next().unwrap_or("?"),
                read("wchan"),
                read("syscall")
            ));
        }
        out.push_str(&format!(
            "memory.events: {}",
            fs::read_to_string("/sys/fs/cgroup/memory.events").unwrap_or_default()
        ));
        out
    }

    fn service(&self, name: &str) -> ServiceLine {
        self.status()
            .services
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    fn wait_until(
        &self,
        what: &str,
        timeout: Duration,
        done: impl Fn(&StatusReply) -> bool,
    ) -> StatusReply {
        let deadline = Instant::now() + timeout;
        loop {
            let status = self.status();
            if done(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "{what} did not happen within {timeout:?}: {status:?}"
            );
            thread::sleep(ms(50));
        }
    }

    fn wait_ready(&self, names: &[&str], timeout: Duration) -> StatusReply {
        self.wait_until("readiness", timeout, |status| {
            names.iter().all(|n| {
                status
                    .services
                    .iter()
                    .any(|s| s.name == *n && s.state == State::Ready)
            })
        })
    }

    fn cli(&self, args: &[&str]) -> Cli {
        self.cli_with(args, |_| {})
    }

    fn cli_with(&self, args: &[&str], setup: impl FnOnce(&mut Command)) -> Cli {
        let mut command = Command::new(CLI);
        command.arg("--socket").arg(&self.socket).args(args);
        setup(&mut command);
        let started = Instant::now();
        let output = command.output().unwrap();
        Cli {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            took: started.elapsed(),
        }
    }

    fn signal(&self, signal: i32) {
        sys::kill(self.pid(), signal, false).unwrap();
    }

    /// Waits for the manager to exit and returns its status and how long
    /// that took from `since`. Its namespace must be empty then.
    fn wait_exit(&mut self, since: Instant, timeout: Duration) -> (i32, Duration) {
        loop {
            if let Some(status) = self.process.try_wait().unwrap() {
                let took = since.elapsed();
                self.exited = true;
                let code = status.code().unwrap_or_else(|| {
                    128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0)
                });
                if !self.ns.is_empty() {
                    let deadline = Instant::now() + secs(1);
                    while !self.processes().is_empty() {
                        assert!(
                            Instant::now() < deadline,
                            "processes outlived the manager: {:?}",
                            self.processes()
                        );
                        thread::sleep(ms(10));
                    }
                }
                return (code, took);
            }
            assert!(
                since.elapsed() < timeout,
                "the manager did not exit within {timeout:?}"
            );
            thread::sleep(ms(5));
        }
    }

    /// Sends SIGTERM and expects exit status 0 within `within`.
    fn stop(&mut self, within: Duration) -> Duration {
        self.stop_with(libc::SIGTERM, within)
    }

    /// Sends `signal` and expects exit status 0 within `within`.
    fn stop_with(&mut self, signal: i32, within: Duration) -> Duration {
        self.assert_no_zombies();
        let sent = Instant::now();
        self.signal(signal);
        let (code, took) = self.wait_exit(sent, within + secs(5));
        assert_eq!(code, 0, "the manager's exit status");
        assert!(
            took <= within,
            "the shutdown took {took:?}, more than {within:?}"
        );
        took
    }

    fn pause_log(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }
}

impl Drop for Pid1 {
    fn drop(&mut self) {
        self.pause_log(false);
        if !self.exited {
            // --kill-child takes the manager and its namespace with it.
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Keeps the manager's lines and the helper's complaints, and drops the rest:
/// a flooding service writes megabytes a second.
fn read_log(mut pipe: std::io::PipeReader, log: &Mutex<String>, paused: &AtomicBool) {
    let verbose = std::env::var_os("PID1_VERBOSE").is_some();
    let mut buf = vec![0u8; 64 * 1024];
    let mut partial: Vec<u8> = Vec::new();
    loop {
        while paused.load(Ordering::SeqCst) {
            thread::sleep(ms(10));
        }
        let n = match pipe.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        partial.extend_from_slice(&buf[..n]);
        while let Some(end) = partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = partial.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if line.starts_with("stellarr-init: ") || line.starts_with("misbehave: ") {
                if verbose {
                    eprint!("    {line}");
                }
                log.lock().unwrap().push_str(&line);
            }
        }
        if partial.len() > 64 * 1024 {
            partial.clear();
        }
    }
}

fn ask_raw(socket: &Path, bytes: &[u8], half_close: bool) -> String {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(secs(4))).unwrap();
    stream.write_all(bytes).unwrap();
    if half_close {
        stream.shutdown(std::net::Shutdown::Write).unwrap();
    }
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    reply
}

/// The waits the manager announced for `name`, from lines such as
/// "a: exited with status 1 after 0.2s; next try in 0.4s".
fn next_tries(pid1: &Pid1, name: &str) -> Vec<String> {
    pid1.log_lines(&format!("stellarr-init: {name}: "))
        .iter()
        .filter_map(|l| l.split("; next try in ").nth(1).map(str::to_string))
        .collect()
}

fn report(path: &Path) -> Vec<(String, String)> {
    let deadline = Instant::now() + secs(3);
    while !path.exists() {
        assert!(Instant::now() < deadline, "no report at {}", path.display());
        thread::sleep(ms(20));
    }
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.split_once(' ').unwrap_or((l, ""));
            (k.to_string(), v.to_string())
        })
        .collect()
}

fn field<'a>(report: &'a [(String, String)], key: &str) -> &'a str {
    &report.iter().find(|(k, _)| k == key).unwrap().1
}

// The scenarios. Each pairs a service table with what is checked against it.

fn spawn_environment(dir: &Path) -> Vec<Service> {
    vec![
        helper("env", &["--report", &path(dir, "env-report")])
            .user("nobody")
            .dir("/tmp")
            .env("TEST_VALUE", "42")
            .critical(),
        helper("rooted", &["--report", &path(dir, "root-report")]),
        // The helper is a Rust program, whose runtime ignores SIGPIPE before
        // main; cat under sh shows the signal state as the manager left it.
        Service::new(
            "shell",
            &[
                "/bin/sh",
                "-c",
                &format!(
                    "cat /proc/self/status > {}; exec sleep 3600",
                    path(dir, "shell-status")
                ),
            ],
        )
        .ready(Readiness::Running(ms(200))),
    ]
}

fn check_spawn_environment(t: &mut Pid1) {
    t.assert_pid1();
    t.wait_ready(&["env", "rooted", "shell"], secs(3));

    // A service ignores what the manager was started ignoring, less SIGPIPE,
    // which the manager's own runtime ignores, and nothing else. Under cargo
    // the manager is started with signal 32 ignored, the C library's own.
    let mask = |text: &str, key: &str| -> u64 {
        let line = text.lines().find(|l| l.starts_with(key)).unwrap();
        u64::from_str_radix(line[key.len()..].trim(), 16).unwrap()
    };
    let shell = fs::read_to_string(t.file("shell-status")).unwrap();
    let manager = fs::read_to_string(format!("/proc/{}/status", t.pid())).unwrap();
    let sigpipe = 1u64 << (libc::SIGPIPE - 1);
    assert_ne!(
        mask(&manager, "SigBlk:"),
        0,
        "the manager blocks its signals"
    );
    assert_eq!(mask(&shell, "SigBlk:"), 0, "the mask is cleared");
    assert_eq!(
        mask(&shell, "SigIgn:"),
        mask(&manager, "SigIgn:") & !sigpipe,
        "only what the manager inherited is ignored"
    );

    let env = report(&t.file("env-report"));
    let pid = field(&env, "pid");
    assert_eq!(field(&env, "uid"), "65534\t65534\t65534\t65534");
    assert_eq!(field(&env, "gid"), "65534\t65534\t65534\t65534");
    assert_eq!(
        field(&env, "groups"),
        "",
        "supplementary groups are dropped"
    );
    assert_eq!(
        field(&env, "sigblk"),
        "0000000000000000",
        "the mask is cleared"
    );
    assert_eq!(field(&env, "pgid"), pid, "a process group of its own");
    assert_eq!(field(&env, "cwd"), "/tmp");
    assert_eq!(field(&env, "env"), "TEST_VALUE=42");
    let fds: Vec<&str> = field(&env, "fds").split(' ').collect();
    assert_eq!(fds.len(), 3, "only stdin, stdout and stderr: {fds:?}");
    assert_eq!(fds[0], "0=/dev/null");
    assert!(
        fds[1].starts_with("1=pipe:") && fds[2].starts_with("2=pipe:"),
        "{fds:?}"
    );

    let root = report(&t.file("root-report"));
    assert!(field(&root, "uid").starts_with("0\t"));
    assert_eq!(field(&root, "pgid"), field(&root, "pid"));
    assert_eq!(field(&root, "sigblk"), "0000000000000000");
    assert_eq!(field(&root, "fds").split(' ').count(), 3);

    let health = t.cli(&["health"]);
    assert_eq!((health.code, health.stdout.as_str()), (0, "healthy\n"));
    t.stop(secs(2));
}

fn exits_at_once(_: &Path) -> Vec<Service> {
    vec![
        helper("crash", &["--exit", "3"]).critical(),
        helper("steady", &[]).critical(),
    ]
}

fn check_exits_at_once(t: &mut Pid1) {
    let status = t.wait_until("five restarts", secs(10), |s| s.services[0].restarts >= 5);
    let steady = &status.services[1];
    assert_eq!((steady.state, steady.restarts), (State::Ready, 0));
    assert_eq!(status.manager.phase, Phase::Running);

    let waits = next_tries(t, "crash");
    assert_eq!(waits[..5], ["0.2s", "0.4s", "0.8s", "1.6s", "1.6s"]);

    let health = t.cli(&["health"]);
    assert_eq!(health.code, 1);
    assert!(health.stderr.contains("crash is"), "{}", health.stderr);
    t.stop(secs(2));
}

fn exits_three_times(dir: &Path) -> Vec<Service> {
    vec![
        helper(
            "flaky",
            &["--fail-first", "3", "--count", &path(dir, "count")],
        )
        .critical(),
    ]
}

fn check_exits_three_times(t: &mut Pid1) {
    let status = t.wait_ready(&["flaky"], secs(5));
    assert_eq!(status.services[0].restarts, 3);
    assert_eq!(next_tries(t, "flaky"), ["0.2s", "0.4s", "0.8s"]);
    assert_eq!(t.cli(&["health"]).code, 0);
    // Ctrl-C in a terminal: the same shutdown as SIGTERM.
    t.stop_with(libc::SIGINT, secs(2));
    assert!(t.log().contains("shutting down on SIGINT"));
}

fn crashes_after_a_while(_: &Path) -> Vec<Service> {
    vec![
        // Each run outlasts reset_after, so each wait is the first one.
        helper("long", &["--exit", "1", "--after", "700"]).backoff(Backoff {
            reset_after: ms(500),
            ..fast()
        }),
        helper("short", &["--exit", "1", "--after", "300"]),
    ]
}

fn check_crashes_after_a_while(t: &mut Pid1) {
    t.wait_until("four restarts each", secs(12), |s| {
        s.services.iter().all(|s| s.restarts >= 4)
    });
    let long = next_tries(t, "long");
    assert!(long.iter().all(|w| w == "0.2s"), "{long:?}");
    let short = next_tries(t, "short");
    assert_eq!(short[..4], ["0.2s", "0.4s", "0.8s", "1.6s"]);
    // They were ready between the crashes.
    let readies = t.log_lines("stellarr-init: long: ready").len();
    assert!(readies >= 4, "{readies}");
    t.assert_no_zombies();
    t.stop(secs(2));
}

fn slow_to_become_ready(dir: &Path) -> Vec<Service> {
    vec![
        helper("slow", &["--http", "21050", "--ready-after", "1500"])
            .ready(Readiness::http(lo(21050), "/ready"))
            .critical(),
        helper("after", &["--report", &path(dir, "after")]).needs(&["slow"]),
    ]
}

fn check_slow_to_become_ready(t: &mut Pid1) {
    thread::sleep(ms(500));
    let status = t.status();
    assert_eq!(status.services[0].state, State::Starting);
    assert_eq!(status.services[1].state, State::Waiting);
    assert_eq!(t.cli(&["health"]).code, 1);

    t.wait_ready(&["slow", "after"], secs(6));
    let log = t.log();
    let ready = log.find("slow: ready after").unwrap();
    let started = log.find("after: started").unwrap();
    assert!(ready < started, "after started before slow was ready");
    let rest = &log[ready + "slow: ready after ".len()..];
    let took: f64 = rest[..rest.find('s').unwrap()].parse().unwrap();
    assert!(took >= 1.4, "ready after {took}s");
    assert_eq!(t.cli(&["health"]).code, 0);
    t.stop(secs(2));
}

fn never_ready(dir: &Path) -> Vec<Service> {
    vec![
        helper("stuck", &["--http", "21060", "--status", "503"])
            .ready(Readiness::http(lo(21060), "/"))
            .critical(),
        helper("blocked", &["--report", &path(dir, "blocked")]).needs(&["stuck"]),
        helper("other", &[]),
    ]
}

fn check_never_ready(t: &mut Pid1) {
    thread::sleep(secs(3));
    let status = t.status();
    let states: Vec<State> = status.services.iter().map(|s| s.state).collect();
    assert_eq!(states, [State::Starting, State::Waiting, State::Ready]);
    let health = t.cli(&["health"]);
    assert_eq!(health.code, 1);
    assert!(
        health.stderr.contains("stuck is starting"),
        "{}",
        health.stderr
    );
    let status = t.cli(&["status"]);
    assert!(status.took < ms(500), "status took {:?}", status.took);
    t.stop(secs(2));
    assert!(!t.file("blocked").exists(), "blocked was started");
}

fn ignores_sigterm(_: &Path) -> Vec<Service> {
    vec![
        helper("stubborn", &["--ignore-term"]).stop(Signal::Term, secs(1)),
        helper("polite", &[]),
    ]
}

fn check_ignores_sigterm(t: &mut Pid1) {
    t.wait_ready(&["stubborn", "polite"], secs(3));
    let took = t.stop(secs(3));
    assert!(
        took >= ms(900),
        "stopped after {took:?}, before the deadline"
    );
    assert!(
        t.log()
            .contains("stubborn: still running 1s after SIGTERM; SIGKILL sent")
    );
}

fn family(_: &Path) -> Vec<Service> {
    vec![helper("family", &["--ignore-term", "--children", "3"]).stop(Signal::Term, secs(1))]
}

fn check_family(t: &mut Pid1) {
    t.wait_ready(&["family"], secs(3));
    let deadline = Instant::now() + secs(3);
    while t.tagged("child").len() < 3 {
        assert!(Instant::now() < deadline, "the children did not start");
        thread::sleep(ms(20));
    }
    let before: Vec<u32> = t
        .processes()
        .iter()
        .filter(|p| matches!(p.tag(), Some("family" | "child")))
        .map(|p| p.pid)
        .collect();
    assert_eq!(before.len(), 4);

    let restart = t.cli(&["restart", "family"]);
    assert_eq!(
        (restart.code, restart.stdout.as_str()),
        (0, "restarting family\n")
    );
    t.wait_until("the restart", secs(5), |s| {
        s.services[0].restarts == 1 && s.services[0].state == State::Ready
    });
    let deadline = Instant::now() + secs(3);
    while t.tagged("child").len() < 3 {
        assert!(Instant::now() < deadline, "the new children did not start");
        thread::sleep(ms(20));
    }
    let after: Vec<u32> = t
        .processes()
        .iter()
        .filter(|p| matches!(p.tag(), Some("family" | "child")))
        .map(|p| p.pid)
        .collect();
    assert_eq!(after.len(), 4, "exactly the new family is left");
    assert!(after.iter().all(|p| !before.contains(p)));
    t.assert_no_zombies();

    let took = t.stop(secs(3));
    assert!(took >= ms(900));
}

fn orphans(_: &Path) -> Vec<Service> {
    let once = Backoff {
        initial: secs(20),
        max: secs(20),
        reset_after: secs(60),
    };
    vec![
        helper("stays", &["--orphan", "--exit", "0", "--after", "300"]).backoff(once),
        helper(
            "escapes",
            &["--orphan", "--escape", "--exit", "0", "--after", "300"],
        )
        .backoff(once),
    ]
}

fn check_orphans(t: &mut Pid1) {
    // A grandchild each; the parents exit after 300 ms.
    let deadline = Instant::now() + secs(2);
    while t.tagged("grandchild").len() < 2 {
        assert!(Instant::now() < deadline, "no grandchildren");
        thread::sleep(ms(10));
    }
    t.wait_until("both parents exited", secs(3), |s| {
        s.services.iter().all(|s| s.state == State::Backoff)
    });
    // The one left in its parent's group went with the group.
    thread::sleep(ms(200));
    let left = t.tagged("grandchild");
    assert_eq!(left.len(), 1, "{left:?}");
    assert_ne!(left[0].state, 'Z');
    assert_eq!(parent_of(left[0].pid), t.pid, "the orphan belongs to PID 1");
    // The other ends on its own and is reaped.
    let deadline = Instant::now() + secs(3);
    while !t.tagged("grandchild").is_empty() {
        assert!(Instant::now() < deadline, "{:?}", t.tagged("grandchild"));
        thread::sleep(ms(20));
    }
    t.assert_no_zombies();
    t.stop(secs(2));
}

fn flood(dir: &Path) -> Vec<Service> {
    vec![
        helper("flood", &["--flood"]).critical(),
        helper("steady", &["--on-hup", &path(dir, "hups")])
            .reload(Signal::Hup)
            .critical(),
    ]
}

fn check_flood(t: &mut Pid1) {
    t.wait_ready(&["flood", "steady"], secs(3));
    // Nobody reads the container's output any more; the pipe fills.
    t.pause_log(true);
    thread::sleep(secs(1));
    for _ in 0..5 {
        let status = t.cli(&["status"]);
        assert_eq!(status.code, 0);
        assert!(status.took < secs(1), "status took {:?}", status.took);
    }
    assert_eq!(t.cli(&["health"]).code, 0);
    assert_eq!(t.cli(&["reload", "steady"]).code, 0);
    let deadline = Instant::now() + secs(3);
    while fs::read_to_string(t.file("hups"))
        .unwrap_or_default()
        .is_empty()
    {
        assert!(Instant::now() < deadline, "no reload");
        thread::sleep(ms(20));
    }
    assert_eq!(t.cli(&["restart", "steady"]).code, 0);
    t.wait_until("the restart", secs(4), |s| {
        s.services[1].restarts == 1 && s.services[1].state == State::Ready
    });
    // The shutdown cannot wait for the log either.
    t.stop(secs(3));
    t.pause_log(false);
}

fn exits_zero(_: &Path) -> Vec<Service> {
    vec![helper("done", &["--exit", "0", "--after", "300"])]
}

fn check_exits_zero(t: &mut Pid1) {
    let status = t.wait_until("two restarts", secs(6), |s| s.services[0].restarts >= 2);
    assert_eq!(status.manager.phase, Phase::Running);
    let exits = t.log_lines("stellarr-init: done: exited with status 0 after 0.");
    assert!(exits.len() >= 2, "{exits:?}");
    assert!(exits[0].ends_with("; next try in 0.2s"), "{exits:?}");
    t.stop(secs(2));
}

fn critical(dir: &Path) -> Vec<Service> {
    let voice = path(dir, "voice");
    vec![
        Service::new("voice", &[voice.as_str(), "--tag", "voice"])
            .ready(Readiness::Running(ms(200)))
            .backoff(fast())
            .critical(),
        helper("chat", &[]).critical(),
    ]
}

fn check_critical(t: &mut Pid1) {
    t.wait_until("a few failed starts", secs(5), |s| {
        s.services[0].restarts >= 2 && s.services[1].state == State::Ready
    });
    let health = t.cli(&["health"]);
    assert_eq!(health.code, 1);
    assert!(health.stderr.contains("voice is"), "{}", health.stderr);
    assert!(health.took < secs(1));
    assert!(t.log().contains(&format!(
        "voice: cannot start: {}: No such file",
        path(&t.dir, "voice")
    )));

    symlink(HELPER, t.file("voice")).unwrap();
    t.wait_ready(&["voice"], secs(4));
    let health = t.cli(&["health"]);
    assert_eq!((health.code, health.stdout.as_str()), (0, "healthy\n"));
    assert_eq!(t.service("chat").restarts, 0, "chat kept running");
    t.stop(secs(2));
}

fn restart_and_reload(dir: &Path) -> Vec<Service> {
    vec![
        helper("web", &["--on-hup", &path(dir, "hups"), "--tcp", "21130"])
            .ready(Readiness::Tcp(lo(21130)))
            .reload(Signal::Hup)
            .critical(),
        helper("plain", &[]),
    ]
}

fn check_restart_and_reload(t: &mut Pid1) {
    t.wait_ready(&["web", "plain"], secs(3));
    let first = t.service("web").pid.unwrap();

    let reload = t.cli(&["reload", "web"]);
    assert_eq!(
        (reload.code, reload.stdout.as_str()),
        (0, "reloading web\n")
    );
    let deadline = Instant::now() + secs(3);
    while fs::read_to_string(t.file("hups")).unwrap_or_default() != "hup\n" {
        assert!(Instant::now() < deadline, "no reload");
        thread::sleep(ms(20));
    }
    let refused = t.cli(&["reload", "plain"]);
    assert_eq!(refused.code, 1);
    assert!(
        refused.stderr.contains("plain cannot reload"),
        "{}",
        refused.stderr
    );

    let restart = t.cli(&["restart", "web"]);
    assert_eq!(
        (restart.code, restart.stdout.as_str()),
        (0, "restarting web\n")
    );
    let status = t.wait_until("the restart", secs(4), |s| {
        s.services[0].state == State::Ready && s.services[0].restarts == 1
    });
    assert_ne!(status.services[0].pid, Some(first));
    assert!(t.log().contains("web: exited with status 0"));
    assert_eq!(t.service("plain").restarts, 0);

    let unknown = t.cli(&["restart", "nope"]);
    assert_eq!(unknown.code, 1);
    assert!(unknown.stderr.contains("there is no service named nope"));

    let json = t.cli(&["status", "--json"]);
    assert_eq!(json.code, 0);
    assert!(
        json.stdout
            .starts_with("{\"manager\":{\"phase\":\"running\",\"pid\":1},"),
        "{}",
        json.stdout
    );
    assert!(
        json.stdout
            .contains("{\"name\":\"web\",\"state\":\"ready\"")
    );
    let table = t.cli(&["status"]);
    assert_eq!(table.code, 0);
    assert!(
        table
            .stdout
            .lines()
            .any(|l| l.starts_with("web ") && l.contains("ready"))
    );

    // Someone other than root: the socket file stops them first, and the
    // manager when the file would let them through.
    let as_nobody = |c: &mut Command| {
        c.uid(65534).gid(65534);
    };
    let denied = t.cli_with(&["health"], as_nobody);
    assert_eq!(denied.code, 1);
    assert!(
        denied.stderr.contains("Permission denied"),
        "{}",
        denied.stderr
    );
    fs::set_permissions(&t.socket, fs::Permissions::from_mode(0o666)).unwrap();
    let denied = t.cli_with(&["restart", "web"], as_nobody);
    assert_eq!(denied.code, 1);
    assert!(
        denied.stderr.contains("only root may use this socket"),
        "{}",
        denied.stderr
    );
    fs::set_permissions(&t.socket, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(t.service("web").restarts, 1);
    t.stop(secs(2));
}

fn socket_clients(_: &Path) -> Vec<Service> {
    vec![helper("one", &[]).critical()]
}

fn check_socket_clients(t: &mut Pid1) {
    t.wait_ready(&["one"], secs(3));

    // One client sends nothing; another is answered meanwhile.
    let idle_since = Instant::now();
    let mut idle = UnixStream::connect(&t.socket).unwrap();
    let started = Instant::now();
    let reply = ask_raw(&t.socket, b"status\n", false);
    assert!(
        reply.starts_with("ok\nmanager phase=running pid=1\n"),
        "{reply}"
    );
    assert!(started.elapsed() < ms(300));
    // The idle one is closed when its time is up.
    idle.set_read_timeout(Some(secs(5))).unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(idle.read(&mut buf).unwrap(), 0);
    let waited = idle_since.elapsed();
    assert!(
        waited >= ms(1900) && waited < ms(3500),
        "closed after {waited:?}"
    );

    let garbage = ask_raw(&t.socket, &[0xff, 0x00, 0x13, b'\n'], false);
    assert_eq!(garbage, "error a request is ASCII text\n");
    let long = ask_raw(&t.socket, &[b'a'; 300], false);
    assert_eq!(long, "error a request is one line of at most 256 bytes\n");
    let unknown = ask_raw(&t.socket, b"stop one\n", false);
    assert!(unknown.starts_with("error unknown request"), "{unknown}");
    let unfinished = ask_raw(&t.socket, b"health", true);
    assert_eq!(unfinished, "ok\n");
    let empty = ask_raw(&t.socket, b"", true);
    assert_eq!(empty, "");

    // More idle clients than the manager serves at once.
    let mut crowd: Vec<UnixStream> = (0..20)
        .map(|_| UnixStream::connect(&t.socket).unwrap())
        .collect();
    let mut busy = 0;
    for client in &mut crowd {
        client.set_read_timeout(Some(ms(300))).unwrap();
        let mut reply = String::new();
        let _ = client.read_to_string(&mut reply);
        if reply == "error busy; try again\n" {
            busy += 1;
        }
    }
    assert_eq!(busy, 4, "the clients past sixteen are turned away");
    drop(crowd);
    thread::sleep(ms(100));
    assert_eq!(t.cli(&["health"]).code, 0);
    t.stop(secs(2));
}

fn startup(dir: &Path) -> Vec<Service> {
    vec![
        helper("slow", &["--http", "21150", "--ready-after", "5000"])
            .ready(Readiness::http(lo(21150), "/"))
            .critical(),
        helper("after", &["--report", &path(dir, "after")]).needs(&["slow"]),
    ]
}

fn check_sigterm_during_startup(t: &mut Pid1) {
    thread::sleep(secs(1));
    assert_eq!(t.service("slow").state, State::Starting);
    t.stop(ms(1500));
    assert!(!t.file("after").exists());
    assert!(t.log().contains("after: stopped"));
}

fn backoff(_: &Path) -> Vec<Service> {
    vec![helper("crash", &["--exit", "1"]).backoff(Backoff {
        initial: secs(5),
        max: secs(5),
        reset_after: secs(60),
    })]
}

fn check_sigterm_during_backoff(t: &mut Pid1) {
    t.wait_until("the backoff", secs(3), |s| {
        s.services[0].state == State::Backoff
    });
    thread::sleep(ms(500));
    t.stop(secs(1));
    assert_eq!(t.log_lines("stellarr-init: crash: started").len(), 1);
}

fn during_restart(_: &Path) -> Vec<Service> {
    vec![
        helper("stubborn", &["--ignore-term"]).stop(Signal::Term, secs(2)),
        helper("other", &[]),
    ]
}

fn check_sigterm_during_restart(t: &mut Pid1) {
    t.wait_ready(&["stubborn", "other"], secs(3));
    assert_eq!(t.cli(&["restart", "stubborn"]).code, 0);
    thread::sleep(ms(300));
    assert_eq!(t.service("stubborn").state, State::Stopping);
    // stubborn is killed 2 s after the restart, about 1.7 s from here.
    let took = t.stop(secs(4));
    assert!(took >= ms(1400), "{took:?}");
    assert_eq!(t.log_lines("stellarr-init: stubborn: started").len(), 1);
    assert!(t.log().contains("stubborn: killed by SIGKILL"));
}

fn deadline(_: &Path) -> Vec<Service> {
    vec![
        helper("a", &["--ignore-term"]).stop(Signal::Term, secs(3)),
        helper("b", &["--ignore-term"]).stop(Signal::Term, secs(3)),
    ]
}

fn check_shutdown_deadline(t: &mut Pid1) {
    t.wait_ready(&["a", "b"], secs(3));
    let took = t.stop(ms(2600));
    assert!(took >= ms(1400), "{took:?}");
    assert!(
        t.log()
            .contains("the shutdown took 1.5s; SIGKILL sent to a, b; exiting")
    );
}

fn cycle(dir: &Path) -> Vec<Service> {
    vec![
        helper("a", &["--report", &path(dir, "a")]).needs(&["b"]),
        helper("b", &[]).needs(&["a"]),
    ]
}

fn check_cycle(t: &mut Pid1) {
    let (code, _) = t.wait_exit(Instant::now(), secs(3));
    assert_eq!(code, 2);
    assert!(t.log().contains(
        "the service table cannot run: the services need each other in a cycle: a needs b needs a"
    ));
    assert!(!t.file("a").exists());
}

fn one_service(_: &Path) -> Vec<Service> {
    vec![helper("one", &[]).critical()]
}

fn check_cannot_listen(t: &mut Pid1) {
    let (code, _) = t.wait_exit(Instant::now(), secs(3));
    assert_eq!(code, 1);
    assert!(t.log().contains("cannot listen on"), "{}", t.log());
    assert!(t.log_lines("stellarr-init: one: started").is_empty());
}

fn check_another_manager(t: &mut Pid1) {
    t.wait_ready(&["one"], secs(3));
    let mut second = t.launch_again("another_manager");
    let (code, _) = second.wait_exit(Instant::now(), secs(3));
    assert_eq!(code, 1);
    assert!(
        second.log().contains("another manager answers on"),
        "{}",
        second.log()
    );
    drop(second);
    assert_eq!(t.cli(&["health"]).code, 0);
    t.stop(secs(2));
}

fn check_health_without_an_answer(t: &mut Pid1) {
    t.wait_ready(&["one"], secs(3));

    let missing = Command::new(CLI)
        .args(["--socket", &path(&t.dir, "missing.sock"), "health"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot reach the manager"));

    let stale = t.file("stale.sock");
    drop(UnixListener::bind(&stale).unwrap());
    let started = Instant::now();
    let refused = Command::new(CLI)
        .args(["--socket", &stale.display().to_string(), "health"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(started.elapsed() < secs(1));

    // A manager that is there but stuck.
    t.signal(libc::SIGSTOP);
    let stuck = t.cli(&["health"]);
    t.signal(libc::SIGCONT);
    assert_eq!(stuck.code, 1, "{stuck:?}");
    assert!(
        stuck.took >= ms(1500) && stuck.took < ms(3500),
        "{:?}",
        stuck.took
    );
    assert!(stuck.stderr.contains("no answer"), "{}", stuck.stderr);

    assert_eq!(t.cli(&["health"]).code, 0);
    t.stop(secs(2));
}

fn check_trickle(t: &mut Pid1) {
    t.wait_ready(&["one"], secs(3));
    // Something that accepts and then sends a byte now and then, never a
    // whole reply: no read times out, and only the client's own deadline
    // ends the wait.
    let path = t.file("trickle.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        for _ in 0..10 {
            if stream.write_all(b"o").is_err() {
                return;
            }
            thread::sleep(ms(500));
        }
    });
    let started = Instant::now();
    let output = Command::new(CLI)
        .args(["--socket", &path.display().to_string(), "health"])
        .output()
        .unwrap();
    let took = started.elapsed();
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("did not answer in time"), "{stderr}");
    assert!(took >= ms(2800) && took < ms(4500), "{took:?}");
    t.stop(secs(2));
}

fn stun(_: &Path) -> Vec<Service> {
    vec![
        helper("relay", &["--stun", "21230", "--ready-after", "1000"])
            .ready(Readiness::Stun(lo(21230)))
            .critical(),
        helper("after", &[]).needs(&["relay"]),
    ]
}

fn check_stun(t: &mut Pid1) {
    thread::sleep(ms(500));
    assert_eq!(t.service("relay").state, State::Starting);
    assert_eq!(t.service("after").state, State::Waiting);
    assert_eq!(t.cli(&["health"]).code, 1);
    t.wait_ready(&["relay", "after"], secs(5));
    let prefix = "stellarr-init: relay: ready after ";
    let line = t.log_lines(prefix).pop().unwrap();
    let took: f64 = line[prefix.len()..].trim_end_matches('s').parse().unwrap();
    assert!(took >= 0.9, "ready after {took}s, before the port opened");
    assert_eq!(t.cli(&["health"]).code, 0);
    t.stop(secs(2));
}

fn orphan_outside(_: &Path) -> Vec<Service> {
    let once = Backoff {
        initial: secs(20),
        max: secs(20),
        reset_after: secs(60),
    };
    vec![
        helper(
            "parent",
            &["--orphan", "--escape", "--exit", "0", "--after", "200"],
        )
        .backoff(once),
    ]
}

fn check_not_pid1(t: &mut Pid1) {
    let manager = t.pid();
    t.wait_until("the parent's exit", secs(3), |s| {
        s.services[0].state == State::Backoff
    });
    let note = format!("running as PID {manager}, not 1; orphans come here");
    assert!(t.log().contains(&note), "{}", t.log());
    // A subreaper takes the orphan as PID 1 would.
    let orphans: Vec<Proc> = processes_where(|_| true)
        .into_iter()
        .filter(|p| p.tag() == Some("grandchild"))
        .collect();
    assert_eq!(orphans.len(), 1, "{orphans:?}");
    assert_eq!(parent_of(orphans[0].pid), Some(manager));
    // It ends on its own and is reaped: no child of the manager is left.
    let deadline = Instant::now() + secs(3);
    loop {
        let left: Vec<Proc> = processes_where(|_| true)
            .into_iter()
            .filter(|p| parent_of(p.pid) == Some(manager))
            .collect();
        if left.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "left behind: {left:?}");
        thread::sleep(ms(20));
    }
    t.stop(secs(2));
}

fn stalls(dir: &Path) -> Vec<Service> {
    vec![
        helper(
            "api",
            &["--http", "21250", "--stall-file", &path(dir, "stall")],
        )
        .ready(Readiness::http(lo(21250), "/health"))
        // Each miss takes 0.8 s: a check of up to 0.5 s, then 0.3 s apart.
        .watch(Watch {
            interval: ms(300),
            timeout: ms(500),
            misses: 3,
        })
        .critical(),
        helper("other", &[]).critical(),
    ]
}

fn check_stalls(t: &mut Pid1) {
    t.wait_ready(&["api", "other"], secs(3));
    let pid = t.service("api").pid;
    let stall = t.file("stall");

    // A pause shorter than three checks is no failure.
    fs::write(&stall, "").unwrap();
    thread::sleep(ms(700));
    fs::remove_file(&stall).unwrap();
    let until = Instant::now() + ms(2500);
    while Instant::now() < until {
        assert_eq!(
            t.service("api").state,
            State::Ready,
            "a short pause flapped"
        );
        thread::sleep(ms(50));
    }

    // Alive and silent: failing, and left running.
    fs::write(&stall, "").unwrap();
    t.wait_until("the failure", secs(6), |s| {
        s.services[0].state == State::Failing
    });
    let health = t.cli(&["health"]);
    assert_eq!(health.code, 1);
    assert!(
        health.stderr.contains("api is failing"),
        "{}",
        health.stderr
    );
    let api = t.service("api");
    assert_eq!((api.pid, api.restarts), (pid, 0));
    assert_eq!(t.service("other").state, State::Ready);

    // Answering again: ready, the same process.
    fs::remove_file(&stall).unwrap();
    t.wait_until("the recovery", secs(4), |s| {
        s.services[0].state == State::Ready
    });
    assert_eq!(t.cli(&["health"]).code, 0);
    let api = t.service("api");
    assert_eq!((api.pid, api.restarts), (pid, 0));
    let log = t.log();
    assert!(log.contains("api: failed its check 3 times in a row; failing, and left running"));
    assert!(log.contains("api: answers its check again; ready"));
    t.stop(secs(2));
}

/// A service that is restarted when it stops answering, as the media server
/// is. Each miss takes 0.8 s, as in `stalls`.
fn restarted_on_stall(name: &str, port: u16, stall: &str, args: &[&str]) -> Service {
    let port_text = port.to_string();
    let mut command = vec!["--http", &port_text, "--stall-file", stall];
    command.extend_from_slice(args);
    helper(name, &command)
        .ready(Readiness::http(lo(port), "/health"))
        .watch(Watch {
            interval: ms(300),
            timeout: ms(500),
            misses: 3,
        })
        .stop(Signal::Term, secs(1))
        .restart_failing()
        .critical()
}

fn restarted(dir: &Path) -> Vec<Service> {
    let stall = path(dir, "stall");
    vec![
        restarted_on_stall("heeds", 21251, &stall, &[]),
        restarted_on_stall("stubborn", 21252, &stall, &["--ignore-term"]),
    ]
}

fn check_restarted(t: &mut Pid1) {
    let names = ["heeds", "stubborn"];
    t.wait_ready(&names, secs(3));
    let old: Vec<Option<u32>> = names.iter().map(|n| t.service(n).pid).collect();
    let stall = t.file("stall");

    // Alive and silent: each is stopped and started again under a new PID,
    // whether it heeds SIGTERM or not. The new run cannot become ready while
    // the stall lasts, so it is not checked, nor restarted again.
    fs::write(&stall, "").unwrap();
    t.wait_until("the restarts", secs(8), |s| {
        s.services
            .iter()
            .zip(&old)
            .all(|(line, old)| line.restarts == 1 && line.pid.is_some() && line.pid != *old)
    });
    fs::remove_file(&stall).unwrap();
    t.wait_ready(&names, secs(4));
    assert_eq!(t.cli(&["health"]).code, 0);
    let log = t.log();
    for name in names {
        assert_eq!(t.service(name).restarts, 1, "{name}");
        // The old process is gone, not only forgotten.
        let left = t.tagged(name);
        assert_eq!(left.len(), 1, "{name}: {left:?}");
        let restarting = format!(
            "{name}: failed its check 3 times in a row; restarting; SIGTERM sent, SIGKILL in 1s"
        );
        assert!(log.contains(&restarting), "{log}");
    }
    assert!(log.contains("heeds: killed by SIGTERM"), "{log}");
    assert!(!log.contains("heeds: still running"), "{log}");
    let killed = t.log_lines("stellarr-init: stubborn: still running");
    assert!(
        killed.len() == 1 && killed[0].ends_with("after SIGTERM; SIGKILL sent"),
        "{log}"
    );
    assert!(log.contains("stubborn: killed by SIGKILL"), "{log}");
    t.stop(secs(2));
}
