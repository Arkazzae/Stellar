//! Turns `@username` in a message into entries in the inbox of the members it
//! names, so that a mention is more than styling. Which words name somebody
//! is read by `stellarr-mentions`; what is decided here is who is told.

use rusqlite::{Connection, params};

use super::dto::one_line;
use crate::values::{Id, Timestamp};

/// The inbox entry of a mention is a notice of the system, under this package.
const PACKAGE_ID: &str = "stellarr.mention";

/// How much of the message an inbox entry quotes, in UTF-16 units.
const PREVIEW_LENGTH: usize = 220;

/// A message as the mentions in it are decided: who wrote it, where, when.
pub struct Mentioning<'a> {
    pub server: Id,
    pub channel: Id,
    pub author: Id,
    pub author_name: &'a str,
    pub content: &'a str,
    pub created_at: Timestamp,
}

/// An inbox entry that was written, which the person is told of once the
/// message is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notified {
    pub user: Id,
    pub server: Id,
    pub sequence: i64,
}

/// Writes an inbox entry for each member of the server the message names who
/// sees its channel, except its author. Nobody is told of a message in a
/// channel they do not see, so nobody named there who does not see it gets an
/// entry, an e-mail or a push. They come in the order of their ids.
pub fn notify(db: &Connection, message: &Mentioning<'_>) -> rusqlite::Result<Vec<Notified>> {
    let names = stellarr_mentions::extract_normalized_usernames(message.content);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let names = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
    let recipients: Vec<Id> = {
        // One parameter however many names the message has: it may have
        // thousands.
        let mut statement = db.prepare_cached(
            "SELECT DISTINCT m.\"UserId\" FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
             WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" <> ?2
               AND u.\"NormalizedUsername\" IN (SELECT value FROM json_each(?3))
             ORDER BY m.\"UserId\"",
        )?;
        statement
            .query_map(params![message.server, message.author, names], |row| {
                row.get(0)
            })?
            .collect::<rusqlite::Result<_>>()?
    };
    let viewers =
        crate::permissions::visibility::who_view(db, message.server, message.channel, &recipients)?;
    if viewers.is_empty() {
        return Ok(Vec::new());
    }

    let body = one_line(message.content, PREVIEW_LENGTH);
    let title = format!("{} mentioned you", message.author_name);
    let mut insert = db.prepare_cached(
        "INSERT INTO \"UserNotifications\" (\"Id\", \"ServerId\", \"UserId\", \"Source\", \"SourcePackageId\", \"SourcePackageName\",
             \"SourceModuleId\", \"SourceModuleName\", \"Title\", \"Body\", \"Tone\", \"TargetChannelId\", \"TargetInviteCode\",
             \"CreatedAt\", \"ReadAt\")
         VALUES (?1, ?2, ?3, 'system', ?4, 'Stellarr', 'mention', ?5, ?6, ?7, 'info', ?8, NULL, ?9, NULL)",
    )?;
    let mut written = Vec::with_capacity(viewers.len());
    for user in viewers {
        insert.execute(params![
            Id::random(),
            message.server,
            user,
            PACKAGE_ID,
            message.author_name,
            title,
            body,
            message.channel,
            message.created_at,
        ])?;
        written.push(Notified {
            user,
            server: message.server,
            sequence: db.last_insert_rowid(),
        });
    }
    Ok(written)
}
