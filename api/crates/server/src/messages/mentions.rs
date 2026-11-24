//! Turns `@username` in a message into entries in the inbox of the members it
//! names, so that a mention is more than styling. Which words name somebody
//! is read by `stellarr-mentions`; what is decided here is who is told.

use rusqlite::{Connection, params};

use super::dto::one_line;
use crate::inbox::{self, Entry, NewEntry};
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

/// Writes an inbox entry for each member of the server the message names who
/// sees its channel, except its author. Nobody is told of a message in a
/// channel they do not see, so nobody named there who does not see it gets an
/// entry, an e-mail or a push. They come in the order of their ids.
pub fn notify(db: &Connection, message: &Mentioning<'_>) -> rusqlite::Result<Vec<Entry>> {
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
    let mut written = Vec::with_capacity(viewers.len());
    for user in viewers {
        written.push(inbox::add(
            db,
            &NewEntry {
                server: message.server,
                user,
                source: "system".to_string(),
                source_package_id: PACKAGE_ID.to_string(),
                source_package_name: "Stellarr".to_string(),
                source_module_id: "mention".to_string(),
                source_module_name: message.author_name.to_string(),
                title: title.clone(),
                body: body.clone(),
                tone: "info".to_string(),
                target_channel: Some(message.channel),
                target_invite_code: None,
                created_at: message.created_at,
            },
        )?);
    }
    Ok(written)
}
