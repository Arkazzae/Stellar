//! What the evaluator reads for one server, loaded in a handful of statements:
//! the server's roles, the members asked for with the roles they hold, and the
//! role and member overrides of the sections and channels asked for. Any of
//! those members can then be evaluated at any of those places without another
//! statement, so deciding for one member in many channels, or for many members
//! in one channel, costs the same few statements.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params_from_iter};

use super::DataError;
use super::evaluator::{self, Input, Level, Masks};
use super::privileges::{NONE, VIEW_CHANNEL};
use super::roles::{self, Role, ServerRoles};
use crate::values::Id;

/// What an override is set on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Channel,
    Section,
}

impl Target {
    /// The number the database holds.
    pub const fn stored(self) -> i64 {
        match self {
            Target::Channel => 0,
            Target::Section => 1,
        }
    }

    fn from_stored(value: i64) -> Option<Target> {
        match value {
            0 => Some(Target::Channel),
            1 => Some(Target::Section),
            _ => None,
        }
    }
}

/// The sections and channels whose overrides a [`Snapshot`] reads.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    pub sections: Vec<Id>,
    pub channels: Vec<Id>,
}

impl Targets {
    pub fn none() -> Targets {
        Targets::default()
    }

    pub fn at(section: Option<Id>, channel: Option<Id>) -> Targets {
        Targets {
            sections: section.into_iter().collect(),
            channels: channel.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty() && self.channels.is_empty()
    }
}

/// A member as the policy sees them: their top role ranks them; the base role
/// and the roles they hold give them their permissions.
#[derive(Debug, Clone)]
pub struct Subject {
    pub user: Id,
    pub top: Role,
    pub base: Option<Role>,
    /// The roles held besides the base role, highest first.
    pub held: Vec<Role>,
}

impl Subject {
    pub fn is_owner(&self) -> bool {
        self.top.is_owner
    }

    fn base_permissions(&self) -> i64 {
        self.base.map_or(NONE, |role| role.permissions)
    }

    fn held_permissions(&self) -> Vec<i64> {
        self.held.iter().map(|role| role.permissions).collect()
    }

    /// The member's server-wide permissions, before any override.
    pub fn permissions(&self) -> i64 {
        evaluator::server_wide(
            self.is_owner(),
            self.base_permissions(),
            &self.held_permissions(),
        )
    }
}

type Key = (Target, Id, Id);

#[derive(Debug)]
pub struct Snapshot {
    pub server: Id,
    roles: Vec<Role>,
    base: Option<Role>,
    subjects: HashMap<Id, Subject>,
    role_overrides: HashMap<Key, Masks>,
    member_overrides: HashMap<Key, Masks>,
    loaded_sections: HashSet<Id>,
    loaded_channels: HashSet<Id>,
}

/// Up to this many values go into a statement as parameters; past it the
/// statement reads the whole server and the rows are picked out here, which
/// is cheaper than a statement of thousands of parameters.
const PARAMETERS: usize = 400;

fn placeholders(from: usize, count: usize) -> String {
    (from..from + count)
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(",")
}

impl Snapshot {
    /// Loads the server's roles, the members named by `users` (every member
    /// when `None`) and the overrides at `targets`. A disabled account is left
    /// out, as if it were no member.
    pub fn load(
        db: &Connection,
        server: Id,
        users: Option<&[Id]>,
        targets: Targets,
    ) -> rusqlite::Result<Snapshot> {
        let server_roles = ServerRoles::load(db, server)?;
        let by_id = server_roles.by_id();
        let base = server_roles.base;

        let wanted: Option<HashSet<Id>> = users.map(|users| users.iter().copied().collect());
        let in_sql = match users {
            Some(users) if users.len() <= PARAMETERS => Some(users),
            _ => None,
        };

        let mut sql = String::from(
            "SELECT m.\"UserId\", r.\"Id\", r.\"ServerId\", r.\"Position\", r.\"IsSystem\", r.\"IsOwner\", r.\"Permissions\"
             FROM \"ServerMembers\" m JOIN \"Users\" u ON u.\"Id\" = m.\"UserId\" JOIN \"ServerRoles\" r ON r.\"Id\" = m.\"RoleId\"
             WHERE m.\"ServerId\" = ?1 AND u.\"IsDisabled\" = 0",
        );
        if let Some(users) = in_sql {
            sql.push_str(&format!(
                " AND m.\"UserId\" IN ({})",
                placeholders(2, users.len())
            ));
        }
        let parameters: Vec<Id> = std::iter::once(server)
            .chain(in_sql.into_iter().flatten().copied())
            .collect();
        let mut tops: Vec<(Id, Role)> = Vec::new();
        {
            let mut statement = db.prepare(&sql)?;
            for row in statement.query_map(params_from_iter(&parameters), |row| {
                Ok((row.get::<_, Id>(0)?, Role::read(row, 1)?))
            })? {
                let (user, top) = row?;
                if wanted.as_ref().is_none_or(|wanted| wanted.contains(&user)) {
                    tops.push((user, top));
                }
            }
        }

        let mut sql = String::from(
            "SELECT \"UserId\", \"RoleId\" FROM \"ServerMemberRoles\" WHERE \"ServerId\" = ?1",
        );
        if let Some(users) = in_sql {
            sql.push_str(&format!(
                " AND \"UserId\" IN ({})",
                placeholders(2, users.len())
            ));
        }
        let mut stored: HashMap<Id, Vec<Id>> = HashMap::new();
        {
            let mut statement = db.prepare(&sql)?;
            for row in statement.query_map(params_from_iter(&parameters), |row| {
                Ok((row.get::<_, Id>(0)?, row.get::<_, Id>(1)?))
            })? {
                let (user, role) = row?;
                stored.entry(user).or_default().push(role);
            }
        }

        let base_id = base.map(|role| role.id);
        let subjects = tops
            .into_iter()
            .map(|(user, top)| {
                let held = roles::held_by(
                    server,
                    top.id,
                    stored.get(&user).map_or(&[], Vec::as_slice),
                    &by_id,
                    base_id,
                );
                (
                    user,
                    Subject {
                        user,
                        top,
                        base,
                        held,
                    },
                )
            })
            .collect();

        let (role_overrides, member_overrides) = if targets.is_empty() {
            (HashMap::new(), HashMap::new())
        } else {
            (
                load_overrides(db, server, &targets, "PermissionOverrides", "RoleId", None)?,
                load_overrides(
                    db,
                    server,
                    &targets,
                    "MemberPermissionOverrides",
                    "UserId",
                    wanted.as_ref(),
                )?,
            )
        };

        Ok(Snapshot {
            server,
            roles: roles::ranked(server_roles.roles),
            base,
            subjects,
            role_overrides,
            member_overrides,
            loaded_sections: targets.sections.into_iter().collect(),
            loaded_channels: targets.channels.into_iter().collect(),
        })
    }

    /// Every role of the server, highest first.
    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    /// The role every member holds without a row; none on a server that lost it.
    pub fn base_role(&self) -> Option<Role> {
        self.base
    }

    /// The members that were loaded, without disabled accounts.
    pub fn subjects(&self) -> impl Iterator<Item = &Subject> {
        self.subjects.values()
    }

    pub fn subject(&self, user: Id) -> Option<&Subject> {
        self.subjects.get(&user)
    }

    /// What the member may do in the channel or section named, or server-wide
    /// when neither is. A channel in a section names both.
    pub fn evaluate(
        &self,
        subject: &Subject,
        section: Option<Id>,
        channel: Option<Id>,
    ) -> rusqlite::Result<i64> {
        let mut levels = Vec::with_capacity(2);
        if let Some(section) = section {
            levels.push(self.level(subject, Target::Section, section)?);
        }
        if let Some(channel) = channel {
            levels.push(self.level(subject, Target::Channel, channel)?);
        }
        Ok(evaluator::evaluate(&Input {
            is_owner: subject.is_owner(),
            base: subject.base_permissions(),
            held: &subject.held_permissions(),
            levels: &levels,
            at_channel: channel.is_some(),
        }))
    }

    /// Whether the member sees the channel, which in a section names both.
    pub fn views(
        &self,
        subject: &Subject,
        section: Option<Id>,
        channel: Id,
    ) -> rusqlite::Result<bool> {
        Ok(self.evaluate(subject, section, Some(channel))? & VIEW_CHANNEL == VIEW_CHANNEL)
    }

    /// What somebody who has just joined would hold in the channel or section
    /// named: the base role alone, with no role or override of their own, as a
    /// membership starts. An invite and the welcome notice ask it before the
    /// membership exists.
    pub fn evaluate_newcomer(
        &self,
        section: Option<Id>,
        channel: Option<Id>,
    ) -> rusqlite::Result<i64> {
        let mut levels = Vec::with_capacity(2);
        let places = section
            .map(|id| (Target::Section, id))
            .into_iter()
            .chain(channel.map(|id| (Target::Channel, id)));
        for (target, id) in places {
            let base = match self.base {
                Some(base) => self.role_override(target, id, base.id)?,
                None => Masks::default(),
            };
            levels.push(Level {
                base,
                held: Vec::new(),
                member: Masks::default(),
            });
        }
        Ok(evaluator::evaluate(&Input {
            is_owner: false,
            base: self.base.map_or(NONE, |role| role.permissions),
            held: &[],
            levels: &levels,
            at_channel: channel.is_some(),
        }))
    }

    /// The overrides that concern the member at one section or channel.
    pub fn level(&self, subject: &Subject, target: Target, id: Id) -> rusqlite::Result<Level> {
        let base = match subject.base {
            Some(base) => self.role_override(target, id, base.id)?,
            None => Masks::default(),
        };
        let held = subject
            .held
            .iter()
            .map(|role| self.role_override(target, id, role.id))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Level {
            base,
            held,
            member: self.member_override(target, id, subject.user)?,
        })
    }

    /// A role's override at a place this snapshot loaded; none is the default.
    pub fn role_override(&self, target: Target, id: Id, role: Id) -> rusqlite::Result<Masks> {
        self.ensure_loaded(target, id)?;
        Ok(self
            .role_overrides
            .get(&(target, id, role))
            .copied()
            .unwrap_or_default())
    }

    /// A member's own override at a place this snapshot loaded, for a member
    /// it loaded.
    pub fn member_override(&self, target: Target, id: Id, user: Id) -> rusqlite::Result<Masks> {
        self.ensure_loaded(target, id)?;
        Ok(self
            .member_overrides
            .get(&(target, id, user))
            .copied()
            .unwrap_or_default())
    }

    /// An override that was never read would look like no override, which
    /// reads as a grant wherever it would have denied.
    fn ensure_loaded(&self, target: Target, id: Id) -> Result<(), DataError> {
        let loaded = match target {
            Target::Section => &self.loaded_sections,
            Target::Channel => &self.loaded_channels,
        };
        if loaded.contains(&id) {
            Ok(())
        } else {
            Err(DataError::new(format!(
                "The overrides of {target:?} {id} were not loaded."
            )))
        }
    }
}

/// The overrides of a table at the targets, keyed by place and by the role or
/// member they are for. Only the members asked for, when some were.
fn load_overrides(
    db: &Connection,
    server: Id,
    targets: &Targets,
    table: &str,
    owner_column: &str,
    only: Option<&HashSet<Id>>,
) -> rusqlite::Result<HashMap<Key, Masks>> {
    let few = targets.sections.len() + targets.channels.len() <= PARAMETERS;
    let mut sql = format!(
        "SELECT \"TargetType\", \"TargetId\", \"{owner_column}\", \"AllowMask\", \"DenyMask\" FROM \"{table}\" WHERE \"ServerId\" = ?1"
    );
    let mut parameters: Vec<Id> = vec![server];
    if few {
        let mut places = Vec::new();
        for (target, ids) in [
            (Target::Section, &targets.sections),
            (Target::Channel, &targets.channels),
        ] {
            if !ids.is_empty() {
                places.push(format!(
                    "(\"TargetType\" = {} AND \"TargetId\" IN ({}))",
                    target.stored(),
                    placeholders(parameters.len() + 1, ids.len())
                ));
                parameters.extend(ids);
            }
        }
        sql.push_str(&format!(" AND ({})", places.join(" OR ")));
    }
    let section_set: HashSet<Id> = targets.sections.iter().copied().collect();
    let channel_set: HashSet<Id> = targets.channels.iter().copied().collect();
    let mut found = HashMap::new();
    let mut statement = db.prepare(&sql)?;
    for row in statement.query_map(params_from_iter(&parameters), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Id>(1)?,
            row.get::<_, Id>(2)?,
            Masks::new(row.get(3)?, row.get(4)?),
        ))
    })? {
        let (kind, id, owner, masks) = row?;
        let Some(target) = Target::from_stored(kind) else {
            continue;
        };
        let in_scope = if few {
            true
        } else {
            match target {
                Target::Section => section_set.contains(&id),
                Target::Channel => channel_set.contains(&id),
            }
        };
        if in_scope && only.is_none_or(|only| only.contains(&owner)) {
            found.insert((target, id, owner), masks);
        }
    }
    Ok(found)
}
