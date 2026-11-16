//! What a service is: how to start it, when it counts as ready, how to stop
//! it, and what it needs first. [`Table`] checks a list of services and works
//! out the order they start in.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// The signals the manager sends on purpose. Anything else a process dies of
/// is reported by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Hup,
    Int,
    Quit,
    Kill,
    Usr1,
    Usr2,
    Term,
}

impl Signal {
    pub fn number(self) -> i32 {
        match self {
            Signal::Hup => libc::SIGHUP,
            Signal::Int => libc::SIGINT,
            Signal::Quit => libc::SIGQUIT,
            Signal::Kill => libc::SIGKILL,
            Signal::Usr1 => libc::SIGUSR1,
            Signal::Usr2 => libc::SIGUSR2,
            Signal::Term => libc::SIGTERM,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Signal::Hup => "SIGHUP",
            Signal::Int => "SIGINT",
            Signal::Quit => "SIGQUIT",
            Signal::Kill => "SIGKILL",
            Signal::Usr1 => "SIGUSR1",
            Signal::Usr2 => "SIGUSR2",
            Signal::Term => "SIGTERM",
        }
    }
}

/// The name of signal `number`, for the log.
pub fn signal_name(number: i32) -> String {
    let known = [
        (libc::SIGHUP, "SIGHUP"),
        (libc::SIGINT, "SIGINT"),
        (libc::SIGQUIT, "SIGQUIT"),
        (libc::SIGILL, "SIGILL"),
        (libc::SIGTRAP, "SIGTRAP"),
        (libc::SIGABRT, "SIGABRT"),
        (libc::SIGBUS, "SIGBUS"),
        (libc::SIGFPE, "SIGFPE"),
        (libc::SIGKILL, "SIGKILL"),
        (libc::SIGUSR1, "SIGUSR1"),
        (libc::SIGSEGV, "SIGSEGV"),
        (libc::SIGUSR2, "SIGUSR2"),
        (libc::SIGPIPE, "SIGPIPE"),
        (libc::SIGALRM, "SIGALRM"),
        (libc::SIGTERM, "SIGTERM"),
        (libc::SIGXCPU, "SIGXCPU"),
        (libc::SIGXFSZ, "SIGXFSZ"),
        (libc::SIGSYS, "SIGSYS"),
    ];
    match known.iter().find(|(n, _)| *n == number) {
        Some((_, name)) => (*name).to_string(),
        None => format!("signal {number}"),
    }
}

/// How the manager tells that a running service is ready, at start and at
/// every [`Watch`] interval after. Every address is on loopback: the manager
/// checks its own container only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Ready once the process has kept running this long. For a service
    /// that offers nothing better to ask; it is not checked after.
    Running(Duration),
    /// Ready once a TCP connection to this address is accepted.
    Tcp(SocketAddr),
    /// Ready once `GET path` on this address, with these extra request
    /// headers, answers with a 2xx status.
    Http {
        addr: SocketAddr,
        path: String,
        headers: Vec<(String, String)>,
    },
    /// Ready once a STUN binding request to this UDP address gets a success
    /// response.
    Stun(SocketAddr),
}

impl Readiness {
    /// `GET path` on `addr` without extra headers.
    pub fn http(addr: SocketAddr, path: &str) -> Readiness {
        Readiness::Http {
            addr,
            path: path.to_string(),
            headers: Vec::new(),
        }
    }

    fn addr(&self) -> Option<SocketAddr> {
        match self {
            Readiness::Running(_) => None,
            Readiness::Tcp(addr) | Readiness::Stun(addr) => Some(*addr),
            Readiness::Http { addr, .. } => Some(*addr),
        }
    }
}

/// How a ready service keeps being checked: every `interval`, each check
/// given `timeout`. After `misses` failed checks in a row it is reported as
/// failing, and health fails; one check that passes makes it ready again.
/// It is never restarted for it, see [`crate::core`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    pub interval: Duration,
    pub timeout: Duration,
    pub misses: u32,
}

impl Watch {
    /// A service that stops answering is reported after about 45 seconds:
    /// three checks of up to five seconds, ten seconds apart. A machine that
    /// is only slow would have to miss three answers of five seconds each in
    /// a row first.
    pub const DEFAULT: Watch = Watch {
        interval: Duration::from_secs(10),
        timeout: Duration::from_secs(5),
        misses: 3,
    };
}

/// How long the manager waits before it starts a service that exited.
///
/// The first try after a run comes `initial` later; each run that ends
/// before `reset_after` doubles the wait, up to `max`. A run that lasted
/// `reset_after` or longer starts the count again, so a service that crashes
/// once a day comes back after `initial`, not after `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    pub reset_after: Duration,
}

impl Backoff {
    pub const DEFAULT: Backoff = Backoff {
        initial: Duration::from_secs(1),
        max: Duration::from_secs(30),
        reset_after: Duration::from_secs(60),
    };

    /// The wait after `failures` short runs in a row, counting the one that
    /// just ended; one or more.
    pub fn delay(&self, failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(31);
        self.initial.saturating_mul(1u32 << doublings).min(self.max)
    }
}

/// A service as the manager runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// A short name in lower case: letters, digits and dashes.
    pub name: String,
    /// The program, by absolute path, and its arguments. No shell.
    pub command: Vec<String>,
    /// The working directory, or the manager's own.
    pub dir: Option<PathBuf>,
    /// Variables added to the environment the manager was started with.
    pub env: Vec<(String, String)>,
    /// The account to run as, from /etc/passwd; root when unset.
    pub user: Option<String>,
    /// The group to run as, from /etc/group; the user's own when unset.
    pub group: Option<String>,
    /// Services that must be ready before this one starts, every time it
    /// starts. A service that stops being ready later does not stop the
    /// services that need it.
    pub needs: Vec<String>,
    pub ready: Readiness,
    /// How `ready` keeps being checked once the service is ready.
    pub watch: Watch,
    /// The signal that asks the process group to stop.
    pub stop_signal: Signal,
    /// How long after `stop_signal` the group gets SIGKILL.
    pub stop_timeout: Duration,
    /// The signal that makes the main process reload, if it has one.
    pub reload: Option<Signal>,
    /// The container is healthy only while every critical service is ready.
    pub critical: bool,
    pub backoff: Backoff,
}

impl Service {
    /// A service that is ready once it has run for a second, stops on
    /// SIGTERM within ten seconds, cannot reload and is not critical.
    pub fn new(name: &str, command: &[&str]) -> Service {
        Service {
            name: name.to_string(),
            command: command.iter().map(|s| s.to_string()).collect(),
            dir: None,
            env: Vec::new(),
            user: None,
            group: None,
            needs: Vec::new(),
            ready: Readiness::Running(Duration::from_secs(1)),
            watch: Watch::DEFAULT,
            stop_signal: Signal::Term,
            stop_timeout: Duration::from_secs(10),
            reload: None,
            critical: false,
            backoff: Backoff::DEFAULT,
        }
    }

    pub fn dir(mut self, dir: &str) -> Service {
        self.dir = Some(PathBuf::from(dir));
        self
    }

    pub fn env(mut self, name: &str, value: &str) -> Service {
        self.env.push((name.to_string(), value.to_string()));
        self
    }

    pub fn user(mut self, user: &str) -> Service {
        self.user = Some(user.to_string());
        self
    }

    pub fn group(mut self, group: &str) -> Service {
        self.group = Some(group.to_string());
        self
    }

    pub fn needs(mut self, names: &[&str]) -> Service {
        self.needs = names.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn ready(mut self, ready: Readiness) -> Service {
        self.ready = ready;
        self
    }

    pub fn watch(mut self, watch: Watch) -> Service {
        self.watch = watch;
        self
    }

    pub fn stop(mut self, signal: Signal, timeout: Duration) -> Service {
        self.stop_signal = signal;
        self.stop_timeout = timeout;
        self
    }

    pub fn reload(mut self, signal: Signal) -> Service {
        self.reload = Some(signal);
        self
    }

    pub fn critical(mut self) -> Service {
        self.critical = true;
        self
    }

    pub fn backoff(mut self, backoff: Backoff) -> Service {
        self.backoff = backoff;
        self
    }
}

/// Why a table of services cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError(pub String);

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TableError {}

/// A checked list of services with the order they start in.
#[derive(Debug, Clone)]
pub struct Table {
    services: Vec<Service>,
    /// Indexes into `services`, each after everything it needs.
    order: Vec<usize>,
    /// For each service, the indexes of the services it needs.
    needs: Vec<Vec<usize>>,
}

impl Table {
    /// Checks the services and orders them. Of the services whose needs are
    /// met, the one listed first starts first, so a table without needs
    /// starts in the order it is written.
    pub fn new(services: Vec<Service>) -> Result<Table, TableError> {
        if services.is_empty() {
            return Err(TableError("there are no services to run".into()));
        }
        for (i, service) in services.iter().enumerate() {
            check_service(service)?;
            if services[..i].iter().any(|s| s.name == service.name) {
                return Err(TableError(format!(
                    "two services are named {}",
                    service.name
                )));
            }
        }

        let index_of = |name: &str| services.iter().position(|s| s.name == name);
        let mut needs = Vec::with_capacity(services.len());
        for service in &services {
            let mut list = Vec::new();
            for need in &service.needs {
                let Some(j) = index_of(need) else {
                    return Err(TableError(format!(
                        "{} needs {need}, which is not a service",
                        service.name
                    )));
                };
                if need == &service.name {
                    return Err(TableError(format!("{} needs itself", service.name)));
                }
                if list.contains(&j) {
                    return Err(TableError(format!(
                        "{} names {need} twice among its needs",
                        service.name
                    )));
                }
                list.push(j);
            }
            needs.push(list);
        }

        let order = start_order(&services, &needs)?;
        Ok(Table {
            services,
            order,
            needs,
        })
    }

    pub fn services(&self) -> &[Service] {
        &self.services
    }

    /// Indexes of the services in the order they start. They stop in the
    /// reverse order.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// Indexes of the services that service `i` needs.
    pub fn needs(&self, i: usize) -> &[usize] {
        &self.needs[i]
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.services.iter().position(|s| s.name == name)
    }
}

fn check_service(service: &Service) -> Result<(), TableError> {
    let name = &service.name;
    let name_ok = !name.is_empty()
        && name.len() <= 32
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !name_ok {
        return Err(TableError(format!(
            "{name:?} is not a service name: use up to 32 lower-case letters, digits and dashes, starting with a letter"
        )));
    }
    match service.command.first() {
        None => return Err(TableError(format!("{name} has no command"))),
        Some(program) if !program.starts_with('/') => {
            return Err(TableError(format!(
                "{name} runs {program:?}, which is not an absolute path"
            )));
        }
        Some(_) => {}
    }
    if let Some(dir) = &service.dir
        && !dir.is_absolute()
    {
        return Err(TableError(format!(
            "{name} works in {}, which is not an absolute path",
            dir.display()
        )));
    }
    if let Some(addr) = service.ready.addr()
        && !addr.ip().is_loopback()
    {
        return Err(TableError(format!(
            "{name} is checked at {addr}, which is not a loopback address"
        )));
    }
    if let Readiness::Http { path, headers, .. } = &service.ready {
        let text = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic() || b == b' ');
        if !path.starts_with('/') || path.contains(' ') || !text(path) {
            return Err(TableError(format!(
                "{name} is checked at {path:?}, which is not a path"
            )));
        }
        for (header, value) in headers {
            let token = !header.is_empty()
                && header
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-');
            if !token || !text(value) {
                return Err(TableError(format!(
                    "{name} sends the header {header:?}: {value:?}, which cannot go in a request"
                )));
            }
        }
    }
    let watch = service.watch;
    if watch.interval.is_zero() || watch.timeout.is_zero() || watch.misses == 0 {
        return Err(TableError(format!(
            "{name} is watched every {:?} with {:?} per check and {} misses allowed; none of them can be zero",
            watch.interval, watch.timeout, watch.misses
        )));
    }
    if service.stop_timeout.is_zero() {
        return Err(TableError(format!("{name} has no time to stop")));
    }
    let backoff = service.backoff;
    if backoff.initial.is_zero() || backoff.max < backoff.initial {
        return Err(TableError(format!(
            "{name} waits between tries from {:?} up to {:?}; the first must be above zero and not above the second",
            backoff.initial, backoff.max
        )));
    }
    Ok(())
}

/// Orders the services so that each comes after everything it needs, taking
/// the first in table order whenever several could go next. A cycle is an
/// error that names it.
fn start_order(services: &[Service], needs: &[Vec<usize>]) -> Result<Vec<usize>, TableError> {
    let mut order = Vec::with_capacity(services.len());
    let mut placed = vec![false; services.len()];
    while order.len() < services.len() {
        let next = (0..services.len()).find(|&i| !placed[i] && needs[i].iter().all(|&j| placed[j]));
        match next {
            Some(i) => {
                placed[i] = true;
                order.push(i);
            }
            None => return Err(cycle_error(services, needs, &placed)),
        }
    }
    Ok(order)
}

/// Every unplaced service waits on another unplaced one, so following those
/// waits from any of them must come back to a service already seen.
fn cycle_error(services: &[Service], needs: &[Vec<usize>], placed: &[bool]) -> TableError {
    let mut path: Vec<usize> = Vec::new();
    let mut at = (0..services.len())
        .find(|&i| !placed[i])
        .expect("a service is left");
    loop {
        if let Some(start) = path.iter().position(|&i| i == at) {
            let mut names: Vec<&str> = path[start..]
                .iter()
                .map(|&i| services[i].name.as_str())
                .collect();
            names.push(&services[at].name);
            return TableError(format!(
                "the services need each other in a cycle: {}",
                names.join(" needs ")
            ));
        }
        path.push(at);
        at = *needs[at]
            .iter()
            .find(|&&j| !placed[j])
            .expect("an unplaced service waits on an unplaced one");
    }
}
