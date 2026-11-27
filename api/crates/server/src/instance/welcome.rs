//! The system notice that greets a person who has just joined a server.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use stellarr_messages::{MessageKey, Messages};

use super::access;
use super::users::{User, UserDto};
use crate::database::DbError;
use crate::events::Event;
use crate::messages::dto::MessageDto;
use crate::permissions::visibility;
use crate::values::{Id, Timestamp};

/// A notice that has been written and is waiting to be announced.
#[derive(Debug, Clone)]
pub struct Welcome {
    pub server: Id,
    pub channel: Id,
    id: Id,
    content: String,
    created_at: Timestamp,
}

/// Writes the notice into the first text channel the newcomer will see. A
/// server without such a channel gets no notice, which is valid for a
/// voice-only server.
///
/// The membership is not written yet when this runs, so who sees what is asked
/// of the base role alone, as a membership starts; the owner of a new server
/// sees every channel, and its first one is open anyway.
pub fn create(
    tx: &Transaction,
    server: Id,
    user: &User,
    joined_at: Timestamp,
    messages: &Messages,
) -> Result<Option<Welcome>, DbError> {
    create_in(tx, server, user, joined_at, messages, None)
}

/// Like [`create`], for somebody who came through an invite that leads to a
/// channel: when it is a text channel of the server, the notice goes there,
/// even if the newcomer cannot see the channel.
pub fn create_in(
    tx: &Transaction,
    server: Id,
    user: &User,
    joined_at: Timestamp,
    messages: &Messages,
    invited_to: Option<Id>,
) -> Result<Option<Welcome>, DbError> {
    let named = match invited_to {
        Some(channel) => tx
            .query_row(
                "SELECT \"Id\" FROM \"Channels\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2 AND \"Kind\" = 0",
                params![channel, server],
                |row| row.get::<_, Id>(0),
            )
            .optional()?,
        None => None,
    };
    let channel = match named {
        Some(channel) => channel,
        None => match first_text_channel(tx, server)? {
            Some(channel) => channel,
            None => return Ok(None),
        },
    };
    // The notice is markup, so every punctuation character of the name is
    // escaped; the transcript shows it as the character that was typed and it
    // cannot open a span, a link, a mention or an emoji of its own.
    let name = escape_ascii_punctuation(&user.display_name);
    let content = messages
        .get(MessageKey::ServerWelcomeNotice, &[("name", &name)])
        .map_err(|e| DbError::Task(e.to_string()))?;
    let id = Id::random();
    tx.execute(
        "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\") VALUES (?1, ?2, ?3, 1, ?4, ?5)",
        params![id, channel, user.id, content, joined_at],
    )?;
    Ok(Some(Welcome {
        server,
        channel,
        id,
        content,
        created_at: joined_at,
    }))
}

/// The event that tells the members who are online and see the channel, to be
/// published once the notice has been committed.
pub fn announcement(connection: &Connection, welcome: &Welcome) -> rusqlite::Result<Event> {
    let audience = visibility::audience_of(connection, welcome.server, welcome.channel)?;
    let message = MessageDto {
        id: welcome.id,
        channel_id: welcome.channel,
        author: UserDto::system(),
        content: welcome.content.clone(),
        created_at: welcome.created_at,
        attachments: Vec::new(),
        embeds: Vec::new(),
        edited_at: None,
        pinned_at: None,
        reply_to: None,
        reactions: Vec::new(),
        poll: None,
        event: None,
        mentions_everyone: false,
        mentions_here: false,
    };
    Ok(Event::new(audience, "MessageCreated", &message))
}

fn first_text_channel(connection: &Connection, server: Id) -> rusqlite::Result<Option<Id>> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"SectionId\" FROM \"Channels\" WHERE \"ServerId\" = ?1 AND \"Kind\" = 0 ORDER BY \"Position\"",
    )?;
    let channels: Vec<(Id, Option<Id>)> = statement
        .query_map([server], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if channels.is_empty() || access::server_open(connection, server)? {
        return Ok(channels.first().map(|(id, _)| *id));
    }
    for (id, section) in channels {
        if access::newcomer_views(connection, server, section, id)? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// The name with every ASCII punctuation character after a backslash, the one
/// escape the message grammar reads anywhere; everything else, Unicode
/// included, needs none.
pub fn escape_ascii_punctuation(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ascii_punctuation_character_is_escaped_and_nothing_else() {
        assert_eq!(escape_ascii_punctuation("Maya Lin"), "Maya Lin");
        assert_eq!(
            escape_ascii_punctuation("**x** _y_ [z](u) <@1> :e: `c`"),
            "\\*\\*x\\*\\* \\_y\\_ \\[z\\]\\(u\\) \\<\\@1\\> \\:e\\: \\`c\\`"
        );
        assert_eq!(escape_ascii_punctuation("Zażółć ~ {}"), "Zażółć \\~ \\{\\}");
    }
}
