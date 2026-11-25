//! Idle HTTP/1.1 connections. Each active exchange owns its connection.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::address::AddressPolicy;
use crate::body::{Outgoing, describe};
use crate::connect::Stream;
use crate::error::{Error, Result};
use crate::url::{Scheme, Url};

const MAX_IDLE: usize = 32;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Key {
    scheme: Scheme,
    host: String,
    port: u16,
    policy: AddressPolicy,
    // Hyper sets the parser's buffer limit on the connection.
    head_room: usize,
}

impl Key {
    pub(crate) fn new(url: &Url, policy: AddressPolicy, head_room: usize) -> Self {
        Self {
            scheme: url.scheme(),
            host: url.host().to_owned(),
            port: url.port(),
            policy,
            head_room,
        }
    }
}

pub(crate) struct Connection {
    pub(crate) sender: http1::SendRequest<Outgoing>,
    key: Key,
    peer: IpAddr,
    driver: JoinHandle<()>,
}

impl Connection {
    pub(crate) async fn handshake(stream: Stream, peer: IpAddr, key: Key) -> Result<Self> {
        let (sender, connection) = http1::Builder::new()
            .max_buf_size(key.head_room)
            .handshake(TokioIo::new(stream))
            .await
            .map_err(|error| Error::Protocol(describe(&error)))?;
        let driver = tokio::spawn(async move {
            // Hyper reports any exchange failure through the request or body.
            let _ = connection.await;
        });
        Ok(Self {
            sender,
            key,
            peer,
            driver,
        })
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Dropping an unfinished response closes its connection. A finished response
/// can return it to the client that opened it.
pub(crate) struct Lease {
    pub(crate) connection: Connection,
    pub(crate) pool: Weak<Pool>,
    pub(crate) upload_complete: Option<Arc<AtomicBool>>,
}

impl Lease {
    pub(crate) fn release(self) {
        // A server can respond before Hyper has read the entire upload.
        if self
            .upload_complete
            .is_none_or(|complete| complete.load(Ordering::Acquire))
            && let Some(pool) = self.pool.upgrade()
        {
            pool.put(self.connection);
        }
    }
}

struct Idle {
    connection: Connection,
    expires: Instant,
}

#[derive(Default)]
struct IdleConnections {
    entries: VecDeque<Idle>,
    reaper: Option<JoinHandle<()>>,
}

impl IdleConnections {
    fn prune(&mut self, now: Instant) {
        self.entries
            .retain(|idle| idle.expires > now && !idle.connection.sender.is_closed());
    }
}

impl Drop for IdleConnections {
    fn drop(&mut self) {
        if let Some(reaper) = self.reaper.take() {
            reaper.abort();
        }
    }
}

#[derive(Default)]
pub(crate) struct Pool {
    idle: Mutex<IdleConnections>,
}

impl Pool {
    pub(crate) fn take(&self, key: &Key, addresses: &[IpAddr]) -> Option<Connection> {
        let mut idle = self.idle.lock().unwrap();
        idle.prune(Instant::now());
        let position = idle.entries.iter().rposition(|entry| {
            entry.connection.key == *key && addresses.contains(&entry.connection.peer)
        })?;
        idle.entries.remove(position).map(|entry| entry.connection)
    }

    fn put(self: &Arc<Self>, connection: Connection) {
        if connection.sender.is_closed() {
            return;
        }
        let mut idle = self.idle.lock().unwrap();
        let now = Instant::now();
        idle.prune(now);
        if idle.entries.len() == MAX_IDLE {
            idle.entries.pop_front();
        }
        let expires = now + IDLE_TIMEOUT;
        idle.entries.push_back(Idle {
            connection,
            expires,
        });
        if idle.reaper.as_ref().is_none_or(JoinHandle::is_finished) {
            idle.reaper = Some(tokio::spawn(Self::reap(Arc::downgrade(self), expires)));
        }
    }

    async fn reap(pool: Weak<Self>, mut next: Instant) {
        loop {
            tokio::time::sleep_until(next).await;
            let Some(pool) = pool.upgrade() else { return };
            let mut idle = pool.idle.lock().unwrap();
            idle.prune(Instant::now());
            let Some(oldest) = idle.entries.front() else {
                idle.reaper = None;
                return;
            };
            next = oldest.expires;
        }
    }
}
