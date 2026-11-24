//! The row of retention and clean-up settings the panel keeps for the
//! instance server, which the backups and the housekeeping both read.
//!
//! Before the operator has saved anything there is no row, and the defaults of
//! the installation stand in for it; the first change writes one.

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::instance::operations as instance;
use crate::settings::Config;

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

/// The configuration the defaults of the row come from, read as the
/// settings were at start. The state keeps only what it parsed, so this reads
/// the process's environment again; the operator's tool does the same.
fn configuration(settings: &Settings) -> Config {
    let variables: Vec<(String, String)> = std::env::vars().collect();
    Config::layered(settings.environment.name(), &variables)
}

/// The row of `server` as the one loader of it reads it (see
/// [`crate::instance::operations`]), or what the installation starts with.
pub fn load(
    transaction: &Transaction,
    settings: &Settings,
    server: Id,
) -> rusqlite::Result<(instance::Loaded, String)> {
    let (name, created): (String, Timestamp) = transaction.query_row(
        "SELECT \"Name\", \"CreatedAt\" FROM \"Servers\" WHERE \"Id\" = ?1",
        [server],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let loaded = instance::load(transaction, server, created, &configuration(settings))?;
    Ok((loaded, name))
}

/// Writes the row with the installation's defaults when the server has none,
/// as the first change of any setting did.
/// Set up a stored operations row for housekeeping tests.
#[cfg(test)]
pub fn ensure(transaction: &Transaction, settings: &Settings, server: Id) -> rusqlite::Result<()> {
    let (loaded, name) = load(transaction, settings, server)?;
    if !loaded.stored {
        instance::save(transaction, &loaded.settings, false, &name)?;
    }
    Ok(())
}

/// Sets how long local backups are kept, writing the row first if there is
/// none.
pub fn set_backup_retention(
    transaction: &Transaction,
    settings: &Settings,
    server: Id,
    days: i64,
    now: Timestamp,
) -> rusqlite::Result<()> {
    let (mut loaded, name) = load(transaction, settings, server)?;
    loaded.settings.backup_retention_days = days;
    loaded.settings.updated_at = now;
    instance::save(transaction, &loaded.settings, loaded.stored, &name)
}
