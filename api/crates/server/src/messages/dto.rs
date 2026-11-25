//! What a message looks like as JSON, in one place.
//!
//! Every route and event that carries a channel message goes through
//! [`load`], which reads any number of messages with a fixed handful of
//! statements and builds their [`MessageDto`]s. The routes of reactions, pins,
//! threads and search use it too, so that a message reads the same wherever
//! it is shown.

use std::collections::HashMap;

use rusqlite::{Connection, Row};
use serde::{Serialize, Serializer};
use stellarr_messages::{MessageKey, Messages};
use stellarr_unicode::utf16;

use crate::database::DbError;
use crate::instance::users::UserDto;
use crate::values::{Id, Timestamp};

/// A reference to a reply's target is cut to this many UTF-16 units, the last
/// of which is the ellipsis.
const PREVIEW_LENGTH: usize = 160;

/// A duration in seconds: a whole number without a
/// fraction (`12`, not `12.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seconds(pub f64);

impl Serialize for Seconds {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = self.0;
        if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
            serializer.serialize_i64(value as i64)
        } else {
            serializer.serialize_f64(value)
        }
    }
}

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
    pub duration_seconds: Option<Seconds>,
    pub poster_url: Option<String>,
    pub is_voice_message: bool,
    pub preview_url: Option<String>,
    pub placeholder: Option<String>,
}

/// An unfurled link under a message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEmbedDto {
    pub id: Id,
    pub url: String,
    /// `link` for a card, `video` for something with a player.
    pub kind: &'static str,
    pub provider: String,
    pub provider_id: Option<String>,
    pub start_seconds: Option<i64>,
    pub domain: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Always one of our own `/assets` addresses, never a third party's.
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageReactionDto {
    pub emoji: String,
    pub count: i64,
    pub mine: bool,
}

/// The short quote shown above a reply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageReferenceDto {
    pub id: Id,
    /// The id of the author, or `system` for a notice of the server.
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
    pub embeds: Vec<MessageEmbedDto>,
    /// Null until the author rewrites it.
    pub edited_at: Option<Timestamp>,
    pub pinned_at: Option<Timestamp>,
    pub reply_to: Option<MessageReferenceDto>,
    pub reactions: Vec<MessageReactionDto>,
}

/// Pushed when the embeds of a message have been rebuilt; an empty list means
/// it has none (any more).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEmbedsDto {
    pub message_id: Id,
    pub channel_id: Id,
    pub embeds: Vec<MessageEmbedDto>,
}

/// Pushed when a message is removed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDeletedDto {
    pub message_id: Id,
    pub channel_id: Id,
}

/// A person's read watermark in one text channel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelReadDto {
    pub channel_id: Id,
    pub last_read_at: Timestamp,
}

/// Collapses a message to one line for a quote or a notice: runs of
/// whitespace become one space, and what is over `limit` UTF-16 units is cut
/// to `limit - 3` units and an ellipsis.
pub fn one_line(content: &str, limit: usize) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if utf16::length(&compact) <= limit {
        return compact;
    }
    // A pair the cut falls inside is left out whole.
    let mut cut = utf16::prefix(&compact, limit - 3).to_string();
    cut.push('…');
    cut
}

fn attachment_from(row: &Row<'_>, from: usize) -> rusqlite::Result<AttachmentDto> {
    let id: Id = row.get(from)?;
    let poster: Option<String> = row.get(from + 6)?;
    let preview: Option<String> = row.get(from + 7)?;
    Ok(AttachmentDto {
        id,
        name: row.get(from + 1)?,
        content_type: row.get(from + 2)?,
        size: row.get(from + 3)?,
        width: row.get(from + 4)?,
        height: row.get(from + 8)?,
        url: format!("/assets/{id}"),
        duration_seconds: row.get::<_, Option<f64>>(from + 5)?.map(Seconds),
        poster_url: poster.map(|_| format!("/assets/{id}/poster")),
        is_voice_message: row.get(from + 9)?,
        preview_url: preview.map(|_| format!("/assets/{id}/preview")),
        placeholder: row.get(from + 10)?,
    })
}

const ATTACHMENT_COLUMNS: &str = "a.\"Id\", a.\"FileName\", a.\"ContentType\", a.\"SizeBytes\", a.\"Width\", a.\"DurationSeconds\",
    a.\"PosterObjectKey\", a.\"PreviewObjectKey\", a.\"Height\", a.\"IsVoiceMessage\", a.\"Placeholder\"";

pub(crate) fn json_ids(ids: &[Id]) -> String {
    let mut text = String::with_capacity(ids.len() * 39 + 2);
    text.push('[');
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push('"');
        text.push_str(&id.to_db_text());
        text.push('"');
    }
    text.push(']');
    text
}

struct Row_ {
    id: Id,
    channel: Id,
    author_id: Id,
    is_system: bool,
    content: String,
    created_at: Timestamp,
    edited_at: Option<Timestamp>,
    pinned_at: Option<Timestamp>,
    reply_to: Option<Id>,
    author: UserDto,
}

fn message_from(row: &Row<'_>) -> rusqlite::Result<Row_> {
    let author_id: Id = row.get(2)?;
    Ok(Row_ {
        id: row.get(0)?,
        channel: row.get(1)?,
        author_id,
        is_system: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        edited_at: row.get(6)?,
        pinned_at: row.get(7)?,
        reply_to: row.get(8)?,
        author: UserDto {
            id: author_id.to_string(),
            username: row.get(9)?,
            display_name: row.get(10)?,
            avatar_color: row.get(11)?,
            // Whoever wrote a message is shown as online beside it; where they
            // really are is the roster's business.
            presence: "online".to_string(),
            avatar_url: row.get(12)?,
            banner_url: row.get(13)?,
            custom_status: row.get(14)?,
            bio: row.get(15)?,
            pronouns: row.get(16)?,
            origin: row.get(17)?,
        },
    })
}

const MESSAGE_COLUMNS: &str = "m.\"Id\", m.\"ChannelId\", m.\"AuthorId\", m.\"IsSystem\", m.\"Content\", m.\"CreatedAt\", m.\"EditedAt\",
    m.\"PinnedAt\", m.\"ReplyToMessageId\", u.\"Username\", u.\"DisplayName\", u.\"AvatarColor\", u.\"AvatarUrl\",
    u.\"BannerUrl\", u.\"CustomStatus\", u.\"Bio\", u.\"Pronouns\", u.\"Origin\"";

/// Reads the messages with these ids as they are shown in a transcript, in the
/// order of `ids`; an id that is no message is left out. Reactions are counted
/// for everybody and flagged for `viewer`.
///
/// It costs five statements however many messages there are: the messages
/// with their authors, the messages they answer, and their attachments,
/// embeds and reactions.
pub fn load(
    db: &Connection,
    messages: &Messages,
    ids: &[Id],
    viewer: Option<Id>,
) -> Result<Vec<MessageDto>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = json_ids(ids);
    let mut found: HashMap<Id, Row_> = HashMap::with_capacity(ids.len());
    {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM \"Messages\" m JOIN \"Users\" u ON u.\"Id\" = m.\"AuthorId\"
             WHERE m.\"Id\" IN (SELECT value FROM json_each(?1))"
        ))?;
        let rows = statement.query_map([&wanted], message_from)?;
        for row in rows {
            let row = row?;
            found.insert(row.id, row);
        }
    }

    // The messages that are answered.
    let mut reply_ids: Vec<Id> = found.values().filter_map(|row| row.reply_to).collect();
    reply_ids.sort();
    reply_ids.dedup();
    let mut replies: HashMap<Id, Row_> = HashMap::new();
    if !reply_ids.is_empty() {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM \"Messages\" m JOIN \"Users\" u ON u.\"Id\" = m.\"AuthorId\"
             WHERE m.\"Id\" IN (SELECT value FROM json_each(?1))"
        ))?;
        for row in statement.query_map([json_ids(&reply_ids)], message_from)? {
            let row = row?;
            replies.insert(row.id, row);
        }
    }

    let everything: Vec<Id> = found
        .keys()
        .copied()
        .chain(replies.keys().copied())
        .collect();
    let everything = json_ids(&everything);

    let mut attachments: HashMap<Id, Vec<AttachmentDto>> = HashMap::new();
    {
        let mut statement = db.prepare_cached(&format!(
            "SELECT ma.\"MessageId\", {ATTACHMENT_COLUMNS}
             FROM \"MessageAttachments\" ma JOIN \"ImageAttachments\" a ON a.\"Id\" = ma.\"AttachmentId\"
             WHERE ma.\"MessageId\" IN (SELECT value FROM json_each(?1))
             ORDER BY ma.\"MessageId\", ma.\"AttachmentId\""
        ))?;
        for row in statement.query_map([&everything], |row| {
            Ok((row.get::<_, Id>(0)?, attachment_from(row, 1)?))
        })? {
            let (message, attachment) = row?;
            attachments.entry(message).or_default().push(attachment);
        }
    }

    let mut embeds: HashMap<Id, Vec<(i64, MessageEmbedDto)>> = HashMap::new();
    {
        let mut statement = db.prepare_cached(
            "SELECT e.\"MessageId\", e.\"Id\", e.\"Url\", e.\"Kind\", e.\"Provider\", e.\"ProviderId\", e.\"StartSeconds\", e.\"Domain\",
                    e.\"Title\", e.\"Description\", e.\"ThumbnailAttachmentId\", e.\"Position\", a.\"ContentType\"
             FROM \"MessageEmbeds\" e LEFT JOIN \"ImageAttachments\" a ON a.\"Id\" = e.\"ThumbnailAttachmentId\"
             WHERE e.\"MessageId\" IN (SELECT value FROM json_each(?1))
             ORDER BY e.\"MessageId\", e.\"Position\", e.rowid",
        )?;
        for row in statement.query_map([&wanted], |row| {
            let media = row.get::<_, Option<Id>>(10)?.map(|id| format!("/assets/{id}"));
            let content_type: Option<String> = row.get(12)?;
            let (thumbnail_url, animation_url) = if content_type.as_deref() == Some("video/mp4") {
                (None, media)
            } else {
                (media, None)
            };
            Ok((
                row.get::<_, Id>(0)?,
                row.get::<_, i64>(11)?,
                MessageEmbedDto {
                    id: row.get(1)?,
                    url: row.get(2)?,
                    kind: if row.get::<_, i64>(3)? == 1 {
                        "video"
                    } else {
                        "link"
                    },
                    provider: row.get(4)?,
                    provider_id: row.get(5)?,
                    start_seconds: row.get(6)?,
                    domain: row.get(7)?,
                    title: row.get(8)?,
                    description: row.get(9)?,
                    thumbnail_url,
                    animation_url,
                },
            ))
        })? {
            let (message, position, embed) = row?;
            embeds.entry(message).or_default().push((position, embed));
        }
    }

    let mut reactions: HashMap<Id, Vec<(String, Id, Timestamp)>> = HashMap::new();
    {
        let mut statement = db.prepare_cached(
            "SELECT \"MessageId\", \"Emoji\", \"UserId\", \"CreatedAt\" FROM \"MessageReactions\"
             WHERE \"MessageId\" IN (SELECT value FROM json_each(?1))
             ORDER BY \"MessageId\", \"Emoji\", \"UserId\"",
        )?;
        for row in statement.query_map([&wanted], |row| {
            Ok((
                row.get::<_, Id>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Id>(2)?,
                row.get::<_, Timestamp>(3)?,
            ))
        })? {
            let (message, emoji, user, at) = row?;
            reactions
                .entry(message)
                .or_default()
                .push((emoji, user, at));
        }
    }

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = found.remove(id) else {
            continue;
        };
        let reply_to = match row.reply_to.and_then(|target| replies.get(&target)) {
            Some(target) => Some(reference(
                target,
                attachments.get(&target.id).is_some_and(|a| !a.is_empty()),
                messages,
            )?),
            None => None,
        };
        let mut own_embeds = embeds.remove(&row.id).unwrap_or_default();
        own_embeds.sort_by_key(|(position, _)| *position);
        out.push(MessageDto {
            id: row.id,
            channel_id: row.channel,
            author: if row.is_system {
                UserDto::system()
            } else {
                row.author
            },
            content: row.content,
            created_at: row.created_at,
            attachments: attachments.get(&row.id).cloned().unwrap_or_default(),
            embeds: own_embeds.into_iter().map(|(_, embed)| embed).collect(),
            edited_at: row.edited_at,
            pinned_at: row.pinned_at,
            reply_to,
            reactions: tally(reactions.remove(&row.id).unwrap_or_default(), viewer),
        });
    }
    Ok(out)
}

/// The quotes of these messages, as a reply or a list of threads shows them,
/// in the order of `ids`; an id that is no message is left out.
pub fn references(
    db: &Connection,
    messages: &Messages,
    ids: &[Id],
) -> Result<Vec<MessageReferenceDto>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = json_ids(ids);
    let mut found: HashMap<Id, Row_> = HashMap::with_capacity(ids.len());
    {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM \"Messages\" m JOIN \"Users\" u ON u.\"Id\" = m.\"AuthorId\"
             WHERE m.\"Id\" IN (SELECT value FROM json_each(?1))"
        ))?;
        for row in statement.query_map([&wanted], message_from)? {
            let row = row?;
            found.insert(row.id, row);
        }
    }
    let mut with_files: std::collections::HashSet<Id> = std::collections::HashSet::new();
    {
        let mut statement = db.prepare_cached(
            "SELECT DISTINCT \"MessageId\" FROM \"MessageAttachments\"
             WHERE \"MessageId\" IN (SELECT value FROM json_each(?1))",
        )?;
        for row in statement.query_map([&wanted], |row| row.get::<_, Id>(0))? {
            with_files.insert(row?);
        }
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(row) = found.get(id) {
            out.push(reference(row, with_files.contains(id), messages)?);
        }
    }
    Ok(out)
}

fn reference(
    target: &Row_,
    has_attachment: bool,
    messages: &Messages,
) -> Result<MessageReferenceDto, DbError> {
    let mut preview = one_line(&target.content, PREVIEW_LENGTH);
    if preview.is_empty() && has_attachment {
        preview = messages
            .get(MessageKey::AttachmentPreview, &[])
            .map_err(|error| DbError::Task(error.to_string()))?;
    }
    Ok(MessageReferenceDto {
        id: target.id,
        author_id: if target.is_system {
            "system".to_string()
        } else {
            target.author_id.to_string()
        },
        author_name: if target.is_system {
            "system".to_string()
        } else {
            target.author.display_name.clone()
        },
        preview,
        has_attachment,
    })
}

/// One entry per emoji, in the order of the first reaction to each.
fn tally(rows: Vec<(String, Id, Timestamp)>, viewer: Option<Id>) -> Vec<MessageReactionDto> {
    // (emoji, earliest, count, mine), in the order each emoji was first met.
    let mut groups: Vec<(String, Timestamp, i64, bool)> = Vec::new();
    for (emoji, user, at) in rows {
        match groups.iter_mut().find(|group| group.0 == emoji) {
            Some(group) => {
                group.1 = group.1.min(at);
                group.2 += 1;
                group.3 |= viewer == Some(user);
            }
            None => groups.push((emoji, at, 1, viewer == Some(user))),
        }
    }
    groups.sort_by_key(|group| group.1);
    groups
        .into_iter()
        .map(|(emoji, _, count, mine)| MessageReactionDto { emoji, count, mine })
        .collect()
}

/// The reference to a message that a reply carries, for a message that has
/// been read: the one-line preview of its words, or the word for an
/// attachment when it has none.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_is_one_line_of_at_most_160_units_ending_in_an_ellipsis() {
        assert_eq!(one_line("  a \n\t b  ", 160), "a b");
        let long = "x".repeat(200);
        let cut = one_line(&long, 160);
        assert_eq!(cut.encode_utf16().count(), 158);
        assert!(cut.ends_with('…'));
        assert_eq!(one_line(&"y".repeat(160), 160), "y".repeat(160));
    }

    #[test]
    fn a_cut_through_a_pair_leaves_the_pair_out() {
        let text = format!("{}\u{1F600}bbbb", "a".repeat(156));

        let cut = one_line(&text, 160);

        assert_eq!(cut, format!("{}…", "a".repeat(156)));
        assert!(!cut.contains('\u{FFFD}'));
        // One unit further on the pair fits whole.
        let fits = one_line(&format!("{}\u{1F600}bbbb", "a".repeat(155)), 160);
        assert_eq!(fits, format!("{}\u{1F600}…", "a".repeat(155)));
    }

    #[test]
    fn whole_seconds_are_written_without_a_fraction() {
        let written = |value: f64| serde_json::to_string(&Seconds(value)).unwrap();

        assert_eq!(written(12.0), "12");
        assert_eq!(written(2.5), "2.5");
        assert_eq!(written(0.0), "0");
    }

    #[test]
    fn reactions_are_listed_by_the_first_one_to_each_emoji() {
        let (a, b) = (Id::random(), Id::random());
        let at = |text: &str| Timestamp::parse(text).unwrap();
        let rows = vec![
            ("🎉".to_string(), a, at("2026-11-24 10:00:05")),
            ("👍".to_string(), a, at("2026-11-24 10:00:01")),
            ("👍".to_string(), b, at("2026-11-24 10:00:09")),
        ];

        let tallied = tally(rows, Some(b));

        assert_eq!(tallied[0].emoji, "👍");
        assert_eq!((tallied[0].count, tallied[0].mine), (2, true));
        assert_eq!((tallied[1].count, tallied[1].mine), (1, false));
    }
}
