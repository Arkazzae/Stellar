//! Read-only or read-write access to the one existing SQLite database.
//!
//! The tool never makes a database, never changes its journal mode and never
//! runs a schema step: a missing file is an error and a file that lacks what
//! the commands need is refused, so that it cannot turn an empty file into an
//! instance by accident.

use std::collections::HashSet;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, params};
use stellarr_api::values::{Id, Timestamp};

use crate::errors::{CliError, CliOutcome, exit};
use crate::json::Json;
use crate::runtime::Runtime;

/// How long a command waits for the server to finish writing before it fails
/// with `database_locked`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct CliDatabase {
    pub connection: Connection,
}

/// The tables and columns the commands read and write.
const REQUIRED: &[(&str, &[&str])] = &[
    (
        "Users",
        &[
            "Id",
            "Username",
            "NormalizedUsername",
            "DisplayName",
            "PasswordHash",
            "IsDisabled",
            "SessionVersion",
            "SecurityRecoveryRequired",
            "AvatarColor",
            "CreatedAt",
            "Origin",
        ],
    ),
    ("Servers", &["Id", "Name", "CreatedAt"]),
    ("ServerRoles", &["Id", "ServerId", "Name", "IsOwner"]),
    (
        "ServerMembers",
        &["ServerId", "UserId", "RoleId", "JoinedAt"],
    ),
    (
        "UserSessions",
        &[
            "Id",
            "UserId",
            "Device",
            "IpAddress",
            "CreatedAt",
            "LastSeenAt",
            "ExpiresAt",
            "RevokedAt",
            "AbsoluteExpiresAt",
            "PrimarySource",
            "MfaCompletedAt",
            "MfaEvidence",
            "SecurityVersion",
        ],
    ),
    (
        "DesktopBrowserLogins",
        &[
            "Id",
            "UserId",
            "CreatedAt",
            "ExpiresAt",
            "ApprovedAt",
            "RedeemedAt",
            "ApprovalTokenHash",
            "ApprovalUserId",
            "ApprovalSessionId",
            "ApprovedSessionId",
            "SecurityVersion",
        ],
    ),
    (
        "MfaMethods",
        &[
            "Id",
            "UserId",
            "Type",
            "Name",
            "CreatedAt",
            "ConfirmedAt",
            "RevokedAt",
        ],
    ),
    (
        "RecoveryCodes",
        &["Id", "UserId", "BatchId", "CodeHash", "CreatedAt", "UsedAt"],
    ),
    (
        "TrustedDevices",
        &[
            "Id",
            "UserId",
            "TokenHash",
            "Name",
            "CreatedAt",
            "ExpiresAt",
            "RevokedAt",
        ],
    ),
    (
        "CliRecoveryCredentials",
        &[
            "Id",
            "UserId",
            "TokenHash",
            "Reason",
            "SecurityVersion",
            "CreatedAt",
            "ExpiresAt",
            "UsedAt",
        ],
    ),
    (
        "SecurityEvents",
        &["Id", "UserId", "Kind", "Detail", "CreatedAt"],
    ),
    (
        "OperationsSettings",
        &[
            "ServerId",
            "TimeZone",
            "MailingEnabled",
            "MailFromAddress",
            "MailSmtpHost",
            "MailSmtpPort",
            "MailSmtpUsername",
            "MailSmtpSecurity",
            "MailResendEnabled",
            "GiphyEnabled",
            "InviteMailEnabled",
            "RecoveryMailEnabled",
            "AlertMailEnabled",
            "BackupRetentionDays",
            "LogRetentionDays",
            "OrphanObjectRetentionDays",
            "AutomaticCleanupEnabled",
            "MfaRequirement",
            "TrustedDeviceDays",
            "AuthChallengeMinutes",
            "EmailCodeMinutes",
            "EmailCodeCooldownSeconds",
            "EmailCodeMaxAttempts",
            "EmailCodeSendsPerHour",
            "CliRecoveryMinutes",
            "UpdatedAt",
        ],
    ),
    ("InstanceSecrets", &["Name", "ProtectedValue", "UpdatedAt"]),
];

const AUDIT_COLUMNS: &[&str] = &["Id", "Actor", "Operation", "Target", "CreatedAt"];

impl CliDatabase {
    pub fn open(runtime: &Runtime, mutation: bool) -> CliOutcome<CliDatabase> {
        // Read-only for inspection, read-write for a mutation. Never with
        // `create`: a missing file is an error, not a new empty installation.
        let flags = if mutation {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(runtime.database_path(), flags)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        // The server runs with foreign keys on, and so does a change made here.
        connection.pragma_update(None, "foreign_keys", true)?;
        let database = CliDatabase { connection };
        database.verify_schema(&runtime.database_path(), mutation)?;
        Ok(database)
    }

    fn tables(&self) -> rusqlite::Result<HashSet<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
        let tables = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        Ok(tables)
    }

    fn columns(&self, table: &str) -> rusqlite::Result<HashSet<String>> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        Ok(columns)
    }

    fn verify_schema(&self, path: &str, mutation: bool) -> CliOutcome<()> {
        let tables = self.tables()?;
        let mut missing: Vec<String> = REQUIRED
            .iter()
            .map(|(table, _)| *table)
            .filter(|table| !tables.contains(*table))
            .map(str::to_string)
            .collect();
        for (table, columns) in REQUIRED {
            if missing.iter().any(|name| name == table) {
                continue;
            }
            let existing = self.columns(table)?;
            missing.extend(
                columns
                    .iter()
                    .filter(|column| !existing.contains(**column))
                    .map(|column| format!("{table}.{column}")),
            );
        }
        if mutation && !tables.contains("OperatorAuditEntries") {
            missing.push("OperatorAuditEntries".to_string());
        } else if mutation {
            let existing = self.columns("OperatorAuditEntries")?;
            missing.extend(
                AUDIT_COLUMNS
                    .iter()
                    .filter(|column| !existing.contains(**column))
                    .map(|column| format!("OperatorAuditEntries.{column}")),
            );
        }
        if missing.is_empty() {
            return Ok(());
        }
        Err(CliError::new(
            "schema_incompatible",
            "This database is missing tables or columns this release needs, so it was not created by this release of Stellarr. The CLI never migrates or initializes a database.",
            exit::CONFLICT,
        )
        .with_details(Json::object([
            ("database", Json::text(path)),
            ("missing", Json::array(missing.into_iter().map(Json::from))),
        ])))
    }

    /// The operator's own record of a change, written in the transaction of
    /// the change.
    pub fn audit(
        transaction: &rusqlite::Connection,
        operation: &str,
        target: &str,
    ) -> rusqlite::Result<()> {
        transaction.execute(
            "INSERT INTO \"OperatorAuditEntries\" (\"Id\", \"Actor\", \"Operation\", \"Target\", \"CreatedAt\")
             VALUES (?1, 'local-cli', ?2, ?3, ?4)",
            params![Id::random(), operation, target, Timestamp::now()],
        )?;
        Ok(())
    }
}

impl From<rusqlite::Error> for CliError {
    fn from(error: rusqlite::Error) -> CliError {
        let mut mapped = map_sqlite(&error);
        if let rusqlite::Error::SqliteFailure(failure, text) = &error {
            // As the first tool's driver writes what SQLite said.
            let said = text.clone().unwrap_or_default();
            // The driver here adds the path to a refusal to open; the first
            // tool's driver did not.
            let said = said
                .strip_prefix("unable to open database file")
                .map_or(said.as_str(), |_| "unable to open database file");
            mapped.cause = Some(format!(
                "SQLite Error {}: '{said}'.",
                failure.extended_code & 0xff
            ));
        }
        mapped
    }
}

fn map_sqlite(error: &rusqlite::Error) -> CliError {
    let message = error.to_string();
    if message.to_lowercase().contains("no such table")
        || message.to_lowercase().contains("no such column")
    {
        return CliError::new(
            "schema_incompatible",
            "The database lacks a table or column this CLI needs, so it was not created by this release of Stellarr.",
            exit::CONFLICT,
        );
    }
    let code = match error {
        rusqlite::Error::SqliteFailure(failure, _) => Some(failure.code),
        _ => None,
    };
    match code {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => CliError::new(
            "database_locked",
            "The database is busy or locked by another process. Try again when the API is less busy.",
            exit::CONFLICT,
        ),
        Some(ErrorCode::CannotOpen) => CliError::new(
            "database_unavailable",
            "The database file cannot be opened. Check the data volume and permissions.",
            exit::UNAVAILABLE,
        ),
        Some(ErrorCode::NotADatabase) => CliError::new(
            "database_unreadable",
            "The file is not a readable SQLite database.",
            exit::CONFLICT,
        ),
        _ => CliError::new(
            "database_error",
            "The database operation failed. Nothing is reported as changed.",
            exit::FAILURE,
        ),
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> CliError {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            return CliError::new(
                "permission_denied",
                "The data volume or database is not readable by this process.",
                exit::CONFLICT,
            );
        }
        CliError::unexpected("IOException")
    }
}
