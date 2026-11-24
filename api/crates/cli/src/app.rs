//! The command line as a whole: which words make which command, what is
//! refused before anything is opened, how a result is written as text or JSON,
//! and the exit status.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::help;
use crate::invocation::Invocation;
use crate::json::Json;
use crate::runtime::{Environment, Io, Runtime};
use crate::{backup, config, secrets, security, sessions, status, users};

/// Runs one command line and says how it ended. Everything it prints goes to
/// `io`, which is given back in the runtime of a command that got that far.
pub fn run(
    args: &[String],
    mut io: Io,
    environment: Environment,
    cancelled: Arc<AtomicBool>,
) -> i32 {
    let json_requested = Invocation::json_requested(args);
    let invocation = match Invocation::parse(args) {
        Ok(invocation) => invocation,
        Err(error) => return write_error(&error, "", json_requested, &mut io),
    };
    let command = invocation
        .positionals
        .first()
        .map(|word| word.to_lowercase())
        .unwrap_or_default();
    // A terminal switch never lets a write command through, even when it was
    // appended after the command's own arguments.
    if invocation.version {
        return write_version(json_requested, &mut io);
    }
    if invocation.help || invocation.positionals.is_empty() {
        let topic = invocation
            .positionals
            .get(1)
            .map_or_else(|| command.clone(), |word| word.to_lowercase());
        return write_help(json_requested, &topic, &mut io);
    }
    if command == "help" {
        let topic = invocation
            .positionals
            .get(1)
            .map_or_else(String::new, |word| word.to_lowercase());
        return write_help(json_requested, &topic, &mut io);
    }
    if command == "version" {
        if invocation.positionals.len() != 1 {
            // The first tool put its own prefix on a message that had one.
            let error = CliError::usage("Usage: stellarr Usage: stellarr version");
            return write_error(&error, &command, json_requested, &mut io);
        }
        return write_version(json_requested, &mut io);
    }

    let mut runtime = Runtime::create(invocation, io, environment, cancelled);
    let outcome = catch_unwind(AssertUnwindSafe(|| dispatch(&mut runtime)))
        .unwrap_or_else(|_| Err(CliError::unexpected("Panic")));
    let json = runtime.json();
    match outcome {
        Ok(result) => match write_result(&result, json, &mut runtime.io) {
            Ok(()) => result.exit,
            // A command that read only has nothing to lose with its answer.
            Err(_) if !result.changed => result.exit,
            Err(error) => write_lost_result(&result, &error, &mut runtime.io),
        },
        Err(error) => write_error(&error, &command, json_requested, &mut runtime.io),
    }
}

fn dispatch(runtime: &mut Runtime) -> CliOutcome {
    let positions = runtime.invocation.positionals.clone();
    let command = positions[0].to_lowercase();
    match command.as_str() {
        "status" => {
            ensure_read_only(runtime, "status", None, Some(1))?;
            status::status(runtime)
        }
        "doctor" => {
            ensure_read_only(runtime, "doctor", None, Some(1))?;
            status::doctor(runtime)
        }
        "config" => dispatch_config(runtime, &positions),
        "secrets" => dispatch_secrets(runtime, &positions),
        "users" => dispatch_users(runtime, &positions),
        "sessions" => dispatch_sessions(runtime, &positions),
        "security" => dispatch_security(runtime, &positions),
        "backup" => dispatch_backup(runtime, &positions),
        _ => Err(CliError::new(
            "unknown_command",
            format!("Unknown command '{command}'. Run 'stellarr help' for the command list."),
            exit::USAGE,
        )),
    }
}

fn dispatch_backup(runtime: &mut Runtime, positions: &[String]) -> CliOutcome {
    let Some(action) = positions.get(1) else {
        return Err(CliError::usage(
            "backup needs a subcommand: verify, restore, cancel or apply.",
        ));
    };
    match action.to_lowercase().as_str() {
        "verify" => {
            ensure_read_only(runtime, "backup verify <archive>", Some(3), None)?;
            backup::verify(runtime, &positions[2])
        }
        "restore" => {
            ensure_mutation(runtime, "backup restore <archive>", 3, false, false)?;
            backup::restore(runtime, &positions[2])
        }
        "cancel" => {
            ensure_mutation(runtime, "backup cancel", 2, false, false)?;
            backup::cancel(runtime)
        }
        "apply" => {
            ensure_mutation(runtime, "backup apply", 2, false, false)?;
            backup::apply(runtime)
        }
        _ => Err(CliError::usage(format!(
            "Unknown backup subcommand '{}'. Use verify, restore, cancel or apply.",
            action.to_lowercase()
        ))),
    }
}

fn dispatch_config(runtime: &mut Runtime, positions: &[String]) -> CliOutcome {
    let Some(action) = positions.get(1) else {
        return Err(CliError::usage(
            "config needs a subcommand: list, get, set or validate.",
        ));
    };
    match action.to_lowercase().as_str() {
        "list" => {
            ensure_read_only(runtime, "config list", None, Some(2))?;
            config::list(runtime)
        }
        "get" => {
            ensure_read_only(runtime, "config get <key>", Some(3), None)?;
            config::get(runtime, &positions[2])
        }
        "set" => {
            ensure_mutation(runtime, "config set <key> <value>", 4, false, false)?;
            config::set(runtime, &positions[2], &positions[3])
        }
        "validate" => {
            ensure_read_only(runtime, "config validate", Some(2), None)?;
            config::validate(runtime)
        }
        _ => Err(CliError::usage(format!(
            "Unknown config subcommand '{}'. Use list, get, set or validate.",
            action.to_lowercase()
        ))),
    }
}

fn dispatch_secrets(runtime: &mut Runtime, positions: &[String]) -> CliOutcome {
    let Some(action) = positions.get(1) else {
        return Err(CliError::usage(
            "secrets needs a subcommand: list, set or clear.",
        ));
    };
    match action.to_lowercase().as_str() {
        "list" => {
            ensure_read_only(runtime, "secrets list", None, Some(2))?;
            secrets::list(runtime)
        }
        "set" => {
            if positions.len() > 3 {
                return Err(CliError::new(
                    "secret_value_argument",
                    "A secret value is not accepted as an argument. Use --stdin or the hidden prompt.",
                    exit::USAGE,
                ));
            }
            ensure_mutation(runtime, "secrets set <name> --stdin", 3, true, false)?;
            secrets::set(runtime, &positions[2])
        }
        "clear" => {
            ensure_mutation(runtime, "secrets clear <name>", 3, false, false)?;
            secrets::clear(runtime, &positions[2])
        }
        _ => Err(CliError::usage(format!(
            "Unknown secrets subcommand '{}'. Use list, set or clear.",
            action.to_lowercase()
        ))),
    }
}

fn dispatch_users(runtime: &mut Runtime, positions: &[String]) -> CliOutcome {
    let Some(action) = positions.get(1) else {
        return Err(CliError::usage("users needs a subcommand: list or show."));
    };
    match action.to_lowercase().as_str() {
        "list" => {
            ensure_read_only(runtime, "users list", None, Some(2))?;
            users::list(runtime)
        }
        "show" => {
            ensure_read_only(runtime, "users show <id-or-username>", Some(3), None)?;
            users::show(runtime, &positions[2])
        }
        _ => Err(CliError::usage(format!(
            "Unknown users subcommand '{action}'. Use list or show."
        ))),
    }
}

fn dispatch_sessions(runtime: &mut Runtime, positions: &[String]) -> CliOutcome {
    let Some(action) = positions.get(1) else {
        return Err(CliError::usage(
            "sessions needs a subcommand: list or revoke.",
        ));
    };
    let action = action.to_lowercase();
    if action == "list" {
        ensure_count(runtime, 2, "sessions list", None, true)?;
        reject(runtime.invocation.yes, "--yes", "sessions list")?;
        reject(runtime.invocation.stdin, "--stdin", "sessions list")?;
        reject(runtime.invocation.all, "--all", "sessions list")?;
        let user = runtime.invocation.user.clone();
        return sessions::list(runtime, user.as_deref());
    }
    if action != "revoke" {
        return Err(CliError::usage(format!(
            "Unknown sessions subcommand '{action}'. Use list or revoke."
        )));
    }
    if runtime.invocation.all {
        let Some(user) = runtime
            .invocation
            .user
            .clone()
            .filter(|_| positions.len() == 2)
        else {
            return Err(CliError::usage(
                "sessions revoke --all needs --user <id-or-username> and no session id.",
            ));
        };
        ensure_mutation(
            runtime,
            "sessions revoke --user <who> --all",
            2,
            false,
            true,
        )?;
        return sessions::revoke_all(runtime, &user);
    }
    if runtime.invocation.user.is_some() {
        return Err(CliError::usage(
            "--user is only for 'sessions list' or 'sessions revoke --all'.",
        ));
    }
    ensure_mutation(runtime, "sessions revoke <id>", 3, false, false)?;
    sessions::revoke(runtime, &positions[2])
}

fn dispatch_security(runtime: &mut Runtime, positions: &[String]) -> CliOutcome {
    let Some(action) = positions.get(1) else {
        return Err(CliError::usage(
            "security needs a subcommand: show, methods, recover or unlink.",
        ));
    };
    match action.to_lowercase().as_str() {
        "show" => {
            ensure_read_only(runtime, "security show <id-or-username>", Some(3), None)?;
            security::show(runtime, &positions[2])
        }
        "methods" => {
            ensure_read_only(runtime, "security methods <id-or-username>", Some(3), None)?;
            security::methods(runtime, &positions[2])
        }
        "recover" => {
            let Some(reason) = runtime
                .invocation
                .reason
                .clone()
                .filter(|_| positions.len() == 3)
            else {
                return Err(CliError::usage(
                    "security recover <id-or-username> --reason <text> [--yes]",
                ));
            };
            ensure_mutation(
                runtime,
                "security recover <id-or-username> --reason <text>",
                3,
                false,
                false,
            )?;
            security::recover(runtime, &positions[2], &reason)
        }
        "unlink" => {
            if positions.len() != 4 {
                return Err(CliError::usage(
                    "security unlink <id-or-username> <identity-id> [--yes]",
                ));
            }
            ensure_mutation(
                runtime,
                "security unlink <id-or-username> <identity-id>",
                4,
                false,
                false,
            )?;
            security::unlink(runtime, &positions[2], &positions[3])
        }
        _ => Err(CliError::usage(format!(
            "Unknown security subcommand '{}'. Use show, methods, recover or unlink.",
            action.to_lowercase()
        ))),
    }
}

/// `exact` words, or at most `maximum`; nothing but the global options of a
/// command that only looks.
fn ensure_read_only(
    runtime: &Runtime,
    usage: &str,
    exact: Option<usize>,
    maximum: Option<usize>,
) -> CliOutcome<()> {
    ensure_count(runtime, exact.unwrap_or(1), usage, maximum, exact.is_some())?;
    let invocation = &runtime.invocation;
    reject(invocation.yes, "--yes", usage)?;
    reject(invocation.stdin, "--stdin", usage)?;
    reject(invocation.all, "--all", usage)?;
    reject(invocation.user.is_some(), "--user", usage)?;
    reject(invocation.reason.is_some(), "--reason", usage)
}

fn ensure_mutation(
    runtime: &Runtime,
    usage: &str,
    exact: usize,
    allow_stdin: bool,
    allow_all: bool,
) -> CliOutcome<()> {
    ensure_count(runtime, exact, usage, None, true)?;
    let invocation = &runtime.invocation;
    reject(invocation.stdin && !allow_stdin, "--stdin", usage)?;
    reject(invocation.all && !allow_all, "--all", usage)?;
    reject(
        invocation.user.is_some() && !usage.contains("--user"),
        "--user",
        usage,
    )?;
    reject(
        invocation.reason.is_some() && !usage.contains("--reason"),
        "--reason",
        usage,
    )
}

fn ensure_count(
    runtime: &Runtime,
    value: usize,
    usage: &str,
    maximum: Option<usize>,
    exact: bool,
) -> CliOutcome<()> {
    let count = runtime.invocation.positionals.len();
    let valid = if exact {
        count == value
    } else {
        maximum.is_none_or(|maximum| count <= maximum)
    };
    if valid {
        Ok(())
    } else {
        Err(CliError::usage(format!("Usage: stellarr {usage}")))
    }
}

fn reject(present: bool, option: &str, usage: &str) -> CliOutcome<()> {
    if present {
        Err(CliError::usage(format!(
            "{option} is not valid here. Usage: stellarr {usage}"
        )))
    } else {
        Ok(())
    }
}

fn write_version(json: bool, io: &mut Io) -> i32 {
    if json {
        let document = Json::object([
            ("ok", true.into()),
            ("command", "version".into()),
            ("data", Json::object([("version", help::version().into())])),
        ]);
        let _ = writeln!(io.output, "{}", document.pretty());
    } else {
        let _ = writeln!(io.output, "{}", help::version());
    }
    exit::SUCCESS
}

fn write_help(json: bool, topic: &str, io: &mut Io) -> i32 {
    let text = if topic.is_empty() {
        help::full()
    } else {
        help::for_command(topic)
    };
    if json {
        let document = Json::object([
            ("ok", true.into()),
            ("command", "help".into()),
            (
                "data",
                Json::object([("topic", Json::text(topic)), ("text", Json::text(&text))]),
            ),
        ]);
        let _ = writeln!(io.output, "{}", document.pretty());
    } else {
        let _ = writeln!(io.output, "{text}");
    }
    exit::SUCCESS
}

fn write_result(result: &CliResult, json: bool, io: &mut Io) -> std::io::Result<()> {
    if json {
        let document = Json::object([
            ("ok", (result.exit == exit::SUCCESS).into()),
            ("command", result.command.into()),
            ("exitCode", Json::Int(i64::from(result.exit))),
            ("data", result.data.clone()),
            (
                "warnings",
                Json::array(result.warnings.iter().cloned().map(Json::from)),
            ),
        ]);
        writeln!(io.output, "{}", document.pretty())?;
        return io.output.flush();
    }
    for line in &result.lines {
        writeln!(io.output, "{line}")?;
    }
    for warning in &result.warnings {
        writeln!(io.error, "warning: {warning}")?;
    }
    io.output.flush()?;
    io.error.flush()
}

/// The answer of a command that changed something could not be written, a
/// closed pipe perhaps: the change stands and the person is told so, on the
/// stream that is most likely still open.
fn write_lost_result(result: &CliResult, error: &std::io::Error, io: &mut Io) -> i32 {
    let mut message = format!(
        "error [output_failed]: The change was made ({}), but its result could not be written: {error}.",
        result.command
    );
    if result.command == "security.recover" {
        message.push_str(
            " The recovery credential was not delivered and cannot be shown again; run `security recover` again to issue a new one.",
        );
    } else {
        message.push_str(" Check the instance before running the command again.");
    }
    let _ = writeln!(io.error, "{message}");
    let _ = io.error.flush();
    exit::FAILURE
}

fn write_error(error: &CliError, command: &str, json: bool, io: &mut Io) -> i32 {
    if json {
        let document = Json::object([
            ("ok", false.into()),
            ("command", Json::text(command)),
            ("exitCode", Json::Int(i64::from(error.exit))),
            (
                "error",
                Json::object([
                    ("code", error.code.into()),
                    ("message", Json::text(&error.message)),
                    ("details", error.details.clone().unwrap_or(Json::Null)),
                ]),
            ),
        ]);
        let _ = writeln!(io.output, "{}", document.pretty());
    } else {
        let _ = writeln!(io.error, "error [{}]: {}", error.code, error.message);
    }
    let _ = io.output.flush();
    let _ = io.error.flush();
    error.exit
}
