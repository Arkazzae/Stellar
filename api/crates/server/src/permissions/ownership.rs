//! The roles members hold and who owns a server: the one writer of what a
//! member holds, the place a new role takes, and handing a server on.
//!
//! Whatever changes the roles of a member keeps the membership's top role the
//! highest role they hold, or the base role when they hold none, and stores
//! that top role as held too. All of it runs in the transaction of the caller,
//! which also writes the audit line of the change.

use rusqlite::{Connection, params};

use super::roles::{self, HeldRoleTable, Member, Role, held_of_member};
use crate::values::Id;

/// Where a new role ranks. The base role, which everybody holds, ranks below
/// every other role: a role somebody was given has to outrank those who were
/// given none, or its holders could moderate nobody.
///
/// Returns the position for a role about to be added: last among the roles
/// people are given. The base role is moved one place below it here, so the
/// caller adds the role in the same transaction.
pub fn place_new_role(db: &Connection, server: Id) -> rusqlite::Result<i64> {
    let base = roles::find_member_role(db, server)?;
    let base_id = base.map(|role| role.id);
    let last: Option<i64> = db.query_row(
        "SELECT MAX(\"Position\") FROM \"ServerRoles\" WHERE \"ServerId\" = ?1 AND (?2 IS NULL OR \"Id\" != ?2)",
        params![server, base_id],
        |row| row.get(0),
    )?;
    let position = last.unwrap_or(-1) + 1;
    if let Some(base) = base {
        db.execute(
            "UPDATE \"ServerRoles\" SET \"Position\" = ?1 WHERE \"Id\" = ?2",
            params![position + 1, base.id],
        )?;
    }
    Ok(position)
}

/// Adds a new membership. A role other than the base role, such as the owner
/// role of a new server, is stored as held as well.
pub fn admit(
    db: &Connection,
    server: Id,
    user: Id,
    role: Id,
    is_base_role: bool,
    owner_slot: Option<i64>,
    joined_at: crate::values::Timestamp,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO \"ServerMembers\" (\"ServerId\", \"UserId\", \"RoleId\", \"OwnerSlot\", \"JoinedAt\") VALUES (?1, ?2, ?3, ?4, ?5)",
        params![server, user, role, owner_slot, joined_at],
    )?;
    if !is_base_role {
        db.execute(
            "INSERT INTO \"ServerMemberRoles\" (\"ServerId\", \"UserId\", \"RoleId\") VALUES (?1, ?2, ?3)",
            params![server, user, role],
        )?;
    }
    Ok(())
}

/// Stores the roles the member holds as exactly `roles`, and their top role
/// with them; the base role, held by everybody, is left out.
pub fn set_roles(
    db: &Connection,
    server: Id,
    user: Id,
    wanted: &[Role],
    base: &Role,
) -> rusqlite::Result<Role> {
    let mut wanted_ids: Vec<Id> = Vec::new();
    let mut kept: Vec<Role> = Vec::new();
    for role in wanted.iter().filter(|role| role.id != base.id) {
        if !wanted_ids.contains(&role.id) {
            wanted_ids.push(role.id);
            kept.push(*role);
        }
    }
    let stored: Vec<Id> = {
        let mut statement = db.prepare(
            "SELECT \"RoleId\" FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
        )?;
        statement
            .query_map(params![server, user], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for row in stored.iter().filter(|row| !wanted_ids.contains(row)) {
        db.execute(
            "DELETE FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2 AND \"RoleId\" = ?3",
            params![server, user, row],
        )?;
    }
    for role in kept.iter().filter(|role| !stored.contains(&role.id)) {
        db.execute(
            "INSERT INTO \"ServerMemberRoles\" (\"ServerId\", \"UserId\", \"RoleId\") VALUES (?1, ?2, ?3)",
            params![server, user, role.id],
        )?;
    }
    let top = roles::top(&kept, *base);
    db.execute(
        "UPDATE \"ServerMembers\" SET \"RoleId\" = ?1 WHERE \"ServerId\" = ?2 AND \"UserId\" = ?3",
        params![top.id, server, user],
    )?;
    Ok(top)
}

/// Puts the top role of every member of the server right again, counting
/// `leaving`, a role about to be deleted, as held by nobody. Moving roles can
/// change which held role is the highest, so whatever changes their positions
/// runs this as well.
pub fn rank_again(db: &Connection, server: Id, leaving: Option<Id>) -> rusqlite::Result<()> {
    let table = HeldRoleTable::load(db, server)?;
    let base = table.base.ok_or_else(roles::missing_base_role)?;
    let mut statement =
        db.prepare("SELECT \"UserId\", \"RoleId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1")?;
    let members: Vec<(Id, Id)> = statement
        .query_map([server], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (user, current) in members {
        let held: Vec<Role> = table
            .of(user)
            .iter()
            .copied()
            .filter(|role| Some(role.id) != leaving)
            .collect();
        let top = roles::top(&held, base).id;
        if top != current {
            db.execute(
                "UPDATE \"ServerMembers\" SET \"RoleId\" = ?1 WHERE \"ServerId\" = ?2 AND \"UserId\" = ?3",
                params![top, server, user],
            )?;
        }
    }
    Ok(())
}

/// Moves the owner slot and the owner role from `owner` to `successor`, who
/// must both be members of the server. The old owner keeps any other role they
/// hold, and ranks by the highest of them, or else becomes a plain member; the
/// successor keeps theirs too. The slot is unique, so it is freed before it is
/// handed on, which is why this belongs in a transaction.
///
/// Nobody sets an override on the owner, who skips them; one left on the
/// successor would wake up again after a later hand-over, so the successor's
/// are deleted.
pub fn hand_over(
    db: &Connection,
    server: Id,
    owner: &Member,
    successor: &Member,
) -> rusqlite::Result<()> {
    let member_role = roles::find_member_role(db, server)?.ok_or_else(roles::missing_base_role)?;

    let owner_role = db.query_row(
        "SELECT r.\"Id\", r.\"ServerId\", r.\"Position\", r.\"IsSystem\", r.\"IsOwner\", r.\"Permissions\" FROM \"ServerRoles\" r WHERE r.\"Id\" = ?1",
        [owner.role.id],
        |row| Role::read(row, 0),
    )?;
    let (kept, _) = held_of_member(db, owner)?;
    let kept: Vec<Role> = kept
        .into_iter()
        .filter(|role| role.id != owner_role.id)
        .collect();
    set_roles(db, server, owner.user, &kept, &member_role)?;
    db.execute(
        "UPDATE \"ServerMembers\" SET \"OwnerSlot\" = NULL WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
        params![server, owner.user],
    )?;

    let (mut held, _) = held_of_member(db, successor)?;
    held.push(owner_role);
    set_roles(db, server, successor.user, &held, &member_role)?;
    db.execute(
        "UPDATE \"ServerMembers\" SET \"OwnerSlot\" = 1 WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
        params![server, successor.user],
    )?;
    db.execute(
        "DELETE FROM \"MemberPermissionOverrides\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
        params![server, successor.user],
    )?;
    Ok(())
}

/// Deletes the server in one statement. Its roles, memberships, invites,
/// emoji, soundboard clips, notifications and the like go with it by cascade;
/// its channels, and everything written in them, by the triggers of the
/// schema, since a channel has no foreign key to its server. Returns whether
/// there was such a server.
pub fn delete_server(db: &Connection, server: Id) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM \"Servers\" WHERE \"Id\" = ?1", [server])? > 0)
}
