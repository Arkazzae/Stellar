//! `sessions list`, `sessions revoke <id>` and `sessions revoke --user <who>
//! --all`: the durable session rows, which the server's watcher of sessions
//! turns into closed connections within a second.

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use stellarr_api::values::{Id, Timestamp};

use crate::database::CliDatabase;
use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::invocation::trim;
use crate::json::{Json, moment, round_trip};
use crate::runtime::Runtime;
use crate::support::{resolve_user, session_not_found};

const MAX_LISTED_SESSIONS: usize = 500;

struct Item {
    id: Id,
    user_id: Id,
    username: String,
    display_name: String,
    device: String,
    ip_address: String,
    created_at: Timestamp,
    last_seen_at: Timestamp,
    expires_at: Timestamp,
}

impl Item {
    fn json(&self) -> Json {
        Json::object([
            ("id", Json::text(self.id.to_string())),
            ("userId", Json::text(self.user_id.to_string())),
            ("username", Json::text(&self.username)),
            ("userDisplayName", Json::text(&self.display_name)),
            ("device", Json::text(&self.device)),
            ("ipAddress", Json::text(&self.ip_address)),
            ("createdAt", moment(self.created_at)),
            ("lastSeenAt", moment(self.last_seen_at)),
            ("expiresAt", moment(self.expires_at)),
        ])
    }
}

pub fn list(runtime: &mut Runtime, id_or_username: Option<&str>) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let connection = &database.connection;
    let user = match id_or_username {
        Some(who) => Some(resolve_user(connection, who)?.id),
        None => None,
    };
    let now = Timestamp::now();
    let total: i64 = connection.query_row(
        "SELECT COUNT(*) FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
         WHERE s.\"RevokedAt\" IS NULL AND s.\"ExpiresAt\" > ?1 AND (?2 IS NULL OR s.\"UserId\" = ?2)",
        params![now, user],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(&format!(
        "SELECT s.\"Id\", s.\"UserId\", u.\"Username\", u.\"DisplayName\", s.\"Device\", s.\"IpAddress\",
                s.\"CreatedAt\", s.\"LastSeenAt\", s.\"ExpiresAt\"
         FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
         WHERE s.\"RevokedAt\" IS NULL AND s.\"ExpiresAt\" > ?1 AND (?2 IS NULL OR s.\"UserId\" = ?2)
         ORDER BY s.\"LastSeenAt\" DESC LIMIT {MAX_LISTED_SESSIONS}"
    ))?;
    let items = statement
        .query_map(params![now, user], |row| {
            Ok(Item {
                id: row.get(0)?,
                user_id: row.get(1)?,
                username: row.get(2)?,
                display_name: row.get(3)?,
                device: row.get(4)?,
                ip_address: row.get(5)?,
                created_at: row.get(6)?,
                last_seen_at: row.get(7)?,
                expires_at: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut lines: Vec<String> = items
        .iter()
        .map(|item| {
            format!(
                "{}  {} ({})  {}  last seen {}  expires {}  from {}",
                item.id,
                item.username,
                item.display_name,
                item.device,
                round_trip(item.last_seen_at),
                round_trip(item.expires_at),
                item.ip_address
            )
        })
        .collect();
    if total > items.len() as i64 {
        lines.push(format!(
            "Showing the first {} of {total} active sessions.",
            items.len()
        ));
    }
    let data = Json::object([
        ("total", total.into()),
        ("shown", items.len().into()),
        ("sessions", Json::array(items.iter().map(Item::json))),
    ]);
    Ok(CliResult::new("sessions.list", data, lines))
}

pub fn revoke(runtime: &mut Runtime, id: &str) -> CliOutcome {
    let Some(session_id) = Id::parse_guid(&trim(id)) else {
        return Err(CliError::new(
            "invalid_session_id",
            format!("'{id}' is not a session id."),
            exit::USAGE,
        ));
    };
    let mut database = runtime.open_database(true)?;
    let found: Option<(Id, String, String, String)> = database
        .connection
        .query_row(
            "SELECT u.\"Id\", u.\"Username\", u.\"DisplayName\", s.\"Device\"
             FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
             WHERE s.\"Id\" = ?1 AND s.\"RevokedAt\" IS NULL",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((user_id, username, display_name, device)) = found else {
        return Err(session_not_found(id));
    };
    runtime.confirm(&format!(
        "Will revoke session {session_id} of {display_name} ({username}) on '{device}'."
    ))?;
    revoke_row(&mut database, session_id, id)?;
    let data = Json::object([
        ("sessionId", Json::text(session_id.to_string())),
        ("userId", Json::text(user_id.to_string())),
        ("username", Json::text(&username)),
        ("revoked", true.into()),
        ("realtimeDisconnectPending", true.into()),
    ]);
    Ok(CliResult::new(
        "sessions.revoke",
        data,
        vec![
            format!("Revoked session {session_id} of {display_name} ({username})."),
            "The API closes a matching WebSocket connection within one second.".to_string(),
            format!("Audit: local-cli sessions.revoke {session_id}"),
        ],
    )
    .changed())
}

/// Ends the session, unless someone got there while the question waited: then
/// nothing is written, no audit row either, and the session counts as not found,
/// as it did when the command began.
fn revoke_row(database: &mut CliDatabase, session: Id, typed: &str) -> CliOutcome<()> {
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = stellarr_api::auth::session::revoke(&transaction, session, Timestamp::now())
        .map_err(|_| CliError::unexpected("DbError"))?;
    if !changed {
        return Err(session_not_found(typed));
    }
    CliDatabase::audit(&transaction, "sessions.revoke", &session.to_string())?;
    transaction.commit()?;
    Ok(())
}

pub fn revoke_all(runtime: &mut Runtime, id_or_username: &str) -> CliOutcome {
    let mut database = runtime.open_database(true)?;
    let user = resolve_user(&database.connection, id_or_username)?;
    let now = Timestamp::now();
    let active: i64 = database.connection.query_row(
        "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![user.id, now],
        |row| row.get(0),
    )?;
    let handoffs: i64 = database.connection.query_row(
        "SELECT COUNT(*) FROM \"DesktopBrowserLogins\" WHERE \"UserId\" = ?1 AND \"ApprovedAt\" IS NOT NULL AND \"RedeemedAt\" IS NULL",
        [user.id],
        |row| row.get(0),
    )?;
    let result = |revoked: i64, invalidated: i64| {
        Json::object([
            ("userId", Json::text(user.id.to_string())),
            ("username", Json::text(&user.username)),
            ("sessionsRevoked", revoked.into()),
            ("handoffsInvalidated", invalidated.into()),
            ("realtimeDisconnectPending", true.into()),
        ])
    };
    if active == 0 && handoffs == 0 {
        return Ok(CliResult::new(
            "sessions.revoke-all",
            result(0, 0),
            vec![format!(
                "No active sessions or approved pending handoffs for {} ({}).",
                user.display_name, user.username
            )],
        ));
    }
    runtime.confirm(&format!(
        "Will revoke {active} active session(s) of {} ({}) and invalidate {handoffs} approved desktop browser handoff(s).",
        user.display_name, user.username
    ))?;
    let transaction = database
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let revoked = transaction.execute(
        "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![user.id, now],
    )? as i64;
    let invalidated = transaction.execute(
        "UPDATE \"DesktopBrowserLogins\" SET \"RedeemedAt\" = ?2 WHERE \"UserId\" = ?1 AND \"ApprovedAt\" IS NOT NULL AND \"RedeemedAt\" IS NULL",
        params![user.id, now],
    )? as i64;
    CliDatabase::audit(&transaction, "sessions.revoke-all", &user.id.to_string())?;
    transaction.commit()?;
    Ok(CliResult::new(
        "sessions.revoke-all",
        result(revoked, invalidated),
        vec![
            format!(
                "Revoked {revoked} active session(s) of {} ({}).",
                user.display_name, user.username
            ),
            format!(
                "Invalidated {invalidated} approved desktop browser handoff(s) not yet redeemed."
            ),
            "The API closes matching WebSocket connections within one second.".to_string(),
            format!("Audit: local-cli sessions.revoke-all {}", user.id),
        ],
    )
    .changed())
}
