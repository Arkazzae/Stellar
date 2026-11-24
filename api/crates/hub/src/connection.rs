//! One client's connection from its handshake to its end: reading and
//! parsing what it sends, running its calls one at a time, writing what the
//! hub queues for it, and the keep-alive in both directions.

use std::sync::Arc;

use bytes::BytesMut;
use serde_json::Value;
use stellarr_websocket::{Event, MessageKind, Receiver, Sender, status};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout};

use crate::app::{Application, Call, HubError};
use crate::config::Config;
use crate::hub::{Hub, Shared, close_requested};
use crate::native::{self, Incoming};

/// How a connection's reading ended.
enum Ended {
    /// The client closed, or went away without a word.
    ByPeer(Option<String>),
    /// The server decided: the close request is on the connection.
    ByServer(Option<String>),
}

/// Runs a connection that has been upgraded to a WebSocket, to its end.
/// Returns when the application has been told the connection is gone.
pub async fn serve<A, S>(
    app: Arc<A>,
    hub: Hub,
    config: Arc<Config>,
    identity: Arc<A::Identity>,
    connection_id: Arc<str>,
    sender: Sender<S>,
    mut receiver: Receiver<S>,
) where
    A: Application,
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let shared = hub.register(connection_id.clone());
    let call = Call::new(connection_id.clone(), identity, hub.clone(), shared.clone());

    // In a task of its own, so that a panic in the application ends this
    // connection and not the code that must still clean it up.
    let mut connect_task = AbortOnDrop({
        let app = app.clone();
        let call = call.clone();
        tokio::spawn(async move { app.connected(&call).await })
    });
    let connected = match timeout(config.startup_timeout, &mut connect_task.0).await {
        Ok(result) => result.unwrap_or_else(|failure| Err(HubError::internal(failure.to_string()))),
        Err(_) => {
            connect_task.0.abort();
            // The hook must stop before disconnected cleans up its state.
            let _ = (&mut connect_task.0).await;
            Err(HubError::internal("the connect hook timed out"))
        }
    };
    let mut ended = match connected {
        Ok(()) => None,
        Err(error) => {
            tracing::warn!(connection = %connection_id, ?error, "the application refused a new connection");
            shared.request_close(native::INTERNAL_ERROR, None);
            Some(Ended::ByServer(Some(format!("{error:?}"))))
        }
    };

    if ended.is_none()
        && shared.close_signal().is_none()
        && !write_text(&sender, &config, native::READY).await
    {
        shared.write_failed();
        ended = Some(Ended::ByPeer(Some("ready could not be written".into())));
    }
    let writer = tokio::spawn(write_loop(sender, shared.clone(), config.clone()));
    let (calls, queue) = mpsc::channel(config.invocation_queue.max(1));
    let dispatcher = tokio::spawn(dispatch_loop(
        app.clone(),
        call.clone(),
        queue,
        shared.clone(),
    ));
    if ended.is_none() {
        ended = Some(read_loop(&mut receiver, &shared, &calls, &config).await);
    }
    drop(calls);

    let abort_writer = writer.abort_handle();
    let disconnect_error = match ended.expect("reading has ended") {
        Ended::ByPeer(error) => {
            // The client is gone or going: nothing more is worth writing.
            abort_writer.abort();
            error
        }
        Ended::ByServer(error) => {
            // Let the writer say why and close, unless it is stuck in a
            // write to a client that does not read.
            let said_goodbye = timeout(config.close_grace, writer_done(writer))
                .await
                .is_ok();
            if said_goodbye && !shared.has_write_failed() {
                drain_until_closed(&mut receiver, &config).await;
            } else {
                abort_writer.abort();
            }
            error
        }
    };

    let mut dispatcher = dispatcher;
    if timeout(config.invocation_grace, &mut dispatcher)
        .await
        .is_err()
    {
        tracing::warn!(connection = %connection_id, "a call outlived its connection; abandoning it");
        dispatcher.abort();
    }
    hub.unregister(&connection_id);
    app.disconnected(&call, disconnect_error.as_deref()).await;
}

async fn writer_done(handle: JoinHandle<()>) {
    let _ = handle.await;
}

async fn drain_until_closed<S>(receiver: &mut Receiver<S>, config: &Config)
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let _ = timeout(config.close_grace, async {
        while !matches!(receiver.next().await, Ok(Event::Close(_)) | Err(_)) {}
    })
    .await;
}

/// Reads one text message at a time, waiting for space in the call queue.
async fn read_loop<S>(
    receiver: &mut Receiver<S>,
    shared: &Shared,
    calls: &mpsc::Sender<Incoming>,
    config: &Config,
) -> Ended
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let mut closing = shared.subscribe_close();
    let mut message = BytesMut::new();
    let close_error = || {
        shared
            .close_signal()
            .and_then(|signal| signal.error)
            .map(str::to_owned)
    };
    loop {
        let next = async {
            match config.client_timeout {
                Some(limit) => timeout(limit, receiver.next()).await.map_err(|_| ()),
                None => Ok(receiver.next().await),
            }
        };
        let event = tokio::select! {
            biased;
            () = close_requested(&mut closing) => return Ended::ByServer(close_error()),
            event = next => event,
        };
        match event {
            Err(()) => {
                tracing::debug!(connection = %shared.id(), "the client sent nothing for too long");
                shared.request_close(native::GOING_AWAY, Some("the client timed out"));
                return Ended::ByServer(close_error());
            }
            Ok(Ok(Event::Data { kind, bytes, end })) => {
                if kind != MessageKind::Text {
                    return fail(shared, status::PROTOCOL_ERROR, "expected a text message");
                }
                if message.len().saturating_add(bytes.len()) > config.max_message_size {
                    return fail(
                        shared,
                        status::TOO_BIG,
                        "the message exceeded the size limit",
                    );
                }
                let incoming = if message.is_empty() && end {
                    native::parse(&bytes)
                } else {
                    message.extend_from_slice(&bytes);
                    if !end {
                        continue;
                    }
                    native::parse(&message.split().freeze())
                };
                match incoming {
                    Ok(incoming) => tokio::select! {
                        biased;
                        () = close_requested(&mut closing) => return Ended::ByServer(close_error()),
                        sent = calls.send(incoming) => {
                            if sent.is_err() {
                                return Ended::ByServer(None);
                            }
                        }
                    },
                    Err(why) => return fail(shared, status::PROTOCOL_ERROR, why),
                }
            }
            Ok(Ok(Event::Ping | Event::Pong)) => {}
            Ok(Ok(Event::Close(_))) => return Ended::ByPeer(None),
            Ok(Err(error)) => return Ended::ByPeer(Some(error.to_string())),
        }
    }
}

fn fail(shared: &Shared, code: u16, why: &'static str) -> Ended {
    tracing::debug!(connection = %shared.id(), why, "closing a connection over a bad message");
    shared.request_close(code, Some(why));
    Ended::ByServer(Some(why.to_owned()))
}

/// Runs the client's calls one at a time and queues each answer.
async fn dispatch_loop<A: Application>(
    app: Arc<A>,
    call: Call<A::Identity>,
    mut queue: mpsc::Receiver<Incoming>,
    shared: Arc<Shared>,
) {
    while let Some(invocation) = queue.recv().await {
        if shared.close_signal().is_some() {
            break;
        }
        let (id, method, args) = match invocation {
            Incoming::Call { id, method, args } => (Some(id), method, args),
            Incoming::Send { method, args } => (None, method, args),
        };
        let outcome = if app.has_method(&method) {
            run_call(&app, &call, &method, args).await
        } else {
            Err(HubError::UnknownMethod)
        };
        if let Some(id) = id {
            shared.push(&native::completion(id, &outcome));
        }
    }
}

/// Stops a task when the connection gives up on waiting for it.
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs one method in a task of its own, so that a panic in it costs the
/// client an error and not the connection.
async fn run_call<A: Application>(
    app: &Arc<A>,
    call: &Call<A::Identity>,
    target: &str,
    arguments: Vec<Value>,
) -> Result<Value, HubError> {
    let mut task = AbortOnDrop({
        let app = app.clone();
        let call = call.clone();
        let target = target.to_owned();
        tokio::spawn(async move { app.invoke(&call, &target, arguments).await })
    });
    let error = match (&mut task.0).await {
        Ok(Ok(value)) => return Ok(value),
        Ok(Err(error)) => error,
        Err(failure) => HubError::internal(format!("the call panicked: {failure}")),
    };
    if let HubError::Internal(detail) = &error {
        tracing::error!(target, detail, "a hub method failed");
    }
    Err(error)
}

/// Writes what the hub queues, the keep-alive, and the close.
async fn write_loop<S>(sender: Sender<S>, shared: Arc<Shared>, config: Arc<Config>)
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let mut closing = shared.subscribe_close();
    let mut next_ping = Instant::now() + config.keep_alive_interval;
    let mut transport_pings = interval_at(
        Instant::now() + config.transport_ping_interval,
        config.transport_ping_interval,
    );
    transport_pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        if let Some(signal) = shared.close_signal() {
            let wrote = timeout(config.write_timeout, sender.close(signal.code, "")).await;
            if !matches!(wrote, Ok(Ok(()))) {
                tracing::debug!(connection = %shared.id(), "the close could not be written");
                shared.write_failed();
            }
            return;
        }
        tokio::select! {
            biased;
            () = close_requested(&mut closing) => {}
            _ = transport_pings.tick() => {
                let sent = timeout(config.write_timeout, sender.send_ping(b"")).await;
                if !matches!(sent, Ok(Ok(()))) {
                    shared.write_failed();
                    return;
                }
            }
            message = shared.take() => {
                if !write_text(&sender, &config, &message).await {
                    shared.write_failed();
                    return;
                }
                next_ping = Instant::now() + config.keep_alive_interval;
            }
            () = sleep_until(next_ping) => {
                if !write_text(&sender, &config, native::PING).await {
                    shared.write_failed();
                    return;
                }
                next_ping = Instant::now() + config.keep_alive_interval;
            }
        }
    }
}

/// One write, which must finish in time. False ends the connection.
async fn write_text<S>(sender: &Sender<S>, config: &Config, bytes: &[u8]) -> bool
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    matches!(
        timeout(config.write_timeout, sender.send_text(bytes)).await,
        Ok(Ok(()))
    )
}
