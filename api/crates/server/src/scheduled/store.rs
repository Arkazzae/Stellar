//! The rows of scheduled messages, and what they look like as JSON.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::Failure;
use crate::messages::dto::{ATTACHMENT_COLUMNS, AttachmentDto, attachment_from};
use crate::values::{Id, Timestamp, json_ids};

/// Where a scheduled message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A text channel, and the server it was on when the message was made.
    Channel { server: Id, channel: Id },
    Conversation(Id),
}

/// A scheduled message as stored, its attachments with it.
#[derive(Debug, Clone)]
pub struct Scheduled {
    pub id: Id,
    pub author: Id,
    pub target: Target,
    pub content: String,
    pub reply_to: Option<Id>,
    pub send_at: Timestamp,
    pub created_at: Timestamp,
    pub revision: i64,
    pub failure: Option<(Failure, Timestamp)>,
    pub attachments: Vec<Id>,
}

/// The claim a send holds on its row: it posts only while the row is still
/// there as it was read, and the row goes in the transaction that posts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
    pub id: Id,
    pub revision: i64,
}

impl Claim {
    /// Removes the row as it was read. False when it was edited or removed
    /// since, and then nothing may be posted for it.
    pub fn take(self, db: &Connection) -> rusqlite::Result<bool> {
        Ok(db.execute(
            "DELETE FROM \"ScheduledMessages\" WHERE \"Id\" = ?1 AND \"Revision\" = ?2",
            params![self.id, self.revision],
        )? > 0)
    }

    /// Removes the row of a message that is already posted, whatever became
    /// of the row since: the message is out, and a row left behind would
    /// only be sent again.
    pub fn settle(self, db: &Connection) -> rusqlite::Result<()> {
        db.execute("DELETE FROM \"ScheduledMessages\" WHERE \"Id\" = ?1", [self.id])?;
        Ok(())
    }
}

impl Scheduled {
    pub fn claim(&self) -> Claim {
        Claim {
            id: self.id,
            revision: self.revision,
        }
    }
}

const COLUMNS: &str = "\"Id\", \"AuthorId\", \"ServerId\", \"ChannelId\", \"ConversationId\", \"Content\",
    \"ReplyToMessageId\", \"SendAt\", \"CreatedAt\", \"Revision\", \"FailureCode\", \"FailedAt\"";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Scheduled> {
    let server: Option<Id> = row.get(2)?;
    let channel: Option<Id> = row.get(3)?;
    let conversation: Option<Id> = row.get(4)?;
    // The table's check keeps exactly one target, and a server with a channel.
    let target = match (server, channel, conversation) {
        (Some(server), Some(channel), _) => Target::Channel { server, channel },
        (_, _, Some(conversation)) => Target::Conversation(conversation),
        _ => {
            return Err(rusqlite::Error::InvalidColumnType(
                3,
                "ChannelId".to_string(),
                rusqlite::types::Type::Null,
            ));
        }
    };
    let code: Option<String> = row.get(10)?;
    let failed_at: Option<Timestamp> = row.get(11)?;
    Ok(Scheduled {
        id: row.get(0)?,
        author: row.get(1)?,
        target,
        content: row.get(5)?,
        reply_to: row.get(6)?,
        send_at: row.get(7)?,
        created_at: row.get(8)?,
        revision: row.get(9)?,
        failure: failed_at.map(|at| (Failure::parse(code.as_deref().unwrap_or("")), at)),
        attachments: Vec::new(),
    })
}

/// The rows a condition picks, with their attachments, in the order of the
/// time they are for.
fn select(
    db: &Connection,
    condition: &str,
    values: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<Scheduled>> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM \"ScheduledMessages\" WHERE {condition} ORDER BY \"SendAt\", \"Id\""
    ))?;
    let mut found: Vec<Scheduled> = statement
        .query_map(values, from_row)?
        .collect::<rusqlite::Result<_>>()?;
    if found.is_empty() {
        return Ok(found);
    }
    let ids: Vec<Id> = found.iter().map(|row| row.id).collect();
    let mut attached: HashMap<Id, Vec<Id>> = HashMap::new();
    let mut statement = db.prepare_cached(
        "SELECT \"ScheduledMessageId\", \"AttachmentId\" FROM \"ScheduledMessageAttachments\"
         WHERE \"ScheduledMessageId\" IN (SELECT value FROM json_each(?1))
         ORDER BY \"ScheduledMessageId\", \"AttachmentId\"",
    )?;
    for row in statement.query_map([json_ids(&ids)], |row| {
        Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?))
    })? {
        let (message, attachment) = row?;
        attached.entry(message).or_default().push(attachment);
    }
    for row in &mut found {
        row.attachments = attached.remove(&row.id).unwrap_or_default();
    }
    Ok(found)
}

/// One of the author's own; somebody else's is one that does not exist.
pub fn find(db: &Connection, author: Id, id: Id) -> rusqlite::Result<Option<Scheduled>> {
    Ok(select(db, "\"Id\" = ?1 AND \"AuthorId\" = ?2", &[&id, &author])?
        .into_iter()
        .next())
}

/// What a person may narrow their list to.
#[derive(Debug, Clone, Copy)]
pub enum Filter {
    All,
    Channel(Id),
    Conversation(Id),
}

/// Everything the author has scheduled, pending and failed, the soonest first.
pub fn of_author(db: &Connection, author: Id, filter: Filter) -> rusqlite::Result<Vec<Scheduled>> {
    match filter {
        Filter::All => select(db, "\"AuthorId\" = ?1", &[&author]),
        Filter::Channel(channel) => select(
            db,
            "\"AuthorId\" = ?1 AND \"ChannelId\" = ?2",
            &[&author, &channel],
        ),
        Filter::Conversation(conversation) => select(
            db,
            "\"AuthorId\" = ?1 AND \"ConversationId\" = ?2",
            &[&author, &conversation],
        ),
    }
}

/// The pending messages whose turn has come by `now`, the longest waiting
/// first, at most `limit` of them.
pub fn due(db: &Connection, now: Timestamp, limit: i64) -> rusqlite::Result<Vec<Scheduled>> {
    let ids: Vec<Id> = {
        let mut statement = db.prepare_cached(
            "SELECT \"Id\" FROM \"ScheduledMessages\" WHERE \"FailedAt\" IS NULL AND \"DueAt\" <= ?1
             ORDER BY \"DueAt\", \"Id\" LIMIT ?2",
        )?;
        statement
            .query_map(params![now, limit], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    select(
        db,
        "\"Id\" IN (SELECT value FROM json_each(?1))",
        &[&json_ids(&ids)],
    )
}

/// When the next pending message falls due, if one waits.
pub fn next_due(db: &Connection) -> rusqlite::Result<Option<Timestamp>> {
    db.query_row(
        "SELECT MIN(\"DueAt\") FROM \"ScheduledMessages\" WHERE \"FailedAt\" IS NULL",
        [],
        |row| row.get(0),
    )
}

/// How many of the author's messages still wait to be sent.
pub fn pending_of(db: &Connection, author: Id) -> rusqlite::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM \"ScheduledMessages\" WHERE \"AuthorId\" = ?1 AND \"FailedAt\" IS NULL",
        [author],
        |row| row.get(0),
    )
}

/// Writes a new scheduled message, due at the time it is for.
pub fn insert(db: &Connection, message: &Scheduled) -> rusqlite::Result<()> {
    let (server, channel, conversation) = match message.target {
        Target::Channel { server, channel } => (Some(server), Some(channel), None),
        Target::Conversation(conversation) => (None, None, Some(conversation)),
    };
    db.execute(
        "INSERT INTO \"ScheduledMessages\" (\"Id\", \"AuthorId\", \"ServerId\", \"ChannelId\", \"ConversationId\", \"Content\",
             \"ReplyToMessageId\", \"SendAt\", \"DueAt\", \"CreatedAt\", \"Revision\", \"FailureCode\", \"FailedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, 0, NULL, NULL)",
        params![
            message.id,
            message.author,
            server,
            channel,
            conversation,
            message.content,
            message.reply_to,
            message.send_at,
            message.created_at,
        ],
    )?;
    for attachment in &message.attachments {
        db.execute(
            "INSERT INTO \"ScheduledMessageAttachments\" (\"ScheduledMessageId\", \"AttachmentId\") VALUES (?1, ?2)",
            params![message.id, attachment],
        )?;
    }
    Ok(())
}

/// Rewrites the text and the time of a pending message, and counts the edit
/// so that a send that read it before cannot post the old one. A new time is
/// also when it next falls due.
pub fn edit(db: &Connection, message: &Scheduled) -> rusqlite::Result<bool> {
    Ok(db.execute(
        "UPDATE \"ScheduledMessages\" SET \"Content\" = ?2, \"SendAt\" = ?3, \"DueAt\" = ?3, \"Revision\" = \"Revision\" + 1
         WHERE \"Id\" = ?1 AND \"FailedAt\" IS NULL",
        params![message.id, message.content, message.send_at],
    )? > 0)
}

/// Removes one, its attachment links with it.
pub fn remove(db: &Connection, id: Id) -> rusqlite::Result<()> {
    db.execute("DELETE FROM \"ScheduledMessages\" WHERE \"Id\" = ?1", [id])?;
    Ok(())
}

/// Marks a pending message failed, unless it changed since it was read.
pub fn fail(db: &Connection, claim: Claim, failure: Failure, at: Timestamp) -> rusqlite::Result<bool> {
    Ok(db.execute(
        "UPDATE \"ScheduledMessages\" SET \"FailureCode\" = ?3, \"FailedAt\" = ?4
         WHERE \"Id\" = ?1 AND \"Revision\" = ?2 AND \"FailedAt\" IS NULL",
        params![claim.id, claim.revision, failure.code(), at],
    )? > 0)
}

/// Moves the next try of a pending message to `due_at`, unless it changed
/// since it was read. The time the author chose stays as it is.
pub fn postpone(db: &Connection, claim: Claim, due_at: Timestamp) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE \"ScheduledMessages\" SET \"DueAt\" = ?3
         WHERE \"Id\" = ?1 AND \"Revision\" = ?2 AND \"FailedAt\" IS NULL",
        params![claim.id, claim.revision, due_at],
    )?;
    Ok(())
}

/// A scheduled message as its author's clients read it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledMessageDto {
    pub id: Id,
    pub target: TargetDto,
    pub content: String,
    pub attachments: Vec<AttachmentDto>,
    pub reply_to_message_id: Option<Id>,
    pub send_at: Timestamp,
    pub created_at: Timestamp,
    /// `pending` or `failed`.
    pub state: &'static str,
    pub failure: Option<FailureDto>,
}

/// Where a message goes. A channel carries the names it and its server have
/// now, since the author's list spans servers their client has not loaded;
/// a name is null once its server or channel is gone. The author chose the
/// channel, so its name is theirs to see even after they lose access to it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TargetDto {
    Channel {
        server_id: Id,
        server_name: Option<String>,
        channel_id: Id,
        channel_name: Option<String>,
    },
    Conversation {
        conversation_id: Id,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureDto {
    pub code: &'static str,
    pub at: Timestamp,
}

/// The current names of these servers or channels, of those that are there.
fn names(db: &Connection, table: &str, ids: &[Id]) -> rusqlite::Result<HashMap<Id, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut statement = db.prepare_cached(&format!(
        "SELECT \"Id\", \"Name\" FROM \"{table}\" WHERE \"Id\" IN (SELECT value FROM json_each(?1))"
    ))?;
    statement
        .query_map([json_ids(ids)], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// The JSON of these messages, in their order, with their attachments and
/// the names of their places read in a statement each.
pub fn dtos(db: &Connection, rows: Vec<Scheduled>) -> rusqlite::Result<Vec<ScheduledMessageDto>> {
    let wanted: Vec<Id> = rows.iter().flat_map(|row| row.attachments.iter().copied()).collect();
    let mut files: HashMap<Id, AttachmentDto> = HashMap::new();
    if !wanted.is_empty() {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {ATTACHMENT_COLUMNS} FROM \"ImageAttachments\" a WHERE a.\"Id\" IN (SELECT value FROM json_each(?1))"
        ))?;
        for file in statement.query_map([json_ids(&wanted)], |row| attachment_from(row, 0))? {
            let file = file?;
            files.insert(file.id, file);
        }
    }
    let (servers, channels): (Vec<Id>, Vec<Id>) = rows
        .iter()
        .filter_map(|row| match row.target {
            Target::Channel { server, channel } => Some((server, channel)),
            Target::Conversation(_) => None,
        })
        .unzip();
    let server_names = names(db, "Servers", &servers)?;
    let channel_names = names(db, "Channels", &channels)?;
    Ok(rows
        .into_iter()
        .map(|row| ScheduledMessageDto {
            id: row.id,
            target: match row.target {
                Target::Channel { server, channel } => TargetDto::Channel {
                    server_id: server,
                    server_name: server_names.get(&server).cloned(),
                    channel_id: channel,
                    channel_name: channel_names.get(&channel).cloned(),
                },
                Target::Conversation(conversation) => TargetDto::Conversation {
                    conversation_id: conversation,
                },
            },
            content: row.content,
            attachments: row
                .attachments
                .iter()
                .filter_map(|id| files.get(id).cloned())
                .collect(),
            reply_to_message_id: row.reply_to,
            send_at: row.send_at,
            created_at: row.created_at,
            state: if row.failure.is_some() { "failed" } else { "pending" },
            failure: row.failure.map(|(failure, at)| FailureDto {
                code: failure.code(),
                at,
            }),
        })
        .collect())
}

/// One message as JSON, read again from the database.
pub fn dto(db: &Connection, author: Id, id: Id) -> rusqlite::Result<Option<ScheduledMessageDto>> {
    let Some(row) = find(db, author, id)? else {
        return Ok(None);
    };
    Ok(dtos(db, vec![row])?.pop())
}

/// Whether the author may still speak: an account that is disabled or that
/// the operator put under security recovery sends nothing on a timer. A
/// request checks this through its session; the worker has none.
pub fn author_may_post(db: &Connection, author: Id) -> rusqlite::Result<bool> {
    Ok(db
        .query_row(
            "SELECT NOT \"IsDisabled\" AND NOT \"SecurityRecoveryRequired\" FROM \"Users\" WHERE \"Id\" = ?1",
            [author],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// The server a channel is on now, none when it is gone.
pub fn server_of(db: &Connection, channel: Id) -> rusqlite::Result<Option<Id>> {
    db.query_row(
        "SELECT \"ServerId\" FROM \"Channels\" WHERE \"Id\" = ?1",
        [channel],
        |row| row.get(0),
    )
    .optional()
}
