//! Who may see what on a server, as far as signing in and joining need it:
//! the permissions of a member server-wide, whether a person who has just
//! joined would see a channel, and who sees a channel at all.
//!
//! This is the small part of the permission rules that registration reads; the
//! group of endpoints that manages roles and overrides owns the rest and may
//! take these functions over. The rules are the first server's. A role holds
//! permission bits. The owner may do everything, and so may anybody whose
//! roles grant Administrator. Anybody else starts from the base role's bits
//! united with those of every role held; a section's and then a channel's
//! overrides apply in turn, each removing the base role's denies and adding
//! its allows, then removing what any held role denies and adding what any
//! held role allows, and finally the member's own. In a channel, whoever ends
//! up without ViewChannel holds nothing.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::values::Id;

pub const MANAGE_SERVER: i64 = 1 << 0;
pub const ADMINISTRATOR: i64 = 1 << 8;
pub const VIEW_CHANNEL: i64 = 1 << 14;
/// Every permission there is.
pub const ALL: i64 = 0x7fff;
/// What a plain member of a new server holds: see channels, send messages,
/// attach files, add reactions, take a seat in voice.
pub const BASE_ROLE_DEFAULTS: i64 = (1 << 14) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12);

const OVERRIDABLE: i64 = ALL & !ADMINISTRATOR;

const TARGET_CHANNEL: i64 = 0;
const TARGET_SECTION: i64 = 1;

/// The first server ever created is the instance server.
pub fn instance_server(connection: &Connection) -> rusqlite::Result<Option<Id>> {
    connection
        .query_row(
            "SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
}

#[derive(Debug, Clone, Copy)]
struct Role {
    id: Id,
    position: i64,
    is_owner: bool,
    permissions: i64,
}

/// The roles of one server, and which of them is the base role.
struct Roles {
    by_id: HashMap<Id, Role>,
    base: Option<Role>,
}

impl Roles {
    fn load(connection: &Connection, server: Id) -> rusqlite::Result<Roles> {
        let mut statement = connection.prepare(
            "SELECT \"Id\", \"Position\", \"IsSystem\", \"IsOwner\", \"Permissions\" FROM \"ServerRoles\"
             WHERE \"ServerId\" = ?1 ORDER BY \"Position\" DESC, \"Id\"",
        )?;
        let mut by_id = HashMap::new();
        let mut base = None;
        for row in statement.query_map([server], |row| {
            Ok((
                Role {
                    id: row.get(0)?,
                    position: row.get(1)?,
                    is_owner: row.get(3)?,
                    permissions: row.get(4)?,
                },
                row.get::<_, bool>(2)?,
            ))
        })? {
            let (role, is_system) = row?;
            // The base role: the system role that is not the owner's, picked
            // from the highest position value down.
            if base.is_none() && is_system && !role.is_owner {
                base = Some(role);
            }
            by_id.insert(role.id, role);
        }
        Ok(Roles { by_id, base })
    }

    /// The roles a member holds besides the base role: the top role stored on
    /// the membership and every stored one.
    fn held(&self, top: Id, stored: &[Id]) -> Vec<Role> {
        let base = self.base.map(|role| role.id);
        let mut seen = HashSet::new();
        let mut held: Vec<Role> = stored
            .iter()
            .copied()
            .chain([top])
            .filter(|id| Some(*id) != base && seen.insert(*id))
            .filter_map(|id| self.by_id.get(&id).copied())
            .collect();
        held.sort_by(|a, b| {
            b.is_owner
                .cmp(&a.is_owner)
                .then(a.position.cmp(&b.position))
                .then(a.id.cmp(&b.id))
        });
        held
    }

    fn base_permissions(&self) -> i64 {
        self.base.map_or(0, |role| role.permissions)
    }
}

#[derive(Clone, Copy, Default)]
struct Masks {
    allow: i64,
    deny: i64,
}

impl Masks {
    fn granted(self) -> i64 {
        self.allow & !self.deny
    }

    fn apply(self, held: i64) -> i64 {
        (held & !(self.deny & OVERRIDABLE)) | (self.granted() & OVERRIDABLE)
    }
}

/// What one member holds at one place: the roles it has, with the overrides
/// of the places from the outside in.
struct Level {
    base: Masks,
    held: Vec<Masks>,
    member: Masks,
}

fn evaluate(is_owner: bool, base: i64, held: &[i64], levels: &[Level], at_channel: bool) -> i64 {
    if is_owner {
        return ALL;
    }
    let mut permissions = held.iter().fold(base, |all, one| all | one);
    if permissions & ADMINISTRATOR != 0 {
        return ALL;
    }
    for level in levels {
        permissions = level.base.apply(permissions);
        let denied = level.held.iter().fold(0, |all, masks| all | masks.deny);
        let allowed = level
            .held
            .iter()
            .fold(0, |all, masks| all | masks.granted());
        permissions = (permissions & !(denied & OVERRIDABLE)) | (allowed & OVERRIDABLE);
        permissions = level.member.apply(permissions);
    }
    if at_channel && permissions & VIEW_CHANNEL == 0 {
        0
    } else {
        permissions
    }
}

/// What a member may do server-wide, before any override.
pub fn server_wide(connection: &Connection, server: Id, user: Id) -> rusqlite::Result<Option<i64>> {
    let Some(top) = connection
        .query_row(
            "SELECT \"RoleId\" FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2",
            params![server, user],
            |row| row.get::<_, Id>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let roles = Roles::load(connection, server)?;
    let stored = stored_roles(connection, server, Some(user))?;
    let held = roles.held(top, stored.get(&user).map_or(&[], Vec::as_slice));
    let permissions: Vec<i64> = held.iter().map(|role| role.permissions).collect();
    Ok(Some(evaluate(
        held.iter().any(|role| role.is_owner),
        roles.base_permissions(),
        &permissions,
        &[],
        false,
    )))
}

/// Whether the person is an administrator of the instance: the owner of the
/// instance server, or a holder of a role with the manage-server permission,
/// whichever role of the several they hold it is on.
pub fn is_instance_admin(connection: &Connection, user: Id) -> rusqlite::Result<bool> {
    let Some(server) = instance_server(connection)? else {
        return Ok(false);
    };
    Ok(server_wide(connection, server, user)?
        .is_some_and(|permissions| permissions & MANAGE_SERVER != 0))
}

fn stored_roles(
    connection: &Connection,
    server: Id,
    user: Option<Id>,
) -> rusqlite::Result<HashMap<Id, Vec<Id>>> {
    let mut statement = connection.prepare(
        "SELECT \"UserId\", \"RoleId\" FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1 AND (?2 IS NULL OR \"UserId\" = ?2)",
    )?;
    let mut stored: HashMap<Id, Vec<Id>> = HashMap::new();
    for row in statement.query_map(params![server, user], |row| {
        Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?))
    })? {
        let (user, role) = row?;
        stored.entry(user).or_default().push(role);
    }
    Ok(stored)
}

fn role_masks(
    connection: &Connection,
    server: Id,
    kind: i64,
    target: Id,
    role: Id,
) -> rusqlite::Result<Masks> {
    Ok(connection
        .query_row(
            "SELECT \"AllowMask\", \"DenyMask\" FROM \"PermissionOverrides\"
             WHERE \"ServerId\" = ?1 AND \"TargetType\" = ?2 AND \"TargetId\" = ?3 AND \"RoleId\" = ?4",
            params![server, kind, target, role],
            |row| Ok(Masks { allow: row.get(0)?, deny: row.get(1)? }),
        )
        .optional()?
        .unwrap_or_default())
}

fn member_masks(
    connection: &Connection,
    server: Id,
    kind: i64,
    target: Id,
    user: Id,
) -> rusqlite::Result<Masks> {
    Ok(connection
        .query_row(
            "SELECT \"AllowMask\", \"DenyMask\" FROM \"MemberPermissionOverrides\"
             WHERE \"ServerId\" = ?1 AND \"TargetType\" = ?2 AND \"TargetId\" = ?3 AND \"UserId\" = ?4",
            params![server, kind, target, user],
            |row| Ok(Masks { allow: row.get(0)?, deny: row.get(1)? }),
        )
        .optional()?
        .unwrap_or_default())
}

/// Whether nobody is denied view anywhere on the server: the base role holds
/// it and no override of a channel or section that exists denies it. Every
/// member then sees every channel.
pub fn server_open(connection: &Connection, server: Id) -> rusqlite::Result<bool> {
    let roles = Roles::load(connection, server)?;
    if roles.base_permissions() & VIEW_CHANNEL == 0 {
        return Ok(false);
    }
    // An override can outlive its channel, which is deleted without it; such
    // a row denies nothing and is not counted.
    let denied = |table: &str| -> rusqlite::Result<bool> {
        connection.query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM \"{table}\" o WHERE o.\"ServerId\" = ?1 AND (o.\"DenyMask\" & {VIEW_CHANNEL}) != 0
                   AND ((o.\"TargetType\" = {TARGET_CHANNEL} AND EXISTS (SELECT 1 FROM \"Channels\" c WHERE c.\"Id\" = o.\"TargetId\" AND c.\"ServerId\" = ?1))
                     OR (o.\"TargetType\" = {TARGET_SECTION} AND EXISTS (SELECT 1 FROM \"Sections\" s WHERE s.\"Id\" = o.\"TargetId\" AND s.\"ServerId\" = ?1))))"
            ),
            [server],
            |row| row.get(0),
        )
    };
    Ok(!denied("PermissionOverrides")? && !denied("MemberPermissionOverrides")?)
}

/// What somebody who has just joined would hold in the place named: the base
/// role alone, with no role or override of their own, as a membership starts.
pub fn newcomer_views(
    connection: &Connection,
    server: Id,
    section: Option<Id>,
    channel: Id,
) -> rusqlite::Result<bool> {
    let roles = Roles::load(connection, server)?;
    let level = |kind: i64, target: Id| -> rusqlite::Result<Level> {
        let base = match roles.base {
            Some(base) => role_masks(connection, server, kind, target, base.id)?,
            None => Masks::default(),
        };
        Ok(Level {
            base,
            held: Vec::new(),
            member: Masks::default(),
        })
    };
    let mut levels = Vec::new();
    if let Some(section) = section {
        levels.push(level(TARGET_SECTION, section)?);
    }
    levels.push(level(TARGET_CHANNEL, channel)?);
    Ok(evaluate(false, roles.base_permissions(), &[], &levels, true) & VIEW_CHANNEL != 0)
}

/// Who an event about a channel reaches. None when everybody on the server
/// sees it (nobody is denied view there), so the event goes to the whole
/// server; otherwise the enabled members who see it.
pub fn channel_viewers(
    connection: &Connection,
    server: Id,
    channel: Id,
) -> rusqlite::Result<Option<Vec<Id>>> {
    let Some(section) = connection
        .query_row(
            "SELECT \"SectionId\" FROM \"Channels\" WHERE \"Id\" = ?1 AND \"ServerId\" = ?2",
            params![channel, server],
            |row| row.get::<_, Option<Id>>(0),
        )
        .optional()?
    else {
        return Ok(Some(Vec::new()));
    };
    if server_open(connection, server)? {
        return Ok(None);
    }
    let roles = Roles::load(connection, server)?;
    let stored = stored_roles(connection, server, None)?;
    let mut statement = connection.prepare(
        "SELECT m.\"UserId\", m.\"RoleId\" FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\"
         WHERE m.\"ServerId\" = ?1 AND u.\"IsDisabled\" = 0",
    )?;
    let members: Vec<(Id, Id)> = statement
        .query_map([server], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut viewers = Vec::new();
    for (user, top) in members {
        let held = roles.held(top, stored.get(&user).map_or(&[], Vec::as_slice));
        let mut levels = Vec::new();
        for (kind, target) in section
            .map(|s| (TARGET_SECTION, s))
            .into_iter()
            .chain([(TARGET_CHANNEL, channel)])
        {
            let base = match roles.base {
                Some(base) => role_masks(connection, server, kind, target, base.id)?,
                None => Masks::default(),
            };
            let mut held_masks = Vec::new();
            for role in &held {
                held_masks.push(role_masks(connection, server, kind, target, role.id)?);
            }
            levels.push(Level {
                base,
                held: held_masks,
                member: member_masks(connection, server, kind, target, user)?,
            });
        }
        let permissions: Vec<i64> = held.iter().map(|role| role.permissions).collect();
        let seen = evaluate(
            held.iter().any(|r| r.is_owner),
            roles.base_permissions(),
            &permissions,
            &levels,
            true,
        );
        if seen & VIEW_CHANNEL != 0 {
            viewers.push(user);
        }
    }
    Ok(Some(viewers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masks(allow: i64, deny: i64) -> Masks {
        Masks { allow, deny }
    }

    fn at_channel(base: i64, held: &[i64], levels: &[Level]) -> i64 {
        evaluate(false, base, held, levels, true)
    }

    #[test]
    fn the_owner_and_an_administrator_hold_everything_everywhere() {
        let deny_view = Level {
            base: masks(0, VIEW_CHANNEL),
            held: vec![],
            member: masks(0, VIEW_CHANNEL),
        };

        assert_eq!(
            evaluate(true, 0, &[], std::slice::from_ref(&deny_view), true),
            ALL
        );
        assert_eq!(at_channel(0, &[ADMINISTRATOR], &[deny_view]), ALL);
    }

    #[test]
    fn a_base_role_override_can_hide_a_channel_and_a_held_role_can_show_it_again() {
        let hidden = |held: Vec<Masks>| Level {
            base: masks(0, VIEW_CHANNEL),
            held,
            member: masks(0, 0),
        };

        assert_eq!(at_channel(BASE_ROLE_DEFAULTS, &[], &[hidden(vec![])]), 0);
        let shown = at_channel(
            BASE_ROLE_DEFAULTS,
            &[1 << 3],
            &[hidden(vec![masks(VIEW_CHANNEL, 0)])],
        );
        assert!(shown & VIEW_CHANNEL != 0);
    }

    #[test]
    fn the_members_own_override_is_the_last_word() {
        let level = Level {
            base: masks(0, 0),
            held: vec![masks(0, VIEW_CHANNEL)],
            member: masks(VIEW_CHANNEL, 0),
        };

        assert!(at_channel(BASE_ROLE_DEFAULTS, &[], &[level]) & VIEW_CHANNEL != 0);
    }

    #[test]
    fn administrator_cannot_be_taken_away_or_given_by_an_override() {
        let level = Level {
            base: masks(ADMINISTRATOR, 0),
            held: vec![],
            member: masks(0, 0),
        };

        assert_eq!(at_channel(VIEW_CHANNEL, &[], &[level]) & ADMINISTRATOR, 0);
    }

    #[test]
    fn the_constants_are_the_ones_stored_rows_have() {
        // `Member` is 24064 and `Owner` 32767 in a database the first server wrote.
        assert_eq!(BASE_ROLE_DEFAULTS, 24064);
        assert_eq!(ALL, 32767);
    }
}
