//! What the installer asks of the system: commands on PATH, the user, the
//! clock, memory and disk, and how an interrupt ends the run.

use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicPtr, Ordering};

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

/// The command's output, when it ran and succeeded; its errors are dropped.
pub fn output(mut command: Command) -> Option<String> {
    let out = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether the command ran and succeeded, with all its output dropped.
pub fn quiet(mut command: Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether the command ran and succeeded, with its output on the screen.
pub fn shown(mut command: Command) -> bool {
    command
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Runs the command with its output and errors interleaved in one log, as a
/// person would see them, and returns whether it succeeded and the log. The
/// log is a file removed as soon as it is open, so nothing is left behind.
pub fn logged(mut command: Command) -> (bool, Vec<u8>) {
    let path = std::env::temp_dir().join(format!(
        "stellarr-installer.{}.{}.log",
        std::process::id(),
        unique()
    ));
    let Ok(mut log) = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
    else {
        let out = command.stdin(Stdio::null()).output();
        return match out {
            Ok(out) => {
                let mut text = out.stdout;
                text.extend_from_slice(&out.stderr);
                (out.status.success(), text)
            }
            Err(error) => (false, error.to_string().into_bytes()),
        };
    };
    let _ = std::fs::remove_file(&path);
    let (Ok(out), Ok(err)) = (log.try_clone(), log.try_clone()) else {
        return (false, b"could not open a log".to_vec());
    };
    let ok = match command
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .status()
    {
        Ok(status) => status.success(),
        Err(error) => {
            return (
                false,
                format!("{}: {error}", command.get_program().to_string_lossy()).into_bytes(),
            );
        }
    };
    let mut text = Vec::new();
    let _ = log.seek(SeekFrom::Start(0));
    let _ = log.read_to_end(&mut text);
    (ok, text)
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

extern "C" fn on_signal(signal: libc::c_int) {
    // Only what is safe in a signal handler: plain writes, unlink and _exit.
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
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}
