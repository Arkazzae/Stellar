//! Turns `@username`, and `@everyone` or `@here` from an author who may call on
//! a whole channel, into entries in the inbox of the members a message calls
//! on, so that a mention is more than styling. Which words name somebody is
//! read by `stellarr-mentions`; what is decided here is who is told.
//!
//! A group is called by a word the reader of mentions reads like any name, so
//! one in code, escaped or in a link's label calls nobody. Both languages'
//! words call the same group, whatever language the author writes in. Whether
//! a message calls a group is decided once, when it is posted, and stored
//! with it as [`EVERYONE`] and [`HERE`] bits: clients highlight and ring by
//! what the server stamped, never by the words alone, so that an author
//! without the permission cannot make every screen ring by typing them.

use std::collections::{BTreeSet, HashSet};

use rusqlite::{Connection, params};

use super::dto::one_line;
use crate::events::Presence;
use crate::inbox::{self, Entry, NewEntry};
use crate::realtime::presence::OFFLINE;
use crate::values::{Id, Timestamp};

#[cfg(test)]
mod tests;

/// The inbox entry of a mention is a notice of the system, under this package.
const PACKAGE_ID: &str = "stellarr.mention";

/// How much of the message an inbox entry quotes, in UTF-16 units.
const PREVIEW_LENGTH: usize = 220;

/// The bit of a message that calls on every member who sees its channel.
pub const EVERYONE: i64 = 1;
/// The bit of a message that calls on those of them who are online.
pub const HERE: i64 = 2;

/// The words that call on a group, upper-cased as the reader of mentions
/// gives names, each with the group it calls.
const GROUP_NAMES: [(&str, i64); 4] = [
    ("EVERYONE", EVERYONE),
    ("WSZYSCY", EVERYONE),
    ("HERE", HERE),
    ("OBECNI", HERE),
];

/// Whether an upper-cased name is one of the words that call on a group. Such
/// a name is nobody's: it cannot be registered, and an account that took it
/// before it was reserved is not called by it, since the group meaning wins.
pub fn is_group_name(normalized: &str) -> bool {
    GROUP_NAMES.iter().any(|(name, _)| *name == normalized)
}

/// The group an upper-cased mentioned name calls on, as its bit, or none. A
/// sentence often ends on the word, and the reader of mentions takes a full
/// stop as part of a name, so a trailing run of dots is left out here:
/// `WSZYSCY.` and `EVERYONE...` call their group, `WSZYSCY.X` calls nobody.
/// Only groups are read this way. A username may end with a dot, and the
/// name is still looked up with its dots, so an account called `wszyscy.` is
/// named by `@wszyscy.` as before.
fn group_of(name: &str) -> i64 {
    let word = name.trim_end_matches('.');
    GROUP_NAMES
        .iter()
        .find(|(group_name, _)| *group_name == word)
        .map_or(0, |(_, group)| *group)
}

/// The groups among the upper-cased names a message mentions, as bits.
pub fn groups_in(names: &BTreeSet<String>) -> i64 {
    names.iter().fold(0, |groups, name| groups | group_of(name))
}

/// What is left of `stored` once the message reads `content`: an edit may take
/// a group's word out, and the call goes with it, but it never adds one. A
/// call is decided when the message is posted, and an edit tells nobody.
pub fn groups_kept(stored: i64, content: &str) -> i64 {
    stored & groups_in(&stellarr_mentions::extract_normalized_usernames(content))
}

/// A message as the mentions in it are decided: who wrote it, where, when,
/// the names it mentions and the groups its author may call on in it.
pub struct Mentioning<'a> {
    pub server: Id,
    pub channel: Id,
    pub author: Id,
    pub author_name: &'a str,
    pub content: &'a str,
    pub created_at: Timestamp,
    /// The upper-cased names the content mentions, group words included.
    pub names: &'a BTreeSet<String>,
    /// The groups the message calls on, as stored with it: none when its
    /// author may not call on them there.
    pub groups: i64,
    /// Who is online, for [`HERE`].
    pub presence: &'a dyn Presence,
}

/// Who a message told: one inbox entry each, in the order of their ids.
#[derive(Debug, Default)]
pub struct Told {
    pub entries: Vec<Entry>,
    /// Those it named by their username, who may also be told by mail.
    pub named: Vec<Id>,
    /// Those it reached through a group alone, who are told by push and never
    /// by mail: a call to a whole channel is not worth a letter.
    pub grouped: Vec<Id>,
}

/// Writes an inbox entry for each member of the server the message calls on
/// who sees its channel, except its author. Nobody is told of a message in a
/// channel they do not see, so nobody named there who does not see it gets an
/// entry, an e-mail or a push. Somebody both named and in a group is told
/// once, as somebody named.
pub fn notify(db: &Connection, message: &Mentioning<'_>) -> rusqlite::Result<Told> {
    let named = named_viewers(db, message)?;
    let grouped: Vec<Id> = {
        let named: HashSet<Id> = named.iter().copied().collect();
        group_viewers(db, message)?
            .into_iter()
            .filter(|user| !named.contains(user))
            .collect()
    };
    let everybody: BTreeSet<Id> = named.iter().chain(&grouped).copied().collect();
    if everybody.is_empty() {
        return Ok(Told::default());
    }

    let body = one_line(message.content, PREVIEW_LENGTH);
    let title = format!("{} mentioned you", message.author_name);
    let mut entries = Vec::with_capacity(everybody.len());
    for user in everybody {
        entries.push(inbox::add(
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
    Ok(Told {
        entries,
        named,
        grouped,
    })
}

/// The members the message names by their username who see its channel, in
/// the order of their ids. A group's word names nobody.
fn named_viewers(db: &Connection, message: &Mentioning<'_>) -> rusqlite::Result<Vec<Id>> {
    let names: Vec<&String> = message
        .names
        .iter()
        .filter(|name| !is_group_name(name))
        .collect();
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
    crate::permissions::visibility::who_view(db, message.server, message.channel, &recipients)
}

/// The members the message's groups reach who see its channel. Accounts that
/// are switched off, as the stand-ins of an import are, read nothing and are
/// left out. Online is what the account itself would say of its status: it
/// has a connection open and has not chosen to appear offline. Nobody else
/// learns who was told, so an account that hides its status from others
/// still counts by the status it has.
fn group_viewers(db: &Connection, message: &Mentioning<'_>) -> rusqlite::Result<Vec<Id>> {
    if message.groups & (EVERYONE | HERE) == 0 {
        return Ok(Vec::new());
    }
    let members: Vec<Id> = {
        let mut statement = db.prepare_cached(
            "SELECT m.\"UserId\" FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
             WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" <> ?2 AND u.\"IsDisabled\" = 0
             ORDER BY m.\"UserId\"",
        )?;
        statement
            .query_map(params![message.server, message.author], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let members: Vec<Id> = if message.groups & EVERYONE == EVERYONE {
        members
    } else {
        members
            .into_iter()
            .filter(|user| message.presence.presence_of(*user) != OFFLINE)
            .collect()
    };
    crate::permissions::visibility::who_view(db, message.server, message.channel, &members)
}
