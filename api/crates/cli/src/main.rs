//! The terminal's side of `stellarr`: the arguments, the environment, the
//! input and the two output streams, and the signals.

use std::io::{self, IsTerminal};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use stellarr_cli::Io;
use stellarr_cli::terminal::{self, PromptFailure};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let environment: Vec<(String, String)> = std::env::vars().collect();
    let cancelled = Arc::new(AtomicBool::new(false));
    watch_for_signals(&cancelled);
    let interactive = io::stdin().is_terminal();
    let prompt_flag = Arc::clone(&cancelled);
    let io = Io {
        input: Box::new(io::BufReader::new(io::stdin())),
        output: Box::new(io::stdout()),
        error: Box::new(io::stderr()),
        interactive,
        secret_prompt: interactive.then(|| {
            Box::new(move || -> Result<String, PromptFailure> {
                terminal::ask_on_terminal(&prompt_flag)
            }) as Box<dyn FnMut() -> Result<String, PromptFailure>>
        }),
    };
    std::process::exit(stellarr_cli::run(&args, io, environment, cancelled));
}

/// Ctrl+C stops a command at the next point where nothing is half done: it
/// does not kill the process in the middle of a transaction. A termination or
/// a hang-up still ends the process at once, as it always did, but not before
/// a prompt that turned the echo off has put it back.
fn watch_for_signals(cancelled: &Arc<AtomicBool>) {
    let flag = Arc::clone(cancelled);
    std::thread::spawn(move || {
        use tokio::signal::unix::{SignalKind, signal};
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async {
            let (Ok(mut interrupt), Ok(mut terminate), Ok(mut hang_up)) = (
                signal(SignalKind::interrupt()),
                signal(SignalKind::terminate()),
                signal(SignalKind::hangup()),
            ) else {
                return;
            };
            loop {
                tokio::select! {
                    _ = interrupt.recv() => flag.store(true, Ordering::Relaxed),
                    _ = terminate.recv() => end_now(143),
                    _ = hang_up.recv() => end_now(129),
                }
            }
        });
    });
}

fn end_now(status: i32) -> ! {
    terminal::restore_after_signal();
    std::process::exit(status)
}
