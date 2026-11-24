//! Taking a person into a server that is not theirs yet, whichever door they
//! came through: a public server's own, or an invite.

use rusqlite::{Transaction, params};
use stellarr_messages::Messages;

use super::starter;
use super::users::User;
use super::welcome::{self, Welcome};
use crate::database::DbError;
use crate::permissions::roles;
use crate::values::{Id, Timestamp};

/// What came of asking to join.
#[derive(Debug)]
pub enum Admission {
    /// The person is banned from the server.
    Banned,
    /// The person is a member already; nothing was written.
    AlreadyMember,
    /// The server has no role for plain members to join with.
    NoMemberRole,
    /// The person is a member now, with the notice that greets them, if the
    /// server has a channel they see. The caller announces it once the
    /// transaction has been committed.
    Added(Option<Welcome>),
}

/// Makes `user` a member of `server` with the base role, in the transaction of
/// the caller, which also decides what else counts as joining.
pub fn admit(
    tx: &Transaction,
    server: Id,
    user: &User,
    now: Timestamp,
    messages: &Messages,
    invited_to: Option<Id>,
) -> Result<Admission, DbError> {
    let banned: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ServerBans\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2)",
        params![server, user.id],
        |row| row.get(0),
    )?;
    if banned {
        return Ok(Admission::Banned);
    }
    let member: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"ServerMembers\" WHERE \"ServerId\" = ?1 AND \"UserId\" = ?2)",
        params![server, user.id],
        |row| row.get(0),
    )?;
    if member {
        return Ok(Admission::AlreadyMember);
    }
    let Some(role) = roles::find_member_role(tx, server)? else {
        return Ok(Admission::NoMemberRole);
    };
    starter::admit(tx, server, user.id, role.id, false, true, now)?;
    Ok(Admission::Added(welcome::create_in(
        tx, server, user, now, messages, invited_to,
    )?))
}

/// Takes the invitations to the server out of the person's inbox, since they
/// have been answered. Returns how many there were.
pub fn clear_invitations(tx: &Transaction, server: Id, user: Id) -> rusqlite::Result<usize> {
    tx.execute(
        "DELETE FROM \"UserNotifications\"
         WHERE \"UserId\" = ?1 AND \"ServerId\" = ?2 AND \"Source\" = 'system' AND \"TargetInviteCode\" IS NOT NULL",
        params![user, server],
    )
}
