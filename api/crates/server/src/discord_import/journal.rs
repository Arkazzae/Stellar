//! Keeps what clients replay and what they count as unread as if the imported
//! history had always been there.
//!
//! The journal triggers write a row for every message inserted, and the sync
//! route pages through rows and asks a client to reload only on a page that
//! holds a reset. Left alone, an import of a million messages would have every
//! client fetch five thousand pages of upserts after its next reconnect. So the
//! rows the import's own inserts and edits caused are removed in the same
//! transaction, and the channel is left with exactly one reset, written last:
//! the journal's sequence is AUTOINCREMENT, so it is newer than any position a
//! client can hold from before, and such a client reloads the channel once. A
//! client that loaded the channel after the reset holds its position and has
//! nothing to replay.
//!
//! Read markers are times. A member's marker in an imported channel may be
//! older than history that Discord's people wrote while the import ran, so every
//! marker of the channel is raised to the newest imported message. Without a
//! marker, the first bootstrap sets one at that moment, which is later than
//! anything imported.

use rusqlite::{Connection, params};

use crate::values::{Id, Timestamp};

/// The journal's kinds, as `messages::history` reads them.
const UPSERT: i64 = 0;
const RESET: i64 = 2;

/// Forgets the journal rows the import's own changes to these messages caused
/// and leaves the channel with one reset, newer than any other row.
pub(super) fn reset(
    db: &Connection,
    channel: Id,
    messages: &[Id],
    now: Timestamp,
) -> rusqlite::Result<()> {
    for chunk in messages.chunks(500) {
        db.execute(
            "DELETE FROM \"MessageChanges\" WHERE \"ChannelId\" = ?1 AND \"Kind\" = ?2
               AND \"MessageId\" IN (SELECT value FROM json_each(?3))",
            params![channel, UPSERT, json_ids(chunk)],
        )?;
    }
    db.execute(
        "DELETE FROM \"MessageChanges\" WHERE \"ChannelId\" = ?1 AND \"Kind\" = ?2",
        params![channel, RESET],
    )?;
    db.execute(
        "INSERT INTO \"MessageChanges\" (\"ChannelId\", \"MessageId\", \"Kind\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
        params![channel, Id::from_bytes([0; 16]), RESET, now],
    )?;
    Ok(())
}

/// Forgets the post times a trigger wrote for these placeholders. They are slow
/// mode's memory of a person's last send, and a placeholder never sends.
pub(super) fn forget_post_times(
    db: &Connection,
    channel: Id,
    placeholders: &[Id],
) -> rusqlite::Result<()> {
    for chunk in placeholders.chunks(500) {
        db.execute(
            "DELETE FROM \"ChannelPostTimes\" WHERE \"ChannelId\" = ?1
               AND \"UserId\" IN (SELECT value FROM json_each(?2))",
            params![channel, json_ids(chunk)],
        )?;
    }
    Ok(())
}

/// Raises every member's marker in the channel to `newest`, where it is
/// earlier.
pub(super) fn mark_read(
    db: &Connection,
    channel: Id,
    newest: Timestamp,
) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE \"ChannelReads\" SET \"LastReadAt\" = ?2 WHERE \"ChannelId\" = ?1 AND \"LastReadAt\" < ?2",
        params![channel, newest],
    )
}

pub(super) fn json_ids(ids: &[Id]) -> String {
    serde_json::to_string(&ids.iter().map(Id::to_db_text).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string())
}

pub(super) fn json_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    serde_json::to_string(&texts.into_iter().collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string())
}
