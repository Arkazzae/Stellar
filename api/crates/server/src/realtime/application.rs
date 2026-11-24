//! The hub's side of a connection: what happens when one starts, what is
//! done with what it sends, and what happens when it ends.

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::Value;
use stellarr_hub::{Application, Call, HubError, Outbound};

use super::method::{Context, Table, Who};
use super::names::{follower_group, server_group, user_group};
use super::presence::OFFLINE;
use super::registry::LiveConnection;
use super::{Realtime, audience, registrations};
use crate::database::DbError;
use crate::state::AppState;
use crate::values::Id;

/// What a presence change says, as the clients read it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PresenceUpdate {
    pub user_id: Id,
    pub presence: String,
}

/// Tells the groups that `user`'s status changed. A connection that is in
/// several of them is told once.
pub(super) fn announce(realtime: &Realtime, groups: &[String], user: Id, presence: &str) {
    let update = PresenceUpdate {
        user_id: user,
        presence: presence.to_string(),
    };
    send_each(realtime, groups, &update);
}

fn send_each(realtime: &Realtime, groups: &[String], update: &PresenceUpdate) {
    match Outbound::single("PresenceChanged", update) {
        Ok(message) => {
            realtime.hub().send_to_groups(groups, &message);
        }
        Err(error) => tracing::error!(%error, "a presence change could not be written"),
    }
}

pub(super) struct AppHub {
    state: AppState,
    realtime: Arc<Realtime>,
    table: Table,
}

impl AppHub {
    pub(super) fn new(state: AppState, realtime: Arc<Realtime>) -> AppHub {
        AppHub {
            state,
            realtime,
            table: Table::new(registrations()),
        }
    }

    fn context(&self, call: &Call<Who>) -> Context {
        Context::new(self.state.clone(), Arc::clone(&self.realtime), call.clone())
    }
}

/// What a new connection needs to know of its account.
struct Account {
    servers: Vec<Id>,
    show_presence: bool,
}

fn internal(error: DbError) -> HubError {
    HubError::internal(error.to_string())
}

impl Application for AppHub {
    type Identity = Who;

    async fn connected(&self, call: &Call<Who>) -> Result<(), HubError> {
        let who = call.identity();
        let (user, connection) = (who.user, call.connection_id());
        self.realtime.registry().register(LiveConnection {
            connection_id: connection.to_string(),
            user,
            session: who.session,
            version: who.version,
        });
        let hub = self.realtime.hub();
        hub.add_to_group(connection, &user_group(user));

        // A connection follows one server at a time: the one the client asked
        // for, else the first it belongs to. That keeps one server's traffic
        // away from clients that are looking at another.
        let account = self
            .state
            .database()
            .read(move |db| -> Result<Account, DbError> {
                let mut statement =
                    db.prepare("SELECT \"ServerId\" FROM \"ServerMembers\" WHERE \"UserId\" = ?1")?;
                let servers = statement
                    .query_map([user], |row| row.get(0))?
                    .collect::<Result<Vec<Id>, _>>()?;
                let show_presence = db
                    .query_row(
                        "SELECT \"ShowPresence\" FROM \"UserPreferences\" WHERE \"UserId\" = ?1",
                        [user],
                        |row| row.get::<_, bool>(0),
                    )
                    .optional()?
                    .unwrap_or(true);
                Ok(Account {
                    servers,
                    show_presence,
                })
            })
            .await
            .map_err(internal)?;
        let requested = who
            .requested_server
            .as_deref()
            .and_then(Id::parse)
            .filter(|id| account.servers.contains(id));
        if let Some(active) = requested.or_else(|| account.servers.first().copied()) {
            hub.add_to_group(connection, &server_group(active));
            hub.add_to_group(connection, &follower_group(active, user));
        }

        let became_connected =
            self.realtime
                .presence()
                .connect(connection, user, account.show_presence);

        // Only the first connection changes what others see. Every connection
        // still gets its own answer, because two tabs may have loaded while
        // the account was offline and then race to connect.
        if became_connected {
            let groups = self
                .state
                .database()
                .read(move |db| -> Result<Vec<String>, DbError> {
                    Ok(audience::presence_groups(db, user)?)
                })
                .await
                .map_err(internal)?;
            let public = self.realtime.presence().public_presence_of(user, None);
            announce(&self.realtime, &groups, user, &public);
        }
        let own = PresenceUpdate {
            user_id: user,
            presence: self.realtime.presence().presence_of(user),
        };
        call.send(
            &Outbound::single("PresenceChanged", &own)
                .map_err(|e| HubError::internal(e.to_string()))?,
        );

        self.table.run_connected(&self.context(call)).await
    }

    async fn disconnected(&self, call: &Call<Who>, error: Option<&str>) {
        let who = call.identity();
        let connection = call.connection_id();
        self.realtime.registry().unregister(connection);
        self.realtime.throttle().forget(connection);
        let became_offline = self.realtime.presence().disconnect(connection);
        self.table
            .run_disconnected(&self.context(call), error)
            .await;

        if became_offline {
            let user = who.user;
            let groups = self
                .state
                .database()
                .read(move |db| -> Result<Vec<String>, DbError> {
                    Ok(audience::presence_groups(db, user)?)
                })
                .await;
            match groups {
                Ok(groups) => announce(&self.realtime, &groups, user, OFFLINE),
                Err(error) => {
                    tracing::warn!(%error, "an account went offline and its audience could not be read")
                }
            }
        }
    }

    fn has_method(&self, target: &str) -> bool {
        self.table.has_method(target)
    }

    async fn invoke(
        &self,
        call: &Call<Who>,
        target: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, HubError> {
        self.table
            .invoke(self.context(call), target, arguments)
            .await
    }
}
