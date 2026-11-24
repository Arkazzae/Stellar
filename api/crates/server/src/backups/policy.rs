//! The schedule of automatic backups the operator saved in the panel, and the
//! destination's settings: two small rows of `InstanceSecrets`, sealed, beside
//! the secrets themselves, so that a snapshot of the database and a backup of
//! the keys carry them without a table of their own.
//!
//! What is read is always what is stored: nothing is cached, so a change made
//! by one request is the next request's and the next hour's. A row that
//! cannot be opened (another master key, or written by the first server)
//! counts as not set, and the settings of the environment stand.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::seal::Sealer;
use crate::settings::BackupSettings;
use crate::values::Timestamp;

pub const BACKUP_POLICY_ROW: &str = "backups.policy";
pub const R2_POLICY_ROW: &str = "cloudflare.r2.policy";
const BACKUP_POLICY_PURPOSE: &str = "backup-policy";
const R2_POLICY_PURPOSE: &str = "cloudflare-r2-policy";

/// Where a setting in force comes from, as the panel shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Environment,
    Panel,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Environment => "environment",
            Source::Panel => "panel",
        }
    }
}

/// How often automatic backups run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub enabled: bool,
    pub interval_hours: i64,
    pub retry_minutes: i64,
    pub initial_delay_minutes: i64,
}

impl Schedule {
    pub fn of(settings: &BackupSettings) -> Schedule {
        Schedule {
            enabled: settings.enabled,
            interval_hours: settings.interval_hours,
            retry_minutes: settings.retry_minutes,
            initial_delay_minutes: settings.initial_delay_minutes,
        }
    }
}

/// The schedule in force and where it comes from.
pub fn current(
    connection: &Connection,
    sealer: &Sealer,
    defaults: &BackupSettings,
) -> rusqlite::Result<(Schedule, Source)> {
    Ok(
        match read::<Schedule>(connection, sealer, BACKUP_POLICY_ROW, BACKUP_POLICY_PURPOSE)? {
            Some(schedule) => (schedule, Source::Panel),
            None => (Schedule::of(defaults), Source::Environment),
        },
    )
}

pub fn save_schedule(
    connection: &Connection,
    sealer: &Sealer,
    schedule: &Schedule,
) -> rusqlite::Result<()> {
    write(connection, sealer, BACKUP_POLICY_ROW, BACKUP_POLICY_PURPOSE, schedule)
}

pub fn reset_schedule(connection: &Connection) -> rusqlite::Result<()> {
    remove(connection, BACKUP_POLICY_ROW)
}

/// The offsite destination without its credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub enabled: bool,
    pub mode: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
}

impl Destination {
    pub fn of(settings: &crate::settings::CloudflareR2Settings) -> Destination {
        Destination {
            enabled: settings.enabled,
            mode: settings.mode.clone(),
            endpoint: settings.endpoint.clone(),
            region: settings.region.clone(),
            bucket: settings.bucket.clone(),
            prefix: settings.prefix.clone(),
        }
    }

    /// Whether the destination could work, apart from its credentials; one
    /// that is off is valid.
    pub fn is_valid(&self) -> bool {
        crate::settings::CloudflareR2Settings {
            enabled: self.enabled,
            mode: self.mode.clone(),
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            bucket: self.bucket.clone(),
            prefix: self.prefix.clone(),
        }
        .is_valid()
    }
}

pub fn current_destination(
    connection: &Connection,
    sealer: &Sealer,
    defaults: &crate::settings::CloudflareR2Settings,
) -> rusqlite::Result<(Destination, Source)> {
    Ok(
        match read::<Destination>(connection, sealer, R2_POLICY_ROW, R2_POLICY_PURPOSE)? {
            Some(destination) => (destination, Source::Panel),
            None => (Destination::of(defaults), Source::Environment),
        },
    )
}

pub fn save_destination(
    connection: &Connection,
    sealer: &Sealer,
    destination: &Destination,
) -> rusqlite::Result<()> {
    write(connection, sealer, R2_POLICY_ROW, R2_POLICY_PURPOSE, destination)
}

pub fn reset_destination(connection: &Connection) -> rusqlite::Result<()> {
    remove(connection, R2_POLICY_ROW)
}

fn read<T: for<'de> Deserialize<'de>>(
    connection: &Connection,
    sealer: &Sealer,
    row: &str,
    purpose: &str,
) -> rusqlite::Result<Option<T>> {
    let Some(sealed) = connection
        .query_row(
            "SELECT \"ProtectedValue\" FROM \"InstanceSecrets\" WHERE \"Name\" = ?1",
            [row],
            |found| found.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let opened = sealer
        .open(purpose, row.as_bytes(), &sealed, Timestamp::now())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<T>(&bytes).ok());
    if opened.is_none() {
        warn!(
            target: "Stellarr.Api.Backups",
            "The saved setting {row} cannot be read with this instance's keys; the settings of the environment stand"
        );
    }
    Ok(opened)
}

fn write<T: Serialize>(
    connection: &Connection,
    sealer: &Sealer,
    row: &str,
    purpose: &str,
    value: &T,
) -> rusqlite::Result<()> {
    let payload = serde_json::to_vec(value).map_err(|error| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
    })?;
    let sealed = sealer.seal(purpose, row.as_bytes(), None, &payload);
    connection.execute(
        "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, ?2, ?3)
         ON CONFLICT (\"Name\") DO UPDATE SET \"ProtectedValue\" = excluded.\"ProtectedValue\", \"UpdatedAt\" = excluded.\"UpdatedAt\"",
        params![row, sealed, Timestamp::now()],
    )?;
    Ok(())
}

fn remove(connection: &Connection, row: &str) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM \"InstanceSecrets\" WHERE \"Name\" = ?1", [row])?;
    Ok(())
}
