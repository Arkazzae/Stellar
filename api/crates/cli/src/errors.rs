//! A failure with a stable code and a process exit status, and what a command
//! gives back. Both are part of the contract scripts depend on.

use crate::json::Json;

/// The documented exit statuses.
pub mod exit {
    pub const SUCCESS: i32 = 0;
    pub const FAILURE: i32 = 1;
    pub const USAGE: i32 = 2;
    pub const NOT_FOUND: i32 = 3;
    pub const CONFLICT: i32 = 4;
    pub const UNAVAILABLE: i32 = 5;
}

/// What a command refuses with.
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    pub exit: i32,
    pub details: Option<Json>,
    /// What SQLite said, when the failure came from it: not for the person
    /// reading an answer, but for `doctor`, which says what it found.
    pub cause: Option<String>,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>, exit: i32) -> CliError {
        CliError {
            code,
            message: message.into(),
            exit,
            details: None,
            cause: None,
        }
    }

    pub fn with_details(mut self, details: Json) -> CliError {
        self.details = Some(details);
        self
    }

    pub fn usage(message: impl Into<String>) -> CliError {
        CliError::new("usage", message, exit::USAGE)
    }

    /// A failure that is the tool's own, whose cause stays out of the output.
    pub fn unexpected(what: &str) -> CliError {
        CliError::new(
            "unexpected_error",
            format!("The command failed unexpectedly ({what}). Nothing is reported as changed."),
            exit::FAILURE,
        )
    }
}

pub type CliOutcome<T = CliResult> = Result<T, CliError>;

/// One command's output, before it is written as text or JSON.
#[derive(Debug, Clone)]
pub struct CliResult {
    pub command: &'static str,
    pub data: Json,
    pub lines: Vec<String>,
    pub exit: i32,
    pub warnings: Vec<String>,
}

impl CliResult {
    pub fn new(command: &'static str, data: Json, lines: Vec<String>) -> CliResult {
        CliResult {
            command,
            data,
            lines,
            exit: exit::SUCCESS,
            warnings: Vec::new(),
        }
    }

    pub fn exit(mut self, exit: i32) -> CliResult {
        self.exit = exit;
        self
    }

    pub fn warnings(mut self, warnings: Vec<String>) -> CliResult {
        self.warnings = warnings;
        self
    }
}
