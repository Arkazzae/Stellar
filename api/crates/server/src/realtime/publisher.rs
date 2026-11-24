//! What handlers say to the hub, and who hears it: each [`Audience`] is a
//! set of groups or connections, and an event goes to each connection in it
//! once.

use serde::Deserialize;
use serde_json::value::RawValue;
use stellarr_hub::Outbound;

use super::Realtime;
use super::names::{follower_group, server_group, user_group};
use crate::events::{Audience, Event, EventPublisher, Presence, SESSION_ENDED};
use crate::values::Id;

#[derive(Deserialize)]
struct Ended {
    session: Id,
}

impl EventPublisher for Realtime {
    fn publish(&self, event: Event) {
        // A session that ended is not news for any client: the connections
        // it opened are closed, and that is all.
        if event.name == SESSION_ENDED {
            match serde_json::from_str::<Ended>(&event.payload) {
                Ok(ended) => {
                    self.end_session(ended.session);
                }
                Err(error) => tracing::warn!(%error, "a session end could not be read"),
            }
            return;
        }
        // The payload is JSON that the publisher wrote, and goes out as it
        // is: reading it into a value would reorder its members.
        let message = RawValue::from_string(event.payload)
            .map_err(|error| error.to_string())
            .and_then(|payload| {
                Outbound::single(&event.name, &*payload).map_err(|error| error.to_string())
            });
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::error!(event = %event.name, %error, "an event could not be written");
                return;
            }
        };
        let hub = self.hub();
        match &event.audience {
            Audience::Server(server) => {
                hub.send_to_group(&server_group(*server), &message);
            }
            // One send for all the groups, which gives a connection that is in
            // several of them (or a person named twice) the event once.
            Audience::Users(users) => {
                hub.send_to_groups(users.iter().map(|user| user_group(*user)), &message);
            }
            Audience::Followers { server, users } => {
                hub.send_to_groups(
                    users.iter().map(|user| follower_group(*server, *user)),
                    &message,
                );
            }
            Audience::Reach { servers, users } => {
                hub.send_to_groups(
                    servers
                        .iter()
                        .map(|server| server_group(*server))
                        .chain(users.iter().map(|user| user_group(*user))),
                    &message,
                );
            }
            Audience::Everyone => {
                hub.send_to_all(&message);
            }
            Audience::Connection(connection) => {
                hub.send_to_connection(connection, &message);
            }
        }
    }

    fn server_left(&self, user: Id, server: Id) {
        for connection in self.presence().connection_ids(user) {
            self.hub()
                .remove_from_group(&connection, &server_group(server));
            self.hub()
                .remove_from_group(&connection, &follower_group(server, user));
        }
    }
}

impl Presence for Realtime {
    fn presence_of(&self, user: Id) -> String {
        self.presence().presence_of(user)
    }

    fn public_presence_of(&self, user: Id, viewer: Option<Id>) -> String {
        self.presence().public_presence_of(user, viewer)
    }

    fn set_visibility(&self, user: Id, visible: bool) -> bool {
        self.presence().set_visibility(user, visible)
    }

    fn connection_ids(&self, user: Id) -> Vec<String> {
        self.presence().connection_ids(user)
    }
}
