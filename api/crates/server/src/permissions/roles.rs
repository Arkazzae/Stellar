//! Which roles a member holds, and which of them ranks them.
//!
//! Every member holds the base role, without a row. The others are rows of
//! `ServerMemberRoles`, and the top role stored on the membership counts as
//! held too. Every writer stores the top role as a row as well, so that only
//! decides for a membership written by hand, and there it keeps what ranks a
//! member and what they may do from parting.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use super::DataError;
use super::privileges;
use crate::values::{Id, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    pub id: Id,
    pub server: Id,
    pub position: i64,
    pub is_system: bool,
    pub is_owner: bool,
    pub permissions: i64,
}

const ROLE_COLUMNS: &str =
    "r.\"Id\", r.\"ServerId\", r.\"Position\", r.\"IsSystem\", r.\"IsOwner\", r.\"Permissions\"";

impl Role {
    /// Reads the six columns of [`ROLE_COLUMNS`] from `first` on.
    pub(super) fn read(row: &rusqlite::Row<'_>, first: usize) -> rusqlite::Result<Role> {
        Ok(Role {
            id: row.get(first)?,
            server: row.get(first + 1)?,
            position: row.get(first + 2)?,
            is_system: row.get(first + 3)?,
            is_owner: row.get(first + 4)?,
            permissions: row.get(first + 5)?,
        })
    }

    /// Rank is the top role. The owner role outranks every other and is
    /// outranked by none; otherwise a lower position ranks higher, and an
    /// equal position does not outrank.
    pub fn outranks(&self, target: &Role) -> bool {
        self.server == target.server
            && self.id != target.id
            && !target.is_owner
            && (self.is_owner || self.position < target.position)
    }
}

/// Highest first: the owner role, then by position, then by id, so that roles
/// at an equal position still come in one order.
pub fn rank_order(a: &Role, b: &Role) -> Ordering {
    b.is_owner
        .cmp(&a.is_owner)
        .then(a.position.cmp(&b.position))
        .then(a.id.cmp(&b.id))
}

pub fn ranked(roles: impl IntoIterator<Item = Role>) -> Vec<Role> {
    let mut roles: Vec<Role> = roles.into_iter().collect();
    roles.sort_by(rank_order);
    roles
}

/// The role that ranks a member who holds `held`: the highest of them, or
/// else the base role.
pub fn top(held: &[Role], base: Role) -> Role {
    held.iter().copied().min_by(rank_order).unwrap_or(base)
}

/// The roles a member holds besides the base role, highest first, from the
/// ids stored for them and the top role. Ids of roles that are gone or belong
/// to another server count for nothing.
pub fn held_by(
    server: Id,
    top_role: Id,
    stored: &[Id],
    server_roles: &HashMap<Id, Role>,
    base: Option<Id>,
) -> Vec<Role> {
    let mut seen = HashSet::new();
    ranked(
        stored
            .iter()
            .copied()
            .chain([top_role])
            .filter(|id| Some(*id) != base && seen.insert(*id))
            .filter_map(|id| server_roles.get(&id).copied())
            .filter(|role| role.server == server),
    )
}

/// Every role of a server, highest position value first as the database
/// orders them, and the base role among them: the system role that is not the
/// owner's, the first of those from the highest position value down, ties by
/// id.
#[derive(Debug, Clone)]
pub struct ServerRoles {
    pub roles: Vec<Role>,
    pub base: Option<Role>,
}

impl ServerRoles {
    pub fn load(db: &Connection, server: Id) -> rusqlite::Result<ServerRoles> {
        let mut statement = db.prepare(&format!(
            "SELECT {ROLE_COLUMNS} FROM \"ServerRoles\" r WHERE r.\"ServerId\" = ?1 ORDER BY r.\"Position\" DESC, r.\"Id\""
        ))?;
        let roles = statement
            .query_map([server], |row| Role::read(row, 0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let base = roles.iter().copied().find(|r| r.is_system && !r.is_owner);
        Ok(ServerRoles { roles, base })
    }

    pub fn by_id(&self) -> HashMap<Id, Role> {
        self.roles.iter().map(|role| (role.id, *role)).collect()
    }
}

/// The base role, which every member holds without a row and which people
/// join with: the system role below the owner's that ranks last.
pub fn find_member_role(db: &Connection, server: Id) -> rusqlite::Result<Option<Role>> {
    db.query_row(
        &format!(
            "SELECT {ROLE_COLUMNS} FROM \"ServerRoles\" r WHERE r.\"ServerId\" = ?1 AND r.\"IsSystem\" = 1 AND r.\"IsOwner\" = 0
             ORDER BY r.\"Position\" DESC, r.\"Id\" LIMIT 1"
        ),
        [server],
        |row| Role::read(row, 0),
    )
    .optional()
}

/// A membership with the role that ranks it, whether the account is enabled
/// or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub server: Id,
    pub user: Id,
    pub role: Role,
    pub owner_slot: Option<i64>,
    pub joined_at: Timestamp,
}

impl Member {
    pub fn is_owner(&self) -> bool {
        self.role.is_owner
    }

    pub fn load(db: &Connection, server: Id, user: Id) -> rusqlite::Result<Option<Member>> {
        db.query_row(
            &format!(
                "SELECT {ROLE_COLUMNS}, m.\"OwnerSlot\", m.\"JoinedAt\" FROM \"ServerMembers\" m
                 JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\" WHERE m.\"ServerId\" = ?1 AND m.\"UserId\" = ?2"
            ),
            params![server, user],
            |row| {
                Ok(Member {
                    server,
                    user,
                    role: Role::read(row, 0)?,
                    owner_slot: row.get(6)?,
                    joined_at: row.get(7)?,
                })
            },
        )
        .optional()
    }
}

fn stored_for(db: &Connection, server: Id, user: Id) -> rusqlite::Result<Vec<Id>> {
    let mut statement = db.prepare(
        "SELECT \"RoleId\" FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
    )?;
    statement
        .query_map(params![server, user], |row| row.get(0))?
        .collect()
}

/// The roles one member holds besides the base role, highest first, and the
/// base role.
pub fn held_of_member(
    db: &Connection,
    member: &Member,
) -> rusqlite::Result<(Vec<Role>, Option<Role>)> {
    let roles = ServerRoles::load(db, member.server)?;
    let stored = stored_for(db, member.server, member.user)?;
    let held = held_by(
        member.server,
        member.role.id,
        &stored,
        &roles.by_id(),
        roles.base.map(|base| base.id),
    );
    Ok((held, roles.base))
}

/// The server-wide permissions of one member, whether their account is
/// enabled or not. `None` for somebody who is not a member. The owner is who
/// holds the owner role as their top role.
pub fn permissions_of_member(
    db: &Connection,
    server: Id,
    user: Id,
) -> rusqlite::Result<Option<i64>> {
    let Some(member) = Member::load(db, server, user)? else {
        return Ok(None);
    };
    let (held, base) = held_of_member(db, &member)?;
    let permissions: Vec<i64> = held.iter().map(|role| role.permissions).collect();
    Ok(Some(super::evaluator::server_wide(
        member.role.is_owner,
        base.map_or(privileges::NONE, |role| role.permissions),
        &permissions,
    )))
}

/// The roles every member of one server holds besides the base role.
#[derive(Debug, Clone)]
pub struct HeldRoleTable {
    pub base: Option<Role>,
    by_member: HashMap<Id, Vec<Role>>,
}

impl HeldRoleTable {
    /// Every member of the server with the roles they hold besides the base
    /// role, highest first, disabled accounts included: the lists people see
    /// and the writers need everybody, not only those who may act.
    pub fn load(db: &Connection, server: Id) -> rusqlite::Result<HeldRoleTable> {
        let roles = ServerRoles::load(db, server)?;
        let by_id = roles.by_id();
        let mut stored: HashMap<Id, Vec<Id>> = HashMap::new();
        let mut statement = db.prepare(
            "SELECT \"UserId\", \"RoleId\" FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1",
        )?;
        for row in statement.query_map([server], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?))
        })? {
            let (user, role) = row?;
            stored.entry(user).or_default().push(role);
        }
        let mut statement = db.prepare(
            "SELECT \"UserId\", \"RoleId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1",
        )?;
        let mut by_member = HashMap::new();
        for row in statement.query_map([server], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?))
        })? {
            let (user, top_role) = row?;
            let held = held_by(
                server,
                top_role,
                stored.get(&user).map_or(&[], Vec::as_slice),
                &by_id,
                roles.base.map(|base| base.id),
            );
            by_member.insert(user, held);
        }
        Ok(HeldRoleTable {
            base: roles.base,
            by_member,
        })
    }

    pub fn members(&self) -> impl Iterator<Item = Id> + '_ {
        self.by_member.keys().copied()
    }

    /// How many members hold the role; the base role every member holds.
    pub fn count_holders(&self, role: Id) -> usize {
        if Some(role) == self.base.map(|base| base.id) {
            self.by_member.len()
        } else {
            self.by_member
                .values()
                .filter(|held| held.iter().any(|one| one.id == role))
                .count()
        }
    }

    /// The members who hold the role; the base role every member holds.
    pub fn holders(&self, role: Id) -> Vec<Id> {
        let mut holders: Vec<Id> = if Some(role) == self.base.map(|base| base.id) {
            self.by_member.keys().copied().collect()
        } else {
            self.by_member
                .iter()
                .filter(|(_, held)| held.iter().any(|one| one.id == role))
                .map(|(user, _)| *user)
                .collect()
        };
        holders.sort();
        holders
    }

    pub fn of(&self, user: Id) -> &[Role] {
        self.by_member.get(&user).map_or(&[], Vec::as_slice)
    }

    /// A member's server-wide permissions: everything for the owner, else the
    /// base role's and every held role's. Here the owner is whoever holds the
    /// owner role at all.
    pub fn permissions_of(&self, user: Id) -> i64 {
        let held = self.of(user);
        let permissions: Vec<i64> = held.iter().map(|role| role.permissions).collect();
        super::evaluator::server_wide(
            held.iter().any(|role| role.is_owner),
            self.base.map_or(privileges::NONE, |role| role.permissions),
            &permissions,
        )
    }
}

pub(super) fn missing_base_role() -> DataError {
    DataError::new("The server has no role for plain members.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(server: u8, id: u8, position: i64, is_owner: bool) -> Role {
        Role {
            id: Id::from_bytes([id; 16]),
            server: Id::from_bytes([server; 16]),
            position,
            is_system: false,
            is_owner,
            permissions: 0,
        }
    }

    #[test]
    fn rank_is_the_owner_role_then_the_position_then_the_id() {
        let owner = role(1, 9, 50, true);
        let low = role(1, 5, 3, false);
        let tie_a = role(1, 2, 3, false);
        let high = role(1, 7, 1, false);

        let order: Vec<u8> = ranked([low, tie_a, high, owner])
            .iter()
            .map(|r| r.id.as_bytes()[0])
            .collect();

        assert_eq!(order, [9, 7, 2, 5]);
    }

    #[test]
    fn equal_positions_do_not_outrank_each_other_and_nobody_outranks_the_owner_role() {
        let a = role(1, 1, 3, false);
        let b = role(1, 2, 3, false);
        let above = role(1, 3, 2, false);
        let owner = role(1, 4, 0, true);
        let elsewhere = role(2, 5, 9, false);

        assert!(!a.outranks(&b) && !b.outranks(&a));
        assert!(above.outranks(&a) && !a.outranks(&above));
        assert!(!a.outranks(&a));
        assert!(owner.outranks(&a) && !a.outranks(&owner) && !owner.outranks(&owner));
        assert!(!above.outranks(&elsewhere));
    }

    #[test]
    fn the_top_role_is_the_highest_held_or_else_the_base_role() {
        let base = role(1, 1, 9, false);
        let a = role(1, 2, 4, false);
        let b = role(1, 3, 2, false);

        assert_eq!(top(&[a, b], base).id, b.id);
        assert_eq!(top(&[], base).id, base.id);
    }

    #[test]
    fn held_roles_leave_out_the_base_role_unknown_ids_and_other_servers() {
        let base = role(1, 1, 9, false);
        let a = role(1, 2, 4, false);
        let b = role(1, 3, 2, false);
        let foreign = role(2, 4, 0, false);
        let all: HashMap<Id, Role> = [base, a, b, foreign].iter().map(|r| (r.id, *r)).collect();
        let ghost = Id::from_bytes([77; 16]);

        let held = held_by(
            a.server,
            a.id,
            &[b.id, base.id, ghost, foreign.id, a.id],
            &all,
            Some(base.id),
        );

        assert_eq!(held.iter().map(|r| r.id).collect::<Vec<_>>(), [b.id, a.id]);
    }
}
