//! Who hears about an account's presence: the servers it belongs to, and the
//! people it has a friendship or a conversation with and has not blocked and
//! is not blocked by, even when their tabs follow different servers.

use rusqlite::Connection;

use super::names::{server_group, user_group};
use crate::values::Id;

/// Who hears a change of `user`'s presence: the servers they belong to, and
/// the people they share a friendship or a conversation with.
pub struct Reach {
    pub servers: Vec<Id>,
    pub peers: Vec<Id>,
}

/// The groups a change of `user`'s presence goes to, servers first.
pub fn presence_groups(connection: &Connection, user: Id) -> rusqlite::Result<Vec<String>> {
    let reach = presence_reach(connection, user)?;
    Ok(reach
        .servers
        .into_iter()
        .map(server_group)
        .chain(reach.peers.into_iter().map(user_group))
        .collect())
}

/// The same, as the ids the groups are made from.
pub fn presence_reach(connection: &Connection, user: Id) -> rusqlite::Result<Reach> {
    // Servers in the order the person joined them, and peers by id, so that the
    // groups are always listed in the same order.
    let mut servers = connection.prepare(
        "SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"UserId\" = ?1 ORDER BY \"JoinedAt\", \"ServerId\"",
    )?;
    let servers: Vec<Id> = servers
        .query_map([user], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    let mut peers = connection.prepare(
        "SELECT peer FROM (
             SELECT CASE WHEN \"UserAId\" = ?1 THEN \"UserBId\" ELSE \"UserAId\" END AS peer
             FROM \"Friendships\" WHERE \"UserAId\" = ?1 OR \"UserBId\" = ?1
             UNION
             SELECT CASE WHEN \"UserAId\" = ?1 THEN \"UserBId\" ELSE \"UserAId\" END AS peer
             FROM \"DirectConversations\" WHERE \"UserAId\" = ?1 OR \"UserBId\" = ?1
         ) AS people
         WHERE NOT EXISTS (
             SELECT 1 FROM \"UserBlocks\" AS block
             WHERE (block.\"BlockerId\" = ?1 AND block.\"BlockedId\" = people.peer)
                OR (block.\"BlockerId\" = people.peer AND block.\"BlockedId\" = ?1))
         ORDER BY peer",
    )?;
    let peers: Vec<Id> = peers
        .query_map([user], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    Ok(Reach { servers, peers })
}
