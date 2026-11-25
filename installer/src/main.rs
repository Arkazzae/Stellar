//! Installs Stellarr with Docker Compose, and updates an installation it
//! made. The repository's install.sh downloads this program for the machine
//! it runs on, checks its SHA-256 sum and runs it with the same arguments;
//! --help lists them.

#[cfg(not(unix))]
compile_error!("The Stellarr installer runs on Linux and macOS.");

mod checks;
mod cli;
mod envfile;
mod flow;
mod http;
mod install_plan;
mod ports;
mod sys;
mod ui;

use std::process::ExitCode;

use cli::{Options, Parsed};
use ui::{Fd, Stop, Ui, emit};

/// A fault in the installer itself is never silent: it says so, where to
/// report it, and that nothing was removed.
fn report_faults() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "an unknown fault".to_string());
        let place = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let mut ui = Ui::new(true);
        ui.settle();
        ui.fail(
            &format!("The installer stopped{place}: {message}"),
            &[] as &[&str],
        );
        ui.text(
            Fd::Err,
            &[
                "",
                "This is a fault in the installer, and nothing was removed. Please report it",
                &format!("with the output above at {}", flow::ISSUES_URL),
            ],
        );
        std::process::exit(70);
    }));
}

fn main() -> ExitCode {
    sys::handle_signals();
    report_faults();
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    let mut options = Options::from_env();
    let mut ui = Ui::new(true);
    match options.parse(&args) {
        Err(message) => {
            ui.fail(&message, &[] as &[&str]);
            return ExitCode::from(2);
        }
        Ok(Parsed::Help) => {
            emit(Fd::Out, ui::usage());
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Run) => {}
    }
    let mut ui = Ui::new(options.color);
    if let Err(message) = options.validate() {
        ui.fail(&message, &[] as &[&str]);
        return ExitCode::from(2);
    }
    let os = match flow::Os::detect() {
        Ok(os) => os,
        Err(problem) => {
            ui.fail(&problem, &[] as &[&str]);
            return ExitCode::from(1);
        }
    };
    let mut run = flow::Run::new(ui, options, os);
    let status = match run.main() {
        Ok(()) => 0,
        Err(Stop(status)) => status,
    };
    run.ui.settle();
    ExitCode::from(u8::try_from(status).unwrap_or(1))
}
