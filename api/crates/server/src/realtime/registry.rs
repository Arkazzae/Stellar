//! Which open connections belong to which session of which person, and the
//! version of the account's sessions each was let in with. What ends a
//! session finds its connections here.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::watch;

use crate::values::Id;

/// What the hub knows of one open connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveConnection {
    pub connection_id: String,
    pub user: Id,
    pub session: Id,
    /// The account's session version when the connection was let in.
    pub version: i64,
}

#[derive(Default)]
pub struct Registry {
    entries: Mutex<HashMap<String, LiveConnection>>,
    changes: watch::Sender<()>,
}

impl Registry {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, LiveConnection>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn register(&self, connection: LiveConnection) {
        self.entries()
            .insert(connection.connection_id.clone(), connection);
        self.changes.send_replace(());
    }

    pub(super) fn changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub fn unregister(&self, connection_id: &str) {
        if self.entries().remove(connection_id).is_some() {
            self.changes.send_replace(());
        }
    }

    pub fn snapshot(&self) -> Vec<LiveConnection> {
        self.entries().values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.entries().len()
    }

    /// Forgets the connections of a session and says which they were.
    pub fn take_session(&self, session: Id) -> Vec<String> {
        let mut entries = self.entries();
        let found: Vec<String> = entries
            .values()
            .filter(|entry| entry.session == session)
            .map(|entry| entry.connection_id.clone())
            .collect();
        for id in &found {
            entries.remove(id);
        }
        if !found.is_empty() {
            self.changes.send_replace(());
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(id: &str, user: Id, session: Id) -> LiveConnection {
        LiveConnection {
            connection_id: id.to_string(),
            user,
            session,
            version: 0,
        }
    }

    #[test]
    fn a_session_takes_only_its_own_connections_with_it() {
        let registry = Registry::default();
        let (maya, jo) = (Id::random(), Id::random());
        let (first, second) = (Id::random(), Id::random());
        registry.register(live("a", maya, first));
        registry.register(live("b", maya, first));
        registry.register(live("c", maya, second));
        registry.register(live("d", jo, Id::random()));

        let mut taken = registry.take_session(first);
        taken.sort();

        assert_eq!(taken, ["a", "b"]);
        assert_eq!(registry.count(), 2);
        assert!(registry.take_session(first).is_empty());
    }

    #[test]
    fn a_connection_that_ends_leaves_the_registry() {
        let registry = Registry::default();
        registry.register(live("a", Id::random(), Id::random()));

        registry.unregister("a");
        registry.unregister("a");

        assert!(registry.snapshot().is_empty());
    }
}
