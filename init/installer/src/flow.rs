//! One run of the installer, from the checks of the machine to the summary.
//! Validate the proposed configuration before publishing files or starting services.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::checks::{
    binding_port, exact_version, is_address, is_exact_version, is_hostname, is_http_binding,
    is_ipv4, is_local_address, is_port, normalize_address, port_number, release_series,
    version_older,
};
use crate::cli::{KEYS, Key, Options, env};
use crate::envfile;
use crate::http::Http;
use crate::install_plan::{self, InstallPlan};
use crate::ports::{Tool, port_numbers};
use crate::sys;
use crate::ui::{Fd, Stop, Ui};

mod deploy;

type Step<T = ()> = Result<T, Stop>;

const REPO_URL: &str = "https://github.com/Arkazzae/Stellarr";
const SERVER_DOWNLOADS: &str = "https://stellarr.online/server-downloads";
const RAW_URL: &str = "https://raw.githubusercontent.com/Arkazzae/Stellarr";
const IMAGE: &str = "ghcr.io/arkazzae/stellarr";
const CHECKOUT_TURN: &str = "infra/compose/turn.yml";
const INSTALLED_TURN: &str = "docker-compose.turn.yml";
const CHECKOUT_BUILD: &str = "infra/compose/build.yml";
const LEGACY_BUILD: &str = "docker-compose.build.yml";
const OVERRIDES: &[&str] = &[
    "compose.override.yml",
    "compose.override.yaml",
    "docker-compose.override.yml",
    "docker-compose.override.yaml",
];
// The documentation will move to its own address; every link starts here.
const DOCS_URL: &str = "https://arkazzae.github.io/Stellarr";
pub const ISSUES_URL: &str = "https://github.com/Arkazzae/Stellarr/issues";
const ENV_MARKER: &str =
    "# Written by the Stellarr installer, which changes only the settings it is given.";
/// The keys of .env the installer reads besides its own settings.
const OTHER_KEYS: &[&str] = &[
    "STELLARR_VERSION",
    "STELLARR_IMAGE",
    "STELLARR_TRACK",
    "STELLARR_TURN_RELAY_RANGE",
    "COMPOSE_PROJECT_NAME",
    "COMPOSE_FILE",
    "COMPOSE_PROFILES",
    "COMPOSE_DISABLE_ENV_FILE",
    "COMPOSE_ENV_FILES",
    "COMPOSE_PATH_SEPARATOR",
];
const NONE: &[&str] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
}

impl Os {
    /// The system this installer runs on; Linux and macOS are the ones it
    /// supports.
    pub fn detect() -> Result<Os, String> {
        let name = if cfg!(target_os = "macos") {
            "Darwin".to_string()
        } else if cfg!(target_os = "linux") {
            "Linux".to_string()
        } else {
            std::env::consts::OS.to_string()
        };
        #[cfg(feature = "test-seams")]
        let name = Some(env("STELLARR_INSTALLER_TEST_OS"))
            .filter(|v| !v.is_empty())
            .unwrap_or(name);
        match name.as_str() {
            "Linux" => Ok(Os::Linux),
            "Darwin" => Ok(Os::MacOs),
            other => Err(format!(
                "The installer supports Linux and macOS, and this machine runs {other}"
            )),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvState {
    None,
    Foreign,
    Marked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvAction {
    New,
    Keep,
    Adopt,
    Replace,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortsResult {
    Free,
    Held,
    Skipped,
    Unknown,
}

/// What Docker knows of the container named stellarr.
#[derive(Default)]
struct Container {
    exists: bool,
    running: bool,
    project: String,
    dir: String,
    version: String,
}

pub struct Run {
    pub ui: Ui,
    o: Options,
    os: Os,
    uid: u32,
    interactive: bool,
    http: Http,
    docker_sudo: bool,
    compose_plugin: bool,
    docker_version: String,
    compose_version: String,
    script_checkout: Option<PathBuf>,
    container: Container,
    install_dir: PathBuf,
    owns_container: bool,
    dir_is_checkout: bool,
    env_state: EnvState,
    exist: BTreeMap<String, String>,
    target_tag: String,
    target_version: String,
    files_ref: String,
    target_track: String,
    target_image: String,
    installed_version: String,
    env_action: EnvAction,
    compose_label: String,
    staged_compose: Option<Vec<u8>>,
    staged_example: Option<Vec<u8>>,
    staged_turn: Option<Vec<u8>>,
    two_port_layout: bool,
    turn_published: bool,
    relay_range: String,
    port_keys: Vec<Key>,
    set_compose_file: String,
    updating: bool,
    ports_result: PortsResult,
    listen_tcp: BTreeSet<u32>,
    listen_udp: BTreeSet<u32>,
    run_stamp: String,
    override_file: Option<&'static str>,
    existing_env: Vec<u8>,
    /// Variables the commands the installer runs must not see.
    removed_env: Vec<String>,
    /// Whether the waiting moves on without waiting, for the tests.
    fast_clock: bool,
}

fn is_stellarr_compose(content: &[u8]) -> bool {
    envfile::records(content).iter().any(|line| {
        let line = String::from_utf8_lossy(line);
        line.strip_prefix("name:")
            .is_some_and(|rest| rest.trim_matches(|c: char| c.is_ascii_whitespace()) == "stellarr")
    })
}

fn is_stellarr_compose_file(path: &Path) -> bool {
    path.is_file() && fs::read(path).is_ok_and(|content| is_stellarr_compose(&content))
}

fn is_checkout(dir: &Path) -> bool {
    dir.join("Dockerfile").is_file() && is_stellarr_compose_file(&dir.join("docker-compose.yml"))
}

/// A path as a shell word: quoted when it holds anything but plain
/// characters.
fn shell_word(word: &str) -> String {
    if word
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// KiB as GiB, with one decimal below 10.
fn gib(kib: u64) -> String {
    let g = kib as f64 / 1_048_576.0;
    if g < 10.0 {
        format!("{g:.1} GiB")
    } else {
        format!("{} GiB", g as u64)
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// `path` made absolute and without symbolic links, as far as the
/// directories in it exist, so two names of one directory compare equal. A
/// leading ~ is the home directory, as --dir=~/x reaches the installer
/// without the shell expanding it.
fn absolute_path(path: &str) -> PathBuf {
    let home = env("HOME");
    let path = if path == "~" {
        PathBuf::from(home)
    } else if let Some(rest) = path.strip_prefix("~/") {
        PathBuf::from(home).join(rest.trim_start_matches('/'))
    } else {
        PathBuf::from(path)
    };
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from(env("PWD")))
            .join(path)
    };
    let mut absolute = PathBuf::from("/");
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                absolute.pop();
            }
            std::path::Component::Normal(name) => {
                absolute.push(name);
                if let Ok(resolved) = fs::canonicalize(&absolute) {
                    absolute = resolved;
                }
            }
            _ => {}
        }
    }
    absolute
}

impl Run {
    pub fn new(ui: Ui, o: Options, os: Os) -> Run {
        #[cfg(feature = "test-seams")]
        let fast_clock = env("STELLARR_INSTALLER_TEST_CLOCK") == "fast";
        #[cfg(not(feature = "test-seams"))]
        let fast_clock = false;
        Run {
            ui,
            o,
            os,
            uid: sys::effective_uid(),
            interactive: false,
            http: Http::new(),
            docker_sudo: false,
            compose_plugin: true,
            docker_version: String::new(),
            compose_version: String::new(),
            script_checkout: None,
            container: Container::default(),
            install_dir: PathBuf::new(),
            owns_container: false,
            dir_is_checkout: false,
            env_state: EnvState::None,
            exist: BTreeMap::new(),
            target_tag: String::new(),
            target_version: String::new(),
            files_ref: String::new(),
            target_track: String::new(),
            target_image: String::new(),
            installed_version: String::new(),
            env_action: EnvAction::New,
            compose_label: String::new(),
            staged_compose: None,
            staged_example: None,
            staged_turn: None,
            two_port_layout: false,
            turn_published: true,
            relay_range: String::new(),
            port_keys: Vec::new(),
            set_compose_file: String::new(),
            updating: false,
            ports_result: PortsResult::Free,
            listen_tcp: BTreeSet::new(),
            listen_udp: BTreeSet::new(),
            run_stamp: sys::stamp(),
            override_file: None,
            existing_env: Vec::new(),
            removed_env: Vec::new(),
            fast_clock,
        }
    }

    pub fn main(&mut self) -> Step {
        // Only an answer file or a terminal the person is at can answer; with
        // --yes nothing is asked at all.
        if !self.o.assume_yes {
            let tty = PathBuf::from(&self.o.tty);
            self.interactive = self.ui.open_answers(&tty);
        }
        self.find_checkout();

        self.ui.banner();
        #[cfg(feature = "test-seams")]
        if env("STELLARR_INSTALLER_TEST_PANIC") == "1" {
            panic!("a fault made on purpose by the tests");
        }
        self.ui.section("Checking this machine");
        self.check_host()?;
        self.check_docker()?;
        let _docker_lock = if self.o.start {
            let identity = sys::output(self.docker(["info", "--format", "{{.ID}}"]))
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| self.die("Could not identify the Docker daemon", NONE))?;
            Some(self.acquire_lock("docker", identity.trim().as_bytes())?)
        } else {
            None
        };
        self.find_container();
        let _directory_lock = self.choose_directory()?;
        self.choose_release()?;
        self.decide_env()?;
        self.fetch_compose_files()?;
        self.settle_settings()?;
        self.settle_compose_files();
        self.confirm_update()?;
        self.check_ports()?;
        self.fetch_turn_overlay()?;

        self.deploy()?;
        if !self.o.start {
            self.show_not_started();
            return Ok(());
        }
        self.show_summary();
        Ok(())
    }

    // ---- Messages that stop the run ----

    /// Stops the run: one line says what is wrong, the lines after it say
    /// what to do.
    fn die<S: AsRef<str>>(&mut self, problem: impl AsRef<str>, lines: &[S]) -> Stop {
        self.ui.fail(problem.as_ref(), NONE);
        if !lines.is_empty() {
            let mut text: Vec<&str> = vec![""];
            text.extend(lines.iter().map(AsRef::as_ref));
            self.ui.text(Fd::Err, &text);
        }
        Stop(1)
    }

    /// A mistake in how the installer was started: one line, and exit
    /// status 2.
    fn usage_error(&mut self, message: impl AsRef<str>) -> Stop {
        self.ui.fail(message.as_ref(), NONE);
        Stop(2)
    }

    // ---- Settings ----

    fn given(&self, key: Key) -> bool {
        self.o.setting(key).given
    }

    fn set(&self, key: Key) -> &str {
        &self.o.setting(key).set
    }

    fn put(&mut self, key: Key, value: impl Into<String>) {
        self.o.setting_mut(key).set = value.into();
    }

    fn default_to(&mut self, key: Key, value: &str) {
        if self.set(key).is_empty() {
            self.put(key, value);
        }
    }

    fn has(&self, name: &str) -> bool {
        self.exist.contains_key(name)
    }

    fn exist(&self, name: &str) -> &str {
        self.exist.get(name).map_or("", String::as_str)
    }

    // ---- Commands ----

    /// A command with the environment every command the installer runs gets:
    /// bytes are bytes whatever the files hold, and what Compose must not
    /// read from the person's shell is gone.
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.env("LC_ALL", "C");
        for name in &self.removed_env {
            command.env_remove(name);
        }
        command
    }

    /// sudo, which without a terminal runs with -n: it fails instead of
    /// asking for a password nobody can type.
    fn sudo<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(&self, args: I) -> Command {
        let mut command = self.command("sudo");
        if !self.interactive {
            command.arg("-n");
        }
        command.args(args);
        command
    }

    /// docker as this user, or through sudo when only root can reach the
    /// daemon.
    fn docker<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(&self, args: I) -> Command {
        if self.docker_sudo {
            let mut command = self.sudo(["docker"]);
            command.args(args);
            command
        } else {
            let mut command = self.command("docker");
            command.args(args);
            command
        }
    }

    fn compose_at(&self, config_dir: &Path, args: &[&str]) -> Command {
        let mut command = if self.compose_plugin {
            self.docker(["compose"])
        } else if self.docker_sudo {
            self.sudo(["docker-compose"])
        } else {
            self.command("docker-compose")
        };
        command
            .args(["--project-name", "stellarr", "--project-directory"])
            .arg(config_dir)
            .arg("--env-file")
            .arg(config_dir.join(".env"))
            .arg("-f")
            .arg(config_dir.join("docker-compose.yml"));
        if self.two_port_layout && self.turn_published {
            command.arg("-f").arg(config_dir.join(self.turn_file()));
        }
        if self.o.build && config_dir != self.install_dir {
            command
                .arg("-f")
                .arg(config_dir.join(self.build_file()))
                .arg("-f")
                .arg(InstallPlan::build_context_path(&self.install_dir));
        }
        if let Some(file) = self.override_file {
            command.arg("-f").arg(config_dir.join(file));
        }
        if args.first() == Some(&"build") {
            command
                .arg("-f")
                .arg(InstallPlan::build_paths_file(&self.install_dir));
        }
        command.args(args);
        command
    }

    /// The command a person types later for Compose in the installation, as
    /// this run needed it: with sudo when Docker needed it.
    fn compose_command(&self, args: &str) -> String {
        let mut tool = if self.compose_plugin {
            "docker compose"
        } else {
            "docker-compose"
        }
        .to_string();
        if self.docker_sudo {
            tool = format!("sudo {tool}");
        }
        format!(
            "cd {} && {tool} {args}",
            shell_word(&display(&self.install_dir))
        )
    }

    fn container_format(&self, format: &str) -> Option<String> {
        sys::output(self.docker(["container", "inspect", "--format", format, "stellarr"]))
    }

    // ---- This machine ----

    fn check_host(&mut self) -> Step {
        let macos = self.os == Os::MacOs;
        let os_name = if macos { "macOS" } else { "Linux" };
        let arch = std::env::consts::ARCH;
        let raw_arch = if macos && arch == "aarch64" {
            "arm64"
        } else {
            arch
        };
        if !matches!(arch, "x86_64" | "aarch64") {
            return Err(self.die(
                format!("There is no Stellarr image for {raw_arch}, only for amd64 and arm64"),
                NONE,
            ));
        }
        let memory = sys::memory_kib(macos);
        // Images and the data volume live where Docker keeps its data, not in
        // the installation's directory, so that is the disk that has to have
        // room.
        let disk_path = if macos {
            PathBuf::from(env("HOME"))
        } else {
            ["/var/lib/docker", "/var/lib", "/"]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_dir())
                .unwrap_or_default()
        };
        let free = sys::free_kib(&disk_path);

        let mut low: Vec<String> = Vec::new();
        let memory_text = memory.map(|kib| format!("{} memory", gib(kib)));
        let disk_text = free.map(|kib| format!("{} free", gib(kib)));
        if let (Some(kib), Some(text)) = (memory, &memory_text)
            && kib < 2_097_152
        {
            low.push(text.clone());
        }
        if let (Some(kib), Some(text)) = (free, &disk_text)
            && kib < 2_097_152
        {
            low.push(text.clone());
        }
        let mut summary = format!("{os_name} {raw_arch}");
        if low.is_empty() {
            for text in [&memory_text, &disk_text].into_iter().flatten() {
                summary = format!("{summary}, {text}");
            }
        }
        self.ui.ok(&summary, NONE);
        if !low.is_empty() {
            self.ui.warn(
                &format!("Only {}; Stellarr wants 2 GiB of each", low.join(" and ")),
                NONE,
            );
        }
        Ok(())
    }

    fn check_docker(&mut self) -> Step {
        if !sys::have("docker") {
            self.docker_missing()?;
        }
        self.docker_reach()?;
        self.compose_find()?;
        let line = format!(
            "Docker {}, Compose {}",
            self.docker_version, self.compose_version
        );
        self.ui.ok(&line, NONE);
        Ok(())
    }

    fn docker_missing(&mut self) -> Step {
        let mut how = "with sudo";
        if self.os == Os::MacOs {
            return Err(self.die(
                "Docker is not installed",
                &[
                    "Install Docker Desktop, start it, and run the installer again:",
                    "  https://docs.docker.com/desktop/setup/install/mac-install/",
                ],
            ));
        }
        if self.uid == 0 {
            how = "as root";
        } else if !sys::have("sudo") {
            return Err(self.die(
                "Docker is not installed, and installing it needs root",
                &[
                    "Run the installer as root, or install Docker Engine yourself:",
                    "  https://docs.docker.com/engine/install/",
                ],
            ));
        }
        if !self.o.install_docker {
            if !self.interactive {
                return Err(self.die(
                    "Docker is not installed",
                    &[
                        "Install Docker Engine and run the installer again:",
                        "  https://docs.docker.com/engine/install/",
                        "or run the installer with --install-docker to have it install Docker.",
                    ],
                ));
            }
            self.ui.fail("Docker is not installed", NONE);
            let explain = format!("from https://get.docker.com and runs it {how}.");
            if !self.ui.confirm(
                true,
                "Install Docker Engine now?",
                &[
                    "Stellarr runs in one container. This downloads Docker's own script",
                    &explain,
                ],
            )? {
                self.ui.text(
                    Fd::Err,
                    &[
                        "",
                        "Install Docker Engine and run the installer again:",
                        "  https://docs.docker.com/engine/install/",
                    ],
                );
                return Err(Stop(1));
            }
        }
        self.docker_install()
    }

    /// Installs Docker Engine with Docker's own script. The person is not
    /// added to the docker group, which would give them root without a
    /// password; this run uses sudo instead, and the summary says so.
    fn docker_install(&mut self) -> Step {
        let script = match self.http.get("https://get.docker.com") {
            Ok(script) => script,
            Err(error) => {
                return Err(self.die(
                    "Could not download Docker's install script from https://get.docker.com",
                    &[error],
                ));
            }
        };
        let path = std::env::temp_dir().join(format!(
            "stellarr-get-docker.{}.{}.sh",
            std::process::id(),
            sys::unique()
        ));
        let written = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(&script));
        if let Err(error) = written {
            return Err(self.die(
                "Could not save Docker's install script",
                &[error.to_string()],
            ));
        }
        sys::temporary(Some(&path));
        self.ui.text(
            Fd::Out,
            &[
                "",
                "Installing Docker Engine with its own script; its output follows.",
                "",
            ],
        );
        let ok = if self.uid == 0 {
            let mut command = self.command("sh");
            command.arg(&path);
            sys::shown(command)
        } else {
            let mut command = self.sudo(["sh"]);
            command.arg(&path);
            sys::shown(command)
        };
        let _ = fs::remove_file(&path);
        sys::temporary(None);
        if !ok {
            return Err(self.die("Docker's install script failed", &["Its output is above."]));
        }
        if !sys::have("docker") {
            return Err(self.die(
                "Docker's install script finished, but there is no docker command",
                NONE,
            ));
        }
        self.ui.ok("Installed Docker Engine", NONE);
        Ok(())
    }

    fn docker_server_version(&mut self, command: Command) -> bool {
        match sys::output(command) {
            Some(version) if !version.trim().is_empty() => {
                self.docker_version = version.trim().to_string();
                true
            }
            _ => false,
        }
    }

    fn docker_version_command(&self, through: &[&str]) -> Command {
        let mut command = if through.is_empty() {
            self.command("docker")
        } else {
            let mut command = self.command(through[0]);
            command.args(&through[1..]);
            command.arg("docker");
            command
        };
        command.args(["version", "--format", "{{.Server.Version}}"]);
        command
    }

    /// Finds how this user reaches the Docker daemon: directly, or on Linux
    /// through sudo when the socket is root's. Docker Desktop runs as the
    /// person, so sudo cannot help on macOS. A stopped daemon on Linux is
    /// started once.
    fn docker_reach(&mut self) -> Step {
        let root = self.uid == 0;
        let mut told = false;
        for attempt in 1..=2 {
            if self.docker_server_version(self.docker_version_command(&[])) {
                self.docker_sudo = false;
                return Ok(());
            }
            if self.os == Os::Linux && !root && sys::have("sudo") {
                let mut check = self.command("sudo");
                check.args(["-n", "true"]);
                if sys::quiet(check)
                    && self.docker_server_version(self.docker_version_command(&["sudo", "-n"]))
                {
                    self.docker_sudo = true;
                    return Ok(());
                }
                if self.interactive {
                    if !told {
                        self.ui.text(
                            Fd::Out,
                            &["This user cannot reach Docker, so the installer runs it with sudo."],
                        );
                        told = true;
                    }
                    if self.docker_server_version(self.docker_version_command(&["sudo"])) {
                        self.docker_sudo = true;
                        return Ok(());
                    }
                }
            }
            if attempt == 2 || self.os != Os::Linux || !sys::have("systemctl") {
                break;
            }
            let started = if root {
                let mut command = self.command("systemctl");
                command.args(["start", "docker"]);
                sys::quiet(command)
            } else if sys::have("sudo") {
                sys::quiet(self.sudo(["systemctl", "start", "docker"]))
            } else {
                false
            };
            if !started {
                break;
            }
        }
        if self.os == Os::MacOs {
            return Err(self.die(
                "Docker Desktop is not running",
                &["Start Docker Desktop and run the installer again."],
            ));
        }
        if !self.interactive && !root {
            return Err(self.die(
                "Docker is installed, but this user cannot reach it",
                &[
                    "Without a terminal the installer uses sudo only when sudo asks for no",
                    "password. Run it in a terminal or as root, or check that Docker runs:",
                    "  sudo systemctl start docker",
                ],
            ));
        }
        Err(self.die(
            "Docker is installed, but its daemon does not answer",
            &[
                "Start it and run the installer again:",
                "  sudo systemctl start docker",
            ],
        ))
    }

    fn compose_find(&mut self) -> Step {
        let short =
            |out: Option<String>| out.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let mut version = if let Some(version) =
            short(sys::output(self.docker(["compose", "version", "--short"])))
        {
            self.compose_plugin = true;
            version
        } else if sys::have("docker-compose")
            && let Some(version) = short(sys::output({
                let mut command = self.command("docker-compose");
                command.args(["version", "--short"]);
                command
            }))
        {
            self.compose_plugin = false;
            version
        } else {
            return Err(self.die(
                "Docker Compose is missing",
                &[
                    "Install the Compose plugin, the docker-compose-plugin package, and run the",
                    "installer again: https://docs.docker.com/compose/install/linux/",
                ],
            ));
        };
        if let Some(rest) = version.strip_prefix('v') {
            version = rest.to_string();
        }
        let mut parts = version.splitn(2, '.');
        let major = parts.next().unwrap_or_default();
        let minor: u64 = parts
            .next()
            .map(|rest| {
                rest.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(0);
        let Ok(major) = major.parse::<u64>() else {
            return Err(self.die(
                format!("Could not read the version of Docker Compose: {version}"),
                NONE,
            ));
        };
        // Stellarr's docker-compose.yml hands .env to the container through
        // env_file with required: false, which Compose reads from 2.24.0 on.
        if major < 2 || (major == 2 && minor < 24) {
            return Err(self.die(
                format!("Docker Compose {version} is too old for Stellarr's docker-compose.yml"),
                &[
                    "It passes .env to the container with env_file's required: false, which needs",
                    "Compose 2.24.0 or newer. Install the docker-compose-plugin package and run the",
                    "installer again: https://docs.docker.com/compose/install/linux/",
                ],
            ));
        }
        self.compose_version = version;
        Ok(())
    }

    // ---- Where Stellarr is ----

    /// The checkout this installer was started from: the bootstrap script
    /// names its own directory when it ran as a file beside Stellarr's
    /// docker-compose.yml and Dockerfile. A script read from a pipe has none.
    fn find_checkout(&mut self) {
        let named = env("STELLARR_INSTALLER_CHECKOUT");
        if named.is_empty() {
            return;
        }
        if let Ok(dir) = fs::canonicalize(&named)
            && is_checkout(&dir)
        {
            self.script_checkout = Some(dir);
        }
    }

    /// What Docker knows of the container named stellarr. Compose labels it
    /// with its project and the directory it was started from.
    fn find_container(&mut self) {
        let format = "{{.State.Running}}\n{{index .Config.Labels \"com.docker.compose.project\"}}\n\
{{index .Config.Labels \"com.docker.compose.project.working_dir\"}}\n{{index .Config.Labels \"org.opencontainers.image.version\"}}";
        self.container = Container::default();
        let Some(info) = self.container_format(format) else {
            return;
        };
        let mut lines = info.lines();
        let mut next = || lines.next().unwrap_or_default().to_string();
        let (running, project, dir, version) = (next(), next(), next(), next());
        let value = |v: String| if v == "<no value>" { String::new() } else { v };
        self.container = Container {
            exists: true,
            running: running == "true",
            project: value(project),
            dir: value(dir),
            version: value(version),
        };
    }

    /// The installation's directory, in this order: --dir, the checkout this
    /// installer was started from, the directory of the installation Docker
    /// runs, the current directory when it holds Stellarr's
    /// docker-compose.yml, and ~/stellarr.
    fn choose_directory(&mut self) -> Step<fs::File> {
        let cwd = std::env::current_dir().unwrap_or_default();
        self.install_dir = if !self.o.dir.is_empty() {
            absolute_path(&self.o.dir.clone())
        } else if let Some(checkout) = &self.script_checkout {
            checkout.clone()
        } else if self.container.exists && !self.container.dir.is_empty() {
            absolute_path(&self.container.dir.clone())
        } else if is_stellarr_compose_file(&cwd.join("docker-compose.yml")) {
            absolute_path(&display(&cwd))
        } else {
            if env("HOME").is_empty() {
                return Err(self.die(
                    "HOME is not set, so there is no default directory",
                    &["Choose one with --dir."],
                ));
            }
            absolute_path(&format!("{}/stellarr", env("HOME")))
        };
        let path = self.install_dir.clone();
        let lock = self.acquire_lock("directory", path.as_os_str().as_encoded_bytes())?;
        if InstallPlan::recover(&path).map_err(|error| {
            self.die(
                "Could not recover the installation files",
                &[error.to_string()],
            )
        })? {
            self.ui.ok(
                "Restored the files from before the interrupted installation",
                NONE,
            );
        }
        let install_dir = display(&self.install_dir);
        self.owns_container = false;

        if self.container.exists {
            if self.container.project.is_empty() || self.container.dir.is_empty() {
                let follow = format!("update this one, follow {DOCS_URL}/install/upgrading");
                return Err(self.die(
                    "A container named stellarr exists, but Docker Compose did not start it",
                    &[
                        "The installer manages installations made with Docker Compose only. To",
                        &follow,
                    ],
                ));
            }
            if self.container.project != "stellarr" {
                let problem = format!(
                    "A container named stellarr belongs to the Compose project \"{}\"",
                    self.container.project
                );
                let follow = format!("follow {DOCS_URL}/install/upgrading");
                return Err(
                    self.die(problem, &["This installer manages the project named stellarr only. To update this one,", &follow])
                );
            }
            let running_dir = fs::canonicalize(&self.container.dir)
                .ok()
                .filter(|d| d.is_dir());
            match running_dir {
                Some(running) if running == self.install_dir => self.owns_container = true,
                Some(running) => {
                    // Compose's project name is fixed, so starting it from a
                    // second directory would recreate the running container
                    // with that directory's settings.
                    let running = display(&running);
                    return Err(self.die(
                        format!("Stellarr is installed in {running}, not in {install_dir}"),
                        &[
                            "Docker Compose would recreate its container with the settings in the".to_string(),
                            "other directory. To update the installation, run the installer again with".to_string(),
                            format!("  --dir {}", shell_word(&running)),
                        ],
                    ));
                }
                None => {
                    let dir = self.container.dir.clone();
                    self.ui.warn(
                        &format!("The container named stellarr was started from {dir},"),
                        &["which no longer exists"],
                    );
                    if !self.o.force {
                        let take_over = self.interactive
                            && self.ui.confirm(
                                false,
                                &format!("Take the container over from {install_dir}?"),
                                &["Compose recreates it with the settings there. Its data volume stays."],
                            )?;
                        if !take_over {
                            return Err(self.die(
                                "The container's directory is gone, so the installer stopped",
                                &[format!("To take the container over from {install_dir}, run the installer again"), "with --force.".into()],
                            ));
                        }
                    }
                    self.owns_container = true;
                }
            }
        }

        // The installer replaces docker-compose.yml on an update, which must
        // never happen to another project's.
        let compose = self.install_dir.join("docker-compose.yml");
        if compose.is_file() && !is_stellarr_compose_file(&compose) {
            return Err(self.die(
                format!("{install_dir} holds a docker-compose.yml that is not Stellarr's"),
                &["Choose another directory with --dir."],
            ));
        }
        // Compose reads any of these in place of docker-compose.yml, so
        // "docker compose up -d" in the directory, as the summary prints it,
        // would run another project.
        for file in ["compose.yaml", "compose.yml", "docker-compose.yaml"] {
            if self.install_dir.join(file).is_file() {
                return Err(self.die(
                    format!("{install_dir} holds {file}, which Docker Compose reads before docker-compose.yml"),
                    &["Choose another directory with --dir."],
                ));
            }
        }
        self.dir_is_checkout = is_checkout(&self.install_dir);
        if self.o.build && !self.dir_is_checkout {
            return Err(self.die(
                format!("--build builds the image from a checkout, and {install_dir} is not one"),
                &[
                    "Clone the repository and run the installer there:".to_string(),
                    format!("  git clone {REPO_URL}.git"),
                    "  cd Stellarr && ./install.sh --build".to_string(),
                ],
            ));
        }
        self.read_existing_env()?;
        Ok(lock)
    }

    fn acquire_lock(&mut self, scope: &str, identity: &[u8]) -> Step<fs::File> {
        install_plan::lock(scope, identity).map_err(|error| {
            self.die(
                "Could not lock the Stellarr installation",
                &[error.to_string()],
            )
        })
    }

    /// Reads the .env already in the directory. Values may be quoted, end in
    /// a carriage return or follow "export"; the last assignment of a key
    /// wins, as in Compose.
    fn read_existing_env(&mut self) -> Step {
        self.env_state = EnvState::None;
        self.exist.clear();
        let path = self.install_dir.join(".env");
        let Some(content) = install_plan::read(&path).map_err(|error| {
            self.die(
                format!("{} cannot be read by this user", display(&path)),
                &[error.to_string()],
            )
        })?
        else {
            return Ok(());
        };
        self.env_state = EnvState::Foreign;
        let first = envfile::records(content.strip_prefix(envfile::BOM).unwrap_or(&content))
            .first()
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line).to_vec());
        let marker_start = ENV_MARKER
            .split(',')
            .next()
            .unwrap_or(ENV_MARKER)
            .as_bytes();
        if first.is_some_and(|line| line.starts_with(marker_start)) {
            self.env_state = EnvState::Marked;
        }
        let wanted: Vec<&str> = KEYS
            .iter()
            .map(|k| k.name())
            .chain(OTHER_KEYS.iter().copied())
            .collect();
        self.exist = envfile::parse(&content)
            .into_iter()
            .filter(|(key, _)| wanted.contains(&key.as_str()))
            .collect();
        self.existing_env = content;
        Ok(())
    }

    // ---- Which release ----

    fn resolve_track(&mut self, track: &str) -> Step<String> {
        let url = format!("{SERVER_DOWNLOADS}/{track}");
        let body = self.http.get_optional(&url).map_err(|error| {
            self.die(
                format!("Could not resolve the Stellarr {track} release"),
                &[error],
            )
        })?;
        let Some(body) = body else {
            if track == "latest" {
                return Err(self.die(
                    "Stellarr has no published release yet, so there is no image to pull.",
                    &[
                        "To build it from a checkout instead:",
                        "  ./install.sh --build",
                    ],
                ));
            }
            return Err(self.die(
                format!("Stellarr has no stable release in the {track} series"),
                NONE,
            ));
        };
        let parsed = std::str::from_utf8(&body)
            .ok()
            .and_then(|text| exact_version(text.trim()));
        if let Some(version) = parsed
            && version.pre.is_empty()
            && (track == "latest" || release_series(track) == Some((version.major, version.minor)))
        {
            return Ok(version.to_string());
        }
        Err(self.die(
            format!("Invalid server version for the {track} track"),
            NONE,
        ))
    }

    /// Resolve moving tags before reading release files or comparing versions.
    fn choose_release(&mut self) -> Step {
        const LOCAL_IMAGE: &str = "stellarr-local:dev";
        let existing_tag = self.exist("STELLARR_VERSION").to_string();
        let existing_track = self.exist("STELLARR_TRACK").to_string();
        self.installed_version = if self.owns_container && !self.container.version.is_empty() {
            self.container.version.clone()
        } else {
            existing_tag.clone()
        };
        if self.o.build {
            self.target_tag = "dev".into();
            self.target_version = self.target_tag.clone();
            self.target_image = LOCAL_IMAGE.into();
            self.ui
                .ok("The image will be built from the checkout", NONE);
            return Ok(());
        }

        let existing_image = self.exist("STELLARR_IMAGE");
        if self.o.version.is_empty()
            && !existing_image.is_empty()
            && existing_image != LOCAL_IMAGE
            && existing_image != format!("{IMAGE}:{existing_tag}")
        {
            return Err(self.die(
                format!("The installation uses a custom image {existing_image}"),
                &["Choose a published release with --version, or manage this image through Compose."],
            ));
        }

        let requested = if !self.o.version.is_empty() {
            self.o.version.clone()
        } else if !existing_track.is_empty() {
            existing_track
        } else if !existing_tag.is_empty()
            && existing_tag != "dev"
            && !is_exact_version(&existing_tag)
        {
            existing_tag.clone()
        } else {
            String::new()
        };
        let mut retained_newer = false;
        self.target_tag = if requested.is_empty() || requested == "latest" {
            self.target_track = requested.clone();
            let latest = self.resolve_track("latest")?;
            if requested.is_empty() && version_older(&latest, &existing_tag) {
                retained_newer = true;
                self.ui.ok(
                    &format!("Latest release is {latest}; {existing_tag}, which is newer, stays"),
                    NONE,
                );
                existing_tag
            } else {
                latest
            }
        } else if release_series(&requested).is_some() {
            self.target_track = requested.clone();
            self.resolve_track(&requested)?
        } else if is_exact_version(&requested) {
            requested.clone()
        } else {
            return Err(self.die(
                format!("{requested} is not an exact release, a release series, or latest"),
                &["Choose a published release with --version, or manage this image through Compose."],
            ));
        };

        if version_older(&self.target_tag, &self.installed_version) {
            let problem = format!(
                "Stellarr {} is installed, and {} is older",
                self.installed_version, self.target_tag
            );
            let restore = format!("update: {DOCS_URL}/install/upgrading#rollback");
            return Err(self.die(
                problem,
                &[
                    "Going back to an older release means restoring the backup made before the",
                    &restore,
                ],
            ));
        }

        self.target_version = self.target_tag.clone();
        self.target_image = format!("{IMAGE}:{}", self.target_tag);
        self.files_ref = if self.o.reference.is_empty() {
            format!("v{}", self.target_tag)
        } else {
            self.ui.ok(
                &format!(
                    "Using Compose and settings files from {} as {} asks",
                    self.o.reference, self.o.ref_from
                ),
                NONE,
            );
            self.o.reference.clone()
        };
        if retained_newer {
            return Ok(());
        }
        if self.owns_container
            && self.container.version == self.target_version
            && requested.is_empty()
        {
            self.ui.ok(
                &format!(
                    "Stellarr {} in {} is the latest release",
                    self.target_version,
                    display(&self.install_dir)
                ),
                NONE,
            );
        } else if requested.is_empty() {
            self.ui
                .ok(&format!("Latest release is {}", self.target_version), NONE);
        } else if self.target_track.is_empty() {
            self.ui.ok(
                &format!(
                    "Version {}, as {} asks",
                    self.target_version, self.o.version_from
                ),
                NONE,
            );
        } else {
            self.ui.ok(
                &format!(
                    "Release {} follows the tag {requested}",
                    self.target_version
                ),
                NONE,
            );
        }
        Ok(())
    }

    // ---- .env and the settings ----

    /// What happens to .env, decided before anything is written: write a new
    /// one, keep one the installer wrote, adopt one written by hand, or
    /// replace it.
    fn decide_env(&mut self) -> Step {
        self.env_action = match self.env_state {
            EnvState::None => return Ok(()),
            EnvState::Marked => EnvAction::Keep,
            EnvState::Foreign => EnvAction::Adopt,
        };
        if self.o.force {
            self.env_action = EnvAction::Replace;
            return Ok(());
        }
        let dir = display(&self.install_dir);
        // Compose reads these from .env over the project's own settings, so
        // they change which project and files it uses, which would start the
        // installation on another, empty volume or with services it does not
        // have. The known TURN stack and default override are the only
        // approved file list.
        let compose_file = self.exist("COMPOSE_FILE");
        let forbidden = [
            "COMPOSE_PROJECT_NAME",
            "COMPOSE_PROFILES",
            "COMPOSE_ENV_FILES",
            "COMPOSE_DISABLE_ENV_FILE",
            "COMPOSE_PATH_SEPARATOR",
        ];
        let mut unsupported: Vec<_> = forbidden
            .into_iter()
            .filter(|key| !self.exist(key).is_empty())
            .collect();
        if !compose_file.is_empty() && !managed_turn_files(compose_file) {
            unsupported.push("COMPOSE_FILE");
        }
        if !unsupported.is_empty() {
            if self.env_state == EnvState::Marked {
                let (set, it) = match unsupported.as_slice() {
                    [one] => (one.to_string(), "it"),
                    [a, b] => (format!("{a} and {b}"), "them"),
                    [a, b, c] => (format!("{a}, {b} and {c}"), "them"),
                    _ => (unsupported.join(", "), "them"),
                };
                return Err(self.die(
                    format!("{dir}/.env sets {set}"),
                    &[
                        format!(
                            "Compose would take {it} over the project's own settings, which could"
                        ),
                        format!(
                            "start the installation on another, empty volume. Remove {it} from .env"
                        ),
                        "and run the installer again, or add --force to keep .env as a backup"
                            .to_string(),
                        "and write a new one.".to_string(),
                    ],
                ));
            }
            return Err(self.die(
                format!("{dir}/.env sets {}", unsupported.join(", ")),
                &[
                    "It looks like a development stack's .env, which the installer must not run",
                    "the production stack with. Choose another directory with --dir, or add",
                    "--force to keep it as a backup and write a new one.",
                ],
            ));
        }
        if self.env_state != EnvState::Foreign {
            return Ok(());
        }
        let explain = format!("The installer did not write {dir}/.env. Keeping it marks it as the");
        if self.interactive
            && !self.ui.confirm(
                true,
                "Keep the settings in the existing .env?",
                &[
                    explain.as_str(),
                    "installer's, which then changes only what it is given. Otherwise it is kept",
                    "as a backup and a new one is written.",
                ],
            )?
        {
            self.env_action = EnvAction::Replace;
        }
        Ok(())
    }

    fn keeps_env(&self) -> bool {
        matches!(self.env_action, EnvAction::Keep | EnvAction::Adopt)
    }

    fn relay_bounds(&self) -> Option<(u32, u32)> {
        if !self.turn_published {
            return None;
        }
        let (first, last) = self.relay_range.split_once('-')?;
        let (first, last) = (port_number(first)?, port_number(last)?);
        (first <= last).then_some((first, last))
    }

    /// The settings this run uses: a given value, else the one in .env, else
    /// the answer to a question, else the default.
    fn settle_settings(&mut self) -> Step {
        let dir = display(&self.install_dir);
        for key in KEYS {
            let value = if self.given(key) {
                self.o.setting(key).value.clone()
            } else if self.keeps_env() {
                self.exist(key.name()).to_string()
            } else {
                String::new()
            };
            self.put(key, value);
        }
        if matches!(self.env_action, EnvAction::New | EnvAction::Replace) {
            self.ask_questions()?;
        }
        self.default_to(Key::Domain, "localhost");
        self.default_to(Key::Language, "en");
        self.turn_published = true;
        if self.two_port_layout {
            let compose = fs::read(self.install_dir.join("docker-compose.yml")).unwrap_or_default();
            let legacy_install = self.env_action != EnvAction::Replace
                && String::from_utf8_lossy(&compose).contains(":3478/udp");
            if legacy_install {
                // A standalone Compose install may have used the previous
                // implicit ports.
                for (key, port) in [(Key::HttpsPort, "8443"), (Key::HttpPort, "8888")] {
                    if self.set(key).is_empty() {
                        self.put(key, port);
                        self.o.setting_mut(key).given = true;
                    }
                }
            }
            self.default_to(Key::HttpsPort, "443");
            self.default_to(Key::HttpPort, "127.0.0.1:8888");
            let selected_turn = self.keeps_env() && managed_turn_files(self.exist("COMPOSE_FILE"));
            if self.set(Key::TurnEnabled).is_empty()
                && (legacy_install
                    || selected_turn
                    || self.given(Key::TurnPort)
                    || self.given(Key::TurnsPort))
            {
                self.put(Key::TurnEnabled, "true");
                self.o.setting_mut(Key::TurnEnabled).given = true;
            }
            self.default_to(Key::TurnEnabled, "false");
            match self.set(Key::TurnEnabled) {
                "true" => {}
                "false" => self.turn_published = false,
                _ => {
                    return Err(self.die(
                        format!("STELLARR_TURN_ENABLED in {dir}/.env needs true or false"),
                        NONE,
                    ));
                }
            }
        } else {
            self.default_to(Key::HttpsPort, "8443");
            self.default_to(Key::HttpPort, "8888");
            if self.given(Key::TurnEnabled) {
                return Err(self.usage_error(
                    "This release always publishes TURN; choose a newer release to use --enable-turn or --disable-turn",
                ));
            }
        }
        self.default_to(Key::MediaPort, "49000");
        self.port_keys = vec![Key::HttpsPort, Key::HttpPort, Key::MediaPort];
        if self.turn_published {
            self.port_keys.extend([Key::TurnPort, Key::TurnsPort]);
        }
        self.default_to(Key::TurnPort, "3478");
        self.default_to(Key::TurnsPort, "5349");
        self.relay_range = match self.exist("STELLARR_TURN_RELAY_RANGE") {
            range if !range.is_empty() && self.keeps_env() => range.to_string(),
            _ => "49160-49200".to_string(),
        };
        if self.turn_published && self.relay_bounds().is_none() {
            return Err(self.die(
                format!("STELLARR_TURN_RELAY_RANGE in {dir}/.env needs an ascending port range"),
                NONE,
            ));
        }

        for key in [
            Key::HttpsPort,
            Key::PublicPort,
            Key::HttpPort,
            Key::MediaPort,
            Key::TurnPort,
            Key::TurnsPort,
        ] {
            let value = self.set(key).to_string();
            if value.is_empty() || (key == Key::HttpPort && is_http_binding(&value)) {
                continue;
            }
            if !is_port(&value) {
                return Err(self.die(
                    format!("{} in {dir}/.env is not a port: \"{value}\"", key.name()),
                    &[format!(
                        "Correct it there, or set it with {}.",
                        key.option()
                    )],
                ));
            }
        }
        // The entrypoint resolves auto from the domain, so it needs a domain.
        // A value .env had before is the person's own and stays as it is.
        let domain = self.set(Key::Domain).to_string();
        if self.set(Key::PublicIp) == "auto"
            && (!is_hostname(&domain) || is_local_address(&domain))
            && (self.given(Key::PublicIp) || self.given(Key::Domain))
        {
            return Err(self.usage_error(format!(
                "STELLARR_PUBLIC_IP=auto needs a domain to resolve, and {domain} is none"
            )));
        }
        self.check_port_overlap()
    }

    /// Two settings on one port would only fail later, inside Docker.
    fn check_port_overlap(&mut self) -> Step {
        let mut tcp = BTreeSet::new();
        let mut udp = BTreeSet::new();
        let relay = self.relay_bounds();
        for key in self.port_keys.clone() {
            let port = binding_port(self.set(key)).to_string();
            let (_, protos) = self.port_role(key);
            let number = port.parse::<u32>().unwrap_or(0);
            if protos.contains(&"tcp") && !tcp.insert(port.clone()) {
                return Err(self.port_overlap(&port));
            }
            if protos.contains(&"udp") {
                if !udp.insert(port.clone()) {
                    return Err(self.port_overlap(&port));
                }
                if relay.is_some_and(|(first, last)| (first..=last).contains(&number)) {
                    return Err(self.port_overlap(&port));
                }
            }
        }
        Ok(())
    }

    fn port_overlap(&mut self, port: &str) -> Stop {
        self.usage_error(format!(
            "Two of Stellarr's ports are set to {port}; each needs its own"
        ))
    }

    /// Up to three questions, for a new installation in a terminal; a value
    /// given as an option is not asked for.
    fn ask_questions(&mut self) -> Step {
        if !self.given(Key::Domain) {
            self.put(Key::Domain, "localhost");
            if self.interactive {
                // Over SSH, the address this machine was reached at is the
                // likely one.
                let ssh = env("SSH_CONNECTION");
                let server = ssh.split_whitespace().nth(2).unwrap_or_default();
                let default = if is_ipv4(server) {
                    server.to_string()
                } else {
                    "localhost".to_string()
                };
                self.ui.question(
                    "Where will people open Stellarr?",
                    &["A domain or an IP address. \"localhost\" works on this machine only."],
                );
                let address = loop {
                    let answer = self.ui.prompt(&default)?;
                    let address =
                        normalize_address(if answer.is_empty() { &default } else { &answer });
                    if is_address(&address) {
                        break address;
                    }
                    self.ui
                        .retry(&format!("\"{answer}\" is not a domain or an IPv4 address"));
                };
                self.put(Key::Domain, address);
            }
        }

        if !self.given(Key::PublicIp) {
            let domain = self.set(Key::Domain).to_string();
            if is_local_address(&domain) {
                self.put(Key::PublicIp, "");
            } else if is_ipv4(&domain) {
                self.put(Key::PublicIp, domain);
            } else if self.interactive {
                let explain = format!("{domain}. \"none\" leaves it empty to decide later.");
                self.ui.question(
                    "Which IPv4 address should voice calls use?",
                    &[
                        "Other devices send their audio here. \"auto\" reads the DNS record of",
                        &explain,
                    ],
                );
                let value = loop {
                    let answer = self.ui.prompt("auto")?;
                    match answer.as_str() {
                        "" | "auto" => break "auto".to_string(),
                        "none" | "-" => break String::new(),
                        address if is_ipv4(address) => break address.to_string(),
                        _ => self
                            .ui
                            .retry(&format!("\"{answer}\" is not an IPv4 address")),
                    }
                };
                self.put(Key::PublicIp, value);
            } else if self.two_port_layout {
                self.put(Key::PublicIp, "auto");
            } else {
                self.put(Key::PublicIp, "");
            }
        }

        if !self.given(Key::Language) {
            self.put(Key::Language, "en");
            if self.interactive {
                let default = if self.o.locale_language.starts_with("pl") {
                    "pl"
                } else {
                    "en"
                };
                self.ui.question("Interface language, en or pl", NONE);
                let language = loop {
                    let answer = self.ui.prompt(default)?;
                    let answer = if answer.is_empty() {
                        default.to_string()
                    } else {
                        answer
                    };
                    if answer == "en" || answer == "pl" {
                        break answer;
                    }
                    self.ui.retry("Answer en or pl");
                };
                self.put(Key::Language, language);
            }
        }
        Ok(())
    }

    fn checkout_overlay(&self, current: &'static str, legacy: &'static str) -> &'static str {
        if !self.install_dir.join(current).is_file() && self.install_dir.join(legacy).is_file() {
            legacy
        } else {
            current
        }
    }

    fn turn_file(&self) -> &'static str {
        if self.dir_is_checkout {
            self.checkout_overlay(CHECKOUT_TURN, INSTALLED_TURN)
        } else {
            INSTALLED_TURN
        }
    }

    fn build_file(&self) -> &'static str {
        self.checkout_overlay(CHECKOUT_BUILD, LEGACY_BUILD)
    }

    fn settle_compose_files(&mut self) {
        self.set_compose_file.clear();
        self.override_file = OVERRIDES
            .iter()
            .copied()
            .find(|name| self.install_dir.join(name).is_file());
        if self.two_port_layout && self.turn_published {
            self.set_compose_file = format!("docker-compose.yml:{}", self.turn_file());
            if let Some(name) = self.override_file {
                self.set_compose_file.push(':');
                self.set_compose_file.push_str(name);
            }
        }
    }

    /// An update asks first in a terminal. Without one it needs --yes, as
    /// nothing may be confirmed on the person's behalf.
    fn confirm_update(&mut self) -> Step {
        self.updating = false;
        if self.o.build || self.installed_version == self.target_version {
            return Ok(());
        }
        // A moving tag in .env names no version to update from.
        let from = if self.installed_version == "dev" {
            "a build of the source".to_string()
        } else if is_exact_version(&self.installed_version) {
            self.installed_version.clone()
        } else {
            return Ok(());
        };
        self.updating = true;
        if self.o.assume_yes {
            return Ok(());
        }
        let target = self.target_version.clone();
        if !self.interactive {
            return Err(self.die(
                format!("Updating Stellarr from {from} to {target} needs a confirmation"),
                &["Run the installer in a terminal, or again with --yes."],
            ));
        }
        let backup = format!("{DOCS_URL}/install/upgrading");
        if !self.ui.confirm(
            true,
            &format!("Update Stellarr from {from} to {target}?"),
            &[
                "Settings and data stay. If you have no backup yet, make one first:",
                &backup,
            ],
        )? {
            self.ui.text(Fd::Out, &["", "Nothing was changed."]);
            return Err(Stop(0));
        }
        Ok(())
    }

    // ---- Ports ----

    /// The role of each port setting: its name on screen and its protocols.
    fn port_role(&self, key: Key) -> (&'static str, &'static [&'static str]) {
        match key {
            Key::HttpsPort => ("HTTPS", &["tcp"]),
            Key::HttpPort => ("HTTP", &["tcp"]),
            Key::TurnPort => ("TURN", &["tcp", "udp"]),
            Key::TurnsPort => ("TURN over TLS", &["tcp", "udp"]),
            Key::MediaPort if self.two_port_layout => ("voice media", &["tcp", "udp"]),
            Key::MediaPort => ("voice media", &["udp"]),
            _ => ("", &[]),
        }
    }

    /// Lists the ports something listens on, from one listing per protocol.
    /// False when none of ss, lsof and netstat is there to list them, or the
    /// one there fails.
    fn list_listening(&mut self) -> bool {
        let run = |run: &Run, program: &str, args: &[&str]| {
            let mut command = run.command(program);
            command.args(args);
            sys::output(command)
        };
        let (tool, tcp, udp) = if sys::have("ss") {
            let (Some(tcp), Some(udp)) = (run(self, "ss", &["-ltn"]), run(self, "ss", &["-lun"]))
            else {
                return false;
            };
            (Tool::Ss, tcp, udp)
        } else if sys::have("lsof") {
            // lsof finds nothing and fails when nothing listens, which is no
            // error.
            let tcp = run(self, "lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"]).unwrap_or_default();
            let udp = run(self, "lsof", &["-nP", "-iUDP"]).unwrap_or_default();
            (Tool::Lsof, tcp, udp)
        } else if sys::have("netstat") {
            let (Some(tcp), Some(udp)) = (
                run(self, "netstat", &["-lnt"]),
                run(self, "netstat", &["-lnu"]),
            ) else {
                return false;
            };
            (Tool::Netstat, tcp, udp)
        } else {
            return false;
        };
        self.listen_tcp = port_numbers(tool, &tcp);
        self.listen_udp = port_numbers(tool, &udp);
        true
    }

    fn port_busy(&self, port: u32, protos: &[&str]) -> bool {
        protos.iter().any(|proto| match *proto {
            "tcp" => self.listen_tcp.contains(&port),
            "udp" => self.listen_udp.contains(&port),
            _ => false,
        })
    }

    /// Whether the port setting `key` can take `port`: free on this machine
    /// for its protocols, and not one of Stellarr's other ports.
    fn port_is_available(&self, key: Key, port: u32) -> bool {
        let (_, protos) = self.port_role(key);
        if self.port_busy(port, protos) {
            return false;
        }
        let taken = self
            .port_keys
            .iter()
            .filter(|other| **other != key)
            .any(|other| {
                binding_port(self.set(*other))
                    .parse::<u32>()
                    .is_ok_and(|p| p == port)
            });
        !taken
            && !self
                .relay_bounds()
                .is_some_and(|(first, last)| (first..=last).contains(&port))
    }

    fn next_free_port(&self, key: Key) -> Option<u32> {
        let start: u32 = binding_port(self.set(key)).parse().unwrap_or(0);
        (start + 1..=65535).find(|port| self.port_is_available(key, *port))
    }

    /// The ports Docker will publish must be free, unless Stellarr's own
    /// container holds them. This runs before any file is written, so a busy
    /// port leaves nothing behind. In a terminal a busy port that was not
    /// chosen gets the next free one; otherwise the installer stops and says
    /// what moves it. The result is shown with the files, by show_ports.
    fn check_ports(&mut self) -> Step {
        self.ports_result = PortsResult::Free;
        if self.o.skip_port_check {
            self.ports_result = PortsResult::Skipped;
            return Ok(());
        }
        if !self.list_listening() {
            self.ports_result = PortsResult::Unknown;
            return Ok(());
        }
        if self.owns_container && self.container.running {
            self.remove_owned_listeners();
        }
        let busy: Vec<Key> = self
            .port_keys
            .iter()
            .copied()
            .filter(|key| {
                let port = binding_port(self.set(*key)).parse().unwrap_or(0);
                self.port_busy(port, self.port_role(*key).1)
            })
            .collect();
        let relay = self.relay_bounds();
        let relay_busy = relay.is_some_and(|(first, last)| {
            (first..=last).any(|port| self.listen_udp.contains(&port))
        });
        if busy.is_empty() && !relay_busy {
            return Ok(());
        }

        if self.interactive && !relay_busy && !busy.iter().any(|key| self.given(*key)) {
            for key in busy {
                let (role, _) = self.port_role(key);
                let current = self.set(key).to_string();
                self.ui.warn(
                    &format!("Port {current} ({role}) is in use on this machine"),
                    NONE,
                );
                let suggestion = self
                    .next_free_port(key)
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                self.ui
                    .question(&format!("Which port should {role} use?"), NONE);
                let port = loop {
                    let answer = self.ui.prompt(&suggestion)?;
                    let port = if answer.is_empty() {
                        suggestion.clone()
                    } else {
                        answer
                    };
                    match port_number(&port) {
                        None => self
                            .ui
                            .retry(&format!("\"{port}\" is not a port from 1 to 65535")),
                        Some(number) if !self.port_is_available(key, number) => {
                            self.ui.retry(&format!(
                                "Port {port} is in use, or another of Stellarr's ports has it"
                            ));
                        }
                        Some(_) => break port,
                    }
                };
                // A port chosen here is the person's own choice, written to
                // .env.
                let value = match current.rsplit_once(':') {
                    Some((host, _)) => format!("{host}:{port}"),
                    None => port,
                };
                self.put(key, value);
                self.o.setting_mut(key).given = true;
            }
            return Ok(());
        }

        let mut lines: Vec<String> = Vec::new();
        for key in busy {
            let (role, _) = self.port_role(key);
            let value = self.set(key);
            if self.given(key) {
                lines.push(format!(
                    "  {value} ({role}), as given: choose another with {}",
                    key.option()
                ));
            } else {
                lines.push(format!("  {value} ({role}): move it with {}", key.option()));
            }
        }
        // The container maps the range one to one, so no option moves it.
        if relay_busy && let Some((first, last)) = relay {
            lines.push(format!(
                "  {} (TURN relay): set STELLARR_TURN_RELAY_RANGE in .env to",
                self.relay_range
            ));
            lines.push(format!(
                "  another free range of {} ports, as described in",
                last - first + 1
            ));
            lines.push(format!("  {DOCS_URL}/rtc-and-ports"));
        }
        lines.push(String::new());
        lines.push("Free them, or run the installer again with the changes above.".to_string());
        Err(self.die("Ports Stellarr needs are in use on this machine", &lines))
    }

    fn remove_owned_listeners(&mut self) {
        let Some(bindings) = self
            .container_format("{{json .NetworkSettings.Ports}}")
            .and_then(|json| {
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&json).ok()
            })
        else {
            return;
        };
        let relay = self.relay_bounds();
        let bindings = bindings
            .iter()
            .filter_map(|(key, entries)| {
                let (_, protocol) = key.split_once('/')?;
                Some(
                    entries
                        .as_array()?
                        .iter()
                        .map(move |entry| (protocol, entry)),
                )
            })
            .flatten();
        for (protocol, entry) in bindings {
            let Some(port) = entry["HostPort"].as_str().and_then(port_number) else {
                continue;
            };
            let address = entry["HostIp"].as_str().unwrap_or_default();
            let wildcard = address == "0.0.0.0";
            let requested = self.port_keys.iter().any(|&key| {
                let value = self.set(key);
                let host = value.rsplit_once(':').map_or("", |(host, _)| host);
                port_number(binding_port(value)) == Some(port)
                    && self.port_role(key).1.contains(&protocol)
                    && (wildcard || (!host.is_empty() && host == address))
            }) || (wildcard
                && protocol == "udp"
                && relay.is_some_and(|(first, last)| (first..=last).contains(&port)));
            if requested {
                let removed = match protocol {
                    "tcp" => self.listen_tcp.remove(&port),
                    "udp" => self.listen_udp.remove(&port),
                    _ => false,
                };
                if removed {
                    self.ports_result = PortsResult::Held;
                }
            }
        }
    }

    fn show_ports(&mut self) {
        let mut text = format!(
            "{}, {}, {}",
            self.set(Key::HttpsPort),
            self.set(Key::HttpPort),
            self.set(Key::MediaPort)
        );
        if self.turn_published {
            text = format!(
                "{text}, {}, {}, {}",
                self.set(Key::TurnPort),
                self.set(Key::TurnsPort),
                self.relay_range
            );
        }
        match self.ports_result {
            PortsResult::Free => self.ui.ok(&format!("Ports are free: {text}"), NONE),
            PortsResult::Held => self.ui.ok(
                "Some of these ports are held by Stellarr's running container",
                NONE,
            ),
            PortsResult::Skipped => self.ui.warn("Did not check the ports, as asked", NONE),
            PortsResult::Unknown => self.ui.warn(
                "Could not check the ports: no ss, lsof or netstat that works",
                NONE,
            ),
        }
    }

    // ---- The files ----

    fn release_file(&mut self, name: &str) -> Step<Vec<u8>> {
        let url = format!("{RAW_URL}/{}/{name}", self.files_ref);
        let contents = self.http.get(&url).map_err(|error| {
            self.die(
                format!("Could not download {name} {}", self.compose_label),
                &[url, error],
            )
        })?;
        self.check_checkout_match(name, &contents)?;
        Ok(contents)
    }

    fn check_checkout_match(&mut self, name: &str, contents: &[u8]) -> Step {
        if self.dir_is_checkout
            && fs::read(self.install_dir.join(name)).as_deref().ok() != Some(contents)
        {
            return Err(self.die(
                format!(
                    "The checkout's {name} differs from the copy {}",
                    self.compose_label
                ),
                &[
                    "Check out the selected Git ref in this working tree to update an existing installation.",
                    "For a separate installation, use --dir. To run this checkout, use --build.",
                ],
            ));
        }
        Ok(())
    }

    fn checkout_file(&mut self, name: &str) -> Step<Vec<u8>> {
        let path = self.install_dir.join(name);
        fs::read(&path).map_err(|error| {
            self.die(
                format!("Could not read {}", display(&path)),
                &[error.to_string()],
            )
        })
    }

    fn fetch_compose_files(&mut self) -> Step {
        let compose = if self.o.build {
            self.checkout_file(self.build_file())?;
            self.checkout_file("docker-compose.yml")?
        } else {
            let label = if self.o.reference.is_empty() {
                &self.target_version
            } else {
                &self.files_ref
            };
            self.compose_label = format!("for {label}");
            let compose = self.release_file("docker-compose.yml")?;
            if !is_stellarr_compose(&compose) {
                return Err(self.die(
                    format!(
                        "The docker-compose.yml downloaded {} is not Stellarr's",
                        self.compose_label
                    ),
                    NONE,
                ));
            }
            let example = self.release_file(".env.example")?;
            if !self.dir_is_checkout {
                self.staged_compose = Some(compose.clone());
                self.staged_example = Some(example);
            }
            compose
        };
        self.two_port_layout = String::from_utf8_lossy(&compose).contains("STELLARR_TURN_ENABLED");
        Ok(())
    }

    fn fetch_turn_overlay(&mut self) -> Step {
        if !(self.two_port_layout && self.turn_published) {
            return Ok(());
        }
        let local = self.turn_file();
        if self.o.build {
            self.checkout_file(local)?;
        } else {
            let url = format!("{RAW_URL}/{}/{CHECKOUT_TURN}", self.files_ref);
            let overlay = match self.http.get_optional(&url).map_err(|error| {
                self.die(
                    format!("Could not download {CHECKOUT_TURN} {}", self.compose_label),
                    &[url, error],
                )
            })? {
                Some(overlay) => overlay,
                None => {
                    let url = format!("{RAW_URL}/{}/{INSTALLED_TURN}", self.files_ref);
                    self.http.get(&url).map_err(|error| {
                        self.die(
                            format!("Could not download {INSTALLED_TURN} {}", self.compose_label),
                            &[url, error],
                        )
                    })?
                }
            };
            self.check_checkout_match(local, &overlay)?;
            if !self.dir_is_checkout {
                self.staged_turn = Some(overlay);
            }
        }
        Ok(())
    }

    // ---- Starting ----

    fn pull_image(&mut self, config_dir: &Path) -> Step {
        let mut args = vec!["pull"];
        if !self.ui.terminal {
            args.push("--quiet");
        }
        args.extend(["--policy", "always", "stellarr"]);
        let command = self.compose_at(config_dir, &args);
        let (ok, log) = if self.ui.terminal {
            (sys::shown(command), Vec::new())
        } else {
            sys::logged(command)
        };
        let image = self.target_image.clone();
        if ok && sys::quiet(self.docker(["image", "inspect", &image])) {
            self.ui.ok(&format!("Pulled {image}"), NONE);
            return Ok(());
        }
        self.ui.fail(&format!("Could not pull {image}"), NONE);
        if self.ui.terminal {
            self.ui.text(Fd::Err, &["", "Docker's message is above."]);
        } else {
            self.ui.block(Fd::Err, &log);
        }
        let check = format!(
            "Check that this machine reaches ghcr.io and that {} is a release:",
            self.target_tag
        );
        self.ui.text(
            Fd::Err,
            &[
                "",
                &check,
                &format!("{SERVER_DOWNLOADS}/{}", self.target_tag),
            ],
        );
        Err(Stop(1))
    }

    /// Waits until stellarr-init reports every service ready, checking once a
    /// second. Docker's own health status lags by up to 30 seconds, so the
    /// installer asks stellarr-init itself.
    fn wait_until_up(&mut self) -> Step {
        let started = Instant::now();
        let mut checks: u64 = 0;
        let mut base: Option<u64> = None;
        let limit = self.o.start_timeout_secs;
        let deadline = started.checked_add(Duration::from_secs(limit));
        loop {
            // Each check waits a second, so the time counts at least one per
            // check, even where the clock does not move.
            let elapsed = started.elapsed().as_secs().max(checks);
            if elapsed >= limit {
                return Err(self.start_failed(&format!("Stellarr did not start within {limit} s")));
            }
            let info = sys::output_until(
                self.docker([
                    "container",
                    "inspect",
                    "--format",
                    "{{.State.Status}} {{.RestartCount}}",
                    "stellarr",
                ]),
                deadline,
            )
            .unwrap_or_default();
            let mut fields = info.split_whitespace();
            let state = fields.next().unwrap_or_default().to_string();
            let restarts: u64 = fields.last().and_then(|r| r.parse().ok()).unwrap_or(0);
            let base = *base.get_or_insert(restarts);
            if state == "exited" || state == "dead" {
                return Err(self.start_failed(&format!(
                    "The container stopped ({state}) while Stellarr was starting"
                )));
            }
            if restarts.saturating_sub(base) >= 2 {
                let times = restarts - base;
                return Err(self.start_failed(&format!(
                    "The container restarted {times} times while Stellarr was starting"
                )));
            }
            if state == "running"
                && sys::quiet_until(
                    self.docker(["exec", "stellarr", "stellarr-init", "health"]),
                    deadline,
                )
            {
                let elapsed = started.elapsed().as_secs().max(checks);
                if elapsed == 0 {
                    self.ui.ok("Stellarr is up", NONE);
                } else {
                    self.ui
                        .ok(&format!("Stellarr is up after {elapsed} s"), NONE);
                }
                return Ok(());
            }
            let remaining = Duration::from_secs(limit).saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(self.start_failed(&format!("Stellarr did not start within {limit} s")));
            }
            self.ui.wait(elapsed, limit);
            if !self.fast_clock {
                std::thread::sleep(remaining.min(Duration::from_secs(1)));
            }
            checks += 1;
        }
    }

    fn start_failed(&mut self, problem: &str) -> Stop {
        self.ui.fail(problem, NONE);
        self.ui.section("What its services report");
        let (ok, status) = sys::logged_until(
            self.docker(["exec", "stellarr", "stellarr-init", "status"]),
            Some(Instant::now() + Duration::from_secs(1)),
        );
        if ok || !status.is_empty() {
            self.ui.block(Fd::Out, &status);
        } else {
            self.ui
                .text(Fd::Out, &["Nothing: the container is not running."]);
        }
        self.ui.section("Last lines of the container's log");
        let (_, logs) = sys::logged_until(
            self.docker(["logs", "--tail", "40", "stellarr"]),
            Some(Instant::now() + Duration::from_secs(1)),
        );
        self.ui.block(Fd::Out, &logs);
        let all = format!("All logs: {}", self.compose_command("logs"));
        self.ui.text(
            Fd::Out,
            &[
                "",
                "The saved configuration is in place. Fix the cause and run the installer again.",
                &all,
            ],
        );
        Stop(1)
    }

    /// Asks the instance whether it still waits for its owner.
    fn query_setup(&self) -> Option<bool> {
        let url = format!(
            "https://127.0.0.1:{}/api/auth/registration",
            self.set(Key::HttpsPort)
        );
        let body: String = self
            .http
            .loopback(&url)?
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if body.contains("\"setupRequired\":true") {
            Some(true)
        } else if body.contains("\"setupRequired\":false") {
            Some(false)
        } else {
            None
        }
    }

    /// The newest setup code in the container's log since it last started;
    /// each start draws a new one.
    fn find_setup_code(&self) -> String {
        let started = self
            .container_format("{{.State.StartedAt}}")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let mut args = vec!["logs"];
        if !started.is_empty() {
            args.extend(["--since", started.as_str()]);
        }
        args.push("stellarr");
        let (_, logs) = sys::logged(self.docker(args));
        let logs = String::from_utf8_lossy(&logs);
        let mut code = String::new();
        for line in logs.lines() {
            if let Some(at) = line.rfind("Setup code: ") {
                code = line[at + 12..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
            }
        }
        code
    }

    fn show_not_started(&mut self) {
        let start = if self.o.build {
            let mut args = "-f docker-compose.yml".to_string();
            if self.two_port_layout && self.turn_published {
                args.push_str(&format!(" -f {}", self.turn_file()));
            }
            args.push_str(&format!(" -f {}", self.build_file()));
            if let Some(file) = self.override_file {
                args.push_str(&format!(" -f {}", shell_word(file)));
            }
            args.push_str(" up -d --build");
            self.compose_command(&args)
        } else {
            self.compose_command("up -d")
        };
        let command = format!("  {start}");
        self.ui.section("Ready to start");
        self.ui.text(
            Fd::Out,
            &[
                "Nothing was started, as asked. To start Stellarr:",
                &command,
            ],
        );
    }

    fn show_summary(&mut self) {
        let version = self
            .container_format("{{index .Config.Labels \"org.opencontainers.image.version\"}}")
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        let title = match version.as_str() {
            "" | "dev" | "<no value>" => "Stellarr is running".to_string(),
            version => format!("Stellarr {version} is running"),
        };
        let port = if self.set(Key::PublicPort).is_empty() {
            self.set(Key::HttpsPort)
        } else {
            self.set(Key::PublicPort)
        };
        let mut url = format!("https://{}", self.set(Key::Domain));
        if port != "443" {
            url = format!("{url}:{port}");
        }

        let setup = self.query_setup();
        let code = if setup == Some(false) {
            String::new()
        } else {
            self.find_setup_code()
        };
        self.ui.section(&title);
        self.ui.text(Fd::Out, &[""]);
        let grep = format!(
            "  {}",
            self.compose_command("logs stellarr | grep \"Setup code\"")
        );
        let setup_url = format!("{url}/setup");
        match setup {
            Some(true) if !code.is_empty() => {
                self.ui
                    .rows(true, &[("Open", &setup_url), ("Setup code", &code)]);
                self.ui.text(
                    Fd::Out,
                    &[
                        "",
                        "Enter the code there to create the owner account. A restart of the",
                        "container draws a new code.",
                    ],
                );
            }
            Some(true) => {
                self.ui.rows(true, &[("Open", &setup_url)]);
                self.ui.text(
                    Fd::Out,
                    &[
                        "",
                        "Create the owner account there with the setup code from the log:",
                        &grep,
                    ],
                );
            }
            Some(false) => {
                self.ui.rows(true, &[("Open", &url)]);
                self.ui.text(
                    Fd::Out,
                    &["", "The owner account exists; sign in as usual."],
                );
            }
            None => {
                if code.is_empty() {
                    self.ui.rows(true, &[("Open", &url)]);
                } else {
                    self.ui.rows(true, &[("Open", &url), ("Setup code", &code)]);
                }
                let at = format!("account at {setup_url} with the newest code from:");
                self.ui.text(
                    Fd::Out,
                    &[
                        "",
                        "The instance did not say whether it has an owner yet. If not, create the",
                        &at,
                        &grep,
                    ],
                );
            }
        }
        if setup == Some(false) {
            return;
        }
        self.show_notes();
        self.ui.section("Later");
        let logs = self.compose_command("logs -f");
        let stop = self.compose_command("stop");
        self.ui.rows(
            false,
            &[
                ("Update", "run this installer again"),
                ("Logs", &logs),
                ("Stop", &stop),
            ],
        );
        self.ui.text(Fd::Out, &[""]);
    }

    fn show_notes(&mut self) {
        let media = self.set(Key::MediaPort).to_string();
        let voice = if self.two_port_layout {
            format!("{media}/UDP and TCP")
        } else {
            format!("{media}/UDP")
        };
        let domain = self.set(Key::Domain).to_string();
        let port = if self.set(Key::PublicPort).is_empty() {
            self.set(Key::HttpsPort)
        } else {
            self.set(Key::PublicPort)
        };
        let https = format!("Web and API clients use HTTPS on {port}/TCP.");
        self.ui.section("Before you invite anyone");
        self.ui.bullet(&https, NONE);
        self.ui.bullet(
            "Browsers warn about the self-signed certificate. For a trusted one:",
            &[format!("{DOCS_URL}/install/reverse-proxy-and-tls")],
        );
        if is_local_address(&domain) {
            self.ui.bullet(
                &format!("{domain} opens on this machine only. For other devices, run the"),
                &["installer again with --domain and this machine's domain or IP address."],
            );
        } else if self.set(Key::PublicIp).is_empty() && !is_ipv4(&domain) {
            self.ui.bullet(
                "Set STELLARR_PUBLIC_IP to the address other devices use, with --public-ip or in .env.",
                &[format!("Forward {voice} to this machine for calls from other networks.")],
            );
        } else {
            self.ui.bullet(
                &format!("For voice from other networks, forward {voice}"),
                &["to this machine."],
            );
        }
        if self.turn_published && !is_local_address(&domain) {
            let (turn, turns, range) = (
                self.set(Key::TurnPort),
                self.set(Key::TurnsPort),
                &self.relay_range,
            );
            let lines = [
                format!("If you enable TURN/TCP, forward {turn}/TCP. For TURN over TLS or DTLS,"),
                format!("forward {turns}/TCP and UDP."),
            ];
            let first =
                format!("For TURN, also forward {turn}/UDP and the UDP relay range {range}.");
            self.ui.bullet(&first, &lines);
        }
    }
}

/// Only the Compose file lists this installer can write are safe to reuse.
fn managed_turn_files(list: &str) -> bool {
    let mut files = list.split(':');
    files.next() == Some("docker-compose.yml")
        && matches!(files.next(), Some(CHECKOUT_TURN | INSTALLED_TURN))
        && files.next().is_none_or(|path| OVERRIDES.contains(&path))
        && files.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_stellarr_compose() {
        assert!(is_stellarr_compose(
            b"# x\nname: stellarr\r\nservices: {}\n"
        ));
        assert!(is_stellarr_compose(b"name:stellarr  \n"));
        assert!(!is_stellarr_compose(b"name: other\n"));
        assert!(!is_stellarr_compose(b"  name: stellarr\n"));
    }

    #[test]
    fn normalizes_missing_path_components() {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(
            absolute_path("./missing/../future/./child/.."),
            cwd.join("future")
        );
        assert_eq!(absolute_path("/../../"), PathBuf::from("/"));
    }

    #[test]
    fn quotes_shell_words() {
        assert_eq!(shell_word("/home/ada/stellarr"), "/home/ada/stellarr");
        assert_eq!(shell_word("/srv/my dir"), "'/srv/my dir'");
        assert_eq!(shell_word("/srv/it's"), "'/srv/it'\\''s'");
    }

    #[test]
    fn shows_gib() {
        assert_eq!(gib(43_000_000), "41 GiB");
        assert_eq!(gib(1_048_576), "1.0 GiB");
        assert_eq!(gib(1_572_864), "1.5 GiB");
    }

    #[test]
    fn approves_only_known_compose_files() {
        assert!(managed_turn_files(
            "docker-compose.yml:docker-compose.turn.yml"
        ));
        assert!(managed_turn_files(
            "docker-compose.yml:infra/compose/turn.yml:compose.override.yml"
        ));
        assert!(!managed_turn_files(
            "docker-compose.yml:infra/compose/turn.yml:compose.override.yml:other.yml"
        ));
        assert!(!managed_turn_files("other.yml"));
    }
}
