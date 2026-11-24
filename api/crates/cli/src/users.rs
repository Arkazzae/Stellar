//! `users list` and `users show`: administrative metadata only, never a
//! password hash, a session or a token.

use rusqlite::OptionalExtension;
use stellarr_api::values::{Id, Timestamp};

use crate::errors::{CliOutcome, CliResult};
use crate::json::{Json, moment, round_trip};
use crate::runtime::Runtime;
use crate::support::{
    USER_COLUMNS, User, require_instance_server, resolve_user, user_from_row, yes_no,
};

const MAX_LISTED_USERS: usize = 200;

struct Item {
    user: User,
    role_name: Option<String>,
    joined_at: Option<Timestamp>,
    active_sessions: i64,
}

impl Item {
    fn json(&self) -> Json {
        Json::object([
            ("id", Json::text(self.user.id.to_string())),
            ("username", Json::text(&self.user.username)),
            ("displayName", Json::text(&self.user.display_name)),
            ("email", Json::optional_text(self.user.email.clone())),
            ("disabled", self.user.disabled.into()),
            ("createdAt", moment(self.user.created_at)),
            ("roleName", Json::optional_text(self.role_name.clone())),
            (
                "joinedAt",
                self.joined_at.map_or(Json::Null, |at| moment(at)),
            ),
            ("activeSessions", self.active_sessions.into()),
            ("origin", Json::optional_text(self.user.origin.clone())),
        ])
    }
}

fn membership(
    connection: &rusqlite::Connection,
    server: Id,
    user: Id,
) -> rusqlite::Result<(Option<String>, Option<Timestamp>)> {
    Ok(connection
        .query_row(
            "SELECT r.\"Name\", m.\"JoinedAt\" FROM \"ServerMembers\" m
             JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
             WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2",
            rusqlite::params![server, user],
            |row| Ok((Some(row.get(0)?), Some(row.get(1)?))),
        )
        .optional()?
        .unwrap_or((None, None)))
}

pub fn list(runtime: &mut Runtime) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let connection = &database.connection;
    let server = require_instance_server(connection)?;
    // The accounts that only stand in for the authors of imported messages
    // are nobody's account: they are counted apart and not listed.
    let total: i64 = connection.query_row(
        "SELECT COUNT(*) FROM \"Users\" WHERE \"Origin\" IS NULL",
        [],
        |row| row.get(0),
    )?;
    let placeholders: i64 = connection.query_row(
        "SELECT COUNT(*) FROM \"Users\" WHERE \"Origin\" IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(&format!(
        "SELECT {USER_COLUMNS} FROM \"Users\" WHERE \"Origin\" IS NULL
         ORDER BY \"DisplayName\", \"Username\" LIMIT {MAX_LISTED_USERS}"
    ))?;
    let users = statement
        .query_map([], user_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut items = Vec::new();
    for user in users {
        let (role_name, joined_at) = membership(connection, server.id, user.id)?;
        items.push(Item {
            user,
            role_name,
            joined_at,
            active_sessions: 0,
        });
    }
    let mut lines: Vec<String> = items
        .iter()
        .map(|item| {
            format!(
                "{}  {}  {}{}{}{}",
                item.user.id,
                item.user.username,
                item.user.display_name,
                item.user
                    .email
                    .as_ref()
                    .map_or(String::new(), |email| format!("  <{email}>")),
                if item.user.disabled {
                    "  [disabled]"
                } else {
                    ""
                },
                item.role_name
                    .as_ref()
                    .map_or(String::new(), |name| format!("  role={name}")),
            )
        })
        .collect();
    if total > items.len() as i64 {
        lines.push(format!(
            "Showing the first {} of {total} accounts.",
            items.len()
        ));
    }
    if placeholders > 0 {
        lines.push(format!(
            "{placeholders} placeholder account(s) of imported messages are not listed; `users show` finds one by its username."
        ));
    }
    let data = Json::object([
        ("total", total.into()),
        ("shown", items.len().into()),
        ("placeholders", placeholders.into()),
        ("users", Json::array(items.iter().map(Item::json))),
    ]);
    Ok(CliResult::new("users.list", data, lines))
}

pub fn show(runtime: &mut Runtime, id_or_username: &str) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let connection = &database.connection;
    let user = resolve_user(connection, id_or_username)?;
    let server = require_instance_server(connection)?;
    let (role_name, joined_at) = membership(connection, server.id, user.id)?;
    let active_sessions: i64 = connection.query_row(
        "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        rusqlite::params![user.id, Timestamp::now()],
        |row| row.get(0),
    )?;
    let item = Item {
        user,
        role_name,
        joined_at,
        active_sessions,
    };
    let lines = vec![
        format!("Id: {}", item.user.id),
        format!("Username: {}", item.user.username),
        format!("Display name: {}", item.user.display_name),
        format!("Email: {}", item.user.email.as_deref().unwrap_or("(none)")),
        format!("Disabled: {}", yes_no(item.user.disabled)),
        format!(
            "Origin: {}",
            item.user.origin.as_deref().unwrap_or("(a person)")
        ),
        format!("Created: {}", round_trip(item.user.created_at)),
        format!(
            "Role: {}",
            item.role_name
                .as_deref()
                .unwrap_or("(not a member of the instance server)")
        ),
        format!(
            "Joined instance server: {}",
            item.joined_at
                .map_or("(not a member)".to_string(), |at| round_trip(at))
        ),
        format!("Active sessions: {}", item.active_sessions),
    ];
    Ok(CliResult::new("users.show", item.json(), lines))
}
