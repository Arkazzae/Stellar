//! Who may see what on a server, as far as signing in and joining need it.
//!
//! The rules live in [`crate::permissions`]; these are the questions
//! registration and the welcome notice ask of them, under the names they have
//! always used.

use rusqlite::Connection;

use crate::permissions::{membership, roles, visibility};
use crate::values::Id;

pub use crate::permissions::privileges::{
    ADMINISTRATOR, ALL, BASE_ROLE_DEFAULTS, MANAGE_SERVER, VIEW_CHANNEL,
};

/// The first server ever created is the instance server.
pub fn instance_server(connection: &Connection) -> rusqlite::Result<Option<Id>> {
    membership::instance_server(connection)
}

/// What a member may do server-wide, before any override; none for somebody
/// who is not a member.
pub fn server_wide(connection: &Connection, server: Id, user: Id) -> rusqlite::Result<Option<i64>> {
    roles::permissions_of_member(connection, server, user)
}

/// Whether the person is an administrator of the instance: the owner of the
/// instance server, or a holder of a role with the manage-server permission,
/// whichever role of the several they hold it is on.
pub fn is_instance_admin(connection: &Connection, user: Id) -> rusqlite::Result<bool> {
    membership::is_instance_admin(connection, user)
}

/// Whether nobody is denied view anywhere on the server: every member then
/// sees every channel.
pub fn server_open(connection: &Connection, server: Id) -> rusqlite::Result<bool> {
    visibility::server_open(connection, server)
}

/// What somebody who has just joined would hold in the place named: the base
/// role alone, with no role or override of their own, as a membership starts.
pub fn newcomer_views(
    connection: &Connection,
    _server: Id,
    _section: Option<Id>,
    channel: Id,
) -> rusqlite::Result<bool> {
    visibility::newcomer_views(connection, channel)
}
