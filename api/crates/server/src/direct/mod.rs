//! Direct conversations: opening one with a person, reading its messages page
//! by page, posting in it and marking it read.
//!
//! `POST /api/social/conversations`, `GET` and `POST
//! /api/social/conversations/{id}/messages` and `POST
//! /api/social/conversations/{id}/read`. Like the friends routes they belong to
//! the instance, not to a server. The state the friends screen shows of a
//! conversation is read by `social::conversations`.
//!
//! What a block, a friendship and the peer's `AllowDirectMessages` change here:
//! opening and posting are refused (403) when either has blocked the other, and
//! history reads as a conversation that does not exist (404); with direct
//! messages off, only friends may write.

mod encryption;
mod history;
mod open;
pub(crate) mod post;
mod read;

use rusqlite::{Connection, OptionalExtension, params};

use crate::http::Endpoints;
use crate::values::{Id, Timestamp};

pub fn routes() -> Endpoints {
    Endpoints::new()
        .post("/api/social/conversations", open::open)
        .authorized()
        .json_body()
        .get("/api/social/conversations/{conversationId}/messages", history::history)
        .authorized()
        .post("/api/social/conversations/{conversationId}/messages", post::create)
        .authorized()
        .json_body()
        .post("/api/social/conversations/{conversationId}/read", read::mark_read)
        .authorized()
}

/// The conversation, when the person is one of its two: the other one's id.
pub(crate) fn peer_in(connection: &Connection, conversation: Id, user: Id) -> rusqlite::Result<Option<Id>> {
    let found: Option<(Id, Id)> = connection
        .query_row(
            "SELECT \"UserAId\", \"UserBId\" FROM \"DirectConversations\"
             WHERE \"Id\" = ?1 AND (\"UserAId\" = ?2 OR \"UserBId\" = ?2)",
            params![conversation, user],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(found.map(|(a, b)| crate::social::other(a, b, user)))
}

/// Sets the person's read marker, to `at` and to nothing else: unlike a
/// channel's, it is not held back by a later one.
pub(crate) fn set_read(connection: &Connection, conversation: Id, user: Id, at: Timestamp) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO \"DirectConversationReads\" (\"ConversationId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)
         ON CONFLICT (\"ConversationId\", \"UserId\") DO UPDATE SET \"LastReadAt\" = excluded.\"LastReadAt\"",
        params![conversation, user, at],
    )?;
    Ok(())
}

/// Whether the peer accepts a message from this person. Somebody who turned
/// direct messages off still hears from their friends.
pub fn peer_takes_messages_from(connection: &Connection, sender: Id, peer: Id) -> rusqlite::Result<bool> {
    let allowed: Option<bool> = connection
        .query_row(
            "SELECT \"AllowDirectMessages\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
            [peer],
            |row| row.get(0),
        )
        .optional()?;
    if allowed != Some(false) {
        return Ok(true);
    }
    crate::social::are_friends(connection, sender, peer)
}
