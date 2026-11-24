//! Invites, as registration and joining read them.

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::values::{Id, Timestamp};

#[derive(Debug, Clone, Copy)]
pub struct Invite {
    pub id: Id,
    pub server: Id,
}

/// The invite behind the code while it can still be used: it has neither
/// expired nor run out of uses. Joining, registering and the preview all ask
/// this, so the three agree.
pub fn find_usable(
    connection: &Connection,
    code: &str,
    now: Timestamp,
) -> rusqlite::Result<Option<Invite>> {
    let code: String = code.trim().chars().flat_map(char::to_lowercase).collect();
    let found = connection
        .query_row(
            "SELECT \"Id\", \"ServerId\", \"Uses\", \"MaxUses\", \"ExpiresAt\" FROM \"ServerInvites\" WHERE \"Code\" = ?1 LIMIT 1",
            [&code],
            |row| {
                Ok((
                    Invite { id: row.get(0)?, server: row.get(1)? },
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<Timestamp>>(4)?,
                ))
            },
        )
        .optional()?;
    Ok(found.and_then(|(invite, uses, max_uses, expires_at)| {
        let expired = expires_at.is_some_and(|at| at <= now);
        let used_up = max_uses.is_some_and(|max| max <= uses);
        (!expired && !used_up).then_some(invite)
    }))
}

/// Counts one use.
pub fn record_use(tx: &Transaction, invite: Id) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE \"ServerInvites\" SET \"Uses\" = \"Uses\" + 1 WHERE \"Id\" = ?1",
        [invite],
    )?;
    Ok(())
}
