//! What a change of roles, of what a role grants or of an override tells
//! people, once it is committed: who now sees something else, and each
//! person's own server-wide permissions.
//!
//! What everybody sees is read before the change and again after it, in the
//! transaction of the change, so that the people told are the ones this change
//! altered and not another. On a server where nobody is denied view before or
//! after, each reading is one query and nobody is told anything.

use std::collections::HashSet;

use rusqlite::Connection;
use serde::Serialize;

use crate::events::{Audience, Event};
use crate::permissions::privileges;
use crate::permissions::roles::HeldRoleTable;
use crate::permissions::access_changed::{self, ViewChange};
use crate::state::AppState;
use crate::values::Id;

/// What to tell people, worked out inside the transaction.
#[derive(Debug, Default)]
pub(super) struct Announcement {
    /// Whose view of the server's channels changed.
    changed: Vec<Id>,
    /// Each member to tell, with their server-wide permissions now.
    notices: Vec<(Id, i64)>,
}

impl Announcement {

    /// Whoever sees something else, and each of `recipients` who is a member.
    pub(super) fn after(
        db: &Connection,
        server: Id,
        before: &ViewChange,
        recipients: &[Id],
    ) -> rusqlite::Result<Announcement> {
        let changed = before.finish(db)?;
        Ok(Announcement {
            changed,
            notices: notices(db, server, recipients)?,
        })
    }

    /// The people who see something else after the change, with nothing said
    /// to anybody of their permissions.
    pub(super) fn views_after(
        db: &Connection,
        server: Id,
        before: &ViewChange,
    ) -> rusqlite::Result<Announcement> {
        Announcement::after(db, server, before, &[])
    }

    /// Hands it to the hub: the people whose view changed first, then each
    /// recipient their own permissions.
    pub(super) fn publish(self, state: &AppState, server: Id) {
        access_changed::publish(state.events(), server, self.changed);
        for (user, permissions) in self.notices {
            state.events().publish(Event::new(
                Audience::Users(vec![user]),
                "PermissionsChanged",
                &PermissionsChanged {
                    server_id: server,
                    permissions: privileges::to_names(permissions),
                },
            ));
        }
    }
}

/// Each of `recipients` who is a member, once, with what they hold
/// server-wide, which every role they hold makes up.
fn notices(db: &Connection, server: Id, recipients: &[Id]) -> rusqlite::Result<Vec<(Id, i64)>> {
    if recipients.is_empty() {
        return Ok(Vec::new());
    }
    let table = HeldRoleTable::load(db, server)?;
    let members: HashSet<Id> = table.members().collect();
    let mut seen = HashSet::new();
    Ok(recipients
        .iter()
        .copied()
        .filter(|user| seen.insert(*user) && members.contains(user))
        .map(|user| (user, table.permissions_of(user)))
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionsChanged {
    server_id: Id,
    permissions: Vec<&'static str>,
}
