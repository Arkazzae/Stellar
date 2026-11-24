//! What several commands ask of the database in the same way.

use rusqlite::{Connection, OptionalExtension};
use stellarr_api::instance::operations::{InstanceServer, instance_server};
use stellarr_api::values::{Id, Timestamp};

use crate::errors::{CliError, CliOutcome, exit};
use crate::invocation::trim;

/// The first server made, which is the instance's own.
pub fn require_instance_server(connection: &Connection) -> CliOutcome<InstanceServer> {
    if let Some(server) = instance_server(connection)? {
        return Ok(server);
    }
    // An instance is set up once it has an account. The API keeps its first
    // server, so only an edit of the database leaves accounts without one, and
    // such an instance cannot be set up again.
    let any_account: bool =
        connection.query_row("SELECT EXISTS (SELECT 1 FROM \"Users\")", [], |row| {
            row.get(0)
        })?;
    Err(if any_account {
        CliError::new(
            "instance_not_found",
            "The database has accounts but no server.",
            exit::NOT_FOUND,
        )
    } else {
        CliError::new(
            "instance_not_found",
            "Stellarr is not set up yet. Create the owner account in the web client with the setup code from the container's log.",
            exit::NOT_FOUND,
        )
    })
}

/// An account, as much of it as the commands show.
#[derive(Debug, Clone)]
pub struct User {
    pub id: Id,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub normalized_email: Option<String>,
    pub disabled: bool,
    pub created_at: Timestamp,
    pub origin: Option<String>,
    pub session_version: i64,
    pub security_recovery_required: bool,
}

pub const USER_COLUMNS: &str = "\"Id\", \"Username\", \"DisplayName\", \"Email\", \"NormalizedEmail\", \"IsDisabled\", \"CreatedAt\", \"Origin\", \"SessionVersion\", \"SecurityRecoveryRequired\"";

pub fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        email: row.get(3)?,
        normalized_email: row.get(4)?,
        disabled: row.get(5)?,
        created_at: row.get(6)?,
        origin: row.get(7)?,
        session_version: row.get(8)?,
        security_recovery_required: row.get(9)?,
    })
}

/// The account an id or a username names.
pub fn resolve_user(connection: &Connection, id_or_username: &str) -> CliOutcome<User> {
    let value = trim(id_or_username);
    if value.is_empty() {
        return Err(CliError::new(
            "invalid_user",
            "An account id or username is required.",
            exit::USAGE,
        ));
    }
    let found = if let Some(id) = Id::parse(&value) {
        connection
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM \"Users\" WHERE \"Id\" = ?1"),
                [id],
                user_from_row,
            )
            .optional()?
    } else {
        let normalized = stellarr_api::instance::names::to_upper(&value);
        connection
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM \"Users\" WHERE \"NormalizedUsername\" = ?1"),
                [normalized],
                user_from_row,
            )
            .optional()?
    };
    found.ok_or_else(|| user_not_found(&value))
}

pub fn user_not_found(value: &str) -> CliError {
    CliError::new(
        "user_not_found",
        format!("No account matches '{value}'."),
        exit::NOT_FOUND,
    )
}

pub fn session_not_found(value: &str) -> CliError {
    CliError::new(
        "session_not_found",
        format!("No active session matches '{value}'."),
        exit::NOT_FOUND,
    )
}

/// "yes" or "no", as a line of text says a flag.
pub fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
