//! What a run of journal rows asks of the watcher. Each API process keeps its
//! own position in the journal, and reading it consumes nothing.

use crate::auth::policy::{ADMINS, SecurityPolicy};
use crate::permissions::membership;
use crate::values::{Id, json_ids};
use rusqlite::{Connection, Row};
use std::collections::{HashMap, HashSet};

/// A server where another process changed who may see or join what.
#[derive(Default)]
pub(super) struct ServerAccess {
    /// The newest of its rows, which voice seats placed later outrank.
    pub latest: i64,
    pub members: HashSet<Id>,
    pub channels: HashSet<Id>,
    pub everyone: bool,
}

#[derive(Default)]
pub(super) struct Changes {
    pub cursor: i64,
    /// Rows the watcher did not read are gone, or one could not be read.
    pub gap: bool,
    pub all_sessions: bool,
    pub sessions: HashSet<Id>,
    pub users: HashSet<Id>,
    pub access: HashMap<Id, ServerAccess>,
}

/// Rows that need one more look at the database once all are read.
#[derive(Default)]
struct Pending {
    roles: HashMap<Id, HashSet<Id>>,
    base_roles: HashSet<Id>,
    policies: HashSet<Id>,
}

impl Changes {
    fn server_access(&mut self, server: Id, seq: i64) -> &mut ServerAccess {
        let access = self.access.entry(server).or_default();
        access.latest = seq;
        access
    }

    /// The rows after `after`. With `resume`, rows pruned before the first
    /// one still there are not a gap.
    pub fn read(db: &Connection, after: i64, origin: Id, resume: bool) -> rusqlite::Result<Self> {
        let mut changes = Self {
            cursor: after,
            ..Self::default()
        };
        let mut pending = Pending::default();
        let mut stmt = db.prepare_cached(
            "SELECT Seq, Kind, Subject, ServerId, Origin FROM RealtimeChanges WHERE Seq > ?1 ORDER BY Seq",
        )?;
        let mut rows = stmt.query([after])?;
        while let Some(row) = rows.next()? {
            let seq: i64 = row.get(0)?;
            if seq != changes.cursor + 1 && !(resume && changes.cursor == after) {
                changes.gap = true;
            }
            changes.cursor = seq;
            // A row written by hand that does not read as one of ours must
            // not stop the watcher for good: everything is checked instead.
            if !changes
                .note(&mut pending, row, seq, origin)
                .unwrap_or(false)
            {
                changes.gap = true;
            }
        }
        if (!pending.roles.is_empty() || !pending.policies.is_empty())
            && let Some(instance) = membership::instance_server(db)?
        {
            changes.all_sessions |= pending.policies.contains(&instance);
            if !changes.all_sessions
                && let Some(ids) = pending.roles.get(&instance)
                && SecurityPolicy::load(db)?.mfa_requirement == ADMINS
            {
                if pending.base_roles.contains(&instance) {
                    // Every member holds the base role, including implicit holders.
                    changes.all_sessions = true;
                } else {
                    let mut holders = db.prepare_cached(
                        "SELECT UserId FROM ServerMembers WHERE RoleId IN (SELECT value FROM json_each(?1)) AND ServerId = ?2
                         UNION SELECT UserId FROM ServerMemberRoles WHERE RoleId IN (SELECT value FROM json_each(?1)) AND ServerId = ?2"
                    )?;
                    let ids: Vec<_> = ids.iter().copied().collect();
                    for user in holders
                        .query_map(rusqlite::params![json_ids(&ids), instance], |r| {
                            r.get::<_, Id>(0)
                        })?
                    {
                        changes.users.insert(user?);
                    }
                }
            }
        }
        changes.all_sessions |= changes.gap;
        Ok(changes)
    }

    /// Adds one row; false when its kind is unknown.
    fn note(
        &mut self,
        pending: &mut Pending,
        row: &Row<'_>,
        seq: i64,
        origin: Id,
    ) -> rusqlite::Result<bool> {
        let kind: String = row.get(1)?;
        // The handlers of this process already told the clients and freed
        // the seats; another process's rows are applied here.
        let external = row.get::<_, Option<Id>>(4)? != Some(origin);
        match kind.as_str() {
            "session" => {
                self.sessions.insert(row.get(2)?);
            }
            "user" => {
                self.users.insert(row.get(2)?);
            }
            "member" => {
                let user = row.get(2)?;
                self.users.insert(user);
                if external {
                    self.server_access(row.get(3)?, seq).members.insert(user);
                }
            }
            "role" | "base-role" => {
                let server = row.get(3)?;
                pending.roles.entry(server).or_default().insert(row.get(2)?);
                if kind == "base-role" {
                    pending.base_roles.insert(server);
                }
                if external {
                    self.server_access(server, seq).everyone = true;
                }
            }
            "policy" => {
                pending.policies.insert(row.get::<_, Id>(3)?);
            }
            "instance" => {
                self.all_sessions = true;
            }
            "channel" | "access" if external => {
                let access = self.server_access(row.get(3)?, seq);
                if kind == "channel" {
                    access.channels.insert(row.get(2)?);
                    access.everyone = true;
                } else if let Some(user) = row.get::<_, Option<Id>>(2)? {
                    access.members.insert(user);
                } else {
                    access.everyone = true;
                }
            }
            "access" | "channel" => {}
            _ => return Ok(false),
        }
        Ok(true)
    }
}
