//! One client's connection from its handshake to its end: reading and
//! parsing what it sends, running its calls one at a time, writing what the
//! hub queues for it, and the keep-alive in both directions.

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use serde_json::Value;
use stellarr_websocket::{Event, Receiver, Sender, status};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout};

use crate::app::{Application, Call, HubError};
use crate::config::Config;
use crate::hub::{CloseRequest, Hub, Shared, close_requested};
use crate::message::{self, Incoming, Invocation};
use crate::record::RecordBuffer;

/// The most bytes of records put into one WebSocket message.
const BATCH_BYTES: usize = 64 * 1024;

/// How a connection's reading ended.
enum Ended {
    /// The client closed, or went away without a word.
    ByPeer(Option<String>),
    /// The server decided: the close request is on the connection.
    ByServer(Option<String>),
}

/// The handshake did not finish.
enum NoHandshake {
    Gone,
    Timeout,
    TooLong,
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
    let mut records = RecordBuffer::new(config.max_record_size);
    let first = timeout(
        config.handshake_timeout,
        read_record(&mut receiver, &mut records),
    )
    .await;
    let first = match first {
        Err(_) => Err(NoHandshake::Timeout),
        Ok(result) => result,
    };
    let refusal = match first {
        Ok(record) => message::parse_handshake(&record).err(),
        Err(NoHandshake::Gone) => return,
        Err(NoHandshake::Timeout | NoHandshake::TooLong) => {
            Some(message::HANDSHAKE_CANCELED.to_owned())
        }
    };
    let answer = message::handshake_response(refusal.as_deref());
    if timeout(config.write_timeout, sender.send_text(&answer))
        .await
        .is_err()
    {
        return;
    }
    if refusal.is_some() {
        finish_close(&sender, &mut receiver, &config).await;
        return;
    }

    let shared = hub.register(connection_id.clone());
    let call = Call::new(connection_id.clone(), identity, hub.clone(), shared.clone());
    let writer = tokio::spawn(write_loop(sender.clone(), shared.clone(), config.clone()));

    // In a task of its own, so that a panic in the application ends this
    // connection and not the code that must still clean it up.
    let connected = {
        let app = app.clone();
        let call = call.clone();
        tokio::spawn(async move { app.connected(&call).await })
    }
    .await
    .unwrap_or_else(|failure| {
        Err(HubError::internal(format!(
            "the connect hook panicked: {failure}"
        )))
    });
    let mut ended = match connected {
        Ok(()) => None,
        Err(error) => {
            tracing::warn!(connection = %connection_id, ?error, "the application refused a new connection");
            shared.request_close(CloseRequest {
                error: Some(message::CLOSED_WITH_ERROR.into()),
                allow_reconnect: false,
            });
            Some(Ended::ByServer(Some(format!("{error:?}"))))
        }
    };

    let (calls, queue) = mpsc::channel(config.invocation_queue.max(1));
    let dispatcher = tokio::spawn(dispatch_loop(
        app.clone(),
        call.clone(),
        queue,
        shared.clone(),
        config.clone(),
    ));
    if ended.is_none() {
        ended = Some(read_loop(&mut receiver, &mut records, &shared, &calls, &config).await);
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

/// Reads until one whole record is there.
async fn read_record<S>(
    receiver: &mut Receiver<S>,
    records: &mut RecordBuffer,
) -> Result<Bytes, NoHandshake>
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    loop {
        match records.next_record() {
            Ok(Some(record)) => return Ok(record),
            Ok(None) => {}
            Err(_) => return Err(NoHandshake::TooLong),
        }
        match receiver.next().await {
            Ok(Event::Data { bytes, .. }) => records.push(&bytes),
            Ok(Event::Ping | Event::Pong) => {}
            Ok(Event::Close(_)) | Err(_) => return Err(NoHandshake::Gone),
        }
    }
}

/// Says goodbye after a refused handshake, and waits briefly for the
/// client's answer so it sees the reason rather than a reset.
async fn finish_close<S>(sender: &Sender<S>, receiver: &mut Receiver<S>, config: &Config)
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let _ = timeout(config.close_grace, sender.close(status::NORMAL, "")).await;
    drain_until_closed(receiver, config).await;
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

/// Reads what the client sends and passes its calls on, until either side
/// ends the connection.
async fn read_loop<S>(
    receiver: &mut Receiver<S>,
    records: &mut RecordBuffer,
    shared: &Arc<Shared>,
    calls: &mpsc::Sender<Invocation>,
    config: &Config,
) -> Ended
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let mut closing = shared.subscribe_close();
    // Whatever the handshake record carried beyond itself may already hold
    // calls.
    let mut leftover = true;
    loop {
        if leftover {
            leftover = false;
            match drain_records(records, shared, calls, &mut closing).await {
                Drained::More => {}
                Drained::Stop(ended) => return ended,
            }
        }
        let next = async {
            match config.client_timeout {
                Some(limit) => timeout(limit, receiver.next()).await.map_err(|_| ()),
                None => Ok(receiver.next().await),
            }
        };
        let event = tokio::select! {
            biased;
            () = close_requested(&mut closing) => {
                return Ended::ByServer(shared.close_request().and_then(|request| request.error));
            }
            event = next => event,
        };
        match event {
            Err(()) => {
                tracing::debug!(connection = %shared.id(), "the client sent nothing for too long");
                shared.request_close(CloseRequest {
                    error: None,
                    allow_reconnect: true,
                });
                return Ended::ByServer(Some("the client timed out".into()));
            }
            Ok(Ok(Event::Data { bytes, .. })) => {
                records.push(&bytes);
                match drain_records(records, shared, calls, &mut closing).await {
                    Drained::More => {}
                    Drained::Stop(ended) => return ended,
                }
            }
            Ok(Ok(Event::Ping | Event::Pong)) => {}
            Ok(Ok(Event::Close(_))) => return Ended::ByPeer(None),
            Ok(Err(error)) => {
                let text = error.to_string();
                return Ended::ByPeer(Some(text));
            }
        }
    }
}

enum Drained {
    More,
    Stop(Ended),
}

/// Parses the records that are whole and hands the calls on. A call waits
/// for room in the queue, which holds the reading back, and so the client
/// with it.
async fn drain_records(
    records: &mut RecordBuffer,
    shared: &Arc<Shared>,
    calls: &mpsc::Sender<Invocation>,
    closing: &mut watch::Receiver<Option<CloseRequest>>,
) -> Drained {
    loop {
        let record = match records.next_record() {
            Ok(Some(record)) => record,
            Ok(None) => return Drained::More,
            Err(_) => return Drained::Stop(fail(shared, "a record was longer than the limit")),
        };
        match message::parse_message(&record) {
            Ok(Incoming::Invocation(invocation)) => {
                tokio::select! {
                    biased;
                    () = close_requested(closing) => {
                        return Drained::Stop(Ended::ByServer(shared.close_request().and_then(|request| request.error)));
                    }
                    sent = calls.send(invocation) => {
                        if sent.is_err() {
                            return Drained::Stop(Ended::ByServer(None));
                        }
                    }
                }
            }
            Ok(Incoming::Close) => {
                shared.request_close(CloseRequest {
                    error: None,
                    allow_reconnect: false,
                });
                return Drained::Stop(Ended::ByServer(None));
            }
            Ok(Incoming::Ignored) => {}
            Err(why) => return Drained::Stop(fail(shared, why.0)),
        }
    }
}

/// The client broke the protocol: it is told so, and may start over.
fn fail(shared: &Shared, why: &str) -> Ended {
    tracing::debug!(connection = %shared.id(), why, "closing a connection over a bad message");
    shared.request_close(CloseRequest {
        error: Some(message::CLOSED_WITH_ERROR.into()),
        allow_reconnect: true,
    });
    Ended::ByServer(Some(why.to_owned()))
}

/// Runs the client's calls one at a time and queues each answer.
async fn dispatch_loop<A: Application>(
    app: Arc<A>,
    call: Call<A::Identity>,
    mut queue: mpsc::Receiver<Invocation>,
    shared: Arc<Shared>,
    _config: Arc<Config>,
) {
    while let Some(invocation) = queue.recv().await {
        if shared.close_request().is_some() {
            // The connection is ending; the client will not see an answer.
            continue;
        }
        let Invocation {
            id,
            target,
            arguments,
            stream_ids,
            streaming,
        } = invocation;
        let outcome = if !app.has_method(&target) {
            Err(HubError::UnknownMethod.client_text(&target))
        } else if streaming {
            Err(format!(
                "The client attempted to invoke the non-streaming '{target}' method with a streaming invocation."
            ))
        } else if stream_ids > 0 {
            Err(HubError::visible(format!(
                "Client sent {stream_ids} stream(s), Hub method expects 0."
            ))
            .client_text(&target))
        } else {
            run_call(&app, &call, &target, arguments).await
        };
        // A call with no id is a send: whatever happened, nobody is waiting.
        if let Some(id) = id {
            let record = match &outcome {
                Ok(result) => message::completion_result(&id, result),
                Err(text) => message::completion_error(&id, text),
            };
            shared.push(&record);
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
) -> Result<Value, String> {
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
    Err(error.client_text(target))
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
    let mut batch: Vec<Bytes> = Vec::new();
    loop {
        if let Some(request) = shared.close_request() {
            let record = message::close(request.error.as_deref(), request.allow_reconnect);
            let wrote = timeout(config.write_timeout, async {
                sender.send_text(&record).await?;
                sender.close(status::NORMAL, "").await
            })
            .await;
            if !matches!(wrote, Ok(Ok(()))) {
                tracing::debug!(connection = %shared.id(), "the close could not be written");
                shared.write_failed();
            }
            return;
        }
        tokio::select! {
            biased;
            () = close_requested(&mut closing) => {}
            () = shared.take(&mut batch, BATCH_BYTES) => {
                let mut wire = BytesMut::with_capacity(batch.iter().map(Bytes::len).sum());
                for record in batch.drain(..) {
                    wire.extend_from_slice(&record);
                }
                if !write_text(&sender, &config, &wire).await {
                    shared.write_failed();
                    return;
                }
            }
            () = sleep_until(next_ping) => {
                if !write_text(&sender, &config, &message::ping()).await {
                    shared.write_failed();
                    return;
                }
                next_ping = Instant::now() + config.keep_alive_interval;
            }
            _ = transport_pings.tick() => {
                let sent = timeout(config.write_timeout, sender.send_ping(b"")).await;
                if !matches!(sent, Ok(Ok(()))) {
                    shared.write_failed();
                    return;
                }
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
