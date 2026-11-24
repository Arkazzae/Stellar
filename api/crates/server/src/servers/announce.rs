//! What joining a server and answering an invitation tell the hub.

use serde::Serialize;

use crate::database::DbError;
use crate::events::{Audience, Event};
use crate::instance::welcome::{self, Welcome};
use crate::state::AppState;
use crate::values::Id;

/// Invitations to a server came or went: no single entry changed, so the
/// payload has no `sequence` at all, and clients tell an absent property from
/// a null one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InvitationsChanged {
    server_id: Id,
    global: bool,
}

/// One invitation arrived in an inbox.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InboxChanged {
    server_id: Id,
    sequence: Option<i64>,
    global: bool,
}

/// Tells a person's connections to reload the inbox they see on every server,
/// after invitations to `server` came or went.
pub fn invitations_changed(user: Id, server: Id) -> Event {
    Event::new(
        Audience::Users(vec![user]),
        "NotificationInboxChanged",
        &InvitationsChanged {
            server_id: server,
            global: true,
        },
    )
}

/// Tells a person's connections that the invitation numbered `sequence` is in
/// their inbox.
pub fn invitation_arrived(user: Id, server: Id, sequence: i64) -> Event {
    Event::new(
        Audience::Users(vec![user]),
        "NotificationInboxChanged",
        &InboxChanged {
            server_id: server,
            sequence: Some(sequence),
            global: true,
        },
    )
}

/// Tells the members who are online and see the channel about the notice that
/// greets a newcomer, once it has been committed. The membership is already
/// there, so a notice that cannot be announced is logged and nothing more.
pub async fn welcomed(state: &AppState, welcome: Option<Welcome>) {
    let Some(welcome) = welcome else {
        return;
    };
    let announced = state
        .database()
        .read(move |connection| -> Result<Event, DbError> {
            Ok(welcome::announcement(connection, &welcome)?)
        })
        .await;
    match announced {
        Ok(event) => state.events().publish(event),
        Err(error) => tracing::warn!("a welcome could not be announced: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_inbox_events_have_the_members_the_clients_read_and_in_their_order() {
        let (user, server) = (Id::random(), Id::random());

        let gone = invitations_changed(user, server);
        let arrived = invitation_arrived(user, server, 7);

        // No `sequence` at all: no single entry changed, and clients tell an
        // absent property from a null one.
        assert_eq!(gone.payload, format!("{{\"serverId\":\"{server}\",\"global\":true}}"));
        assert_eq!(arrived.payload, format!("{{\"serverId\":\"{server}\",\"sequence\":7,\"global\":true}}"));
        assert_eq!(gone.name, "NotificationInboxChanged");
        assert_eq!(gone.audience, Audience::Users(vec![user]));
    }
}
