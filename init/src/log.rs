//! The manager's own log, on stderr, written by a thread of its own.
//!
//! The services write to the same stderr. One that floods it, or a log reader
//! that falls behind, fills the pipe, and a write to a full pipe waits. The
//! main loop must never wait there, so it only queues its lines; when the
//! queue is full a line is dropped and counted, and the count is written
//! once the pipe drains.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::thread;
use std::time::Duration;

/// Every line starts with this, to tell it from the services' own lines.
pub const PREFIX: &str = "stellarr-init: ";

const QUEUE: usize = 1024;

pub struct Log {
    queue: Option<SyncSender<String>>,
    dropped: Arc<AtomicU64>,
    finished: Receiver<()>,
}

impl Log {
    /// Starts the writer thread. Signals the manager handles must be blocked
    /// before, so that the thread does not take them.
    pub fn start() -> io::Result<Log> {
        let (queue, lines) = sync_channel::<String>(QUEUE);
        let (finish, finished) = channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);
        thread::Builder::new()
            .name("log".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let mut stderr = io::stderr();
                for line in lines {
                    let lost = counter.swap(0, Ordering::Relaxed);
                    if lost > 0 {
                        let note = format!(
                            "{PREFIX}{lost} lines of this log were dropped while stderr was full\n"
                        );
                        let _ = stderr.write_all(note.as_bytes());
                    }
                    let _ = stderr.write_all(line.as_bytes());
                }
                let _ = finish.send(());
            })?;
        Ok(Log {
            queue: Some(queue),
            dropped,
            finished,
        })
    }

    /// Queues one line; never waits.
    pub fn line(&self, text: &str) {
        let line = format!("{PREFIX}{text}\n");
        if let Some(queue) = &self.queue
            && queue.try_send(line).is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Lets the queue drain for up to `timeout`, for the last lines before
    /// the manager exits.
    pub fn finish(mut self, timeout: Duration) {
        drop(self.queue.take());
        let _ = self.finished.recv_timeout(timeout);
    }
}
