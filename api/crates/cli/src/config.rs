//! `config list | get | set | validate`: the settings the panel shares with
//! this tool, and the startup settings it only reads.

use stellarr_api::instance::operations;
use stellarr_api::secrets;
use stellarr_api::values::Timestamp;
use stellarr_messages::{Language, Messages};

use crate::catalog::{
    StartupSetting, State, WRITABLE, WriteSetting, find_startup, find_writable, mail_transport,
    normalize, startup_settings,
};
use crate::database::CliDatabase;
use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::json::Json;
use crate::runtime::Runtime;
use crate::secrets::environment_secrets;
use crate::support::require_instance_server;

fn messages() -> Messages {
    // The tool speaks English whatever the instance's language is.
    Messages::new(Language::English)
}

fn load_state(runtime: &Runtime, connection: &rusqlite::Connection) -> CliOutcome<State> {
    let server = require_instance_server(connection)?;
    let loaded = operations::load(connection, server.id, server.created_at, &runtime.config)?;
    let sealer = runtime.read_sealer()?;
    let secrets = secrets::read(connection, &environment_secrets(&runtime.config), &sealer)?;
    Ok(State {
        server,
        loaded,
        secrets,
    })
}

/// Where the value of a setting in the database comes from.
fn source_of(setting: &WriteSetting, state: &State, runtime: &Runtime) -> &'static str {
    if setting.key == "instance.name" {
        return "database";
    }
    if state.loaded.stored && (setting.stored)(&state.loaded.settings) {
        return "database";
    }
    match setting.seed {
        Some(seed) if runtime.config.raw(seed).is_some() => "configuration",
        _ => "default",
    }
}

struct Item {
    key: String,
    value: Json,
    effective: Json,
    source: &'static str,
    editable: bool,
    restart_required: bool,
    environment_variable: Option<String>,
    guidance: Option<String>,
    previous: Json,
    audit_actor: Option<&'static str>,
}

impl Item {
    fn json(&self) -> Json {
        Json::object([
            ("key", Json::text(&self.key)),
            ("value", self.value.clone()),
            ("effectiveValue", self.effective.clone()),
            ("source", self.source.into()),
            ("editable", self.editable.into()),
            ("restartRequired", self.restart_required.into()),
            (
                "environmentVariable",
                Json::optional_text(self.environment_variable.clone()),
            ),
            ("guidance", Json::optional_text(self.guidance.clone())),
            ("previousValue", self.previous.clone()),
            ("auditActor", Json::optional_text(self.audit_actor)),
        ])
    }
}

fn writable_item(setting: &WriteSetting, state: &State, runtime: &Runtime) -> (Item, String) {
    let value = (setting.configured)(&state.loaded.settings, &state.server, &state.secrets);
    let effective = (setting.effective)(&state.loaded.settings, &state.server, &state.secrets);
    let source = source_of(setting, state, runtime);
    let line = format!(
        "{} = {}  [{source}; editable{}]",
        setting.key,
        value.display(),
        if value == effective {
            String::new()
        } else {
            format!("; effective {}", effective.display())
        }
    );
    let item = Item {
        key: setting.key.to_string(),
        value: value.json(),
        effective: effective.json(),
        source,
        editable: true,
        restart_required: false,
        environment_variable: None,
        guidance: None,
        previous: Json::Null,
        audit_actor: None,
    };
    (item, line)
}

fn startup_item(setting: &StartupSetting, runtime: &Runtime) -> (Item, String) {
    let (value, source) = (setting.read)(runtime);
    let line = format!(
        "{} = {value}  [{source}; read-only, restart required]",
        setting.key
    );
    let item = Item {
        key: setting.key.to_string(),
        value: Json::text(&value),
        effective: Json::text(&value),
        source,
        editable: false,
        restart_required: true,
        environment_variable: Some(setting.environment_variable.to_string()),
        guidance: Some(setting.guidance.clone()),
        previous: Json::Null,
        audit_actor: None,
    };
    (item, line)
}

pub fn list(runtime: &mut Runtime) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let state = load_state(runtime, &database.connection)?;
    let mut settings = Vec::new();
    let mut lines = Vec::new();
    for setting in WRITABLE {
        let (item, line) = writable_item(setting, &state, runtime);
        settings.push(item.json());
        lines.push(line);
    }
    for setting in startup_settings() {
        let (item, line) = startup_item(&setting, runtime);
        settings.push(item.json());
        lines.push(line);
    }
    Ok(CliResult::new(
        "config.list",
        Json::object([("settings", Json::Array(settings))]),
        lines,
    ))
}

pub fn get(runtime: &mut Runtime, key: &str) -> CliOutcome {
    // A startup setting comes from the environment alone, so it reads the same
    // before the owner account exists, or without a database at all.
    if let Some(setting) = find_startup(key) {
        let (item, line) = startup_item(&setting, runtime);
        return Ok(CliResult::new(
            "config.get",
            item.json(),
            vec![line, setting.guidance.clone()],
        ));
    }
    let database = runtime.open_database(false)?;
    let state = load_state(runtime, &database.connection)?;
    let Some(setting) = find_writable(key) else {
        return Err(unknown_setting(key));
    };
    let (item, line) = writable_item(setting, &state, runtime);
    Ok(CliResult::new("config.get", item.json(), vec![line]))
}

pub fn set(runtime: &mut Runtime, key: &str, raw_value: &str) -> CliOutcome {
    if let Some(secret) = secrets::find(key) {
        return Err(CliError::new(
            "secret_not_configuration",
            format!(
                "'{key}' is a secret, not a configuration setting. Use 'stellarr secrets set {}'.",
                secret.name
            ),
            exit::USAGE,
        ));
    }
    if let Some(startup) = find_startup(key) {
        return Err(CliError::new(
            "read_only_setting",
            format!(
                "'{key}' is a startup setting and cannot be stored from the CLI. {}",
                startup.guidance
            ),
            exit::CONFLICT,
        )
        .with_details(Json::object([
            ("key", Json::text(key)),
            (
                "environmentVariable",
                Json::text(startup.environment_variable),
            ),
            ("guidance", Json::text(&startup.guidance)),
        ])));
    }
    let definition = find_writable(key).ok_or_else(|| unknown_setting(key))?;
    let value = normalize(definition, raw_value)?;

    let mut database = runtime.open_database(true)?;
    let mut state = load_state(runtime, &database.connection)?;
    let previous = (definition.configured)(&state.loaded.settings, &state.server, &state.secrets);
    let mut update = operations::snapshot(&state.server.name, &state.loaded.settings);
    (definition.apply)(&mut update, &value)?;
    let problems = operations::validate(&update, &messages())
        .map_err(|_| CliError::unexpected("MissingValue"))?;
    if !problems.is_empty() {
        return Err(invalid_settings(&problems));
    }

    runtime.confirm(&format!(
        "Will set '{}' from '{}' to '{value}'.",
        definition.key,
        previous.display()
    ))?;

    let transaction = database.begin_write()?;
    // What the confirmation waited on may have changed under it: the row is
    // read again inside the write lock, and the one change is made on it.
    let server = require_instance_server(&transaction)?;
    let loaded = operations::load(&transaction, server.id, server.created_at, &runtime.config)?;
    let mut settings = loaded.settings;
    let mut update = operations::snapshot(&server.name, &settings);
    (definition.apply)(&mut update, &value)?;
    let problems = operations::validate(&update, &messages())
        .map_err(|_| CliError::unexpected("MissingValue"))?;
    if !problems.is_empty() {
        return Err(invalid_settings(&problems));
    }
    let name = operations::apply(&mut settings, &update, Timestamp::now());
    operations::save(&transaction, &settings, loaded.stored, &name)?;
    CliDatabase::audit(&transaction, "config.set", definition.key)?;
    transaction.commit()?;

    state.loaded.settings = settings;
    state.server.name = name;
    let new_value = (definition.configured)(&state.loaded.settings, &state.server, &state.secrets);
    let new_effective =
        (definition.effective)(&state.loaded.settings, &state.server, &state.secrets);
    let item = Item {
        key: definition.key.to_string(),
        value: new_value.json(),
        effective: new_effective.json(),
        source: "database",
        editable: true,
        restart_required: false,
        environment_variable: None,
        guidance: None,
        previous: previous.json(),
        audit_actor: Some("local-cli"),
    };
    let mut warnings = Vec::new();
    if new_value != new_effective {
        warnings.push(format!(
            "{} is stored as {}, but its effective availability is {} until its secret is set.",
            definition.key,
            new_value.display(),
            new_effective.display()
        ));
    }
    Ok(CliResult::new(
        "config.set",
        item.json(),
        vec![
            format!(
                "Set {}: {} -> {}{}",
                definition.key,
                previous.display(),
                new_value.display(),
                if new_value == new_effective {
                    String::new()
                } else {
                    format!(" (effective {})", new_effective.display())
                }
            ),
            "Stored in the instance database; Cockpit reads the same row. No restart is needed."
                .to_string(),
            format!("Audit: local-cli config.set {}", definition.key),
        ],
    )
    .warnings(warnings)
    .changed())
}

pub fn validate(runtime: &mut Runtime) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let state = load_state(runtime, &database.connection)?;
    let settings = &state.loaded.settings;
    let update = operations::snapshot(&state.server.name, settings);
    let problems = operations::validate(&update, &messages())
        .map_err(|_| CliError::unexpected("MissingValue"))?;
    let (provider, ready) = mail_transport(settings, &state.secrets);
    let mut warnings = Vec::new();
    if settings.mailing_enabled && !ready {
        warnings.push(if provider == stellarr_mail::Provider::Resend {
            "Mail is enabled but the Resend key is not set."
        } else {
            "Mail is enabled but the SMTP host is not set."
        });
    }
    if settings.resend_enabled == Some(true) && !state.secrets.resend_api_key().is_set() {
        warnings.push("Resend is selected but its API key is not set.");
    }
    if settings.giphy_enabled == Some(true) && !state.secrets.get(&secrets::GIPHY_API_KEY).is_set()
    {
        warnings.push("GIFs are enabled but the GIPHY API key is not set.");
    }
    let valid = problems.is_empty();
    let mut lines: Vec<String> = problems
        .iter()
        .map(|(field, messages)| format!("{field}: {}", messages.join(" ")))
        .collect();
    lines.extend(warnings.iter().map(|warning| format!("warning: {warning}")));
    lines.push(
        if valid {
            "Configuration is valid. Nothing was saved or sent."
        } else {
            "Configuration is invalid. Nothing was saved."
        }
        .to_string(),
    );
    let data = Json::object([
        ("valid", valid.into()),
        ("problems", problems_json(&problems)),
        (
            "warnings",
            Json::array(warnings.iter().map(|w| Json::text(*w))),
        ),
        ("mailProvider", provider.name().into()),
        ("mailReady", ready.into()),
        ("giphyEnabled", Json::optional_bool(settings.giphy_enabled)),
    ]);
    Ok(
        CliResult::new("config.validate", data, lines).exit(if valid {
            exit::SUCCESS
        } else {
            exit::USAGE
        }),
    )
}

fn problems_json(problems: &[(&'static str, Vec<String>)]) -> Json {
    Json::from_members(
        problems
            .iter()
            .map(|(field, messages)| {
                (
                    (*field).to_string(),
                    Json::array(messages.iter().cloned().map(Json::from)),
                )
            })
            .collect(),
    )
}

fn invalid_settings(problems: &[(&'static str, Vec<String>)]) -> CliError {
    let message = problems
        .iter()
        .map(|(_, messages)| messages.join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    CliError::new("invalid_value", message, exit::USAGE)
        .with_details(Json::object([("fields", problems_json(problems))]))
}

fn unknown_setting(key: &str) -> CliError {
    CliError::new(
        "setting_not_found",
        format!("No setting named '{key}'. Run 'stellarr config list' to see the available keys."),
        exit::NOT_FOUND,
    )
}
