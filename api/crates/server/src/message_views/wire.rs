//! A message as the clients read it, and the reading of it from the database.
//!
//! This is the shape every route that answers with messages writes, and the
//! one `MessageUpdated` carries. The members come in this order.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Serialize, Serializer};

use super::text;
use crate::instance::users::{User, UserDto};
use crate::values::{Id, Timestamp};

/// How much of a message a reply's quote keeps, in UTF-16 units.
const PREVIEW_UNITS: usize = 160;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDto {
    pub id: Id,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub url: String,
    #[serde(serialize_with = "seconds")]
    pub duration_seconds: Option<f64>,
    pub poster_url: Option<String>,
    pub is_voice_message: bool,
    pub preview_url: Option<String>,
    pub placeholder: Option<String>,
}

/// A whole number of seconds is written without a fraction, as the first
/// server wrote it.
fn seconds<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(number) if number.fract() == 0.0 && number.abs() < 1e15 => {
            serializer.serialize_i64(*number as i64)
        }
        Some(number) => serializer.serialize_f64(*number),
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedDto {
    pub id: Id,
    pub url: String,
    pub kind: &'static str,
    pub provider: String,
    pub provider_id: Option<String>,
    pub start_seconds: Option<i64>,
    pub domain: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionDto {
    pub emoji: String,
    pub count: i64,
    pub mine: bool,
}

/// The one-line quote above a reply, and the root in a list of threads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceDto {
    pub id: Id,
    /// A person's id, or `system`.
    pub author_id: String,
    pub author_name: String,
    pub preview: String,
    pub has_attachment: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub id: Id,
    pub channel_id: Id,
    pub author: UserDto,
    pub content: String,
    pub created_at: Timestamp,
    pub attachments: Vec<AttachmentDto>,
    pub embeds: Vec<EmbedDto>,
    pub edited_at: Option<Timestamp>,
    pub pinned_at: Option<Timestamp>,
    pub reply_to: Option<ReferenceDto>,
    pub reactions: Vec<ReactionDto>,
}

/// The texts a message needs from the catalogue.
#[derive(Debug, Clone)]
pub struct Words {
    /// What a quote says of a message that is only attachments.
    pub attachment: String,
}

/// A row of `Messages`.
#[derive(Debug, Clone)]
struct Row0 {
    id: Id,
    channel: Id,
    author: Id,
    is_system: bool,
    content: String,
    created_at: Timestamp,
    edited_at: Option<Timestamp>,
    pinned_at: Option<Timestamp>,
    reply_to: Option<Id>,
}

const COLUMNS: &str = "\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"EditedAt\", \"PinnedAt\", \"ReplyToMessageId\"";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Row0> {
    Ok(Row0 {
        id: row.get(0)?,
        channel: row.get(1)?,
        author: row.get(2)?,
        is_system: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        edited_at: row.get(6)?,
        pinned_at: row.get(7)?,
        reply_to: row.get(8)?,
    })
}

fn find(db: &Connection, id: Id) -> rusqlite::Result<Option<Row0>> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM \"Messages\" WHERE \"Id\" = ?1"),
        [id],
        from_row,
    )
    .optional()
}

/// The messages with the given ids, in that order, as `viewer` sees them (the
/// reactions they made are flagged). An id with no message is skipped.
pub fn load(
    db: &Connection,
    ids: &[Id],
    viewer: Option<Id>,
    words: &Words,
) -> rusqlite::Result<Vec<MessageDto>> {
    let mut loaded = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(row) = find(db, *id)?
            && let Some(dto) = assemble(db, row, viewer, words)?
        {
            loaded.push(dto);
        }
    }
    Ok(loaded)
}

fn assemble(
    db: &Connection,
    row: Row0,
    viewer: Option<Id>,
    words: &Words,
) -> rusqlite::Result<Option<MessageDto>> {
    let author = if row.is_system {
        UserDto::system()
    } else {
        match User::find(db, row.author)? {
            Some(user) => user.dto("online"),
            None => return Ok(None),
        }
    };
    let reply_to = match row.reply_to {
        Some(target) => match find(db, target)? {
            Some(quoted) => reference_of(db, &quoted, words)?,
            None => None,
        },
        None => None,
    };
    Ok(Some(MessageDto {
        id: row.id,
        channel_id: row.channel,
        author,
        content: row.content,
        created_at: row.created_at,
        attachments: attachments_of(db, row.id)?,
        embeds: embeds_of(db, row.id)?,
        edited_at: row.edited_at,
        pinned_at: row.pinned_at,
        reply_to,
        reactions: reactions_of(db, row.id, viewer)?,
    }))
}

fn attachments_of(db: &Connection, message: Id) -> rusqlite::Result<Vec<AttachmentDto>> {
    let mut statement = db.prepare_cached(
        "SELECT a.\"Id\", a.\"FileName\", a.\"ContentType\", a.\"SizeBytes\", a.\"Width\", a.\"Height\",
                a.\"DurationSeconds\", a.\"PosterObjectKey\" IS NOT NULL, a.\"IsVoiceMessage\",
                a.\"PreviewObjectKey\" IS NOT NULL, a.\"Placeholder\"
         FROM \"MessageAttachments\" ma JOIN \"ImageAttachments\" a ON a.\"Id\" = ma.\"AttachmentId\"
         WHERE ma.\"MessageId\" = ?1",
    )?;
    let rows = statement.query_map([message], |row| {
        let id: Id = row.get(0)?;
        let poster: bool = row.get(7)?;
        let preview: bool = row.get(9)?;
        Ok(AttachmentDto {
            id,
            name: row.get(1)?,
            content_type: row.get(2)?,
            size: row.get(3)?,
            width: row.get(4)?,
            height: row.get(5)?,
            url: format!("/assets/{id}"),
            duration_seconds: row.get(6)?,
            poster_url: poster.then(|| format!("/assets/{id}/poster")),
            is_voice_message: row.get(8)?,
            preview_url: preview.then(|| format!("/assets/{id}/preview")),
            placeholder: row.get(10)?,
        })
    })?;
    rows.collect()
}

fn embeds_of(db: &Connection, message: Id) -> rusqlite::Result<Vec<EmbedDto>> {
    let mut statement = db.prepare_cached(
        "SELECT \"Id\", \"Url\", \"Kind\", \"Provider\", \"ProviderId\", \"StartSeconds\", \"Domain\", \"Title\",
                \"Description\", \"ThumbnailAttachmentId\"
         FROM \"MessageEmbeds\" WHERE \"MessageId\" = ?1 ORDER BY \"Position\"",
    )?;
    let rows = statement.query_map([message], |row| {
        let kind: i64 = row.get(2)?;
        let thumbnail: Option<Id> = row.get(9)?;
        Ok(EmbedDto {
            id: row.get(0)?,
            url: row.get(1)?,
            kind: if kind == 1 { "video" } else { "link" },
            provider: row.get(3)?,
            provider_id: row.get(4)?,
            start_seconds: row.get(5)?,
            domain: row.get(6)?,
            title: row.get(7)?,
            description: row.get(8)?,
            thumbnail_url: thumbnail.map(|id| format!("/assets/{id}")),
        })
    })?;
    rows.collect()
}

/// The reactions grouped by emoji, the one that was made first leading.
fn reactions_of(
    db: &Connection,
    message: Id,
    viewer: Option<Id>,
) -> rusqlite::Result<Vec<ReactionDto>> {
    let mut statement = db.prepare_cached(
        "SELECT \"Emoji\", \"UserId\", \"CreatedAt\" FROM \"MessageReactions\" WHERE \"MessageId\" = ?1",
    )?;
    let made = statement
        .query_map([message], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Id>(1)?,
                row.get::<_, Timestamp>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Groups in the order their emoji first came up, then by the moment of
    // their earliest reaction; the sort is stable, so equal moments keep that
    // first order.
    let mut groups: Vec<(String, i64, bool, Timestamp)> = Vec::new();
    for (emoji, user, at) in made {
        let mine = viewer == Some(user);
        match groups.iter_mut().find(|group| group.0 == emoji) {
            Some(group) => {
                group.1 += 1;
                group.2 |= mine;
                group.3 = group.3.min(at);
            }
            None => groups.push((emoji, 1, mine, at)),
        }
    }
    groups.sort_by_key(|group| group.3);
    Ok(groups
        .into_iter()
        .map(|(emoji, count, mine, _)| ReactionDto { emoji, count, mine })
        .collect())
}

/// The quote of a message: its first 160 units on one line, and who wrote it.
pub fn reference_by_id(
    db: &Connection,
    id: Id,
    words: &Words,
) -> rusqlite::Result<Option<ReferenceDto>> {
    match find(db, id)? {
        Some(row) => reference_of(db, &row, words),
        None => Ok(None),
    }
}

fn reference_of(
    db: &Connection,
    row: &Row0,
    words: &Words,
) -> rusqlite::Result<Option<ReferenceDto>> {
    let author = if row.is_system {
        UserDto::system()
    } else {
        match User::find(db, row.author)? {
            Some(user) => user.dto("online"),
            None => return Ok(None),
        }
    };
    let has_attachment: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MessageAttachments\" WHERE \"MessageId\" = ?1)",
        params![row.id],
        |found| found.get(0),
    )?;
    let mut preview = one_line(&row.content);
    if preview.is_empty() && has_attachment {
        preview = words.attachment.clone();
    }
    Ok(Some(ReferenceDto {
        id: row.id,
        author_id: author.id,
        author_name: author.display_name,
        preview,
        has_attachment,
    }))
}

/// The words of the text on one line, cut to 160 units with an ellipsis.
fn one_line(content: &str) -> String {
    let joined = content
        .split(text::is_white_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text::units(&joined) > PREVIEW_UNITS {
        format!("{}\u{2026}", text::first_units(&joined, PREVIEW_UNITS - 3))
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_is_one_line() {
        assert_eq!(one_line("  a \n\t b\u{a0}c  "), "a b c");
        assert_eq!(one_line("   "), "");
    }

    #[test]
    fn a_long_quote_is_cut_to_160_units_with_an_ellipsis() {
        let long = "x".repeat(200);
        let quote = one_line(&long);
        assert_eq!(text::units(&quote), 158);
        assert!(quote.ends_with('\u{2026}'));
        assert_eq!(one_line(&"y".repeat(160)).len(), 160);
    }

    #[test]
    fn a_whole_number_of_seconds_has_no_fraction() {
        #[derive(Serialize)]
        struct Holder {
            #[serde(serialize_with = "seconds")]
            value: Option<f64>,
        }
        let written = |value| serde_json::to_string(&Holder { value }).unwrap();
        assert_eq!(written(Some(3.0)), "{\"value\":3}");
        assert_eq!(written(Some(3.5)), "{\"value\":3.5}");
        assert_eq!(written(None), "{\"value\":null}");
    }
}
