//! Asking for a secret at a terminal without showing it.
//!
//! The standard library cannot switch the echo off and this code may not call
//! into the system itself, so `stty` does. When it cannot, nothing is read: a
//! prompt that said the input is hidden must never show it, and what was
//! typed would stay in the scrollback.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Why a secret was not read at a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptFailure {
    /// The echo could not be turned off, so nothing was asked.
    EchoStays,
    /// The person pressed Ctrl+C, or the terminal went away, while asking.
    Interrupted,
}

/// What switches the terminal's echo.
pub trait Echo {
    /// Turns the echo on or off; whether it is now as asked.
    fn set(&self, on: bool) -> bool;
}

/// `stty`, found on the path or at a place a test names.
pub struct Stty {
    program: PathBuf,
}

impl Stty {
    pub fn system() -> Stty {
        Stty {
            program: PathBuf::from("stty"),
        }
    }

    pub fn at(program: impl Into<PathBuf>) -> Stty {
        Stty {
            program: program.into(),
        }
    }
}

impl Echo for Stty {
    fn set(&self, on: bool) -> bool {
        Command::new(&self.program)
            .arg(if on { "echo" } else { "-echo" })
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// Set while the echo is off, so that a signal that ends the process can put
/// it back first.
static ECHO_IS_OFF: AtomicBool = AtomicBool::new(false);

/// Turns the echo back on if a prompt left it off. For the code that handles
/// the signals that end the process, which no guard outlives.
pub fn restore_after_signal() {
    if ECHO_IS_OFF.swap(false, Ordering::SeqCst) {
        Stty::system().set(true);
    }
}

/// Puts the echo back on every way out of the prompt, a panic included.
struct EchoGuard<'a>(&'a dyn Echo);

impl Drop for EchoGuard<'_> {
    fn drop(&mut self) {
        self.0.set(true);
        ECHO_IS_OFF.store(false, Ordering::SeqCst);
    }
}

/// How often a prompt looks at whether Ctrl+C was pressed while it waits.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// Asks for one line with the echo off. `read_line` runs on a thread of its
/// own so that Ctrl+C ends the wait at once: the read itself cannot be
/// interrupted, and the process leaves it behind.
pub fn ask_hidden(
    echo: &dyn Echo,
    cancelled: &AtomicBool,
    error: &mut dyn Write,
    read_line: impl FnOnce() -> io::Result<String> + Send + 'static,
) -> Result<String, PromptFailure> {
    if !echo.set(false) {
        return Err(PromptFailure::EchoStays);
    }
    ECHO_IS_OFF.store(true, Ordering::SeqCst);
    let _guard = EchoGuard(echo);
    let _ = write!(error, "Secret value (input is hidden): ");
    let _ = error.flush();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(read_line());
    });
    let line = loop {
        if cancelled.load(Ordering::Relaxed) {
            break None;
        }
        match receiver.recv_timeout(LOOK_EVERY) {
            Ok(line) => break Some(line),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break None,
        }
    };
    let _ = writeln!(error);
    // A line that arrived together with Ctrl+C is not an answer.
    if cancelled.load(Ordering::Relaxed) {
        return Err(PromptFailure::Interrupted);
    }
    match line {
        Some(Ok(line)) => Ok(line.trim_end_matches(['\n', '\r']).to_string()),
        Some(Err(_)) | None => Err(PromptFailure::Interrupted),
    }
}

/// The prompt on the real terminal.
pub fn ask_on_terminal(cancelled: &AtomicBool) -> Result<String, PromptFailure> {
    ask_hidden(&Stty::system(), cancelled, &mut io::stderr(), || {
        let mut line = String::new();
        io::BufRead::read_line(&mut io::stdin().lock(), &mut line).map(|_| line)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Records what was done to the echo and in what order against the read.
    struct Recorder {
        works: bool,
        events: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Echo for Recorder {
        fn set(&self, on: bool) -> bool {
            self.events
                .lock()
                .unwrap()
                .push(if on { "echo on" } else { "echo off" }.to_string());
            self.works || on
        }
    }

    fn recorder(works: bool) -> (Recorder, Arc<std::sync::Mutex<Vec<String>>>) {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        (
            Recorder {
                works,
                events: events.clone(),
            },
            events,
        )
    }

    #[test]
    fn nothing_is_asked_or_read_when_the_echo_cannot_be_turned_off() {
        let (echo, events) = recorder(false);
        let mut shown = Vec::new();
        let result = ask_hidden(&echo, &AtomicBool::new(false), &mut shown, || {
            panic!("the input was read with the echo on")
        });
        assert_eq!(result, Err(PromptFailure::EchoStays));
        assert!(shown.is_empty(), "the prompt said the input was hidden");
        assert_eq!(*events.lock().unwrap(), ["echo off"]);
    }

    #[test]
    fn the_echo_is_off_while_reading_and_back_on_after() {
        let (echo, events) = recorder(true);
        let seen = events.clone();
        let mut shown = Vec::new();
        let result = ask_hidden(&echo, &AtomicBool::new(false), &mut shown, move || {
            seen.lock().unwrap().push("read".to_string());
            Ok("s3cret\r\n".to_string())
        });
        assert_eq!(result.as_deref(), Ok("s3cret"));
        assert_eq!(*events.lock().unwrap(), ["echo off", "read", "echo on"]);
        assert!(String::from_utf8_lossy(&shown).contains("input is hidden"));
    }

    #[test]
    fn ctrl_c_ends_the_wait_and_puts_the_echo_back() {
        let (echo, events) = recorder(true);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            flag.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = ask_hidden(&echo, &cancelled, &mut Vec::new(), || {
            std::thread::sleep(Duration::from_secs(30));
            Ok(String::new())
        });
        assert_eq!(result, Err(PromptFailure::Interrupted));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(events.lock().unwrap().last().unwrap(), "echo on");
    }

    #[test]
    fn a_line_that_comes_with_ctrl_c_is_not_an_answer() {
        let (echo, _events) = recorder(true);
        let cancelled = AtomicBool::new(true);
        let result = ask_hidden(&echo, &cancelled, &mut Vec::new(), || Ok("late\n".into()));
        assert_eq!(result, Err(PromptFailure::Interrupted));
    }

    #[test]
    fn the_echo_is_back_on_when_the_read_panics() {
        let (echo, events) = recorder(true);
        let outcome = ask_hidden(&echo, &AtomicBool::new(false), &mut Vec::new(), || {
            panic!("the read failed")
        });
        // The reader's panic ends its thread; the prompt sees no line.
        assert_eq!(outcome, Err(PromptFailure::Interrupted));
        assert_eq!(events.lock().unwrap().last().unwrap(), "echo on");
    }

    #[test]
    fn stty_that_fails_is_a_failure_to_hide() {
        let dir = std::env::temp_dir().join(format!("stty-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls");
        let script = dir.join("stty");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho \"$@\" >> {}\nexit 1\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let result = ask_hidden(
            &Stty::at(&script),
            &AtomicBool::new(false),
            &mut Vec::new(),
            || panic!("the input was read with the echo on"),
        );
        assert_eq!(result, Err(PromptFailure::EchoStays));
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "-echo");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
