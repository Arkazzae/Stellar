//! The connections of the hub, the groups they belong to, and sending.
//!
//! Sending never waits: a message goes into the queue of each connection it
//! is for and the call returns. Each queue is bounded; a client that cannot
//! keep up is closed (and starts over) instead of holding up whoever sends,
//! or growing without limit.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};
use tokio::sync::{Notify, watch};

use crate::config::Config;
use crate::native;

/// A serialized event shared across all its recipients.
#[derive(Debug, Clone)]
pub struct Outbound {
    bytes: Bytes,
    presence_user: Option<Arc<str>>,
}

impl Outbound {
    /// An event carrying one payload.
    pub fn single<T: Serialize + ?Sized>(name: &str, data: &T) -> Result<Self, serde_json::Error> {
        let data = to_raw_value(data)?;
        Ok(Self::encode(name, &data))
    }

    /// An event carrying a null payload.
    pub fn bare(name: &str) -> Self {
        Self::encode(name, RawValue::NULL)
    }

    fn encode(name: &str, data: &RawValue) -> Self {
        Self {
            bytes: native::event(name, data),
            presence_user: native::presence_user(name, data).map(Arc::from),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CloseSignal {
    pub(crate) code: u16,
    pub(crate) error: Option<&'static str>,
}

#[derive(Default)]
struct OutboxState {
    queue: VecDeque<Bytes>,
    bytes: usize,
    trailing_presence_user: Option<Arc<str>>,
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
    close: watch::Sender<Option<CloseSignal>>,
    write_failed: AtomicBool,
    max_messages: usize,
    max_bytes: usize,
}

impl Shared {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Asks for the connection to be closed. The first request wins: a
    /// connection that is already closing does not change its mind.
    pub(crate) fn request_close(&self, code: u16, error: Option<&'static str>) {
        self.close.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(CloseSignal { code, error });
            true
        });
    }

    /// A write to the client failed or took too long, so the client is gone
    /// for practical purposes: the connection ends, and there is nobody to
    /// say goodbye to.
    pub(crate) fn write_failed(&self) {
        self.write_failed.store(true, Ordering::Release);
        self.request_close(native::GOING_AWAY, None);
    }

    pub(crate) fn has_write_failed(&self) -> bool {
        self.write_failed.load(Ordering::Acquire)
    }

    pub(crate) fn subscribe_close(&self) -> watch::Receiver<Option<CloseSignal>> {
        self.close.subscribe()
    }

    pub(crate) fn close_signal(&self) -> Option<CloseSignal> {
        *self.close.borrow()
    }

    /// Queues a record. False when the connection is closing or has just
    /// fallen too far behind, in which case it is closed.
    pub(crate) fn push(&self, record: &Bytes) -> bool {
        self.push_record(record, None)
    }

    fn push_outbound(&self, message: &Outbound) -> bool {
        self.push_record(&message.bytes, message.presence_user.as_ref())
    }

    fn push_record(&self, record: &Bytes, presence_user: Option<&Arc<str>>) -> bool {
        if self.close.borrow().is_some() {
            return false;
        }
        let mut state = lock(&self.outbox.state);
        // Only a trailing snapshot for the same subject may be replaced:
        // moving a snapshot across other messages can change their meaning.
        let supersedes =
            presence_user.is_some() && presence_user == state.trailing_presence_user.as_ref();
        let (queued_messages, queued_bytes) = if supersedes {
            let replaced = state.queue.back().expect("presence snapshot is queued");
            (state.queue.len() - 1, state.bytes - replaced.len())
        } else {
            (state.queue.len(), state.bytes)
        };
        // An otherwise empty queue may carry one message of any size.
        let over = queued_messages > 0
            && (queued_messages >= self.max_messages
                || queued_bytes.saturating_add(record.len()) > self.max_bytes);
        if over {
            drop(state);
            tracing::warn!(connection = %self.id, "a client fell too far behind; closing its connection");
            self.request_close(native::OVERLOADED, Some("the client could not keep up"));
            return false;
        }
        if supersedes {
            state.queue.pop_back();
        }
        state.bytes = queued_bytes + record.len();
        state.queue.push_back(record.clone());
        state.trailing_presence_user = presence_user.cloned();
        drop(state);
        self.outbox.wake.notify_one();
        true
    }

    /// Waits for the next message. Safe to drop while waiting.
    pub(crate) async fn take(&self) -> Bytes {
        loop {
            {
                let mut state = lock(&self.outbox.state);
                if let Some(record) = state.queue.pop_front() {
                    state.bytes -= record.len();
                    if state.queue.is_empty() {
                        state.trailing_presence_user = None;
                    }
                    return record;
                }
            }
            self.outbox.wake.notified().await;
        }
    }
}

/// Completes once a close has been asked for. A function of its own so that
/// the borrow `wait_for` returns is gone before anything else is awaited.
pub(crate) async fn close_requested(closing: &mut watch::Receiver<Option<CloseSignal>>) {
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
            write_failed: AtomicBool::new(false),
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

    /// The ids of all the connections, in no particular order.
    pub fn connection_ids(&self) -> Vec<String> {
        lock(&self.inner.state)
            .connections
            .keys()
            .map(|id| id.to_string())
            .collect()
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

    /// Ends a connection without asking the client to reconnect.
    pub fn abort(&self, connection_id: &str) -> bool {
        self.close(connection_id, stellarr_websocket::status::NORMAL)
    }

    /// Ends a connection whose authenticated session ended.
    pub fn end_session(&self, connection_id: &str) -> bool {
        self.close(connection_id, native::SESSION_ENDED)
    }

    fn close(&self, connection_id: &str, code: u16) -> bool {
        let shared = lock(&self.inner.state)
            .connections
            .get(connection_id)
            .map(|entry| entry.shared.clone());
        match shared {
            Some(shared) => {
                shared.request_close(code, None);
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
            shared.request_close(native::RESTART, None);
        }
    }

    fn deliver(targets: impl IntoIterator<Item = Arc<Shared>>, message: &Outbound) -> usize {
        targets
            .into_iter()
            .filter(|shared| shared.push_outbound(message))
            .count()
    }

    /// Sends to one connection. False when it is gone.
    pub fn send_to_connection(&self, connection_id: &str, message: &Outbound) -> bool {
        let shared = lock(&self.inner.state)
            .connections
            .get(connection_id)
            .map(|entry| entry.shared.clone());
        shared.is_some_and(|shared| shared.push_outbound(message))
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
