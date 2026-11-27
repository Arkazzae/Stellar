//! Closes the connections of sessions that no longer count, and applies to
//! live sockets the access changes other processes make.
//!
//! The watcher sleeps while nothing changes. It wakes when a transaction
//! commits a row of the access journal (`RealtimeChanges`, see
//! [`crate::database::AccessSignal`]), when a connection opens, and when the
//! earliest session of a connection reaches its end. It then checks only the
//! sessions the change concerns, with the same check a request's cookie goes
//! through, and closes the connections of those that are revoked, expired,
//! of a disabled account, of an account whose sessions were bumped, or
//! without the second step the instance now asks for. Changes of membership,
//! roles, overrides and channels made outside this process (the CLI, an
//! operator's SQL) are applied as the handlers apply their own: people who
//! left a server leave its groups, voice seats without the right are freed,
//! and clients are told to read their channels again.

mod changes;
mod signal;

use super::Realtime;
use super::registry::LiveConnection;
use crate::auth::session::{self, ActiveSessions};
use crate::background::{Stop, Worker};
use crate::database::{Database, DbError};
use crate::events::{Audience, Event};
use crate::state::AppState;
use crate::values::{Id, Timestamp, json_ids};
use changes::Changes;
use signal::{SignalListener, Wake};
use std::collections::HashSet;
use std::time::Duration;

pub fn workers() -> Vec<Worker> {
    vec![Worker::new("realtime-sessions", run)]
}

/// When the next read of the journal happens.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    /// At once, after the first connection opened. Rows pruned while nobody
    /// was connected concerned no live socket.
    Resume,
    /// At once, after a pass that failed.
    Retry,
    /// After the next signal.
    Wait,
}

async fn run(state: AppState, stop: Stop) {
    let Some(hub) = state.realtime() else { return };
    let mut registry = hub.registry().changes();
    let mut monitor = Monitor {
        cursor: state.database().initial_realtime_cursor,
        ..Monitor::default()
    };
    while !stop.is_stopping() {
        registry.borrow_and_update();
        if hub.registry().count() == 0 {
            tokio::select! {
                _ = registry.changed() => continue,
                () = stop.stopped() => return,
            }
        }
        // Listening starts before the first read, so a change committed in
        // between wakes the next one.
        let mut listener = SignalListener::new(state.database());
        let mut read = Read::Resume;
        registry.mark_changed();
        while hub.registry().count() != 0 {
            let journal = next_changes(state.database(), &mut listener, monitor.cursor, read);
            tokio::pin!(journal);
            loop {
                // A writer can hold its transaction open. Keep admitting new
                // connections and enforcing deadlines while the journal waits.
                let batch = tokio::select! {
                    () = stop.stopped() => return,
                    _ = registry.changed() => None,
                    () = wait_for_expiry(monitor.next_expiry) => None,
                    result = &mut journal => Some(result),
                };
                if hub.registry().count() == 0 {
                    break;
                }
                let read_journal = batch.is_some();
                let changes = batch.unwrap_or_else(|| {
                    Ok(Changes {
                        cursor: monitor.cursor,
                        ..Changes::default()
                    })
                });
                let result = match changes {
                    Ok(changes) => monitor.apply(&state, hub, changes).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    tracing::warn!(target: "Stellarr.Api.Realtime", %error, "Could not reconcile realtime access");
                    if !stop.sleep(Duration::from_millis(250)).await {
                        return;
                    }
                    registry.mark_changed();
                    if read == Read::Wait {
                        read = Read::Retry;
                    }
                    break;
                }
                if read_journal {
                    read = Read::Wait;
                    break;
                }
            }
        }
        monitor.active.clear();
        monitor.next_expiry = None;
    }
}

async fn next_changes(
    db: &Database,
    listener: &mut SignalListener,
    cursor: i64,
    mut read: Read,
) -> Result<Changes, DbError> {
    loop {
        // Another process signals while it holds the write lock. Taking the
        // lock sees its final commit or rollback, a crashed writer's too.
        // This process signals after its commit, which any snapshot sees.
        let locked = read != Read::Wait || listener.changed().await == Wake::File;
        let resume = read == Read::Resume;
        read = Read::Wait;
        let origin = db.realtime_origin;
        let work = move |db: &rusqlite::Connection| Changes::read(db, cursor, origin, resume);
        let changes = if locked {
            db.write(move |tx| work(tx)).await?
        } else {
            db.read(work).await?
        };
        if changes.cursor != cursor {
            return Ok(changes);
        }
    }
}

async fn wait_for_expiry(expiry: Option<Timestamp>) {
    match expiry {
        Some(at) => tokio::time::sleep(Timestamp::now().until(at)).await,
        None => std::future::pending().await,
    }
}

#[derive(Default)]
struct Monitor {
    cursor: i64,
    active: ActiveSessions,
    next_expiry: Option<Timestamp>,
}

impl Monitor {
    async fn apply(
        &mut self,
        state: &AppState,
        hub: &Realtime,
        changes: Changes,
    ) -> Result<(), DbError> {
        let now = Timestamp::now();
        let connections = hub.registry().snapshot();
        let ids = self.due(&connections, &changes, now);
        if !ids.is_empty() {
            let requested: Vec<_> = ids.iter().copied().collect();
            let active = state
                .database()
                .read(move |db| session::active_sessions(db, &requested, Timestamp::now()))
                .await?;
            self.active.retain(|(_, id, _), _| !ids.contains(id));
            self.active.extend(active);
            for entry in &connections {
                if ids.contains(&entry.session)
                    && !self
                        .active
                        .contains_key(&(entry.user, entry.session, entry.version))
                {
                    hub.end_connection(&entry.connection_id);
                }
            }
        }
        self.next_expiry = self.active.values().min().copied();
        let cursor = changes.cursor;
        if changes.gap {
            // Old ACL rows have been pruned. Rejoining rebuilds routing groups
            // and voice seats from current membership. Valid cookies can reconnect.
            hub.hub().close_all();
        } else {
            apply_access(state, changes).await?;
        }
        self.cursor = cursor;
        Ok(())
    }

    fn due(
        &mut self,
        connections: &[LiveConnection],
        changes: &Changes,
        now: Timestamp,
    ) -> HashSet<Id> {
        let live: HashSet<_> = connections
            .iter()
            .map(|c| (c.user, c.session, c.version))
            .collect();
        self.active.retain(|key, _| live.contains(key));
        live.into_iter()
            .filter_map(|key @ (user, id, _)| {
                (changes.all_sessions
                    || changes.sessions.contains(&id)
                    || changes.users.contains(&user)
                    || self.active.get(&key).is_none_or(|expires| now >= *expires))
                .then_some(id)
            })
            .collect()
    }
}

async fn apply_access(state: &AppState, changes: Changes) -> Result<(), DbError> {
    for (server, access) in changes.access {
        let members: Vec<_> = access.members.into_iter().collect();
        if !members.is_empty() {
            let checked = json_ids(&members);
            let removed = state
                .database()
                .read(move |db| -> rusqlite::Result<Vec<_>> {
                    let mut stmt = db.prepare_cached(
                        "SELECT value FROM json_each(?1) WHERE NOT EXISTS (
                           SELECT 1 FROM ServerMembers WHERE ServerId = ?2 AND UserId = value
                         )",
                    )?;
                    stmt.query_map(rusqlite::params![checked, server], |row| row.get(0))?
                        .collect()
                })
                .await?;
            for user in removed {
                state.events().server_left(user, server);
            }
        }
        crate::voice::unseat_at(
            state,
            server,
            Some(access.channels.into_iter().collect()),
            Some(access.latest),
        )
        .await?
        .announce(state)
        .await;
        let audience = if access.everyone {
            Audience::Server(server)
        } else {
            Audience::Followers {
                server,
                users: members,
            }
        };
        state.events().publish(Event::new(
            audience,
            "ChannelAccessChanged",
            &serde_json::json!({"serverId": server}),
        ));
    }
    Ok(())
}
