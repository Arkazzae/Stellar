use std::time::Duration;

/// Limits and timings of the hub. The defaults are the ones the application
/// runs with.
#[derive(Debug, Clone)]
pub struct Config {
    /// The longest record a client may send, its separator included.
    pub max_record_size: usize,
    /// How long a client has to send its handshake after connecting.
    pub handshake_timeout: Duration,
    /// How often a ping record goes out, whatever else is sent.
    pub keep_alive_interval: Duration,
    /// A connection that has sent nothing at all, not even a reply to the
    /// transport's own ping, for this long is closed. `None` never closes one.
    pub client_timeout: Option<Duration>,
    /// How often the transport pings the client. The pings are what makes a
    /// healthy client send something at least this often.
    pub transport_ping_interval: Duration,
    /// How long one write to a client may take before the client is taken to
    /// be gone and its connection is ended. A client that reads nothing at
    /// all ends up here, once the buffers between it and the server are full.
    pub write_timeout: Duration,
    /// How many messages may wait for one client, and how many bytes of them.
    /// A client that falls further behind is closed, and starts over. These
    /// bounds are a safety net behind `write_timeout`: they let a healthy
    /// client take a burst of that many messages that the server produced
    /// faster than the client could be written to.
    pub max_queued_messages: usize,
    pub max_queued_bytes: usize,
    /// How long a close may take, the client's reply included, before the
    /// connection is dropped.
    pub close_grace: Duration,
    /// How long a call that is running when its connection ends may take to
    /// finish before it is abandoned.
    pub invocation_grace: Duration,
    /// How long a negotiated connection waits for its transport to connect.
    pub pending_connection_ttl: Duration,
    /// How many negotiated connections may wait at once.
    pub max_pending_connections: usize,
    /// How many calls from one client may wait for the one being run.
    pub invocation_queue: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_record_size: 160 * 1024,
            handshake_timeout: Duration::from_secs(15),
            keep_alive_interval: Duration::from_secs(15),
            client_timeout: Some(Duration::from_secs(45)),
            transport_ping_interval: Duration::from_secs(20),
            write_timeout: Duration::from_secs(10),
            max_queued_messages: 1024,
            max_queued_bytes: 8 * 1024 * 1024,
            close_grace: Duration::from_secs(3),
            invocation_grace: Duration::from_secs(30),
            pending_connection_ttl: Duration::from_secs(15),
            max_pending_connections: 50_000,
            invocation_queue: 32,
        }
    }
}
