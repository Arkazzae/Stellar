//! `secrets list`, `secrets set <name>` and `secrets clear <name>`: the
//! secrets an operator can hold in the database, sealed under the instance's
//! master key. A value is never printed, never logged and never accepted as
//! an argument.

use rusqlite::{OptionalExtension, TransactionBehavior};
use stellarr_api::seal::Sealer;
use stellarr_api::secrets::{self, ResolvedSecrets, SecretDefinition, SecretStatus, Source};
use stellarr_api::settings::{Config, EnvironmentSecrets};
use stellarr_api::values::Timestamp;

use crate::database::CliDatabase;
use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::json::{Json, moment, round_trip};
use crate::runtime::{Runtime, stdin_needs_yes};

/// The secrets of the environment, as the server reads them.
pub fn environment_secrets(config: &Config) -> EnvironmentSecrets {
    let values: Vec<(&'static str, &str)> = EnvironmentSecrets::KEYS
        .iter()
        .map(|key| (*key, config.raw(key).unwrap_or("")))
        .collect();
    EnvironmentSecrets::with(&values)
}

/// This tool says `database` where the server says `panel`, so that its output
/// does not suggest the value came from the operator's panel.
fn source_name(source: Source) -> &'static str {
    match source {
        Source::Panel => "database",
        Source::Environment => "environment",
        Source::None => "none",
    }
}

fn item(status: &SecretStatus, updated: bool) -> Json {
    Json::object([
        ("name", status.name.into()),
        ("configured", status.configured.into()),
        (
            "source",
            source_name(match status.source {
                "panel" => Source::Panel,
                "environment" => Source::Environment,
                _ => Source::None,
            })
            .into(),
        ),
        ("variable", status.variable.into()),
        (
            "updatedAt",
            match (updated, status.updated_at) {
                (true, Some(at)) => moment(at),
                _ => Json::Null,
            },
        ),
    ])
}

pub fn list(runtime: &mut Runtime) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let connection = &database.connection;
    let sealer = runtime.read_sealer()?;
    let resolved = secrets::read(connection, &environment_secrets(&runtime.config), &sealer)?;
    let mut warnings = Vec::new();
    let mut statement = connection.prepare("SELECT \"Name\" FROM \"InstanceSecrets\"")?;
    let stored = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !stored.is_empty() && !runtime.has_master_key() {
        warnings.push(format!(
            "The database has {} stored secret(s), but the master key is missing from {}; those values count as unset until the API and the keys are restored.",
            stored.len(),
            runtime.keys_directory().display()
        ));
    } else {
        // A value sealed by the first server, or under another key, is there
        // and cannot be opened: it counts as unset, as it does for the server.
        for definition in &secrets::ALL {
            if stored.iter().any(|name| name == definition.name)
                && resolved.get(definition).source != Source::Panel
            {
                warnings.push(format!(
                    "The stored value of {} cannot be opened with this instance's master key (it was sealed under another one, by the first server perhaps); it counts as unset.",
                    definition.name
                ));
            }
        }
    }
    let statuses = resolved.statuses();
    let lines = statuses
        .iter()
        .map(|status| {
            format!(
                "{}: {} [{}; fallback {}]{}",
                status.name,
                if status.configured {
                    "configured"
                } else {
                    "not set"
                },
                source_name(match status.source {
                    "panel" => Source::Panel,
                    "environment" => Source::Environment,
                    _ => Source::None,
                }),
                status.variable,
                status
                    .updated_at
                    .map_or(String::new(), |at| format!(" updated {}", round_trip(at)))
            )
        })
        .collect();
    let data = Json::object([
        (
            "secrets",
            Json::array(statuses.iter().map(|status| item(status, true))),
        ),
        (
            "warnings",
            Json::array(warnings.iter().cloned().map(Json::from)),
        ),
    ]);
    Ok(CliResult::new("secrets.list", data, lines).warnings(warnings))
}

fn unknown(name: &str) -> CliError {
    let names: Vec<&str> = secrets::ALL
        .iter()
        .map(|definition| definition.name)
        .collect();
    CliError::new(
        "secret_not_found",
        format!(
            "No secret named '{name}'. Available names: {}.",
            names.join(", ")
        ),
        exit::NOT_FOUND,
    )
}

fn find(name: &str) -> CliOutcome<&'static SecretDefinition> {
    secrets::find(name).ok_or_else(|| unknown(name))
}

/// What the person should know before a stored value is replaced or removed
/// when this server cannot open it: it was sealed under another key, by the
/// first server perhaps, or the key is gone or cannot be read. It counts as
/// unset now, and what is about to happen is not a change the server sees.
fn unopenable_note(
    runtime: &Runtime,
    connection: &rusqlite::Connection,
    definition: &SecretDefinition,
    what_happens: &str,
) -> CliOutcome<Option<String>> {
    let stored: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"InstanceSecrets\" WHERE \"Name\" = ?1)",
        [definition.name],
        |row| row.get(0),
    )?;
    if !stored {
        return Ok(None);
    }
    let why = match Sealer::open_existing(&runtime.keys_directory()) {
        Ok(None) => format!(
            "there is no master key in {}",
            runtime.keys_directory().display()
        ),
        Err(error) => format!("the master key cannot be read ({error})"),
        Ok(Some(sealer)) => {
            let resolved =
                secrets::read(connection, &environment_secrets(&runtime.config), &sealer)?;
            if resolved.get(definition).source == Source::Panel {
                return Ok(None);
            }
            "it was sealed under another master key, by the first server perhaps".to_string()
        }
    };
    Ok(Some(format!(
        "The stored value of {} cannot be opened by this server ({why}) and counts as unset; it will be {what_happens}.",
        definition.name
    )))
}

pub fn set(runtime: &mut Runtime, name: &str) -> CliOutcome {
    let definition = find(name)?;
    if runtime.invocation.stdin && !runtime.invocation.yes {
        return Err(stdin_needs_yes());
    }
    // Fail before asking for anything when the keys folder the API uses is
    // absent. The key itself is made only after the confirmation and the check
    // of the value: a cancelled or refused command leaves the folder as it was.
    runtime.require_keys_directory()?;
    let mut database = runtime.open_database(true)?;
    let mut preview = format!(
        "Will store the secret '{}' (value hidden).",
        definition.name
    );
    for note in unopenable_note(runtime, &database.connection, definition, "replaced")?
        .into_iter()
        .chain(runtime.key_preview())
    {
        preview.push(' ');
        preview.push_str(&note);
    }
    runtime.confirm(&preview)?;
    let value = runtime.read_secret()?;
    // Ctrl+C is a cancellation whatever was typed with it, judged first.
    runtime.check_cancelled()?;
    if !secrets::is_valid_value(&value) {
        return Err(CliError::new(
            "invalid_secret",
            format!(
                "The secret must be non-blank and at most {} characters.",
                secrets::MAX_LENGTH
            ),
            exit::USAGE,
        ));
    }
    runtime.check_cancelled()?;
    let (sealer, notes) = runtime.write_sealer()?;
    store(&mut database, &sealer, definition, &value)?;
    let resolved = secrets::read(
        &database.connection,
        &environment_secrets(&runtime.config),
        &sealer,
    )?;
    let status = resolved.status(definition);
    Ok(CliResult::new(
        "secrets.set",
        item(&status, true),
        vec![
            format!(
                "Stored {} encrypted in the instance database. The value was not printed.",
                definition.name
            ),
            "Cockpit and the API read the same value; the database override wins over the environment."
                .to_string(),
            format!("Audit: local-cli secrets.set {}", definition.name),
        ],
    )
    .warnings(notes)
    .changed())
}

fn store(
    database: &mut CliDatabase,
    sealer: &Sealer,
    definition: &SecretDefinition,
    value: &str,
) -> CliOutcome<()> {
    // Never hand a database message about the value back to the terminal: the
    // original failure stays out of the output.
    let failed = || {
        CliError::new(
            "secret_write_failed",
            format!(
                "The secret '{}' could not be stored. Nothing was changed.",
                definition.name
            ),
            exit::FAILURE,
        )
    };
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| failed())?;
    secrets::set(&transaction, sealer, definition, value).map_err(|_| failed())?;
    CliDatabase::audit(&transaction, "secrets.set", definition.name).map_err(|_| failed())?;
    transaction.commit().map_err(|_| failed())?;
    Ok(())
}

pub fn clear(runtime: &mut Runtime, name: &str) -> CliOutcome {
    let definition = find(name)?;
    let mut database = runtime.open_database(true)?;
    let environment = environment_secrets(&runtime.config);
    let row: Option<Timestamp> = database
        .connection
        .query_row(
            "SELECT \"UpdatedAt\" FROM \"InstanceSecrets\" WHERE \"Name\" = ?1",
            [definition.name],
            |row| row.get(0),
        )
        .optional()?;
    // Removing a row needs no key: once it is gone nothing is left to open,
    // so the status that follows is read with a key that opens nothing.
    let nothing_opens = crate::runtime::unmatched_sealer();
    let current = |resolved: &ResolvedSecrets| resolved.status(definition);
    if row.is_none() {
        let resolved = secrets::read(&database.connection, &environment, &nothing_opens)?;
        let status = current(&resolved);
        let environmental = status.source == "environment";
        return Ok(CliResult::new(
            "secrets.clear",
            item(&status, false),
            vec![
                format!(
                    "No database override exists for {}; nothing changed.",
                    definition.name
                ),
                if environmental {
                    format!("The environment value ({}) still applies.", status.variable)
                } else {
                    format!("No value is set in the database or in {}.", status.variable)
                },
            ],
        ));
    }
    let mut preview = format!(
        "Will remove the database override for the secret '{}'.",
        definition.name
    );
    if let Some(note) = unopenable_note(runtime, &database.connection, definition, "removed")? {
        preview.push(' ');
        preview.push_str(&note);
    }
    runtime.confirm(&preview)?;
    runtime.check_cancelled()?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    secrets::clear(&transaction, definition)?;
    CliDatabase::audit(&transaction, "secrets.clear", definition.name)?;
    transaction.commit()?;
    let resolved = secrets::read(&database.connection, &environment, &nothing_opens)?;
    let status = current(&resolved);
    let environmental = status.source == "environment";
    Ok(CliResult::new(
        "secrets.clear",
        item(&status, false),
        vec![
            format!("Removed the database override for {}.", definition.name),
            if environmental {
                format!("The environment value ({}) applies again.", status.variable)
            } else {
                format!(
                    "No value is configured in the environment ({}); integrations that need it stay unavailable.",
                    status.variable
                )
            },
            format!("Audit: local-cli secrets.clear {}", definition.name),
        ],
    )
    .changed())
}
