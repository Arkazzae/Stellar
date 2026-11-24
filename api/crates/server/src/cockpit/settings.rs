//! What the operator configures in the panel: the instance's name and time
//! zone, mail, retention, the rules of the second step.
//!
//! The settings live in one row of the instance server. Until the operator
//! saves, there is none and the installation's own values stand in for it;
//! the first save writes it. Everything that reads a setting reads the row
//! again, so a change counts from the next request on (mail, the second
//! step's rules, retention and the GIF switch all do).

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use stellarr_mail::{MailSettings, MailTransport, Provider};
use stellarr_smtp::Security;

use super::bind;
use crate::auth::{fresh, policy};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Problem, json_response};
use crate::instance::operations::{self, Loaded, Settings as Row, Update};
use crate::panel::{self};
use crate::permissions::membership::InstanceOwner;
use crate::secrets::{self, ResolvedSecrets};
use crate::settings::Config;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The layered configuration the installation's defaults come from: what the
/// process was started with.
fn config(state: &AppState) -> Config {
    let variables: Vec<(String, String)> = std::env::vars().collect();
    Config::layered(state.settings().environment.name(), &variables)
}

/// The settings row of the instance server, or what stands in for it.
pub(super) fn load(db: &Connection, state: &AppState, server: Id) -> rusqlite::Result<Loaded> {
    let created: Timestamp = db.query_row(
        "SELECT \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
        [server],
        |row| row.get(0),
    )?;
    operations::load(db, server, created, &config(state))
}

/// The transport this row and the secrets make.
pub(super) fn transport(row: &Row, secrets: &ResolvedSecrets) -> MailTransport {
    MailTransport::resolve(
        Some(&MailSettings {
            smtp_host: row.smtp_host.clone(),
            smtp_port: u16::try_from(row.smtp_port).unwrap_or(stellarr_mail::DEFAULT_SMTP_PORT),
            smtp_security: row.smtp_security.clone(),
            smtp_username: row.smtp_username.clone(),
            resend_enabled: row.resend_enabled,
        }),
        secrets.resend_api_key().expose(),
        secrets.smtp_password().expose(),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsDto {
    instance_name: String,
    public_url: String,
    time_zone: String,
    mailing_enabled: bool,
    mail_provider: &'static str,
    mail_from_address: String,
    resend_api_key_configured: bool,
    invite_mail_enabled: bool,
    recovery_mail_enabled: bool,
    alert_mail_enabled: bool,
    backup_retention_days: i64,
    log_retention_days: i64,
    orphan_object_retention_days: i64,
    automatic_cleanup_enabled: bool,
    updated_at: String,
    resend_enabled: bool,
    smtp_host: String,
    smtp_port: i64,
    smtp_security: &'static str,
    smtp_username: String,
    smtp_password_configured: bool,
    mail_ready: bool,
    giphy_enabled: bool,
    giphy_key_configured: bool,
    mfa_requirement: &'static str,
    trusted_device_days: i64,
    auth_challenge_minutes: i64,
    email_code_minutes: i64,
    email_code_cooldown_seconds: i64,
    email_code_max_attempts: i64,
    email_code_sends_per_hour: i64,
    cli_recovery_minutes: i64,
}

fn dto(state: &AppState, name: String, row: &Row, secrets: &ResolvedSecrets) -> SettingsDto {
    let transport = transport(row, secrets);
    SettingsDto {
        instance_name: name,
        public_url: state.settings().operations.public_url.clone(),
        time_zone: row.time_zone.clone(),
        mailing_enabled: row.mailing_enabled,
        mail_provider: transport.provider.name(),
        mail_from_address: row.mail_from_address.clone(),
        resend_api_key_configured: secrets.resend_api_key().is_set(),
        invite_mail_enabled: row.invite_mail_enabled,
        recovery_mail_enabled: row.recovery_mail_enabled,
        alert_mail_enabled: row.alert_mail_enabled,
        backup_retention_days: row.backup_retention_days,
        log_retention_days: row.log_retention_days,
        orphan_object_retention_days: row.orphan_object_retention_days,
        automatic_cleanup_enabled: row.automatic_cleanup_enabled,
        updated_at: row.updated_at.to_wire_text(),
        resend_enabled: transport.provider == Provider::Resend,
        smtp_host: row.smtp_host.clone(),
        smtp_port: row.smtp_port,
        smtp_security: match Security::parse(Some(&row.smtp_security)) {
            Security::Tls => "tls",
            Security::None => "none",
            Security::StartTls => "starttls",
        },
        smtp_username: row.smtp_username.clone(),
        smtp_password_configured: secrets.smtp_password().is_set(),
        mail_ready: transport.ready,
        giphy_enabled: crate::bootstrap::resolve_giphy(row.giphy_enabled, secrets).enabled,
        giphy_key_configured: secrets.get(&secrets::GIPHY_API_KEY).is_set(),
        mfa_requirement: policy::normalize_requirement(Some(&row.mfa_requirement)),
        trusted_device_days: row.trusted_device_days,
        auth_challenge_minutes: row.auth_challenge_minutes,
        email_code_minutes: row.email_code_minutes,
        email_code_cooldown_seconds: row.email_code_cooldown_seconds,
        email_code_max_attempts: row.email_code_max_attempts,
        email_code_sends_per_hour: row.email_code_sends_per_hour,
        cli_recovery_minutes: row.cli_recovery_minutes,
    }
}

fn instance_name(db: &Connection, server: Id) -> rusqlite::Result<String> {
    db.query_row(
        "SELECT \"Name\" FROM \"Servers\" WHERE \"Id\" = ?1",
        [server],
        |row| row.get(0),
    )
}

pub(super) async fn read(
    State(state): State<AppState>,
    InstanceOwner(server): InstanceOwner,
) -> ApiResult<Response> {
    let reading = state.clone();
    let answer = state
        .database()
        .read(move |db| -> Result<SettingsDto, DbError> {
            let row = load(db, &reading, server)?.settings;
            let secrets = secrets::read(db, &reading.settings().secrets, reading.sealer())?;
            Ok(dto(&reading, instance_name(db, server)?, &row, &secrets))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}

/// What a request to change the settings may say. A flag or a number it
/// leaves out reads as off or zero, which the rules below then refuse; the
/// members that are optional are left as they are when left out.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    instance_name: Option<String>,
    time_zone: Option<String>,
    #[serde(default)]
    mailing_enabled: bool,
    mail_from_address: Option<String>,
    #[serde(default)]
    invite_mail_enabled: bool,
    #[serde(default)]
    recovery_mail_enabled: bool,
    #[serde(default)]
    alert_mail_enabled: bool,
    #[serde(default, deserialize_with = "super::lenient")]
    backup_retention_days: i32,
    #[serde(default, deserialize_with = "super::lenient")]
    log_retention_days: i32,
    #[serde(default, deserialize_with = "super::lenient")]
    orphan_object_retention_days: i32,
    #[serde(default)]
    automatic_cleanup_enabled: bool,
    smtp_host: Option<String>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    smtp_port: Option<i32>,
    smtp_security: Option<String>,
    smtp_username: Option<String>,
    resend_enabled: Option<bool>,
    giphy_enabled: Option<bool>,
    mfa_requirement: Option<String>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    trusted_device_days: Option<i32>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    auth_challenge_minutes: Option<i32>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    email_code_minutes: Option<i32>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    email_code_cooldown_seconds: Option<i32>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    email_code_max_attempts: Option<i32>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    email_code_sends_per_hour: Option<i32>,
    #[serde(default, deserialize_with = "super::lenient_option")]
    cli_recovery_minutes: Option<i32>,
}

impl Change {
    /// Who may skip a second step needs a fresh one to be changed.
    fn changes_account_security(&self) -> bool {
        self.mfa_requirement.is_some()
            || self.trusted_device_days.is_some()
            || self.auth_challenge_minutes.is_some()
            || self.email_code_minutes.is_some()
            || self.email_code_cooldown_seconds.is_some()
            || self.email_code_max_attempts.is_some()
            || self.email_code_sends_per_hour.is_some()
            || self.cli_recovery_minutes.is_some()
    }

    /// A change of anything about mail: the server that sends every account
    /// mail (which a stolen session could point elsewhere), the sender, whether
    /// mail is sent at all and which kinds of it are. The first server asked
    /// for the step for the transport alone; this server asks for all of it,
    /// on purpose.
    fn changes_mail(&self, row: &Row) -> bool {
        self.mailing_enabled != row.mailing_enabled
            || self.mail_from_address.as_deref().unwrap_or("").trim() != row.mail_from_address
            || self.invite_mail_enabled != row.invite_mail_enabled
            || self.recovery_mail_enabled != row.recovery_mail_enabled
            || self.alert_mail_enabled != row.alert_mail_enabled
            || self.smtp_host.as_deref().is_some_and(|host| host.trim() != row.smtp_host)
            || self.smtp_port.is_some_and(|port| i64::from(port) != row.smtp_port)
            || self.smtp_security.as_deref().is_some_and(|security| {
                !security.trim().eq_ignore_ascii_case(&row.smtp_security)
            })
            || self
                .smtp_username
                .as_deref()
                .is_some_and(|name| name.trim() != row.smtp_username)
            || self.resend_enabled.is_some_and(|resend| Some(resend) != row.resend_enabled)
    }

    /// The whole of the request as the shared rules read it.
    fn update(&self) -> Update {
        let wide = |value: Option<i32>| value.map(i64::from);
        Update {
            instance_name: self.instance_name.clone().unwrap_or_default(),
            time_zone: self.time_zone.clone().unwrap_or_default(),
            mailing_enabled: self.mailing_enabled,
            mail_from_address: self.mail_from_address.clone().unwrap_or_default(),
            invite_mail_enabled: self.invite_mail_enabled,
            recovery_mail_enabled: self.recovery_mail_enabled,
            alert_mail_enabled: self.alert_mail_enabled,
            backup_retention_days: i64::from(self.backup_retention_days),
            log_retention_days: i64::from(self.log_retention_days),
            orphan_object_retention_days: i64::from(self.orphan_object_retention_days),
            automatic_cleanup_enabled: self.automatic_cleanup_enabled,
            smtp_host: self.smtp_host.clone(),
            smtp_port: wide(self.smtp_port),
            smtp_security: self.smtp_security.clone(),
            smtp_username: self.smtp_username.clone(),
            resend_enabled: self.resend_enabled,
            giphy_enabled: self.giphy_enabled,
            mfa_requirement: self.mfa_requirement.clone(),
            trusted_device_days: wide(self.trusted_device_days),
            auth_challenge_minutes: wide(self.auth_challenge_minutes),
            email_code_minutes: wide(self.email_code_minutes),
            email_code_cooldown_seconds: wide(self.email_code_cooldown_seconds),
            email_code_max_attempts: wide(self.email_code_max_attempts),
            email_code_sends_per_hour: wide(self.email_code_sends_per_hour),
            cli_recovery_minutes: wide(self.cli_recovery_minutes),
        }
    }
}

pub(super) async fn update(State(state): State<AppState>, request: Request) -> ApiResult<Response> {
    let panel::Bound {
        caller,
        server,
        body,
    } = panel::bind_then_gate(&state, request, false, bind::<Change>).await?;
    let Some(change) = body else {
        return Err(ApiError::bad_request());
    };
    let messages = *state.messages();
    let update = change.update();
    let problems = operations::validate(&update, &messages)?;
    if !problems.is_empty() {
        return Err(Problem::validation(
            problems
                .into_iter()
                .map(|(field, texts)| (field.to_string(), texts))
                .collect(),
        )
        .into());
    }
    let writing = state.clone();
    let answer = state
        .database()
        .write(move |tx| -> Result<SettingsDto, ApiError> {
            let Loaded { mut settings, stored } = load(tx, &writing, server)?;
            // Who may skip a second step, and where the mail with codes and
            // reset links goes, need a fresh one; the rest of the page does not.
            if (change.changes_account_security() || change.changes_mail(&settings))
                && !fresh::has_fresh_second_step(tx, caller.user, Some(caller.session), Timestamp::now())?
            {
                return Err(fresh::refusal(&messages)?.into());
            }
            let name = operations::apply(&mut settings, &update, Timestamp::now());
            operations::save(tx, &settings, stored, &name)?;
            panel::record(tx, server, caller.user, panel::settings_updated(&writing)?)?;
            let secrets = secrets::read(tx, &writing.settings().secrets, writing.sealer())?;
            Ok(dto(&writing, name, &settings, &secrets))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}
