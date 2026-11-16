//! The `stellarr-init` binary: the manager as PID 1, and the subcommands
//! that ask it about the services.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use stellarr_init::client;
use stellarr_init::manager::{self, Options};
use stellarr_init::protocol::Request;
use stellarr_init::services;

const USAGE: &str = "\
usage: stellarr-init [--socket PATH] COMMAND

  run                 run the container's services; the container's PID 1
  status [--json]     every service: state, PID, restarts, uptime
  health              exit 0 only when every critical service is ready
  restart SERVICE     stop a service and start it again
  reload SERVICE      make a service reload its configuration

The control socket is /run/stellarr/init.sock unless --socket says otherwise.
";

enum Command {
    Run,
    Status { json: bool },
    Health,
    Restart(String),
    Reload(String),
}

fn parse(args: &[String]) -> Result<(Command, PathBuf), String> {
    let mut socket = PathBuf::from(services::SOCKET);
    let mut words = Vec::new();
    let mut json = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                socket = PathBuf::from(args.next().ok_or("--socket takes a path")?);
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(String::new()),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            word => words.push(word),
        }
    }
    let command = match words.as_slice() {
        ["run"] => Command::Run,
        ["status"] => Command::Status { json },
        ["health"] => Command::Health,
        ["restart", name] => Command::Restart(name.to_string()),
        ["reload", name] => Command::Reload(name.to_string()),
        [] => return Err(String::new()),
        _ => return Err(format!("cannot read the command {:?}", words.join(" "))),
    };
    if json && !matches!(command, Command::Status { .. }) {
        return Err("--json goes with status".into());
    }
    Ok((command, socket))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, socket) = match parse(&args) {
        Ok(parsed) => parsed,
        Err(problem) => {
            if !problem.is_empty() {
                eprintln!("stellarr-init: {problem}");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    if let Command::Run = command {
        let code = manager::run(
            services::container(),
            Options {
                socket,
                shutdown_timeout: services::SHUTDOWN_TIMEOUT,
            },
        );
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }

    if let Err(problem) = client::set_hard_deadline() {
        eprintln!("stellarr-init: {problem}");
        return ExitCode::FAILURE;
    }
    let result = match command {
        Command::Run => unreachable!(),
        Command::Status { json } => client::status(&socket).map(|reply| {
            if json {
                client::render_json(&reply)
            } else {
                client::render_table(&reply)
            }
        }),
        Command::Health => ask(&socket, Request::Health).map(|message| {
            if message.is_empty() {
                "healthy\n".to_string()
            } else {
                format!("{message}\n")
            }
        }),
        Command::Restart(name) => ask(&socket, Request::Restart(name)).map(|m| format!("{m}\n")),
        Command::Reload(name) => ask(&socket, Request::Reload(name)).map(|m| format!("{m}\n")),
    };
    match result {
        Ok(text) => {
            // A reader that went away, as `status | head` does, is no error.
            let _ = std::io::stdout().write_all(text.as_bytes());
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("stellarr-init: {problem}");
            ExitCode::FAILURE
        }
    }
}

/// The message of an `ok` reply, or the reason for anything else.
fn ask(socket: &std::path::Path, request: Request) -> Result<String, String> {
    let (verdict, _) = client::ask(socket, &request)?;
    verdict
}
