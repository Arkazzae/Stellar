//! The settings the commands know: the ones stored in the database, which the
//! operator's panel edits too, and the startup ones that only the environment
//! can change.

use stellarr_api::instance::operations::{
    InstanceServer, Loaded, Settings, Update, is_known_requirement,
};
use stellarr_api::secrets::ResolvedSecrets;
use stellarr_mail::{MailSettings, Provider};

use crate::errors::{CliError, CliOutcome, exit};
use crate::invocation::trim;
use crate::json::Json;
use crate::runtime::{Runtime, value_in, variable};

/// A value as a line shows it and as JSON carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Bool(bool),
    Int(i64),
    /// A choice the operator has not made.
    Auto,
}

impl Value {
    pub fn display(&self) -> String {
        match self {
            Value::Auto => "(auto)".to_string(),
            Value::Bool(true) => "true".to_string(),
            Value::Bool(false) => "false".to_string(),
            Value::Int(number) => number.to_string(),
            Value::Text(text) => text.clone(),
        }
    }

    pub fn json(&self) -> Json {
        match self {
            Value::Auto => Json::Null,
            Value::Bool(value) => Json::Bool(*value),
            Value::Int(number) => Json::Int(*number),
            Value::Text(text) => Json::text(text),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Boolean,
    Integer,
    TimeZone,
    MailAddress,
    SmtpSecurity,
    MfaRequirement,
}

/// What a setting is read from.
pub struct State {
    pub server: InstanceServer,
    pub loaded: Loaded,
    pub secrets: ResolvedSecrets,
}

type Read = fn(&Settings, &InstanceServer, &ResolvedSecrets) -> Value;
type Apply = fn(&mut Update, &str) -> CliOutcome<()>;

/// A setting the panel and the command line both store in the database. The
/// configured preference and the effective availability are separate: GIFs
/// can be switched on while no key is set, and the tool must not report the
/// preference as false.
pub struct WriteSetting {
    pub key: &'static str,
    pub kind: Kind,
    pub seed: Option<&'static str>,
    pub configured: Read,
    pub effective: Read,
    pub stored: fn(&Settings) -> bool,
    pub apply: Apply,
}

fn transport_provider(
    settings: &Settings,
    secrets: &ResolvedSecrets,
) -> stellarr_mail::MailTransport {
    let stored = MailSettings {
        smtp_host: settings.smtp_host.clone(),
        smtp_port: u16::try_from(settings.smtp_port).unwrap_or(stellarr_mail::DEFAULT_SMTP_PORT),
        smtp_security: settings.smtp_security.clone(),
        smtp_username: settings.smtp_username.clone(),
        resend_enabled: settings.resend_enabled,
    };
    stellarr_mail::MailTransport::resolve(
        Some(&stored),
        secrets.resend_api_key().expose(),
        secrets.smtp_password().expose(),
    )
}

/// The provider mail goes through, and whether it has what it needs.
pub fn mail_transport(settings: &Settings, secrets: &ResolvedSecrets) -> (Provider, bool) {
    let transport = transport_provider(settings, secrets);
    (transport.provider, transport.ready)
}

/// Whether the web client offers GIFs: a key is set and the operator has not
/// turned them off.
pub fn giphy_enabled(settings: &Settings, secrets: &ResolvedSecrets) -> bool {
    secrets.get(&stellarr_api::secrets::GIPHY_API_KEY).is_set()
        && settings.giphy_enabled.unwrap_or(true)
}

macro_rules! scalar {
    ($key:expr, $kind:expr, $seed:expr, $read:expr, $apply:expr) => {
        WriteSetting {
            key: $key,
            kind: $kind,
            seed: $seed,
            configured: $read,
            effective: $read,
            stored: |_| true,
            apply: $apply,
        }
    };
}

fn text(value: &str) -> CliOutcome<String> {
    Ok(value.to_string())
}

pub static WRITABLE: &[WriteSetting] = &[
    scalar!(
        "instance.name",
        Kind::Text,
        None,
        |_, server, _| Value::Text(server.name.clone()),
        |u, v| {
            u.instance_name = text(v)?;
            Ok(())
        }
    ),
    scalar!(
        "time-zone",
        Kind::TimeZone,
        Some("Operations:TimeZone"),
        |s, _, _| Value::Text(s.time_zone.clone()),
        |u, v| {
            u.time_zone = text(v)?;
            Ok(())
        }
    ),
    scalar!(
        "mail.enabled",
        Kind::Boolean,
        Some("Mail:Enabled"),
        |s, _, _| Value::Bool(s.mailing_enabled),
        |u, v| {
            u.mailing_enabled = parse_boolean(v)?;
            Ok(())
        }
    ),
    scalar!(
        "mail.from-address",
        Kind::MailAddress,
        Some("Mail:FromAddress"),
        |s, _, _| Value::Text(s.mail_from_address.clone()),
        |u, v| {
            u.mail_from_address = text(v)?;
            Ok(())
        }
    ),
    scalar!(
        "mail.smtp.host",
        Kind::Text,
        None,
        |s, _, _| Value::Text(s.smtp_host.clone()),
        |u, v| {
            u.smtp_host = Some(text(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mail.smtp.port",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.smtp_port),
        |u, v| {
            u.smtp_port = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mail.smtp.username",
        Kind::Text,
        None,
        |s, _, _| Value::Text(s.smtp_username.clone()),
        |u, v| {
            u.smtp_username = Some(text(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mail.smtp.security",
        Kind::SmtpSecurity,
        None,
        |s, _, _| Value::Text(s.smtp_security.clone()),
        |u, v| {
            u.smtp_security = Some(text(v)?);
            Ok(())
        }
    ),
    WriteSetting {
        key: "mail.resend.enabled",
        kind: Kind::Boolean,
        seed: None,
        configured: |s, _, _| s.resend_enabled.map_or(Value::Auto, Value::Bool),
        effective: |s, _, secrets| {
            Value::Bool(transport_provider(s, secrets).provider == Provider::Resend)
        },
        stored: |s| s.resend_enabled.is_some(),
        apply: |u, v| {
            u.resend_enabled = Some(parse_boolean(v)?);
            Ok(())
        },
    },
    scalar!(
        "mail.invites.enabled",
        Kind::Boolean,
        None,
        |s, _, _| Value::Bool(s.invite_mail_enabled),
        |u, v| {
            u.invite_mail_enabled = parse_boolean(v)?;
            Ok(())
        }
    ),
    scalar!(
        "mail.recovery.enabled",
        Kind::Boolean,
        None,
        |s, _, _| Value::Bool(s.recovery_mail_enabled),
        |u, v| {
            u.recovery_mail_enabled = parse_boolean(v)?;
            Ok(())
        }
    ),
    scalar!(
        "mail.alerts.enabled",
        Kind::Boolean,
        None,
        |s, _, _| Value::Bool(s.alert_mail_enabled),
        |u, v| {
            u.alert_mail_enabled = parse_boolean(v)?;
            Ok(())
        }
    ),
    WriteSetting {
        key: "giphy.enabled",
        kind: Kind::Boolean,
        seed: None,
        configured: |s, _, _| s.giphy_enabled.map_or(Value::Auto, Value::Bool),
        effective: |s, _, secrets| Value::Bool(giphy_enabled(s, secrets)),
        stored: |s| s.giphy_enabled.is_some(),
        apply: |u, v| {
            u.giphy_enabled = Some(parse_boolean(v)?);
            Ok(())
        },
    },
    scalar!(
        "retention.backups-days",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.backup_retention_days),
        |u, v| {
            u.backup_retention_days = parse_integer(v)?;
            Ok(())
        }
    ),
    scalar!(
        "retention.logs-days",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.log_retention_days),
        |u, v| {
            u.log_retention_days = parse_integer(v)?;
            Ok(())
        }
    ),
    scalar!(
        "retention.orphans-days",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.orphan_object_retention_days),
        |u, v| {
            u.orphan_object_retention_days = parse_integer(v)?;
            Ok(())
        }
    ),
    scalar!(
        "cleanup.enabled",
        Kind::Boolean,
        None,
        |s, _, _| Value::Bool(s.automatic_cleanup_enabled),
        |u, v| {
            u.automatic_cleanup_enabled = parse_boolean(v)?;
            Ok(())
        }
    ),
    scalar!(
        "mfa.requirement",
        Kind::MfaRequirement,
        None,
        |s, _, _| Value::Text(
            stellarr_api::auth::policy::normalize_requirement(Some(&s.mfa_requirement)).to_string()
        ),
        |u, v| {
            u.mfa_requirement =
                Some(stellarr_api::auth::policy::normalize_requirement(Some(v)).to_string());
            Ok(())
        }
    ),
    scalar!(
        "mfa.trusted-device-days",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.trusted_device_days),
        |u, v| {
            u.trusted_device_days = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mfa.challenge-minutes",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.auth_challenge_minutes),
        |u, v| {
            u.auth_challenge_minutes = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mfa.email-code-minutes",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.email_code_minutes),
        |u, v| {
            u.email_code_minutes = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mfa.email-cooldown-seconds",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.email_code_cooldown_seconds),
        |u, v| {
            u.email_code_cooldown_seconds = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mfa.email-attempts",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.email_code_max_attempts),
        |u, v| {
            u.email_code_max_attempts = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mfa.email-sends-per-hour",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.email_code_sends_per_hour),
        |u, v| {
            u.email_code_sends_per_hour = Some(parse_integer(v)?);
            Ok(())
        }
    ),
    scalar!(
        "mfa.cli-recovery-minutes",
        Kind::Integer,
        None,
        |s, _, _| Value::Int(s.cli_recovery_minutes),
        |u, v| {
            u.cli_recovery_minutes = Some(parse_integer(v)?);
            Ok(())
        }
    ),
];

pub fn find_writable(key: &str) -> Option<&'static WriteSetting> {
    WRITABLE.iter().find(|setting| setting.key == key)
}

pub fn parse_boolean(value: &str) -> CliOutcome<bool> {
    match trim(value).to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(CliError::new(
            "invalid_value",
            format!("'{value}' is not a boolean. Use true or false."),
            exit::USAGE,
        )),
    }
}

/// An integer as `int.Parse` takes one: an optional sign and digits, in the
/// range of 32 bits.
pub fn parse_integer(value: &str) -> CliOutcome<i64> {
    let trimmed = trim(value);
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(&trimmed);
    let parsed = if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        None
    } else {
        trimmed
            .strip_prefix('+')
            .unwrap_or(&trimmed)
            .parse::<i32>()
            .ok()
    };
    parsed.map(i64::from).ok_or_else(|| {
        CliError::new(
            "invalid_value",
            format!("'{value}' is not an integer."),
            exit::USAGE,
        )
    })
}

/// What `config set` stores for the text the operator wrote.
pub fn normalize(setting: &WriteSetting, raw: &str) -> CliOutcome<String> {
    let value = trim(raw);
    if value.is_empty() && !matches!(setting.kind, Kind::Text | Kind::MailAddress) {
        return Err(CliError::new(
            "invalid_value",
            format!("'{}' needs a value.", setting.key),
            exit::USAGE,
        ));
    }
    Ok(match setting.kind {
        Kind::Boolean => if parse_boolean(&value)? {
            "true"
        } else {
            "false"
        }
        .to_string(),
        Kind::Integer => parse_integer(&value)?.to_string(),
        Kind::MfaRequirement => {
            let normalized = value.to_lowercase();
            if !is_known_requirement(&normalized) {
                return Err(CliError::new(
                    "invalid_value",
                    format!(
                        "'{value}' is not an MFA requirement. Use optional, admins, everyone or cockpit."
                    ),
                    exit::USAGE,
                ));
            }
            normalized
        }
        _ => value,
    })
}

/// A setting that only the environment can change.
pub struct StartupSetting {
    pub key: &'static str,
    pub environment_variable: &'static str,
    pub guidance: String,
    pub read: fn(&Runtime) -> (String, &'static str),
}

/// The configured value of `key`, from the environment when its `STELLARR_*`
/// variable or its raw `Section__Key` is set.
fn config_value(runtime: &Runtime, key: &str, fallback: &str) -> (String, &'static str) {
    let Some(value) = runtime.config.raw(key) else {
        return (fallback.to_string(), "default");
    };
    (value.to_string(), source_of_configured(runtime, key))
}

fn source_of_configured(runtime: &Runtime, key: &str) -> &'static str {
    let from_variable = stellarr_api::settings::VARIABLES
        .iter()
        .find(|variable| variable.key.is_some_and(|k| k.eq_ignore_ascii_case(key)))
        .is_some_and(|variable| value_in(&runtime.environment, variable).is_some());
    let raw = key.replace(':', "__");
    let from_raw = runtime
        .environment
        .iter()
        .any(|(name, value)| *name == raw && !value.is_empty());
    if from_variable || from_raw {
        "environment"
    } else {
        "configuration"
    }
}

fn public_host(runtime: &Runtime) -> (String, &'static str) {
    match value_in(&runtime.environment, variable("STELLARR_DOMAIN")) {
        None => ("localhost".to_string(), "entrypoint"),
        Some(domain) => (domain.to_string(), "environment"),
    }
}

fn https_port(runtime: &Runtime) -> (String, &'static str) {
    match value_in(&runtime.environment, variable("STELLARR_HTTPS_PORT")) {
        None => ("443".to_string(), "default"),
        Some(port) => (port.to_string(), "environment"),
    }
}

/// The named identity providers, by id, each marked when the instance trusts
/// its own second step. Only ids and that flag: never a secret.
fn oidc_providers(runtime: &Runtime) -> (String, &'static str) {
    let mut ids: Vec<String> = Vec::new();
    for (name, _) in runtime.config.prefixed("Oidc:Providers:") {
        let id = name.split(':').next().unwrap_or("").to_string();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.sort();
    if ids.is_empty() {
        return ("(none)".to_string(), "default");
    }
    let shown: Vec<String> = ids
        .iter()
        .map(|id| {
            let trusts = runtime
                .config
                .raw(&format!("Oidc:Providers:{id}:TrustProviderMfa"))
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            if trusts {
                format!("{id} (trusts provider MFA)")
            } else {
                id.clone()
            }
        })
        .collect();
    let from_environment = runtime
        .environment
        .iter()
        .any(|(name, _)| name.to_ascii_lowercase().starts_with("oidc__providers__"));
    (
        shown.join(", "),
        if from_environment {
            "environment"
        } else {
            "configuration"
        },
    )
}

macro_rules! from_variable {
    ($key:expr, $name:expr, $config_key:expr, $fallback:expr, $note:expr) => {
        StartupSetting {
            key: $key,
            environment_variable: $name,
            guidance: format!("Set {} in .env and recreate the container.{}", $name, $note),
            read: |runtime| config_value(runtime, $config_key, $fallback),
        }
    };
}

pub fn startup_settings() -> Vec<StartupSetting> {
    let (url, domain, port) = (
        "STELLARR_PUBLIC_URL",
        "STELLARR_DOMAIN",
        "STELLARR_PUBLIC_PORT",
    );
    vec![
        StartupSetting {
            key: "public-url",
            environment_variable: "STELLARR_PUBLIC_URL / STELLARR_DOMAIN / STELLARR_PUBLIC_PORT",
            guidance: format!("Set {url}, or {domain} and {port}, in .env; then recreate the container."),
            read: |runtime| config_value(runtime, "Operations:PublicUrl", ""),
        },
        StartupSetting {
            key: "tls.domain",
            environment_variable: "STELLARR_DOMAIN",
            guidance: format!("Set {domain} in .env; the entrypoint also derives TLS and TURN from it. Recreate the container."),
            read: public_host,
        },
        StartupSetting {
            key: "https-port",
            environment_variable: "STELLARR_HTTPS_PORT",
            guidance: "Set STELLARR_HTTPS_PORT in .env to the host port that HTTPS is published on, then recreate the container.".to_string(),
            read: https_port,
        },
        from_variable!("oidc.enabled", "STELLARR_OIDC_ENABLED", "Oidc:Enabled", "false", ""),
        from_variable!("oidc.authority", "STELLARR_OIDC_ISSUER", "Oidc:Authority", "", ""),
        from_variable!("oidc.client-id", "STELLARR_OIDC_CLIENT_ID", "Oidc:ClientId", "", ""),
        from_variable!(
            "oidc.trust-provider-mfa",
            "STELLARR_OIDC_TRUST_PROVIDER_MFA",
            "Oidc:TrustProviderMfa",
            "false",
            " Off unless the provider reports a real second factor in amr."
        ),
        from_variable!("oidc.provider-mfa-amr", "STELLARR_OIDC_PROVIDER_MFA_AMR", "Oidc:ProviderMfaAmr", "mfa", ""),
        StartupSetting {
            key: "oidc.providers",
            environment_variable: "Oidc__Providers__<id>__*",
            guidance: "Add or remove Oidc__Providers__<id>__Authority, __ClientId, __ClientSecret(File), __DisplayName and the optional __TrustProviderMfa and __ProviderMfaAmr in .env, then recreate the container.".to_string(),
            read: oidc_providers,
        },
        StartupSetting {
            key: "rtc.public-host",
            environment_variable: "STELLARR_DOMAIN",
            guidance: format!("Set {domain} in .env and recreate the container."),
            read: public_host,
        },
        from_variable!("rtc.turn-port", "STELLARR_TURN_PORT", "Rtc:TurnPort", "3478", ""),
        from_variable!("rtc.turns-port", "STELLARR_TURNS_PORT", "Rtc:TurnsPort", "5349", ""),
        from_variable!("rtc.turns-enabled", "STELLARR_TURNS_ENABLED", "Rtc:TurnsEnabled", "false", ""),
        from_variable!("rtc.turn-tcp-enabled", "STELLARR_TURN_TCP_ENABLED", "Rtc:TurnTcpEnabled", "false", ""),
        from_variable!("rtc.media-port", "STELLARR_MEDIA_PORT", "Rtc:MediaPort", "49000", ""),
    ]
}

pub fn find_startup(key: &str) -> Option<StartupSetting> {
    startup_settings()
        .into_iter()
        .find(|setting| setting.key == key)
}
