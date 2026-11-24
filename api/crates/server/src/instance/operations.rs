//! The settings of the instance that the operator's panel and the operator's
//! command line both write: one row of `OperationsSettings` for the instance
//! server, and the name of that server.
//!
//! Keeping the defaults, the validation and the update here means a command
//! run in a container cannot accept a value the panel would refuse, or start a
//! row with other values. Before anything was saved there is no row, and the
//! defaults of this installation stand in for it: [`load`] returns them, and
//! only [`save`] stores them.
//!
//! Every function takes the connection or transaction of its caller, so that
//! what else commits with a change (the operator's audit entry) is the
//! caller's to decide.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use stellarr_messages::{MessageKey, Messages, MissingValue};

use crate::auth::policy;
use crate::settings::Config;
use crate::values::{Id, Timestamp};

pub const MAX_RETENTION_DAYS: i64 = 3650;
pub const MAX_MAIL_ADDRESS_LENGTH: usize = 320;
pub const DEFAULT_BACKUP_RETENTION_DAYS: i64 = 14;
pub const DEFAULT_SMTP_PORT: i64 = 587;

/// The ways a connection to an SMTP server is secured, as the settings name them.
pub const SMTP_SECURITY_NAMES: [&str; 3] = ["starttls", "tls", "none"];

/// The instance server: the first one made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceServer {
    pub id: Id,
    pub name: String,
    pub created_at: Timestamp,
}

/// The first server made, which is the instance's own.
pub fn instance_server(connection: &Connection) -> rusqlite::Result<Option<InstanceServer>> {
    connection
        .query_row(
            "SELECT \"Id\", \"Name\", \"CreatedAt\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1",
            [],
            |row| {
                Ok(InstanceServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                })
            },
        )
        .optional()
}

/// The row of settings of the instance server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub server: Id,
    pub public_url: String,
    pub time_zone: String,
    pub mailing_enabled: bool,
    pub mail_from_address: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_username: String,
    pub smtp_security: String,
    /// `None` until the operator chooses: the key then decides.
    pub resend_enabled: Option<bool>,
    /// `None` until the operator chooses: the key then decides.
    pub giphy_enabled: Option<bool>,
    pub invite_mail_enabled: bool,
    pub recovery_mail_enabled: bool,
    pub alert_mail_enabled: bool,
    pub backup_retention_days: i64,
    pub log_retention_days: i64,
    pub orphan_object_retention_days: i64,
    pub automatic_cleanup_enabled: bool,
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

/// The settings of the instance server and whether they are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub settings: Settings,
    /// False while the row is only the installation's defaults.
    pub stored: bool,
}

const COLUMNS: &str = "\"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\", \"MailSmtpHost\", \"MailSmtpPort\",
    \"MailSmtpUsername\", \"MailSmtpSecurity\", \"MailResendEnabled\", \"GiphyEnabled\", \"InviteMailEnabled\",
    \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
    \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"MfaRequirement\", \"TrustedDeviceDays\",
    \"AuthChallengeMinutes\", \"EmailCodeMinutes\", \"EmailCodeCooldownSeconds\", \"EmailCodeMaxAttempts\",
    \"EmailCodeSendsPerHour\", \"CliRecoveryMinutes\", \"UpdatedAt\"";

/// The row of `server`, or what the installation starts with. `config` is the
/// layered configuration the defaults come from, and `server_created_at` is
/// the moment such a row is dated with until it is saved.
pub fn load(
    connection: &Connection,
    server: Id,
    server_created_at: Timestamp,
    config: &Config,
) -> rusqlite::Result<Loaded> {
    let stored = connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1"),
            [server],
            |row| {
                Ok(Settings {
                    server,
                    public_url: row.get(0)?,
                    time_zone: row.get(1)?,
                    mailing_enabled: row.get(2)?,
                    mail_from_address: row.get(3)?,
                    smtp_host: row.get(4)?,
                    smtp_port: row.get(5)?,
                    smtp_username: row.get(6)?,
                    smtp_security: row.get(7)?,
                    resend_enabled: row.get(8)?,
                    giphy_enabled: row.get(9)?,
                    invite_mail_enabled: row.get(10)?,
                    recovery_mail_enabled: row.get(11)?,
                    alert_mail_enabled: row.get(12)?,
                    backup_retention_days: row.get(13)?,
                    log_retention_days: row.get(14)?,
                    orphan_object_retention_days: row.get(15)?,
                    automatic_cleanup_enabled: row.get(16)?,
                    mfa_requirement: row.get(17)?,
                    trusted_device_days: row.get(18)?,
                    auth_challenge_minutes: row.get(19)?,
                    email_code_minutes: row.get(20)?,
                    email_code_cooldown_seconds: row.get(21)?,
                    email_code_max_attempts: row.get(22)?,
                    email_code_sends_per_hour: row.get(23)?,
                    cli_recovery_minutes: row.get(24)?,
                    updated_at: row.get(25)?,
                })
            },
        )
        .optional()?;
    Ok(match stored {
        Some(settings) => Loaded {
            settings,
            stored: true,
        },
        None => Loaded {
            settings: defaults(server, server_created_at, config),
            stored: false,
        },
    })
}

/// What an installation starts with, from its configuration.
pub fn defaults(server: Id, server_created_at: Timestamp, config: &Config) -> Settings {
    Settings {
        server,
        public_url: config
            .raw("Operations:PublicUrl")
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        time_zone: config
            .raw("Operations:TimeZone")
            .map(str::trim)
            .map_or_else(
                || stellarr_timezone::TimeZone::local().id().to_string(),
                str::to_string,
            ),
        mailing_enabled: config.flag("Mail:Enabled").ok().flatten().unwrap_or(false),
        mail_from_address: config
            .raw("Mail:FromAddress")
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        smtp_host: String::new(),
        smtp_port: DEFAULT_SMTP_PORT,
        smtp_username: String::new(),
        smtp_security: SMTP_SECURITY_NAMES[0].to_string(),
        resend_enabled: None,
        giphy_enabled: None,
        invite_mail_enabled: true,
        recovery_mail_enabled: true,
        alert_mail_enabled: false,
        backup_retention_days: DEFAULT_BACKUP_RETENTION_DAYS,
        log_retention_days: 30,
        orphan_object_retention_days: 1,
        automatic_cleanup_enabled: true,
        mfa_requirement: policy::OPTIONAL.to_string(),
        trusted_device_days: 14,
        auth_challenge_minutes: 5,
        email_code_minutes: 8,
        email_code_cooldown_seconds: 45,
        email_code_max_attempts: 5,
        email_code_sends_per_hour: 10,
        cli_recovery_minutes: 10,
        updated_at: server_created_at,
    }
}

/// A change of settings, every field of it: the panel sends the whole form.
/// A field that is `None` is left as it is, where the setting has no value to
/// give back (the choice of Resend and of GIPHY, which can be unmade only by a
/// database edit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub instance_name: String,
    pub time_zone: String,
    pub mailing_enabled: bool,
    pub mail_from_address: String,
    pub invite_mail_enabled: bool,
    pub recovery_mail_enabled: bool,
    pub alert_mail_enabled: bool,
    pub backup_retention_days: i64,
    pub log_retention_days: i64,
    pub orphan_object_retention_days: i64,
    pub automatic_cleanup_enabled: bool,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    pub smtp_username: Option<String>,
    pub resend_enabled: Option<bool>,
    pub giphy_enabled: Option<bool>,
    pub mfa_requirement: Option<String>,
    pub trusted_device_days: Option<i64>,
    pub auth_challenge_minutes: Option<i64>,
    pub email_code_minutes: Option<i64>,
    pub email_code_cooldown_seconds: Option<i64>,
    pub email_code_max_attempts: Option<i64>,
    pub email_code_sends_per_hour: Option<i64>,
    pub cli_recovery_minutes: Option<i64>,
}

/// The update that changes nothing, so that one field can be replaced before
/// the whole is validated.
pub fn snapshot(server_name: &str, settings: &Settings) -> Update {
    Update {
        instance_name: server_name.to_string(),
        time_zone: settings.time_zone.clone(),
        mailing_enabled: settings.mailing_enabled,
        mail_from_address: settings.mail_from_address.clone(),
        invite_mail_enabled: settings.invite_mail_enabled,
        recovery_mail_enabled: settings.recovery_mail_enabled,
        alert_mail_enabled: settings.alert_mail_enabled,
        backup_retention_days: settings.backup_retention_days,
        log_retention_days: settings.log_retention_days,
        orphan_object_retention_days: settings.orphan_object_retention_days,
        automatic_cleanup_enabled: settings.automatic_cleanup_enabled,
        smtp_host: Some(settings.smtp_host.clone()),
        smtp_port: Some(settings.smtp_port),
        smtp_security: Some(settings.smtp_security.clone()),
        smtp_username: Some(settings.smtp_username.clone()),
        resend_enabled: settings.resend_enabled,
        giphy_enabled: settings.giphy_enabled,
        mfa_requirement: Some(settings.mfa_requirement.clone()),
        trusted_device_days: Some(settings.trusted_device_days),
        auth_challenge_minutes: Some(settings.auth_challenge_minutes),
        email_code_minutes: Some(settings.email_code_minutes),
        email_code_cooldown_seconds: Some(settings.email_code_cooldown_seconds),
        email_code_max_attempts: Some(settings.email_code_max_attempts),
        email_code_sends_per_hour: Some(settings.email_code_sends_per_hour),
        cli_recovery_minutes: Some(settings.cli_recovery_minutes),
    }
}

/// Applies a validated update to the settings in memory and returns the name
/// the instance server is to have. Nothing is stored until [`save`].
pub fn apply(settings: &mut Settings, update: &Update, now: Timestamp) -> String {
    settings.time_zone = update.time_zone.trim().to_string();
    settings.mailing_enabled = update.mailing_enabled;
    settings.mail_from_address = update.mail_from_address.trim().to_string();
    settings.invite_mail_enabled = update.invite_mail_enabled;
    settings.recovery_mail_enabled = update.recovery_mail_enabled;
    settings.alert_mail_enabled = update.alert_mail_enabled;
    settings.backup_retention_days = update.backup_retention_days;
    settings.log_retention_days = update.log_retention_days;
    settings.orphan_object_retention_days = update.orphan_object_retention_days;
    settings.automatic_cleanup_enabled = update.automatic_cleanup_enabled;
    // The mail transport fields are left as they are when an update leaves them out.
    if let Some(host) = &update.smtp_host {
        settings.smtp_host = host.trim().to_string();
    }
    if let Some(port) = update.smtp_port {
        settings.smtp_port = port;
    }
    if let Some(security) = &update.smtp_security {
        settings.smtp_security = security.trim().to_lowercase();
    }
    if let Some(username) = &update.smtp_username {
        settings.smtp_username = username.trim().to_string();
    }
    settings.resend_enabled = update.resend_enabled.or(settings.resend_enabled);
    settings.giphy_enabled = update.giphy_enabled.or(settings.giphy_enabled);
    if let Some(requirement) = &update.mfa_requirement {
        settings.mfa_requirement = policy::normalize_requirement(Some(requirement)).to_string();
    }
    let keep = |new: Option<i64>, old: i64| new.unwrap_or(old);
    settings.trusted_device_days = keep(update.trusted_device_days, settings.trusted_device_days);
    settings.auth_challenge_minutes = keep(
        update.auth_challenge_minutes,
        settings.auth_challenge_minutes,
    );
    settings.email_code_minutes = keep(update.email_code_minutes, settings.email_code_minutes);
    settings.email_code_cooldown_seconds = keep(
        update.email_code_cooldown_seconds,
        settings.email_code_cooldown_seconds,
    );
    settings.email_code_max_attempts = keep(
        update.email_code_max_attempts,
        settings.email_code_max_attempts,
    );
    settings.email_code_sends_per_hour = keep(
        update.email_code_sends_per_hour,
        settings.email_code_sends_per_hour,
    );
    settings.cli_recovery_minutes =
        keep(update.cli_recovery_minutes, settings.cli_recovery_minutes);
    settings.updated_at = now;
    update.instance_name.trim().to_string()
}

/// Stores the settings, as an insert when `stored` is false and as an update
/// of the row otherwise, and names the instance server `name`.
pub fn save(
    transaction: &Transaction,
    settings: &Settings,
    stored: bool,
    name: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE \"Servers\" SET \"Name\" = ?2 WHERE \"Id\" = ?1",
        params![settings.server, name],
    )?;
    let values = params![
        settings.server,
        settings.public_url,
        settings.time_zone,
        settings.mailing_enabled,
        settings.mail_from_address,
        settings.smtp_host,
        settings.smtp_port,
        settings.smtp_username,
        settings.smtp_security,
        settings.resend_enabled,
        settings.giphy_enabled,
        settings.invite_mail_enabled,
        settings.recovery_mail_enabled,
        settings.alert_mail_enabled,
        settings.backup_retention_days,
        settings.log_retention_days,
        settings.orphan_object_retention_days,
        settings.automatic_cleanup_enabled,
        settings.mfa_requirement,
        settings.trusted_device_days,
        settings.auth_challenge_minutes,
        settings.email_code_minutes,
        settings.email_code_cooldown_seconds,
        settings.email_code_max_attempts,
        settings.email_code_sends_per_hour,
        settings.cli_recovery_minutes,
        settings.updated_at,
    ];
    if stored {
        transaction.execute(
            "UPDATE \"OperationsSettings\" SET \"PublicUrl\" = ?2, \"TimeZone\" = ?3, \"MailingEnabled\" = ?4,
                \"MailFromAddress\" = ?5, \"MailSmtpHost\" = ?6, \"MailSmtpPort\" = ?7, \"MailSmtpUsername\" = ?8,
                \"MailSmtpSecurity\" = ?9, \"MailResendEnabled\" = ?10, \"GiphyEnabled\" = ?11,
                \"InviteMailEnabled\" = ?12, \"RecoveryMailEnabled\" = ?13, \"AlertMailEnabled\" = ?14,
                \"BackupRetentionDays\" = ?15, \"LogRetentionDays\" = ?16, \"OrphanObjectRetentionDays\" = ?17,
                \"AutomaticCleanupEnabled\" = ?18, \"MfaRequirement\" = ?19, \"TrustedDeviceDays\" = ?20,
                \"AuthChallengeMinutes\" = ?21, \"EmailCodeMinutes\" = ?22, \"EmailCodeCooldownSeconds\" = ?23,
                \"EmailCodeMaxAttempts\" = ?24, \"EmailCodeSendsPerHour\" = ?25, \"CliRecoveryMinutes\" = ?26,
                \"UpdatedAt\" = ?27
             WHERE \"ServerId\" = ?1",
            values,
        )?;
    } else {
        transaction.execute(
            "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\",
                \"MailFromAddress\", \"MailSmtpHost\", \"MailSmtpPort\", \"MailSmtpUsername\", \"MailSmtpSecurity\",
                \"MailResendEnabled\", \"GiphyEnabled\", \"InviteMailEnabled\", \"RecoveryMailEnabled\",
                \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\",
                \"AutomaticCleanupEnabled\", \"MfaRequirement\", \"TrustedDeviceDays\", \"AuthChallengeMinutes\",
                \"EmailCodeMinutes\", \"EmailCodeCooldownSeconds\", \"EmailCodeMaxAttempts\",
                \"EmailCodeSendsPerHour\", \"CliRecoveryMinutes\", \"UpdatedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                     ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
            values,
        )?;
    }
    Ok(())
}

/// Whether `text` is an address that mail can be sent from: `user@host`, or
/// that with a name before it, `Name <user@host>`.
pub fn is_mail_address(text: &str) -> bool {
    super::sender_address::is_sender_address(text)
}

/// What is wrong with an update, field by field, in the order the checks are
/// made, with the sentences of the instance's messages. Empty when it is fine.
pub fn validate(
    update: &Update,
    messages: &Messages,
) -> Result<Vec<(&'static str, Vec<String>)>, MissingValue> {
    let mut problems: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut problem = |field: &'static str, text: String| problems.push((field, vec![text]));
    let utf16 = |text: &str| text.trim().encode_utf16().count();

    if !(1..=80).contains(&utf16(&update.instance_name)) {
        problem(
            "instanceName",
            messages.get(MessageKey::CockpitInstanceNameLength, &[])?,
        );
    }
    match stellarr_timezone::TimeZone::find(update.time_zone.trim()) {
        Ok(_) => {}
        Err(stellarr_timezone::Error::NotFound) => problem(
            "timeZone",
            messages.get(MessageKey::CockpitTimeZoneUnavailable, &[])?,
        ),
        Err(_) => problem(
            "timeZone",
            messages.get(MessageKey::CockpitTimeZoneBroken, &[])?,
        ),
    }
    if update.smtp_host.as_deref().map_or(0, utf16) > 255 {
        problem(
            "smtpHost",
            messages.get(MessageKey::CockpitSmtpHostLength, &[])?,
        );
    }
    if update
        .smtp_port
        .is_some_and(|port| !(1..=65535).contains(&port))
    {
        problem(
            "smtpPort",
            messages.get(MessageKey::CockpitSmtpPortRange, &[])?,
        );
    }
    if let Some(security) = &update.smtp_security
        && !SMTP_SECURITY_NAMES.contains(&security.trim().to_lowercase().as_str())
    {
        problem(
            "smtpSecurity",
            messages.get(MessageKey::CockpitSmtpSecurityUnknown, &[])?,
        );
    }
    if update.smtp_username.as_deref().map_or(0, utf16) > MAX_MAIL_ADDRESS_LENGTH {
        problem(
            "smtpUsername",
            messages.get(MessageKey::CockpitSmtpUsernameLength, &[])?,
        );
    }
    let from = update.mail_from_address.trim();
    if from.encode_utf16().count() > MAX_MAIL_ADDRESS_LENGTH
        || (update.mailing_enabled && !is_mail_address(from))
    {
        problem(
            "mailFromAddress",
            messages.get(MessageKey::CockpitMailFromRequired, &[])?,
        );
    }
    let max = MAX_RETENTION_DAYS;
    if !(1..=max).contains(&update.backup_retention_days) {
        problem(
            "backupRetentionDays",
            messages.get(MessageKey::CockpitBackupRetentionRange, &[("max", &max)])?,
        );
    }
    if !(1..=max).contains(&update.log_retention_days) {
        problem(
            "logRetentionDays",
            messages.get(MessageKey::CockpitLogRetentionRange, &[("max", &max)])?,
        );
    }
    if !(1..=max).contains(&update.orphan_object_retention_days) {
        problem(
            "orphanObjectRetentionDays",
            messages.get(MessageKey::CockpitOrphanRetentionRange, &[("max", &max)])?,
        );
    }
    if let Some(requirement) = &update.mfa_requirement
        && !is_known_requirement(requirement)
    {
        problem(
            "mfaRequirement",
            messages.get(MessageKey::SecurityRequirementUnknown, &[])?,
        );
    }
    let ranged = |value: Option<i64>, low: i64, high: i64| {
        value.is_some_and(|value| !(low..=high).contains(&value))
    };
    if ranged(update.trusted_device_days, 1, 365) {
        problem(
            "trustedDeviceDays",
            messages.get(MessageKey::SecurityTrustedDeviceDaysRange, &[])?,
        );
    }
    if ranged(update.auth_challenge_minutes, 1, 60) {
        problem(
            "authChallengeMinutes",
            messages.get(MessageKey::SecurityChallengeMinutesRange, &[])?,
        );
    }
    if ranged(update.email_code_minutes, 1, 60) {
        problem(
            "emailCodeMinutes",
            messages.get(MessageKey::SecurityEmailCodeMinutesRange, &[])?,
        );
    }
    if ranged(update.email_code_cooldown_seconds, 0, 600) {
        problem(
            "emailCodeCooldownSeconds",
            messages.get(MessageKey::SecurityEmailCooldownRange, &[])?,
        );
    }
    if ranged(update.email_code_max_attempts, 1, 10) {
        problem(
            "emailCodeMaxAttempts",
            messages.get(MessageKey::SecurityEmailAttemptsRange, &[])?,
        );
    }
    if ranged(update.email_code_sends_per_hour, 1, 60) {
        problem(
            "emailCodeSendsPerHour",
            messages.get(MessageKey::SecurityEmailSendsRange, &[])?,
        );
    }
    if ranged(update.cli_recovery_minutes, 1, 60) {
        problem(
            "cliRecoveryMinutes",
            messages.get(MessageKey::SecurityCliRecoveryMinutesRange, &[])?,
        );
    }
    Ok(problems)
}

/// Whether `value` names a requirement: `optional`, `admins`, `everyone` or
/// `cockpit`, in any letter case.
pub fn is_known_requirement(value: &str) -> bool {
    let lowered = value.trim().to_lowercase();
    [
        policy::OPTIONAL,
        policy::ADMINS,
        policy::EVERYONE,
        policy::COCKPIT,
    ]
    .contains(&lowered.as_str())
}

#[cfg(test)]
mod tests {
    use stellarr_messages::Language;

    use super::*;
    use crate::testing::new_database;

    fn english() -> Messages {
        Messages::new(Language::English)
    }

    fn a_server(connection: &Connection) -> InstanceServer {
        let id = Id::random();
        let created = Timestamp::parse("2026-11-01 10:00:00").unwrap();
        connection
            .execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\")
                 VALUES (?1, 'Orbit', ?2, 1, 1)",
                params![id, created],
            )
            .unwrap();
        instance_server(connection).unwrap().unwrap()
    }

    #[test]
    fn before_anything_is_saved_the_installations_defaults_stand_in() {
        let (_folder, database) = new_database();
        let connection = Connection::open(database.path()).unwrap();
        let server = a_server(&connection);
        let mut config = Config::default();
        config.set("Mail:Enabled", "true");
        config.set("Mail:FromAddress", " noreply@example.test ");
        config.set("Operations:TimeZone", "Europe/Warsaw");

        let loaded = load(&connection, server.id, server.created_at, &config).unwrap();

        assert!(!loaded.stored);
        assert_eq!(loaded.settings.time_zone, "Europe/Warsaw");
        assert!(loaded.settings.mailing_enabled);
        assert_eq!(loaded.settings.mail_from_address, "noreply@example.test");
        assert_eq!(loaded.settings.backup_retention_days, 14);
        assert_eq!(loaded.settings.updated_at, server.created_at);
        let rows: i64 = connection
            .query_row("SELECT count(*) FROM \"OperationsSettings\"", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "loading stores nothing");
    }

    #[test]
    fn a_saved_change_is_read_back_and_renames_the_server() {
        let (_folder, database) = new_database();
        let mut connection = Connection::open(database.path()).unwrap();
        let server = a_server(&connection);
        let config = Config::default();
        let mut loaded = load(&connection, server.id, server.created_at, &config).unwrap();
        let mut update = snapshot(&server.name, &loaded.settings);
        update.instance_name = "  Moon  ".into();
        update.smtp_port = Some(2525);
        update.giphy_enabled = Some(true);
        assert!(validate(&update, &english()).unwrap().is_empty());

        let name = apply(&mut loaded.settings, &update, Timestamp::now());
        let transaction = connection.transaction().unwrap();
        save(&transaction, &loaded.settings, loaded.stored, &name).unwrap();
        transaction.commit().unwrap();

        let again = load(&connection, server.id, server.created_at, &config).unwrap();
        assert!(again.stored);
        assert_eq!(again.settings.smtp_port, 2525);
        assert_eq!(again.settings.giphy_enabled, Some(true));
        assert_eq!(again.settings.resend_enabled, None);
        assert_eq!(instance_server(&connection).unwrap().unwrap().name, "Moon");
    }

    #[test]
    fn what_cannot_be_right_is_named_field_by_field_in_order() {
        let (_folder, database) = new_database();
        let connection = Connection::open(database.path()).unwrap();
        let server = a_server(&connection);
        let loaded = load(
            &connection,
            server.id,
            server.created_at,
            &Config::default(),
        )
        .unwrap();
        let mut update = snapshot("", &loaded.settings);
        update.time_zone = "Nowhere/Land".into();
        update.smtp_port = Some(99_999);
        update.mfa_requirement = Some("sometimes".into());
        update.backup_retention_days = 0;
        update.email_code_cooldown_seconds = Some(601);

        let problems = validate(&update, &english()).unwrap();

        let fields: Vec<&str> = problems.iter().map(|(field, _)| *field).collect();
        assert_eq!(
            fields,
            [
                "instanceName",
                "timeZone",
                "smtpPort",
                "backupRetentionDays",
                "mfaRequirement",
                "emailCodeCooldownSeconds"
            ]
        );
        assert_eq!(problems[3].1[0], "Backup retention must be 1 to 3650 days.");
    }
}
