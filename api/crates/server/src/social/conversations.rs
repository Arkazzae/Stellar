//! What the friends screen shows of a person's direct conversations: the
//! peer, the newest message, how many the person has not read and how the
//! conversation stands with encryption. Opening a conversation and posting in
//! it are another group's; the reading is here because the state needs it.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::encoding;
use crate::instance::users::{User, UserDto};
use crate::values::{Id, Timestamp};

/// A conversation as stored.
pub struct Conversation {
    pub id: Id,
    pub user_a: Id,
    pub user_b: Id,
    pub created_at: Timestamp,
    pub encryption_state: String,
    pub encryption_requested_by: Option<Id>,
    pub encryption_requested_at: Option<Timestamp>,
    pub encryption_enabled_at: Option<Timestamp>,
    pub current_epoch: Option<Id>,
}

impl Conversation {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Conversation> {
        Ok(Conversation {
            id: row.get(0)?,
            user_a: row.get(1)?,
            user_b: row.get(2)?,
            created_at: row.get(3)?,
            encryption_state: row.get(4)?,
            encryption_requested_by: row.get(5)?,
            encryption_requested_at: row.get(6)?,
            encryption_enabled_at: row.get(7)?,
            current_epoch: row.get(8)?,
        })
    }

    /// The other person of the conversation, seen from `me`.
    pub fn peer(&self, me: Id) -> Id {
        super::other(self.user_a, self.user_b, me)
    }

    /// Where the conversation stands with encryption.
    pub fn encryption(&self) -> Encryption {
        Encryption {
            state: match self.encryption_state.as_str() {
                "Requested" => "requested",
                "On" => "on",
                _ => "off",
            },
            requested_by_id: self.encryption_requested_by.map(|id| id.to_string()),
            requested_at: self.encryption_requested_at.map(Timestamp::to_wire_text),
            enabled_at: self.encryption_enabled_at.map(Timestamp::to_wire_text),
            current_epoch_id: self.current_epoch.map(|id| id.to_string()),
        }
    }
}

/// The conversations a person is in, in the order the rows come.
pub fn of_person(connection: &Connection, me: Id) -> rusqlite::Result<Vec<Conversation>> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\", \"EncryptionRequestedById\",
                \"EncryptionRequestedAt\", \"EncryptionEnabledAt\", \"CurrentEpochId\"
         FROM \"DirectConversations\" WHERE \"UserAId\" = ?1 OR \"UserBId\" = ?1",
    )?;
    statement
        .query_map([me], Conversation::from_row)?
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Encryption {
    state: &'static str,
    requested_by_id: Option<String>,
    requested_at: Option<String>,
    enabled_at: Option<String>,
    current_epoch_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    id: String,
    name: String,
    content_type: String,
    size: i64,
    width: Option<i64>,
    height: Option<i64>,
    url: String,
    duration_seconds: Option<f64>,
    poster_url: Option<String>,
    is_voice_message: bool,
    preview_url: Option<String>,
    placeholder: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedPayload {
    epoch_id: String,
    iv: String,
    ciphertext: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemEntry {
    kind: String,
    actor_id: Option<String>,
}

/// A message of a private conversation in the shape of a channel message.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastMessage {
    id: String,
    channel_id: String,
    author: UserDto,
    content: String,
    created_at: String,
    attachments: Vec<Attachment>,
    embeds: Vec<()>,
    edited_at: Option<String>,
    pinned_at: Option<String>,
    reply_to: Option<()>,
    reactions: Option<()>,
    encrypted: Option<EncryptedPayload>,
    system: Option<SystemEntry>,
}

/// The newest message of the conversation, or line about its encryption. A
/// line about somebody's keys stays in the history but is left out here: a new
/// sign-in is no reason to move a conversation up the list.
pub fn last_message(
    connection: &Connection,
    conversation: Id,
) -> rusqlite::Result<Option<(LastMessage, Timestamp)>> {
    let found = connection
        .query_row(
            "SELECT \"Id\", \"AuthorId\", \"Content\", \"CreatedAt\", \"EncryptedEpochId\", \"EncryptedIv\",
                    \"EncryptedCiphertext\", \"SystemKind\"
             FROM \"DirectMessages\"
             WHERE \"ConversationId\" = ?1
               AND (\"SystemKind\" IS NULL OR (\"SystemKind\" <> 'device_added' AND \"SystemKind\" <> 'keys_reset'))
             ORDER BY \"CreatedAt\" DESC, \"Id\" DESC LIMIT 1",
            [conversation],
            |row| {
                Ok((
                    row.get::<_, Id>(0)?,
                    row.get::<_, Id>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Timestamp>(3)?,
                    row.get::<_, Option<Id>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((id, author, content, created_at, epoch, iv, ciphertext, kind)) = found else {
        return Ok(None);
    };
    let Some(author_row) = User::find(connection, author)? else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT a.\"Id\", a.\"FileName\", a.\"ContentType\", a.\"SizeBytes\", a.\"Width\", a.\"Height\", a.\"DurationSeconds\",
                a.\"PosterObjectKey\" IS NOT NULL, a.\"IsVoiceMessage\", a.\"PreviewObjectKey\" IS NOT NULL, a.\"Placeholder\"
         FROM \"DirectMessageAttachments\" l JOIN \"ImageAttachments\" a ON a.\"Id\" = l.\"AttachmentId\"
         WHERE l.\"DirectMessageId\" = ?1 ORDER BY l.\"AttachmentId\"",
    )?;
    let attachments = statement
        .query_map([id], |row| {
            let attachment: Id = row.get(0)?;
            Ok(Attachment {
                id: attachment.to_string(),
                name: row.get(1)?,
                content_type: row.get(2)?,
                size: row.get(3)?,
                width: row.get(4)?,
                height: row.get(5)?,
                url: format!("/assets/{attachment}"),
                duration_seconds: row.get(6)?,
                poster_url: row
                    .get::<_, bool>(7)?
                    .then(|| format!("/assets/{attachment}/poster")),
                is_voice_message: row.get(8)?,
                preview_url: row
                    .get::<_, bool>(9)?
                    .then(|| format!("/assets/{attachment}/preview")),
                placeholder: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let encrypted = match (epoch, iv, ciphertext) {
        (Some(epoch), Some(iv), Some(ciphertext)) => Some(EncryptedPayload {
            epoch_id: epoch.to_string(),
            iv: encoding::base64(&iv),
            ciphertext: encoding::base64(&ciphertext),
        }),
        _ => None,
    };
    let message = LastMessage {
        id: id.to_string(),
        channel_id: conversation.to_string(),
        author: author_row.dto("online"),
        content,
        created_at: created_at.to_wire_text(),
        attachments,
        embeds: Vec::new(),
        edited_at: None,
        pinned_at: None,
        reply_to: None,
        reactions: None,
        encrypted,
        system: kind.map(|kind| SystemEntry {
            kind,
            actor_id: Some(author.to_string()),
        }),
    };
    Ok(Some((message, created_at)))
}

/// What `me` has not read: the other person's messages after the read marker,
/// or after the conversation began while there is no marker. Of the timeline
/// lines only a request to switch encryption on counts, because it waits for
/// the viewer's answer.
pub fn unread(connection: &Connection, me: Id, conversation: &Conversation) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COUNT(*) FROM \"DirectMessages\" m
         WHERE m.\"ConversationId\" = ?1 AND m.\"AuthorId\" <> ?2
           AND (m.\"SystemKind\" IS NULL OR m.\"SystemKind\" = 'encryption_requested')
           AND m.\"CreatedAt\" > COALESCE(
                (SELECT r.\"LastReadAt\" FROM \"DirectConversationReads\" r
                 WHERE r.\"ConversationId\" = ?1 AND r.\"UserId\" = ?2),
                ?3)",
        params![conversation.id, me, conversation.created_at],
        |row| row.get(0),
    )
}
