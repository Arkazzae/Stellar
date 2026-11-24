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
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use stellarr_mail::{MailSettings, MailTransport, Provider};
use stellarr_messages::{MessageKey, Messages};
use stellarr_smtp::Security;

use super::address;
use super::{length, bind};
use crate::auth::{fresh, policy};
use crate::database::DbError;
use crate::http::{ApiError, ApiResult, Problem, json_response};
use crate::panel::{self, operations};
use crate::permissions::membership::InstanceOwner;
use crate::secrets::{self, ResolvedSecrets};
use crate::state::AppState;
use crate::values::{Id, Timestamp};

const MAX_MAIL_ADDRESS_LENGTH: usize = 320;
const MAX_RETENTION_DAYS: i32 = operations::MAX_RETENTION_DAYS as i32;
const DEFAULT_SMTP_PORT: i64 = 587;

/// The settings row as it stands, or as the installation's values make it.
pub(super) struct Row {
    pub time_zone: String,
    pub mailing_enabled: bool,
    pub mail_from_address: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_username: String,
    pub smtp_security: String,
    pub resend_enabled: Option<bool>,
    pub giphy_enabled: Option<bool>,
    pub invite_mail: bool,
    pub recovery_mail: bool,
    pub alert_mail: bool,
    pub backup_retention_days: i64,
    pub log_retention_days: i64,
    pub orphan_retention_days: i64,
    pub automatic_cleanup: bool,
    pub desktop_minimum: Option<String>,
    pub desktop_recommended: Option<String>,
    pub mfa_requirement: String,
    pub trusted_device_days: i64,
    pub auth_challenge_minutes: i64,
    pub email_code_minutes: i64,
    pub email_code_cooldown_seconds: i64,
    pub email_code_max_attempts: i64,
    pub email_code_sends_per_hour: i64,
    pub cli_recovery_minutes: i64,
    pub updated_at: Timestamp,
}

impl Row {
    /// What an installation that never saved has.
    fn defaults(state: &AppState, created: Timestamp) -> Row {
        let settings = state.settings();
        Row {
            time_zone: settings
                .operations
                .time_zone
                .clone()
                .unwrap_or_else(|| stellarr_timezone::TimeZone::local().id().to_string()),
            mailing_enabled: settings.mail.enabled,
            mail_from_address: settings.mail.from_address.trim().to_string(),
            smtp_host: String::new(),
            smtp_port: DEFAULT_SMTP_PORT,
            smtp_username: String::new(),
            smtp_security: "starttls".into(),
            resend_enabled: None,
            giphy_enabled: None,
            invite_mail: true,
            recovery_mail: true,
            alert_mail: false,
            backup_retention_days: operations::DEFAULT_BACKUP_RETENTION_DAYS,
            log_retention_days: operations::DEFAULT_LOG_RETENTION_DAYS,
            orphan_retention_days: operations::DEFAULT_ORPHAN_RETENTION_DAYS,
            automatic_cleanup: true,
            desktop_minimum: None,
            desktop_recommended: None,
            mfa_requirement: "optional".into(),
            trusted_device_days: 14,
            auth_challenge_minutes: 5,
            email_code_minutes: 8,
            email_code_cooldown_seconds: 45,
            email_code_max_attempts: 5,
            email_code_sends_per_hour: 10,
            cli_recovery_minutes: 10,
            updated_at: created,
        }
    }

    /// The row of the instance server, or what stands in for it.
    pub(super) fn load(db: &Connection, state: &AppState, server: Id) -> rusqlite::Result<Row> {
        let created: Timestamp = db.query_row(
            "SELECT \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
            [server],
            |row| row.get(0),
        )?;
        let stored = db
            .query_row(
                "SELECT \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\", \"MailSmtpPort\", \"MailSmtpUsername\",
                        \"MailSmtpSecurity\", \"MailResendEnabled\", \"GiphyEnabled\", \"InviteMailEnabled\", \"RecoveryMailEnabled\",
                        \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\",
                        \"AutomaticCleanupEnabled\", \"DesktopMinimumVersion\", \"DesktopRecommendedVersion\", \"MfaRequirement\",
                        \"TrustedDeviceDays\", \"AuthChallengeMinutes\", \"EmailCodeMinutes\", \"EmailCodeCooldownSeconds\",
                        \"EmailCodeMaxAttempts\", \"EmailCodeSendsPerHour\", \"CliRecoveryMinutes\", \"UpdatedAt\"
                 FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
                [server],
                |row| {
                    Ok(Row {
                        time_zone: row.get(0)?,
                        mailing_enabled: row.get(1)?,
                        mail_from_address: row.get(2)?,
                        smtp_host: row.get(3)?,
                        smtp_port: row.get(4)?,
                        smtp_username: row.get(5)?,
                        smtp_security: row.get(6)?,
                        resend_enabled: row.get(7)?,
                        giphy_enabled: row.get(8)?,
                        invite_mail: row.get(9)?,
                        recovery_mail: row.get(10)?,
                        alert_mail: row.get(11)?,
                        backup_retention_days: row.get(12)?,
                        log_retention_days: row.get(13)?,
                        orphan_retention_days: row.get(14)?,
                        automatic_cleanup: row.get(15)?,
                        desktop_minimum: row.get(16)?,
                        desktop_recommended: row.get(17)?,
                        mfa_requirement: row.get(18)?,
                        trusted_device_days: row.get(19)?,
                        auth_challenge_minutes: row.get(20)?,
                        email_code_minutes: row.get(21)?,
                        email_code_cooldown_seconds: row.get(22)?,
                        email_code_max_attempts: row.get(23)?,
                        email_code_sends_per_hour: row.get(24)?,
                        cli_recovery_minutes: row.get(25)?,
                        updated_at: row.get(26)?,
                    })
                },
            )
            .optional()?;
        Ok(stored.unwrap_or_else(|| Row::defaults(state, created)))
    }

    /// The transport this row and the secrets make.
    pub(super) fn transport(&self, secrets: &ResolvedSecrets) -> MailTransport {
        MailTransport::resolve(
            Some(&MailSettings {
                smtp_host: self.smtp_host.clone(),
                smtp_port: u16::try_from(self.smtp_port).unwrap_or(stellarr_mail::DEFAULT_SMTP_PORT),
                smtp_security: self.smtp_security.clone(),
                smtp_username: self.smtp_username.clone(),
                resend_enabled: self.resend_enabled,
            }),
            secrets.resend_api_key().expose(),
            secrets.smtp_password().expose(),
        )
    }
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

fn dto(state: &AppState, name: String, row: &Row, secrets: &ResolvedSecrets, saved_now: bool) -> SettingsDto {
    let transport = row.transport(secrets);
    SettingsDto {
        instance_name: name,
        public_url: state.settings().operations.public_url.clone(),
        time_zone: row.time_zone.clone(),
        mailing_enabled: row.mailing_enabled,
        mail_provider: transport.provider.name(),
        mail_from_address: row.mail_from_address.clone(),
        resend_api_key_configured: secrets.resend_api_key().is_set(),
        invite_mail_enabled: row.invite_mail,
        recovery_mail_enabled: row.recovery_mail,
        alert_mail_enabled: row.alert_mail,
        backup_retention_days: row.backup_retention_days,
        log_retention_days: row.log_retention_days,
        orphan_object_retention_days: row.orphan_retention_days,
        automatic_cleanup_enabled: row.automatic_cleanup,
        // A moment the server has just made says it is UTC; one read back from
        // the database does not.
        updated_at: if saved_now {
            row.updated_at.to_wire_text()
        } else {
            row.updated_at.to_unspecified_wire_text()
        },
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
            let row = Row::load(db, &reading, server)?;
            let secrets = secrets::read(db, &reading.settings().secrets, reading.sealer())?;
            Ok(dto(&reading, instance_name(db, server)?, &row, &secrets, false))
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

    /// A change of the server that sends every account mail, which a stolen
    /// session could point elsewhere.
    fn changes_mail_transport(&self, row: &Row) -> bool {
        self.smtp_host.as_deref().is_some_and(|host| host.trim() != row.smtp_host)
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

    fn validate(&self, messages: &Messages) -> Result<Vec<(String, Vec<String>)>, ApiError> {
        let mut problems: Vec<(String, Vec<String>)> = Vec::new();
        let mut refuse = |field: &str, text: String| problems.push((field.to_string(), vec![text]));
        if !(1..=80).contains(&length(self.instance_name.as_deref().unwrap_or("").trim())) {
            refuse("instanceName", messages.get(MessageKey::CockpitInstanceNameLength, &[])?);
        }
        match stellarr_timezone::TimeZone::find(self.time_zone.as_deref().unwrap_or("").trim()) {
            Ok(_) => {}
            Err(stellarr_timezone::Error::NotFound) => {
                refuse("timeZone", messages.get(MessageKey::CockpitTimeZoneUnavailable, &[])?);
            }
            Err(stellarr_timezone::Error::Invalid(_)) => {
                refuse("timeZone", messages.get(MessageKey::CockpitTimeZoneBroken, &[])?);
            }
        }
        if self.smtp_host.as_deref().is_some_and(|host| length(host.trim()) > 255) {
            refuse("smtpHost", messages.get(MessageKey::CockpitSmtpHostLength, &[])?);
        }
        if self.smtp_port.is_some_and(|port| !(1..=65535).contains(&port)) {
            refuse("smtpPort", messages.get(MessageKey::CockpitSmtpPortRange, &[])?);
        }
        if let Some(security) = &self.smtp_security {
            let word = security.trim().to_lowercase();
            if !["starttls", "tls", "none"].contains(&word.as_str()) {
                refuse("smtpSecurity", messages.get(MessageKey::CockpitSmtpSecurityUnknown, &[])?);
            }
        }
        if self.smtp_username.as_deref().is_some_and(|name| length(name.trim()) > MAX_MAIL_ADDRESS_LENGTH) {
            refuse("smtpUsername", messages.get(MessageKey::CockpitSmtpUsernameLength, &[])?);
        }
        let from = self.mail_from_address.as_deref().unwrap_or("").trim();
        if length(from) > MAX_MAIL_ADDRESS_LENGTH || (self.mailing_enabled && address::try_create(from).is_none()) {
            refuse("mailFromAddress", messages.get(MessageKey::CockpitMailFromRequired, &[])?);
        }
        let max = MAX_RETENTION_DAYS;
        if !(1..=max).contains(&self.backup_retention_days) {
            refuse("backupRetentionDays", messages.get(MessageKey::CockpitBackupRetentionRange, &[("max", &max)])?);
        }
        if !(1..=max).contains(&self.log_retention_days) {
            refuse("logRetentionDays", messages.get(MessageKey::CockpitLogRetentionRange, &[("max", &max)])?);
        }
        if !(1..=max).contains(&self.orphan_object_retention_days) {
            refuse("orphanObjectRetentionDays", messages.get(MessageKey::CockpitOrphanRetentionRange, &[("max", &max)])?);
        }
        if let Some(requirement) = &self.mfa_requirement {
            let word = requirement.trim().to_lowercase();
            if !["optional", "admins", "everyone", "cockpit"].contains(&word.as_str()) {
                refuse("mfaRequirement", messages.get(MessageKey::SecurityRequirementUnknown, &[])?);
            }
        }
        let outside = |value: Option<i32>, low: i32, high: i32| value.is_some_and(|v| !(low..=high).contains(&v));
        if outside(self.trusted_device_days, 1, 365) {
            refuse("trustedDeviceDays", messages.get(MessageKey::SecurityTrustedDeviceDaysRange, &[])?);
        }
        if outside(self.auth_challenge_minutes, 1, 60) {
            refuse("authChallengeMinutes", messages.get(MessageKey::SecurityChallengeMinutesRange, &[])?);
        }
        if outside(self.email_code_minutes, 1, 60) {
            refuse("emailCodeMinutes", messages.get(MessageKey::SecurityEmailCodeMinutesRange, &[])?);
        }
        if outside(self.email_code_cooldown_seconds, 0, 600) {
            refuse("emailCodeCooldownSeconds", messages.get(MessageKey::SecurityEmailCooldownRange, &[])?);
        }
        if outside(self.email_code_max_attempts, 1, 10) {
            refuse("emailCodeMaxAttempts", messages.get(MessageKey::SecurityEmailAttemptsRange, &[])?);
        }
        if outside(self.email_code_sends_per_hour, 1, 60) {
            refuse("emailCodeSendsPerHour", messages.get(MessageKey::SecurityEmailSendsRange, &[])?);
        }
        if outside(self.cli_recovery_minutes, 1, 60) {
            refuse("cliRecoveryMinutes", messages.get(MessageKey::SecurityCliRecoveryMinutesRange, &[])?);
        }
        Ok(problems)
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
    let problems = change.validate(&messages)?;
    if !problems.is_empty() {
        return Err(Problem::validation(problems).into());
    }
    let writing = state.clone();
    let answer = state
        .database()
        .write(move |tx| -> Result<SettingsDto, ApiError> {
            let created: Timestamp = tx.query_row(
                "SELECT \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
                [server],
                |row| row.get(0),
            )?;
            operations::ensure(tx, writing.settings(), server, created)?;
            let mut row = Row::load(tx, &writing, server)?;
            // Who may skip a second step, and where the mail with codes and
            // reset links goes, need a fresh one; the rest of the page does not.
            if (change.changes_account_security() || change.changes_mail_transport(&row))
                && !fresh::has_fresh_second_step(tx, caller.user, Some(caller.session), Timestamp::now())?
            {
                return Err(fresh::refusal(&messages)?.into());
            }
            let name = change.instance_name.as_deref().unwrap_or("").trim().to_string();
            row.time_zone = change.time_zone.as_deref().unwrap_or("").trim().to_string();
            row.mailing_enabled = change.mailing_enabled;
            row.mail_from_address = change.mail_from_address.as_deref().unwrap_or("").trim().to_string();
            row.invite_mail = change.invite_mail_enabled;
            row.recovery_mail = change.recovery_mail_enabled;
            row.alert_mail = change.alert_mail_enabled;
            row.backup_retention_days = i64::from(change.backup_retention_days);
            row.log_retention_days = i64::from(change.log_retention_days);
            row.orphan_retention_days = i64::from(change.orphan_object_retention_days);
            row.automatic_cleanup = change.automatic_cleanup_enabled;
            // The mail transport fields are left as they are when a request
            // leaves them out.
            if let Some(host) = &change.smtp_host {
                row.smtp_host = host.trim().to_string();
            }
            if let Some(port) = change.smtp_port {
                row.smtp_port = i64::from(port);
            }
            if let Some(security) = &change.smtp_security {
                row.smtp_security = security.trim().to_lowercase();
            }
            if let Some(username) = &change.smtp_username {
                row.smtp_username = username.trim().to_string();
            }
            row.resend_enabled = change.resend_enabled.or(row.resend_enabled);
            row.giphy_enabled = change.giphy_enabled.or(row.giphy_enabled);
            if let Some(requirement) = &change.mfa_requirement {
                row.mfa_requirement = policy::normalize_requirement(Some(requirement)).to_string();
            }
            let keep = |new: Option<i32>, old: i64| new.map_or(old, i64::from);
            row.trusted_device_days = keep(change.trusted_device_days, row.trusted_device_days);
            row.auth_challenge_minutes = keep(change.auth_challenge_minutes, row.auth_challenge_minutes);
            row.email_code_minutes = keep(change.email_code_minutes, row.email_code_minutes);
            row.email_code_cooldown_seconds = keep(change.email_code_cooldown_seconds, row.email_code_cooldown_seconds);
            row.email_code_max_attempts = keep(change.email_code_max_attempts, row.email_code_max_attempts);
            row.email_code_sends_per_hour = keep(change.email_code_sends_per_hour, row.email_code_sends_per_hour);
            row.cli_recovery_minutes = keep(change.cli_recovery_minutes, row.cli_recovery_minutes);
            row.updated_at = Timestamp::now();
            tx.execute("UPDATE \"Servers\" SET \"Name\" = ?2 WHERE \"Id\" = ?1", params![server, name])?;
            tx.execute(
                "UPDATE \"OperationsSettings\" SET \"TimeZone\" = ?2, \"MailingEnabled\" = ?3, \"MailFromAddress\" = ?4, \"MailSmtpHost\" = ?5,
                        \"MailSmtpPort\" = ?6, \"MailSmtpUsername\" = ?7, \"MailSmtpSecurity\" = ?8, \"MailResendEnabled\" = ?9, \"GiphyEnabled\" = ?10,
                        \"InviteMailEnabled\" = ?11, \"RecoveryMailEnabled\" = ?12, \"AlertMailEnabled\" = ?13, \"BackupRetentionDays\" = ?14,
                        \"LogRetentionDays\" = ?15, \"OrphanObjectRetentionDays\" = ?16, \"AutomaticCleanupEnabled\" = ?17, \"MfaRequirement\" = ?18,
                        \"TrustedDeviceDays\" = ?19, \"AuthChallengeMinutes\" = ?20, \"EmailCodeMinutes\" = ?21, \"EmailCodeCooldownSeconds\" = ?22,
                        \"EmailCodeMaxAttempts\" = ?23, \"EmailCodeSendsPerHour\" = ?24, \"CliRecoveryMinutes\" = ?25, \"UpdatedAt\" = ?26
                 WHERE \"ServerId\" = ?1",
                params![
                    server, row.time_zone, row.mailing_enabled, row.mail_from_address, row.smtp_host, row.smtp_port,
                    row.smtp_username, row.smtp_security, row.resend_enabled, row.giphy_enabled, row.invite_mail,
                    row.recovery_mail, row.alert_mail, row.backup_retention_days, row.log_retention_days,
                    row.orphan_retention_days, row.automatic_cleanup, row.mfa_requirement, row.trusted_device_days,
                    row.auth_challenge_minutes, row.email_code_minutes, row.email_code_cooldown_seconds,
                    row.email_code_max_attempts, row.email_code_sends_per_hour, row.cli_recovery_minutes, row.updated_at
                ],
            )?;
            panel::record(tx, server, caller.user, panel::settings_updated(&writing)?)?;
            let secrets = secrets::read(tx, &writing.settings().secrets, writing.sealer())?;
            Ok(dto(&writing, name, &row, &secrets, true))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &answer))
}
