//! What the installer asks of the system: commands on PATH, the user, the
//! clock, memory and disk, and how an interrupt ends the run.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use std::time::{Duration, Instant};

use crate::ui::{ASK_FD, CURSOR_HIDDEN, OPEN, TERMINAL};

/// The full path of `name` on PATH, when it is an executable file there.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

pub fn have(name: &str) -> bool {
    which(name).is_some()
}

// A negative ID addresses the entire process group. Interactive sudo stays
// in the foreground group so it can still open the terminal for a password.
static ACTIVE_CHILD: AtomicI32 = AtomicI32::new(0);

fn run(mut command: Command, deadline: Option<Instant>) -> io::Result<ExitStatus> {
    let interactive_sudo = command.get_program() == "sudo"
        && !command.get_args().any(|arg| arg == "-n")
        && deadline.is_none();
    if !interactive_sudo {
        command.process_group(0);
    }

    // Block cancellation across spawn and registration. The child restores
    // the caller's signal mask before it executes the requested program.
    let mut signals: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: both masks are local, initialized by libc and used by this thread.
    unsafe {
        libc::sigemptyset(&mut signals);
        libc::sigaddset(&mut signals, libc::SIGINT);
        libc::sigaddset(&mut signals, libc::SIGTERM);
        let result = libc::pthread_sigmask(libc::SIG_BLOCK, &signals, &mut previous);
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result));
        }
        command.pre_exec(move || {
            if libc::sigprocmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut()) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
    let spawned = command.stdin(Stdio::null()).spawn();
    if let Ok(child) = &spawned {
        let pid = child.id() as i32;
        ACTIVE_CHILD.store(if interactive_sudo { pid } else { -pid }, Ordering::SeqCst);
    }
    // SAFETY: restores the signal mask saved above.
    unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut()) };
    let mut child = spawned?;
    let result = match deadline {
        None => child.wait(),
        Some(deadline) => loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // SAFETY: the group belongs to the spawned, unreaped child.
                unsafe { signal_child(-(child.id() as i32), libc::SIGTERM) };
                let _ = child.wait();
                break Err(io::Error::new(io::ErrorKind::TimedOut, "command timed out"));
            }
            match child.try_wait() {
                Ok(None) => std::thread::sleep(remaining.min(Duration::from_millis(25))),
                result => break result.map(Option::unwrap),
            }
        },
    };
    ACTIVE_CHILD.store(0, Ordering::SeqCst);
    result
}

fn log_file() -> io::Result<File> {
    let path = std::env::temp_dir().join(format!(
        "stellarr-installer.{}.{}.log",
        std::process::id(),
        unique()
    ));
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

fn read_log(mut file: File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Captures stdout and stderr separately without blocking on inherited pipes.
pub fn capture(mut command: Command) -> io::Result<Output> {
    let stdout = log_file()?;
    let stderr = log_file()?;
    command
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    let status = run(command, None)?;
    Ok(Output {
        status,
        stdout: read_log(stdout)?,
        stderr: read_log(stderr)?,
    })
}

/// The command's output, when it ran and succeeded; its errors are dropped.
pub fn output(command: Command) -> Option<String> {
    output_until(command, None)
}

pub fn output_until(mut command: Command, deadline: Option<Instant>) -> Option<String> {
    let stdout = log_file().ok()?;
    command
        .stdout(stdout.try_clone().ok()?)
        .stderr(Stdio::null());
    let status = run(command, deadline).ok()?;
    status.success().then(|| {
        read_log(stdout)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    })?
}

/// Whether the command ran and succeeded, with all its output dropped.
pub fn quiet(command: Command) -> bool {
    quiet_until(command, None)
}

pub fn quiet_until(mut command: Command, deadline: Option<Instant>) -> bool {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    run(command, deadline).is_ok_and(|status| status.success())
}

/// Whether the command ran and succeeded, with its output on the screen.
pub fn shown(command: Command) -> bool {
    run(command, None).is_ok_and(|status| status.success())
}

/// Runs the command with its output and errors interleaved in one log.
pub fn logged(command: Command) -> (bool, Vec<u8>) {
    logged_until(command, None)
}

pub fn logged_until(mut command: Command, deadline: Option<Instant>) -> (bool, Vec<u8>) {
    let result = (|| {
        let log = log_file()?;
        command.stdout(log.try_clone()?).stderr(log.try_clone()?);
        let status = run(command, deadline)?;
        Ok::<_, io::Error>((status.success(), read_log(log)?))
    })();
    result.unwrap_or_else(|error| (false, error.to_string().into_bytes()))
}

/// A number no other name of this run has had.
pub fn unique() -> u64 {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    (u64::from(nanos) << 16) ^ COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub fn effective_uid() -> u32 {
    #[cfg(feature = "test-seams")]
    if let Some(uid) = std::env::var("STELLARR_INSTALLER_TEST_UID")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return uid;
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The local time as 20261120-183012, for the names of backups.
pub fn stamp() -> String {
    // SAFETY: time and localtime_r write only into the values passed.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return format!("{now}");
        }
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

/// KiB free for this user on the file system of `path`.
pub fn free_kib(path: &Path) -> Option<u64> {
    let path = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: statvfs writes only into the value passed, and the path is a
    // valid C string.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut stat) != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(stat.f_bavail as u64 * stat.f_frsize as u64 / 1024)
    }
}

/// The memory of this machine in KiB.
pub fn memory_kib(macos: bool) -> Option<u64> {
    if macos {
        let bytes: u64 = output({
            let mut c = Command::new("sysctl");
            c.args(["-n", "hw.memsize"]);
            c
        })?
        .trim()
        .parse()
        .ok()?;
        return Some(bytes / 1024);
    }
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next() == Some("MemTotal:"))
            .then(|| fields.next()?.parse().ok())
            .flatten()
    })
}

// A file to remove when the run is interrupted, kept where the signal
// handler can reach it.
static TEMPORARY: AtomicPtr<libc::c_char> = AtomicPtr::new(std::ptr::null_mut());

/// Notes a file to remove if the run is interrupted before it is done with
/// it; `None` forgets it.
pub fn temporary(path: Option<&Path>) {
    let new = path
        .and_then(|p| CString::new(p.as_os_str().as_encoded_bytes()).ok())
        .map_or(std::ptr::null_mut(), CString::into_raw);
    let old = TEMPORARY.swap(new, Ordering::SeqCst);
    if !old.is_null() {
        // SAFETY: every pointer stored there came from CString::into_raw.
        drop(unsafe { CString::from_raw(old) });
    }
}

fn write_raw(fd: libc::c_int, text: &[u8]) {
    // SAFETY: write is async-signal-safe and reads only the given bytes.
    unsafe {
        libc::write(fd, text.as_ptr().cast(), text.len());
    }
}

/// Sends the requested signal to the child or its own group. The group gets
/// a short grace period to stop before any remaining processes are killed.
unsafe fn signal_child(child: libc::pid_t, signal: libc::c_int) {
    // SAFETY: the caller supplies a tracked child or process group, and
    // kill and nanosleep are async-signal-safe.
    unsafe {
        libc::kill(child, signal);
        if child < 0 {
            let grace = libc::timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000,
            };
            libc::nanosleep(&grace, std::ptr::null_mut());
            libc::kill(child, libc::SIGKILL);
        }
    }
}

extern "C" fn on_signal(signal: libc::c_int) {
    let child = ACTIVE_CHILD.load(Ordering::SeqCst);
    if child != 0 {
        // SAFETY: only run registers the active child's ID here.
        unsafe { signal_child(child, signal) };
    }
    match OPEN.load(Ordering::Relaxed) {
        2 => write_raw(1, b"\r\x1b[K"),
        1 => write_raw(ASK_FD.load(Ordering::Relaxed), b"\n"),
        _ => {}
    }
    if CURSOR_HIDDEN.load(Ordering::Relaxed) {
        write_raw(1, b"\x1b[?25h");
    }
    if TERMINAL.load(Ordering::Relaxed) {
        write_raw(2, b"\n  Cancelled.\n");
    } else {
        write_raw(2, b"Cancelled.\n");
    }
    let temporary = TEMPORARY.load(Ordering::SeqCst);
    // SAFETY: unlink and _exit are async-signal-safe; the path, when set, is
    // a valid C string that is never freed while set.
    unsafe {
        if !temporary.is_null() {
            libc::unlink(temporary);
        }
        libc::_exit(if signal == libc::SIGINT { 130 } else { 143 });
    }
}

/// An interrupt or a TERM ends the run with the screen tidied, "Cancelled."
/// and the status a shell would give, 130 or 143.
pub fn handle_signals() {
    // SAFETY: the handler does only async-signal-safe work.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaddset(&mut action.sa_mask, libc::SIGINT);
        libc::sigaddset(&mut action.sa_mask, libc::SIGTERM);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}
