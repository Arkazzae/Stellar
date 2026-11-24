//! Invites, as registration and joining read them.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

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

/// Counts one use, if the invite is still there, has uses left and has not
/// expired. False when it has not been counted.
pub fn record_use(tx: &Transaction, invite: Id, now: Timestamp) -> rusqlite::Result<bool> {
    let changed = tx.execute(
        "UPDATE \"ServerInvites\" SET \"Uses\" = \"Uses\" + 1
         WHERE \"Id\" = ?1 AND (\"MaxUses\" IS NULL OR \"Uses\" < \"MaxUses\") AND (\"ExpiresAt\" IS NULL OR \"ExpiresAt\" > ?2)",
        params![invite, now],
    )?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::testing::new_database;

    #[tokio::test]
    async fn a_use_is_counted_only_while_the_invite_exists_has_uses_left_and_has_not_expired() {
        let (_folder, database) = new_database();
        let now = Timestamp::now();
        let later = now.plus(Duration::from_secs(3600));
        let invite = |max: Option<i64>, expires: Option<Timestamp>| {
            let id = Id::random();
            (id, max, expires)
        };
        let cases = [
            ("open", invite(None, None), true),
            ("room left", invite(Some(2), None), true),
            ("used up", invite(Some(0), None), false),
            (
                "expired",
                invite(None, Some(now.minus(Duration::from_secs(1)))),
                false,
            ),
            ("not yet expired", invite(None, Some(later)), true),
        ];
        // A connection of its own: foreign keys are off on it, so the invite
        // needs no server and no creator.
        let mut connection = rusqlite::Connection::open(database.path()).unwrap();
        connection.pragma_update(None, "foreign_keys", false).unwrap();
        let tx = connection.transaction().unwrap();
        let mut outcomes = Vec::new();
        for (name, (id, max, expires), expected) in cases {
            tx.execute(
                "INSERT INTO \"ServerInvites\" VALUES (?1, ?1, ?1, NULL, ?2, 0, ?3, ?4, ?5)",
                params![id, id.to_string(), max, expires, now],
            )
            .unwrap();
            outcomes.push((name, record_use(&tx, id, now).unwrap(), expected));
        }
        // An invite that is not there is not counted either.
        outcomes.push((
            "missing",
            record_use(&tx, Id::random(), now).unwrap(),
            false,
        ));

        for (name, counted, expected) in outcomes {
            assert_eq!(counted, expected, "{name}");
        }
    }
}
