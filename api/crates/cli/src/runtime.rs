//! Everything a command needs outside its database: the resolved data root,
//! the configuration the defaults come from, the terminal streams and the
//! confirmation rules.

use std::io::{BufRead, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use stellarr_api::seal::Sealer;
use stellarr_api::settings::{Config, VARIABLES, Variable};

use crate::backup::{NotYetBuilt, RestoreLibrary};
use crate::database::CliDatabase;
use crate::errors::{CliError, CliOutcome, exit};
use crate::invocation::{Invocation, is_blank, trim};
use crate::json::Json;
use crate::terminal::PromptFailure;

pub const DEFAULT_CONTAINER_DATA_ROOT: &str = "/data/stellarr";

/// The environment of one run, as `(name, value)` pairs: the container's
/// configured one, without what the entrypoint exports, and in the image with
/// the public address the launcher derived.
pub type Environment = Vec<(String, String)>;

/// The terminal and what is behind it.
pub struct Io {
    pub input: Box<dyn BufRead>,
    pub output: Box<dyn Write>,
    pub error: Box<dyn Write>,
    /// Whether a person is at the other end of the input, to be asked.
    pub interactive: bool,
    /// Asks for a secret without showing it; none where nobody can be asked.
    pub secret_prompt: Option<Box<dyn FnMut() -> Result<String, PromptFailure>>>,
}

pub struct Runtime {
    pub invocation: Invocation,
    pub data_root: String,
    pub config: Config,
    pub environment: Environment,
    pub io: Io,
    /// Set when the person pressed Ctrl+C: nothing more is changed.
    pub cancelled: Arc<AtomicBool>,
    /// What reads and replaces backup archives.
    pub restore: Box<dyn RestoreLibrary>,
}

/// The variable named `name`, as the first tool's table has it.
pub fn variable(name: &str) -> &'static Variable {
    VARIABLES
        .iter()
        .find(|variable| variable.name == name)
        .unwrap_or_else(|| panic!("{name} is not a variable of the table"))
}

/// The value of `variable` in `environment`, or none when it is unset or blank.
pub fn value_in<'a>(environment: &'a Environment, variable: &Variable) -> Option<&'a str> {
    environment
        .iter()
        .find(|(name, _)| name == variable.name)
        .map(|(_, value)| value.as_str())
        .filter(|value| !is_blank(value))
}

/// The environment the host is run in, `Production` when none is named.
fn host_environment(environment: &Environment) -> String {
    for name in ["ASPNETCORE_ENVIRONMENT", "DOTNET_ENVIRONMENT"] {
        if let Some((_, value)) = environment.iter().find(|(have, _)| have == name)
            && !is_blank(value)
        {
            return trim(value);
        }
    }
    "Production".to_string()
}

/// `--data-root`, then the configured `DataRoot` (`STELLARR_DATA_ROOT` among
/// it), then the container's path.
fn resolve_data_root(explicit: Option<&str>, config: &Config) -> String {
    let selected = match explicit {
        Some(path) if !is_blank(path) => Some(path),
        _ => config.raw("DataRoot"),
    };
    let selected = match selected {
        Some(path) if !is_blank(path) => trim(path),
        _ => DEFAULT_CONTAINER_DATA_ROOT.to_string(),
    };
    full_path(&selected)
}

/// A path made absolute and plain, as `Path.GetFullPath` makes one: `.` and
/// `..` and repeated separators are gone, a separator at the end stays.
pub fn full_path(path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        format!("{}/{path}", cwd.display())
    };
    let mut kept: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                kept.pop();
            }
            other => kept.push(other),
        }
    }
    let mut full = format!("/{}", kept.join("/"));
    if joined.ends_with('/') && full != "/" {
        full.push('/');
    }
    full
}

impl Runtime {
    pub fn create(
        invocation: Invocation,
        io: Io,
        environment: Environment,
        cancelled: Arc<AtomicBool>,
    ) -> Runtime {
        let config = Config::layered(&host_environment(&environment), &environment);
        let data_root = resolve_data_root(invocation.data_root.as_deref(), &config);
        Runtime {
            invocation,
            data_root,
            config,
            environment,
            io,
            cancelled,
            restore: Box::new(NotYetBuilt),
        }
    }

    pub fn json(&self) -> bool {
        self.invocation.json
    }

    fn join(&self, parts: &[&str]) -> PathBuf {
        let mut path = PathBuf::from(&self.data_root);
        for part in parts {
            path.push(part);
        }
        path
    }

    /// `<data root>/db/stellarr.db`, as the text a message shows.
    pub fn database_path(&self) -> String {
        self.join(&["db", "stellarr.db"]).display().to_string()
    }

    pub fn keys_directory(&self) -> PathBuf {
        self.join(&["keys"])
    }

    pub fn database_exists(&self) -> bool {
        Path::new(&self.database_path()).is_file()
    }

    pub fn open_database(&self, mutation: bool) -> CliOutcome<CliDatabase> {
        if !self.database_exists() {
            return Err(CliError::new(
                "database_not_found",
                format!(
                    "No Stellarr database exists at {}. Check --data-root or STELLARR_DATA_ROOT; the CLI never creates one.",
                    self.database_path()
                ),
                exit::NOT_FOUND,
            )
            .with_details(Json::object([
                ("dataRoot", Json::text(&self.data_root)),
                ("database", Json::text(self.database_path())),
            ])));
        }
        CliDatabase::open(self, mutation)
    }

    /// The master key as it exists. A read command never makes the folder or
    /// the key: without one, secrets set in the panel simply cannot be opened
    /// and the environment's values are the ones that count.
    pub fn read_sealer(&self) -> CliOutcome<Sealer> {
        match Sealer::open_existing(&self.keys_directory()) {
            Ok(Some(sealer)) => Ok(sealer),
            Ok(None) => Ok(Sealer::from_key(unmatched_key())),
            Err(error) => Err(CliError::new(
                "keys_unreadable",
                format!("The master key cannot be read: {error}"),
                exit::CONFLICT,
            )),
        }
    }

    pub fn has_master_key(&self) -> bool {
        self.keys_directory().join("master.key").is_file()
    }

    /// The folder the master key lives in must be there before a secret is
    /// written: it is the one the running server reads, and the server makes
    /// it when it starts, so a folder it never made means a server that never
    /// ran, and a secret written then would be written for nobody.
    pub fn require_keys_directory(&self) -> CliOutcome<()> {
        let keys = self.keys_directory();
        if keys.is_dir() {
            return Ok(());
        }
        Err(CliError::new(
            "data_protection_keys_missing",
            format!(
                "The keys directory {} does not exist, so a secret written now could not be read by the running API. Start the API once to create it, then retry.",
                keys.display()
            ),
            exit::CONFLICT,
        )
        .with_details(Json::object([(
            "keysDirectory",
            Json::text(keys.display().to_string()),
        )])))
    }

    /// What the master key's file would be if a secret were written now, for
    /// the question before it: said when the tool would have to make it.
    pub fn key_preview(&self) -> Option<String> {
        let path = self.keys_directory().join("master.key");
        if path.exists() {
            return None;
        }
        Some(format!(
            "There is no master key in {} yet, so this makes {} (mode 0600, owned by whoever runs this tool); a key made by another account than the server's may not be readable by it.",
            self.keys_directory().display(),
            path.display()
        ))
    }

    /// The master key a secret is written under, and what the tool did to the
    /// keys folder to have it. An existing key is read as it is: its mode is
    /// not changed, the server does that when it starts. When the folder is
    /// there and the key is not, the key is made, as the server's own start
    /// would make it (the two cannot make two), and that is said.
    pub fn write_sealer(&self) -> CliOutcome<(Sealer, Vec<String>)> {
        self.require_keys_directory()?;
        let keys = self.keys_directory();
        let path = keys.join("master.key");
        let unreadable = |error: &dyn std::fmt::Display| {
            CliError::new(
                "keys_unreadable",
                format!("The master key cannot be read or made: {error}"),
                exit::CONFLICT,
            )
        };
        let mut notes = Vec::new();
        let existing = Sealer::open_existing(&keys).map_err(|e| unreadable(&e))?;
        let sealer = match existing {
            Some(sealer) => {
                if let Ok(meta) = std::fs::metadata(&path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode & 0o077 != 0 {
                        notes.push(format!(
                            "The master key {} has mode {mode:04o}, so others can read it; it was left as it is (the server makes it 0600 when it starts, or chmod 600 it).",
                            path.display()
                        ));
                    }
                }
                sealer
            }
            None => {
                let made = Sealer::load_or_create(&keys).map_err(|e| unreadable(&e))?;
                notes.push(made_key_note(&path, &keys));
                made
            }
        };
        Ok((sealer, notes))
    }

    /// Fails when the person has pressed Ctrl+C.
    pub fn check_cancelled(&self) -> CliOutcome<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        Ok(())
    }

    /// Shows what is about to change and asks for confirmation. Automation
    /// must pass `--yes`; a closed input fails at once instead of waiting.
    pub fn confirm(&mut self, preview: &str) -> CliOutcome<()> {
        if self.invocation.yes {
            return Ok(());
        }
        if !self.io.interactive {
            return Err(CliError::new(
                "confirmation_required",
                "This change needs confirmation. Re-run with --yes in a non-interactive session.",
                exit::USAGE,
            )
            .with_details(Json::object([("preview", Json::text(preview))])));
        }
        if self.json() {
            let _ = writeln!(self.io.error, "{preview}");
        } else {
            let _ = writeln!(self.io.output, "{preview}");
        }
        let _ = write!(self.io.error, "Continue? [y/N] ");
        let _ = self.io.error.flush();
        let _ = self.io.output.flush();
        let mut answer = String::new();
        let _ = self.io.input.read_line(&mut answer);
        let answer = trim(&answer);
        if !answer.eq_ignore_ascii_case("y") && !answer.eq_ignore_ascii_case("yes") {
            return Err(cancelled());
        }
        self.check_cancelled()
    }

    /// A secret comes only from `--stdin` or a hidden prompt. It is never read
    /// from an argument, where a shell has already copied it into the process
    /// list and history.
    pub fn read_secret(&mut self) -> CliOutcome<String> {
        if self.invocation.stdin {
            if !self.invocation.yes {
                return Err(stdin_needs_yes());
            }
            let mut piped = String::new();
            let mut bytes = Vec::new();
            self.io
                .input
                .read_to_end(&mut bytes)
                .map_err(|_| CliError::unexpected("IOException"))?;
            piped.push_str(&String::from_utf8_lossy(&bytes));
            return Ok(strip_one_line_ending(&piped));
        }
        if !self.io.interactive || self.io.secret_prompt.is_none() {
            return Err(CliError::new(
                "secret_value_required",
                "Provide the secret with --stdin, or run interactively for a hidden prompt. Secret values are not accepted as arguments.",
                exit::USAGE,
            ));
        }
        let prompt = self.io.secret_prompt.as_mut().expect("checked above");
        let asked = prompt();
        // Ctrl+C at the prompt is a cancellation, whatever came with it.
        self.check_cancelled()?;
        match asked {
            Ok(value) => Ok(value),
            Err(PromptFailure::Interrupted) => Err(cancelled()),
            Err(PromptFailure::EchoStays) => Err(CliError::new(
                "secret_value_required",
                "The terminal's echo could not be turned off (`stty -echo` failed), so the secret was not asked for: it would have been shown as it was typed. Provide it with --stdin instead.",
                exit::USAGE,
            )),
        }
    }

    pub fn stdout_line(&mut self, line: &str) {
        let _ = writeln!(self.io.output, "{line}");
    }
}

/// Says that the tool made the master key, with the path and the account that
/// owns it, and warns when that is not the account that owns the folder: the
/// server runs as the folder's owner, and a key it cannot read is a key it
/// would have to replace, losing every secret sealed under it.
fn made_key_note(path: &Path, keys: &Path) -> String {
    let owner = std::fs::metadata(path).map(|meta| meta.uid()).ok();
    let folder_owner = std::fs::metadata(keys).map(|meta| meta.uid()).ok();
    let describe =
        |uid: Option<u32>| uid.map_or_else(|| "unknown".to_string(), |uid| uid.to_string());
    let mut note = format!(
        "Made the master key {} (mode 0600, owner uid {}), because there was none.",
        path.display(),
        describe(owner)
    );
    if owner.is_some() && owner != folder_owner {
        note.push_str(&format!(
            " The keys folder belongs to uid {}, so the server may not be able to read this key; make it readable for that account or remove it and run the command as that account.",
            describe(folder_owner)
        ));
    }
    note
}

pub fn stdin_needs_yes() -> CliError {
    CliError::new(
        "confirmation_required",
        "--stdin supplies the secret itself, so it cannot also answer a confirmation. Add --yes.",
        exit::USAGE,
    )
}

pub fn cancelled() -> CliError {
    CliError::new("cancelled", "Cancelled; nothing was changed.", exit::USAGE)
}

/// Removes one line ending, `\r\n`, `\n` or `\r`, from the end.
pub fn strip_one_line_ending(value: &str) -> String {
    if let Some(rest) = value.strip_suffix("\r\n") {
        return rest.to_string();
    }
    value
        .strip_suffix('\n')
        .or_else(|| value.strip_suffix('\r'))
        .unwrap_or(value)
        .to_string()
}

/// A sealer no sealed value was made with, for a status that needs no key.
pub fn unmatched_sealer() -> Sealer {
    Sealer::from_key(unmatched_key())
}

/// A key no sealed value was made with, for a read that has no key to use.
fn unmatched_key() -> [u8; 32] {
    use std::hash::{BuildHasher, Hasher};
    let state = std::collections::hash_map::RandomState::new();
    let mut key = [0u8; 32];
    for (index, chunk) in key.chunks_mut(8).enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_usize(index);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_made_absolute_and_plain() {
        assert_eq!(full_path("/data/stellarr"), "/data/stellarr");
        assert_eq!(full_path("/data//stellarr/./x/../"), "/data/stellarr/");
        assert_eq!(full_path("/.."), "/");
        assert!(full_path("relative").ends_with("/relative"));
    }

    #[test]
    fn one_line_ending_is_taken_off_and_no_more() {
        assert_eq!(strip_one_line_ending("a\r\n"), "a");
        assert_eq!(strip_one_line_ending("a\n\n"), "a\n");
        assert_eq!(strip_one_line_ending("a\r"), "a");
        assert_eq!(strip_one_line_ending(" a "), " a ");
    }
}
