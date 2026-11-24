//! The row of retention and clean-up settings the panel keeps for the
//! instance server, which the backups and the housekeeping both read.
//!
//! Before the operator has saved anything there is no row, and the defaults of
//! the installation stand in for it; the first change writes one.

use rusqlite::{Connection, OptionalExtension, params};

use crate::settings::Settings;
use crate::values::{Id, Timestamp};

pub const DEFAULT_BACKUP_RETENTION_DAYS: i64 = 14;
pub const DEFAULT_LOG_RETENTION_DAYS: i64 = 30;
pub const DEFAULT_ORPHAN_RETENTION_DAYS: i64 = 1;
/// The most days a retention may be set to.
pub const MAX_RETENTION_DAYS: i64 = 3650;

/// What the housekeeping and the backups read of the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub backup_days: i64,
    pub log_days: i64,
    pub orphan_object_days: i64,
    pub automatic_cleanup: bool,
}

impl Default for Retention {
    fn default() -> Retention {
        Retention {
            backup_days: DEFAULT_BACKUP_RETENTION_DAYS,
            log_days: DEFAULT_LOG_RETENTION_DAYS,
            orphan_object_days: DEFAULT_ORPHAN_RETENTION_DAYS,
            automatic_cleanup: true,
        }
    }
}

/// The settings of one server, or the defaults when it has no row.
pub fn of_server(connection: &Connection, server: Id) -> rusqlite::Result<Retention> {
    Ok(connection
        .query_row(
            "SELECT \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\"
             FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
            [server],
            from_row,
        )
        .optional()?
        .unwrap_or_default())
}

/// The settings the housekeeping goes by: the row written last, whichever
/// server it is for.
pub fn newest(connection: &Connection) -> rusqlite::Result<Retention> {
    Ok(connection
        .query_row(
            "SELECT \"BackupRetentionDays\", \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\"
             FROM \"OperationsSettings\" ORDER BY \"UpdatedAt\" DESC LIMIT 1",
            [],
            from_row,
        )
        .optional()?
        .unwrap_or_default())
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Retention> {
    Ok(Retention {
        backup_days: row.get(0)?,
        log_days: row.get(1)?,
        orphan_object_days: row.get(2)?,
        automatic_cleanup: row.get(3)?,
    })
}

/// Writes the row with the installation's defaults when the server has none,
/// as the first change of any setting did. `created_at` is when the server
/// was made, which is what the row's own time starts as.
pub fn ensure(
    transaction: &Connection,
    settings: &Settings,
    server: Id,
    created_at: Timestamp,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\",
             \"MailFromAddress\", \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\",
             \"LogRetentionDays\", \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"MfaRequirement\",
             \"TrustedDeviceDays\", \"AuthChallengeMinutes\", \"EmailCodeMinutes\", \"EmailCodeCooldownSeconds\",
             \"EmailCodeMaxAttempts\", \"EmailCodeSendsPerHour\", \"CliRecoveryMinutes\", \"UpdatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, 0, ?6, ?7, ?8, 1, 'optional', 14, 5, 8, 45, 5, 10, 10, ?9)",
        params![
            server,
            settings.operations.public_url,
            settings.operations.time_zone.as_deref().unwrap_or("UTC"),
            settings.mail.enabled,
            settings.mail.from_address.trim(),
            DEFAULT_BACKUP_RETENTION_DAYS,
            DEFAULT_LOG_RETENTION_DAYS,
            DEFAULT_ORPHAN_RETENTION_DAYS,
            created_at,
        ],
    )?;
    Ok(())
}

/// Sets how long local backups are kept.
pub fn set_backup_retention(
    transaction: &Connection,
    server: Id,
    days: i64,
    now: Timestamp,
) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE \"OperationsSettings\" SET \"BackupRetentionDays\" = ?2, \"UpdatedAt\" = ?3 WHERE \"ServerId\" = ?1",
        params![server, days, now],
    )?;
    Ok(())
}
