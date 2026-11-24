//! The terminal's side of `stellarr`: the arguments, the environment, the
//! input and the two output streams, and Ctrl+C.

use std::io::{self, BufRead, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use stellarr_cli::Io;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let environment: Vec<(String, String)> = std::env::vars().collect();
    let cancelled = Arc::new(AtomicBool::new(false));
    watch_for_interrupt(&cancelled);
    let interactive = io::stdin().is_terminal();
    let io = Io {
        input: Box::new(io::BufReader::new(io::stdin())),
        output: Box::new(io::stdout()),
        error: Box::new(io::stderr()),
        interactive,
        secret_prompt: interactive
            .then(|| Box::new(hidden_prompt) as Box<dyn FnMut() -> Option<String>>),
    };
    std::process::exit(stellarr_cli::run(&args, io, environment, cancelled));
}

/// Ctrl+C stops a command at the next point where nothing is half done: it
/// does not kill the process in the middle of a transaction.
fn watch_for_interrupt(cancelled: &Arc<AtomicBool>) {
    let flag = Arc::clone(cancelled);
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async {
            while tokio::signal::ctrl_c().await.is_ok() {
                flag.store(true, Ordering::Relaxed);
            }
        });
    });
}

/// Reads a line from the terminal with what is typed not shown. The standard
/// library cannot switch the echo off and this code may not call into the
/// system itself, so `stty` does.
fn hidden_prompt() -> Option<String> {
    let mut error = io::stderr();
    let _ = write!(error, "Secret value (input is hidden): ");
    let _ = error.flush();
    let stty = |argument: &str| {
        Command::new("stty")
            .arg(argument)
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let hidden = stty("-echo");
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line);
    if hidden {
        stty("echo");
    }
    let _ = writeln!(error);
    read.ok()?;
    Some(line.trim_end_matches(['\n', '\r']).to_string())
}
