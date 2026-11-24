//! Telling people that what they see on a server has changed.
//!
//! A change that may show somebody a channel or hide one from them (of roles,
//! of what a role grants, of overrides, of the section a channel is in, of who
//! owns the server) reads what everybody sees before it and again once it is
//! made, in the transaction of the change, and the people whose view differs
//! are sent `ChannelAccessChanged` after the commit. Their client then reads
//! its own channels again.
//!
//! ```ignore
//! let view = ViewChange::begin(tx, server)?;   // before anything is written
//! // ... the change ...
//! let told = view.finish(tx)?;                 // still in the transaction
//! // after the commit:
//! publish(state.events(), server, told);
//! ```
//!
//! On a server where nobody is denied view before or after, each reading is
//! one query and nobody is told anything.

use rusqlite::Connection;
use serde::Serialize;

use super::visibility::ServerVisibility;
use crate::events::{Audience, Event, EventPublisher};
use crate::values::Id;

/// What everybody saw before a change.
#[derive(Debug, Clone)]
pub struct ViewChange {
    before: ServerVisibility,
}

impl ViewChange {
    /// Reads what everybody sees now. Call it once the change may go on and
    /// before anything is written.
    pub fn begin(db: &Connection, server: Id) -> rusqlite::Result<ViewChange> {
        Ok(ViewChange {
            before: ServerVisibility::load(db, server)?,
        })
    }

    /// Reads it again and returns the people who see something else now.
    pub fn finish(&self, db: &Connection) -> rusqlite::Result<Vec<Id>> {
        let after = ServerVisibility::load(db, self.before.server)?;
        ServerVisibility::changed(db, &self.before, &after)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelAccessChanged {
    server_id: Id,
}

/// Tells each of `users`, on the connections that follow the server, that what
/// they see has changed. Nobody to tell sends nothing. Call it after the
/// commit.
pub fn publish(events: &dyn EventPublisher, server: Id, users: Vec<Id>) {
    if users.is_empty() {
        return;
    }
    events.publish(Event::new(
        Audience::Followers { server, users },
        "ChannelAccessChanged",
        &ChannelAccessChanged { server_id: server },
    ));
}
