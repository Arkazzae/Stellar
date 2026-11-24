//! The connections of the hub, the groups they belong to, and sending.
//!
//! Sending never waits: a message goes into the queue of each connection it
//! is for and the call returns. Each queue is bounded; a client that cannot
//! keep up is closed (and starts over) instead of holding up whoever sends,
//! or growing without limit.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Notify, watch};

use crate::config::Config;
use crate::message;

/// A message for clients, serialised once however many it goes to.
#[derive(Debug, Clone)]
pub struct Outbound(Bytes);

impl Outbound {
    /// A call of `target` on the client with these arguments, which must
    /// serialise as a JSON array (a tuple, a `Vec`, a slice).
    pub fn new<A: Serialize + ?Sized>(
        target: &str,
        arguments: &A,
    ) -> Result<Self, serde_json::Error> {
        message::invocation(target, arguments).map(Self)
    }

    /// A call with the one argument most events have.
    pub fn single<T: Serialize + ?Sized>(
        target: &str,
        argument: &T,
    ) -> Result<Self, serde_json::Error> {
        Self::new(target, &(argument,))
    }

    /// A call without arguments.
    pub fn bare(target: &str) -> Self {
        Self(message::invocation(target, &Vec::<Value>::new()).expect("an empty array serialises"))
    }

    /// The record as it goes on the wire.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn bytes(&self) -> &Bytes {
        &self.0
    }
}

/// Why and how the server ends a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseRequest {
    /// What the client is told; `None` for a close that is not a failure.
    pub error: Option<String>,
    /// Whether the client library starts a new connection by itself. After
    /// a close that does not allow it, it stays closed.
    pub allow_reconnect: bool,
}

#[derive(Default)]
struct OutboxState {
    queue: VecDeque<Bytes>,
    bytes: usize,
}

/// The messages waiting to be written to one client.
struct Outbox {
    state: Mutex<OutboxState>,
    wake: Notify,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while the lock was held cannot leave these tables half-written
    // in a way that matters more than losing every other connection too.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What the hub keeps of one connection.
pub(crate) struct Shared {
    id: Arc<str>,
    outbox: Outbox,
    close: watch::Sender<Option<CloseRequest>>,
    max_messages: usize,
    max_bytes: usize,
}

impl Shared {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Asks for the connection to be closed. The first request wins: a
    /// connection that is already closing does not change its mind.
    pub(crate) fn request_close(&self, request: CloseRequest) {
        self.close.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(request);
            true
        });
    }

    pub(crate) fn subscribe_close(&self) -> watch::Receiver<Option<CloseRequest>> {
        self.close.subscribe()
    }

    pub(crate) fn close_request(&self) -> Option<CloseRequest> {
        self.close.borrow().clone()
    }

    /// Queues a record. False when the connection is closing or has just
    /// fallen too far behind, in which case it is closed.
    pub(crate) fn push(&self, record: &Bytes) -> bool {
        if self.close.borrow().is_some() {
            return false;
        }
        let mut state = lock(&self.outbox.state);
        // One message bigger than the whole allowance still goes through
        // when nothing else waits: refusing it would close a connection that
        // is keeping up.
        let over = !state.queue.is_empty()
            && (state.queue.len() >= self.max_messages
                || state.bytes + record.len() > self.max_bytes);
        if over {
            drop(state);
            tracing::warn!(connection = %self.id, "a client fell too far behind; closing its connection");
            self.request_close(CloseRequest {
                error: Some(
                    "The connection was closed because it could not keep up with the server."
                        .into(),
                ),
                allow_reconnect: true,
            });
            return false;
        }
        state.bytes += record.len();
        state.queue.push_back(record.clone());
        drop(state);
        self.outbox.wake.notify_one();
        true
    }

    /// Waits for messages and takes them, as many as fit in `limit` bytes
    /// (at least one). Safe to drop while waiting: nothing is taken until it
    /// returns.
    pub(crate) async fn take(&self, out: &mut Vec<Bytes>, limit: usize) {
        loop {
            {
                let mut state = lock(&self.outbox.state);
                let mut taken = 0;
                while let Some(front) = state.queue.front() {
                    if taken > 0 && taken + front.len() > limit {
                        break;
                    }
                    let record = state.queue.pop_front().expect("front was just seen");
                    taken += record.len();
                    state.bytes -= record.len();
                    out.push(record);
                }
                if taken > 0 {
                    // Leave the wake-up for the next call if more is waiting.
                    if !state.queue.is_empty() {
                        self.outbox.wake.notify_one();
                    }
                    return;
                }
            }
            self.outbox.wake.notified().await;
        }
    }
}

/// Completes once a close has been asked for. A function of its own so that
/// the borrow `wait_for` returns is gone before anything else is awaited.
pub(crate) async fn close_requested(closing: &mut watch::Receiver<Option<CloseRequest>>) {
    let _ = closing.wait_for(Option::is_some).await;
}

#[derive(Default)]
struct State {
    connections: HashMap<Arc<str>, Entry>,
    groups: HashMap<String, HashSet<Arc<str>>>,
}

struct Entry {
    shared: Arc<Shared>,
    groups: HashSet<String>,
}

struct Inner {
    state: Mutex<State>,
    max_messages: usize,
    max_bytes: usize,
}

/// The live connections and their groups, and what the application sends
/// through them. Cheap to clone; every clone is the same hub.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Hub {
    pub fn new(config: &Config) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::default(),
                max_messages: config.max_queued_messages,
                max_bytes: config.max_queued_bytes,
            }),
        }
    }

    /// Adds a connection. Its id must not be in use.
    pub(crate) fn register(&self, id: Arc<str>) -> Arc<Shared> {
        let (close, _) = watch::channel(None);
        let shared = Arc::new(Shared {
            id: id.clone(),
            outbox: Outbox {
                state: Mutex::default(),
                wake: Notify::new(),
            },
            close,
            max_messages: self.inner.max_messages,
            max_bytes: self.inner.max_bytes,
        });
        let mut state = lock(&self.inner.state);
        state.connections.insert(
            id,
            Entry {
                shared: shared.clone(),
                groups: HashSet::new(),
            },
        );
        shared
    }

    /// Removes a connection and takes it out of every group.
    pub(crate) fn unregister(&self, id: &str) {
        let mut state = lock(&self.inner.state);
        let Some(entry) = state.connections.remove(id) else {
            return;
        };
        for group in entry.groups {
            if let Some(members) = state.groups.get_mut(&group) {
                members.remove(id);
                if members.is_empty() {
                    state.groups.remove(&group);
                }
            }
        }
    }

    pub fn connection_count(&self) -> usize {
        lock(&self.inner.state).connections.len()
    }

    pub fn is_connected(&self, connection_id: &str) -> bool {
        lock(&self.inner.state)
            .connections
            .contains_key(connection_id)
    }

    /// The ids of the connections in a group, in no particular order.
    pub fn group_members(&self, group: &str) -> Vec<String> {
        let state = lock(&self.inner.state);
        state
            .groups
            .get(group)
            .map(|members| members.iter().map(|id| id.to_string()).collect())
            .unwrap_or_default()
    }

    /// Puts a connection in a group, which comes into being with its first
    /// member. False when there is no such connection, as when it has just
    /// gone.
    pub fn add_to_group(&self, connection_id: &str, group: &str) -> bool {
        let mut state = lock(&self.inner.state);
        let State {
            connections,
            groups,
        } = &mut *state;
        let Some((id, entry)) = connections.get_key_value(connection_id) else {
            return false;
        };
        if entry.groups.contains(group) {
            return true;
        }
        groups
            .entry(group.to_owned())
            .or_default()
            .insert(id.clone());
        connections
            .get_mut(connection_id)
            .expect("it was just found")
            .groups
            .insert(group.to_owned());
        true
    }

    /// Takes a connection out of a group; the group goes with its last member.
    pub fn remove_from_group(&self, connection_id: &str, group: &str) -> bool {
        let mut state = lock(&self.inner.state);
        let State {
            connections,
            groups,
        } = &mut *state;
        let Some(entry) = connections.get_mut(connection_id) else {
            return false;
        };
        let was_in = entry.groups.remove(group);
        if was_in && let Some(members) = groups.get_mut(group) {
            members.remove(connection_id);
            if members.is_empty() {
                groups.remove(group);
            }
        }
        was_in
    }

    /// Ends a connection. The client is told, and does not start over.
    pub fn abort(&self, connection_id: &str) -> bool {
        self.close(
            connection_id,
            CloseRequest {
                error: None,
                allow_reconnect: false,
            },
        )
    }

    /// Ends a connection with a reason the client is told and, when
    /// `allow_reconnect` is set, lets it start over at once.
    pub fn close(&self, connection_id: &str, request: CloseRequest) -> bool {
        let shared = lock(&self.inner.state)
            .connections
            .get(connection_id)
            .map(|entry| entry.shared.clone());
        match shared {
            Some(shared) => {
                shared.request_close(request);
                true
            }
            None => false,
        }
    }

    /// Ends every connection, as for a server that is stopping. The clients
    /// are told they may start over.
    pub fn close_all(&self) {
        let all: Vec<_> = lock(&self.inner.state)
            .connections
            .values()
            .map(|entry| entry.shared.clone())
            .collect();
        for shared in all {
            shared.request_close(CloseRequest {
                error: None,
                allow_reconnect: true,
            });
        }
    }

    fn deliver(targets: impl IntoIterator<Item = Arc<Shared>>, message: &Outbound) -> usize {
        targets
            .into_iter()
            .filter(|shared| shared.push(message.bytes()))
            .count()
    }

    /// Sends to one connection. False when it is gone.
    pub fn send_to_connection(&self, connection_id: &str, message: &Outbound) -> bool {
        let shared = lock(&self.inner.state)
            .connections
            .get(connection_id)
            .map(|entry| entry.shared.clone());
        shared.is_some_and(|shared| shared.push(message.bytes()))
    }

    /// Sends to each of the connections named; ones that are gone are
    /// skipped. Returns how many it was queued for.
    pub fn send_to_connections<I>(&self, connection_ids: I, message: &Outbound) -> usize
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let targets: Vec<_> = {
            let state = lock(&self.inner.state);
            let mut seen = HashSet::new();
            connection_ids
                .into_iter()
                .filter_map(|id| {
                    let (key, entry) = state.connections.get_key_value(id.as_ref())?;
                    seen.insert(key.clone()).then(|| entry.shared.clone())
                })
                .collect()
        };
        Self::deliver(targets, message)
    }

    pub fn send_to_group(&self, group: &str, message: &Outbound) -> usize {
        self.send_to_groups_except([group], &[], message)
    }

    /// Sends to the members of any of the groups. A connection that is in
    /// several of them gets the message once.
    pub fn send_to_groups<I>(&self, groups: I, message: &Outbound) -> usize
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.send_to_groups_except(groups, &[], message)
    }

    /// Sends to a group's members but the connections named.
    pub fn send_to_group_except(&self, group: &str, except: &[&str], message: &Outbound) -> usize {
        self.send_to_groups_except([group], except, message)
    }

    fn send_to_groups_except<I>(&self, groups: I, except: &[&str], message: &Outbound) -> usize
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let targets: Vec<_> = {
            let state = lock(&self.inner.state);
            let mut seen: HashSet<&Arc<str>> = HashSet::new();
            let mut targets = Vec::new();
            for group in groups {
                let Some(members) = state.groups.get(group.as_ref()) else {
                    continue;
                };
                for member in members {
                    if except.contains(&&**member) || !seen.insert(member) {
                        continue;
                    }
                    if let Some(entry) = state.connections.get(member) {
                        targets.push(entry.shared.clone());
                    }
                }
            }
            targets
        };
        Self::deliver(targets, message)
    }

    /// Sends to every connection.
    pub fn send_to_all(&self, message: &Outbound) -> usize {
        let targets: Vec<_> = lock(&self.inner.state)
            .connections
            .values()
            .map(|entry| entry.shared.clone())
            .collect();
        Self::deliver(targets, message)
    }
}

#[cfg(test)]
mod tests;
