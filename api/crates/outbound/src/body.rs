//! The two bodies: the one that is sent, and the one that is read.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use http_body_util::BodyExt;
use hyper::body::{Body as HyperBody, Frame, Incoming, SizeHint};
use tokio::io::{AsyncRead, ReadBuf};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::encoding::Decoder;
use crate::error::{Error, Result};

/// The most read from a stream body in one piece.
const SEND_CHUNK: usize = 64 * 1024;

/// What a request sends.
pub enum Body {
    Empty,
    Bytes(Bytes),
    /// A reader and the number of bytes it will deliver. The length goes in
    /// `Content-Length`; a reader that ends early fails the request. It can
    /// be sent once only, so a redirect that needs it again is an error.
    Stream {
        reader: Box<dyn AsyncRead + Send + Unpin>,
        length: u64,
    },
}

impl Body {
    pub fn empty() -> Body {
        Body::Empty
    }

    pub fn bytes(bytes: impl Into<Bytes>) -> Body {
        Body::Bytes(bytes.into())
    }

    pub fn stream(reader: impl AsyncRead + Send + Unpin + 'static, length: u64) -> Body {
        Body::Stream {
            reader: Box::new(reader),
            length,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Body::Empty => true,
            Body::Bytes(bytes) => bytes.is_empty(),
            Body::Stream { length, .. } => *length == 0,
        }
    }

    /// The same body again, for a redirect that repeats the request.
    pub(crate) fn replay(&self) -> Option<Body> {
        match self {
            Body::Empty => Some(Body::Empty),
            Body::Bytes(bytes) => Some(Body::Bytes(bytes.clone())),
            Body::Stream { .. } => None,
        }
    }
}

/// Adapts a [`Body`] for hyper.
pub(crate) struct Outgoing {
    body: Body,
    sent: u64,
}

impl Outgoing {
    pub(crate) fn new(body: Body) -> Outgoing {
        Outgoing { body, sent: 0 }
    }
}

impl HyperBody for Outgoing {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<io::Result<Frame<Bytes>>>> {
        let this = self.get_mut();
        match &mut this.body {
            Body::Empty => Poll::Ready(None),
            Body::Bytes(bytes) => {
                if bytes.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(Frame::data(std::mem::take(bytes)))))
                }
            }
            Body::Stream { reader, length } => {
                let left = *length - this.sent;
                if left == 0 {
                    return Poll::Ready(None);
                }
                let want = usize::try_from(left).unwrap_or(usize::MAX).min(SEND_CHUNK);
                let mut buffer = BytesMut::zeroed(want);
                let mut read = ReadBuf::new(&mut buffer);
                match Pin::new(reader).poll_read(cx, &mut read) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
                    Poll::Ready(Ok(())) => {
                        let got = read.filled().len();
                        if got == 0 {
                            return Poll::Ready(Some(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "the body ended before its announced length",
                            ))));
                        }
                        buffer.truncate(got);
                        this.sent += got as u64;
                        Poll::Ready(Some(Ok(Frame::data(buffer.freeze()))))
                    }
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match &self.body {
            Body::Empty => true,
            Body::Bytes(bytes) => bytes.is_empty(),
            Body::Stream { length, .. } => *length == self.sent,
        }
    }

    fn size_hint(&self) -> SizeHint {
        let left = match &self.body {
            Body::Empty => 0,
            Body::Bytes(bytes) => bytes.len() as u64,
            Body::Stream { length, .. } => *length - self.sent,
        };
        SizeHint::with_exact(left)
    }
}

/// Aborts the task that drives the connection when it is dropped, so that
/// a body that is given up on closes its socket at once.
pub(crate) struct DriverGuard(pub(crate) JoinHandle<()>);

impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The body of an answer, read piece by piece. The ceiling on the size of
/// the answer and the deadline of the whole request apply to every read.
pub struct ResponseBody {
    incoming: Incoming,
    decoder: Decoder,
    /// What the headers already used of the ceiling.
    head_bytes: u64,
    received: u64,
    ceiling: u64,
    deadline: Instant,
    finished: bool,
    _driver: DriverGuard,
}

impl ResponseBody {
    pub(crate) fn new(
        incoming: Incoming,
        decoder: Decoder,
        head_bytes: u64,
        ceiling: u64,
        deadline: Instant,
        driver: DriverGuard,
    ) -> ResponseBody {
        let finished = incoming.is_end_stream();
        ResponseBody {
            incoming,
            decoder,
            head_bytes,
            received: 0,
            ceiling,
            deadline,
            finished,
            _driver: driver,
        }
    }

    /// The next piece of the body, or `None` at its end.
    ///
    /// # Errors
    /// Fails with [`Error::TooLarge`] when headers and body together pass
    /// the ceiling, [`Error::Timeout`] when the request's time is up, and
    /// [`Error::Body`] when the connection breaks or the framing is wrong.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>> {
        loop {
            if self.finished {
                return Ok(None);
            }
            let frame = match tokio::time::timeout_at(self.deadline, self.incoming.frame()).await {
                Err(_) => return Err(Error::Timeout),
                Ok(None) => {
                    self.finished = true;
                    return Ok(None);
                }
                Ok(Some(Err(error))) => return Err(Error::Body(describe(&error))),
                Ok(Some(Ok(frame))) => frame,
            };
            // Trailers carry nothing this client reads.
            let Ok(data) = frame.into_data() else {
                continue;
            };
            self.received += data.len() as u64;
            if self.head_bytes.saturating_add(self.received) > self.ceiling {
                self.finished = true;
                return Err(Error::TooLarge);
            }
            let data = self.decoder.feed(data)?;
            if data.is_empty() {
                continue;
            }
            return Ok(Some(data));
        }
    }

    /// The whole body. Fails when it is over the ceiling.
    pub async fn collect(mut self) -> Result<Bytes> {
        let mut all = BytesMut::new();
        while let Some(chunk) = self.chunk().await? {
            all.extend_from_slice(&chunk);
        }
        Ok(all.freeze())
    }

    /// At most `limit` bytes of the body, and whether there was more. For a
    /// page whose useful part comes first: what follows is not read, and
    /// dropping the body closes the connection. This stops before the
    /// ceiling can, when `limit` is lower than what the ceiling leaves.
    pub async fn read_up_to(&mut self, limit: usize) -> Result<(Bytes, bool)> {
        let mut all = BytesMut::new();
        loop {
            let Some(chunk) = self.chunk().await? else {
                return Ok((all.freeze(), false));
            };
            let room = limit - all.len();
            if chunk.len() > room {
                all.extend_from_slice(&chunk[..room]);
                return Ok((all.freeze(), true));
            }
            all.extend_from_slice(&chunk);
            if all.len() == limit {
                // Exactly full: there is more only if another piece comes.
                return match self.chunk().await? {
                    None => Ok((all.freeze(), false)),
                    Some(_) => Ok((all.freeze(), true)),
                };
            }
        }
    }
}

/// What went wrong in the framing, without text from the other end.
pub(crate) fn describe(error: &hyper::Error) -> String {
    if error.is_incomplete_message() {
        "the connection closed before the message was complete".into()
    } else if error.is_parse() || error.is_parse_status() {
        "the message does not follow HTTP/1.1".into()
    } else if error.is_timeout() {
        "timed out".into()
    } else {
        "the connection broke".into()
    }
}
