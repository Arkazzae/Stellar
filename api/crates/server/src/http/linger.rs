//! Ending a connection so that the last answer reaches the client.
//!
//! A server that answers a request before it has read its body (a 413 on a
//! declared length, a 401 or 403 before the body, a bad path, a rate limit, a
//! form refused halfway) and then closes the socket leaves what the client is
//! still sending unread in its buffer. Closing a socket with unread data makes
//! the operating system answer with a reset instead of a quiet close, and a
//! client that gets the reset before it has read the answer may lose the answer
//! with it: a browser shows a network error where the server said 413.
//!
//! So the listener hands hyper a [`Lingering`] stream, and when hyper is done
//! with it and lets it go, what is left of the connection is passed to a task
//! that sends the server's end first (the answer is already written, since
//! hyper flushes before it lets go), then reads and discards what the client is
//! still sending, until the client closes its side or a bound is reached, and
//! only then closes. The answer is delivered ahead of anything that follows.
//!
//! Lingering is never a way to keep a connection busy. It is bounded in
//! bytes ([`MAX_BYTES`]), in time ([`MAX_TIME`], counted from when the
//! server was done, not from the last byte, so a client that dribbles gains
//! nothing) and in number ([`MAX_AT_ONCE`] connections at a time: past that a
//! connection is closed as it would have been). It only delays a close; it
//! never delays an answer.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::serve::{IncomingStream, Listener};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tracing::warn;

/// The most that is read and thrown away after a connection is done with: the
/// ceiling of the whole server (128 MiB), so that a refused request never costs
/// more to read than an accepted one would have, and a client that was sending
/// a file when the server said no still gets to the end of it.
pub const MAX_BYTES: usize = super::body::MAX_REQUEST_BODY_BYTES;

/// The longest a connection lingers, counted from when the server let it go:
/// enough for
/// a refused 128 MiB at 13 MB/s, and a bound on how long a socket is held for
/// a client that has been refused.
pub const MAX_TIME: Duration = Duration::from_secs(10);

/// How many connections linger at the same time.
pub const MAX_AT_ONCE: usize = 1024;

static LINGERING: Semaphore = Semaphore::const_new(MAX_AT_ONCE);

/// A connection that lingers when it is let go.
pub struct Lingering(Option<TcpStream>);

impl Lingering {
    fn stream(&mut self) -> &mut TcpStream {
        self.0.as_mut().expect("a connection that is in use")
    }
}

impl AsyncRead for Lingering {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.stream()).poll_read(cx, buf)
    }
}

impl AsyncWrite for Lingering {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.stream()).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.stream()).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.stream()).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.stream()).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.as_ref().is_some_and(AsyncWrite::is_write_vectored)
    }
}

impl Drop for Lingering {
    fn drop(&mut self) {
        let Some(stream) = self.0.take() else {
            return;
        };
        // A peer that has closed already, with nothing left to read, needs no
        // lingering: closing is quiet. Anything else might be reset.
        let mut probe = [0u8; 1];
        if matches!(stream.try_read(&mut probe), Ok(0)) {
            return;
        }
        let Ok(permit) = LINGERING.try_acquire() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            linger(stream).await;
            drop(permit);
        });
    }
}

/// Sends the server's end, reads and discards until the client's end comes, the
/// bytes or the time run out, and closes.
async fn linger(mut stream: TcpStream) {
    let _ = stream.shutdown().await;
    let deadline = Instant::now() + MAX_TIME;
    let mut left = MAX_BYTES;
    let mut buffer = vec![0u8; 8 * 1024];
    while left > 0 {
        match tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            Ok(Ok(0) | Err(_)) | Err(_) => break,
            Ok(Ok(read)) => left = left.saturating_sub(read),
        }
    }
}

/// The address of the client's connection, for `ConnectInfo<Peer>`.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

impl axum::extract::connect_info::Connected<IncomingStream<'_, LingeringListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, LingeringListener>) -> Peer {
        Peer(*stream.remote_addr())
    }
}

/// A listener whose connections linger when they are let go.
pub struct LingeringListener(pub TcpListener);

impl Listener for LingeringListener {
    type Io = Lingering;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Lingering, SocketAddr) {
        loop {
            match self.0.accept().await {
                Ok((stream, address)) => {
                    // An answer goes out as it is written. With Nagle's
                    // algorithm a body written after its head waits for the
                    // client to acknowledge the head, which a client that
                    // delays its acknowledgements does some 40 ms later: every
                    // file answered on a kept connection took that long.
                    let _ = stream.set_nodelay(true);
                    return (Lingering(Some(stream)), address);
                }
                // A client that went away between the handshake and the accept
                // is nobody's trouble; anything else (no file descriptors left)
                // is waited out, so that the loop does not spin.
                Err(error) if is_connection_error(&error) => {}
                Err(error) => {
                    warn!("accepting a connection failed: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

fn is_connection_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}
