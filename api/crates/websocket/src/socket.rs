//! A WebSocket connection after the upgrade: reading frames into events,
//! answering what the protocol answers by itself, and writing frames.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use crate::frame::{
    self, Header, MAX_CONTROL_PAYLOAD, OP_BINARY, OP_CLOSE, OP_CONTINUATION, OP_PING, OP_PONG,
    OP_TEXT,
};
use crate::utf8::Utf8Check;

/// Close status codes the crate sends.
pub mod status {
    pub const NORMAL: u16 = 1000;
    pub const PROTOCOL_ERROR: u16 = 1002;
    pub const INVALID_DATA: u16 = 1007;
    pub const TOO_BIG: u16 = 1009;
}

#[derive(Debug, Clone)]
pub struct Config {
    /// The most bytes one message may hold, all its fragments together; a
    /// longer one closes the connection with status 1009.
    pub max_message_size: usize,
    /// How much is asked of the socket per read.
    pub read_buffer: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_message_size: 16 * 1024 * 1024,
            read_buffer: 16 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Text,
    Binary,
}

/// What the peer said in its close frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseFrame {
    /// `None` when the frame carried no status at all.
    pub code: Option<u16>,
    pub reason: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A piece of a message, in order. The pieces of a text message are
    /// checked as UTF-8 as a whole, but one piece may end inside a character.
    Data {
        kind: MessageKind,
        bytes: Bytes,
        end: bool,
    },
    /// The peer pinged; the pong is already sent.
    Ping,
    Pong,
    /// The peer closed. When the close was not ours, the reply is sent; the
    /// connection is over after this.
    Close(CloseFrame),
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The peer broke the protocol; the connection was closed with 1002.
    Protocol(&'static str),
    /// A text message was not UTF-8; closed with 1007.
    InvalidUtf8,
    /// A message went past the size limit; closed with 1009.
    MessageTooBig,
    /// The connection is already closed.
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "connection error: {error}"),
            Self::Protocol(why) => write!(f, "protocol error: {why}"),
            Self::InvalidUtf8 => f.write_str("a text message was not valid UTF-8"),
            Self::MessageTooBig => f.write_str("a message was too big"),
            Self::Closed => f.write_str("the connection is closed"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

struct Shared<S> {
    writer: Mutex<WriteHalf<S>>,
    close_sent: AtomicBool,
}

/// The writing half. Clones share the connection, and a frame is written
/// whole before another starts.
pub struct Sender<S> {
    shared: Arc<Shared<S>>,
}

impl<S> Clone for Sender<S> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// True for a status a peer may put in a close frame.
fn valid_close_code(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

impl<S: AsyncRead + AsyncWrite + Send + Unpin> Sender<S> {
    async fn write_frame(&self, opcode: u8, payload: &[u8]) -> Result<(), Error> {
        let header = frame::encode_header(true, opcode, payload.len(), None);
        let mut writer = self.shared.writer.lock().await;
        if payload.len() <= 16 * 1024 {
            // One write for a small frame keeps it in one TCP segment.
            let mut whole = header;
            whole.extend_from_slice(payload);
            writer.write_all(&whole).await?;
        } else {
            writer.write_all(&header).await?;
            writer.write_all(payload).await?;
        }
        writer.flush().await?;
        Ok(())
    }

    /// Sends a whole message as one frame.
    pub async fn send(&self, kind: MessageKind, payload: &[u8]) -> Result<(), Error> {
        if self.shared.close_sent.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let opcode = match kind {
            MessageKind::Text => OP_TEXT,
            MessageKind::Binary => OP_BINARY,
        };
        self.write_frame(opcode, payload).await
    }

    pub async fn send_text(&self, text: &[u8]) -> Result<(), Error> {
        self.send(MessageKind::Text, text).await
    }

    /// A ping; the peer answers with a pong carrying the same bytes.
    pub async fn send_ping(&self, payload: &[u8]) -> Result<(), Error> {
        self.send_control(OP_PING, payload).await
    }

    /// A pong nobody asked for, which the protocol allows as a one-way
    /// heartbeat that keeps proxies from timing the connection out.
    pub async fn send_pong(&self, payload: &[u8]) -> Result<(), Error> {
        self.send_control(OP_PONG, payload).await
    }

    async fn send_control(&self, opcode: u8, payload: &[u8]) -> Result<(), Error> {
        if payload.len() > MAX_CONTROL_PAYLOAD {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control payload too long",
            )));
        }
        if self.shared.close_sent.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        self.write_frame(opcode, payload).await
    }

    /// Starts the closing handshake. Only the first close goes out; the rest
    /// find the connection already closing.
    pub async fn close(&self, code: u16, reason: &str) -> Result<(), Error> {
        if self.shared.close_sent.swap(true, Ordering::AcqRel) {
            return Err(Error::Closed);
        }
        // The reason shares the 125 bytes of a control frame with the code;
        // cut at a character boundary so it stays valid UTF-8.
        let mut end = reason.len().min(MAX_CONTROL_PAYLOAD - 2);
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        let mut payload = Vec::with_capacity(2 + end);
        payload.extend_from_slice(&code.to_be_bytes());
        payload.extend_from_slice(&reason.as_bytes()[..end]);
        self.write_frame(OP_CLOSE, &payload).await
    }

    /// Whether a close frame has gone out.
    pub fn is_closing(&self) -> bool {
        self.shared.close_sent.load(Ordering::Acquire)
    }
}

struct FrameState {
    header: Header,
    remaining: u64,
    consumed: usize,
}

struct MessageState {
    kind: MessageKind,
    size: usize,
    utf8: Option<Utf8Check>,
}

/// The reading half. Reading answers pings and the peer's close by itself,
/// through the sender it holds.
pub struct Receiver<S> {
    reader: ReadHalf<S>,
    sender: Sender<S>,
    buf: BytesMut,
    config: Config,
    frame: Option<FrameState>,
    message: Option<MessageState>,
    finished: bool,
}

/// Takes over a connection that has been upgraded and splits it in two.
pub fn split<S>(io: S, config: Config) -> (Sender<S>, Receiver<S>)
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let (reader, writer) = tokio::io::split(io);
    let sender = Sender {
        shared: Arc::new(Shared {
            writer: Mutex::new(writer),
            close_sent: AtomicBool::new(false),
        }),
    };
    let receiver = Receiver {
        reader,
        sender: sender.clone(),
        buf: BytesMut::new(),
        config,
        frame: None,
        message: None,
        finished: false,
    };
    (sender, receiver)
}

impl<S: AsyncRead + AsyncWrite + Send + Unpin> Receiver<S> {
    pub fn sender(&self) -> Sender<S> {
        self.sender.clone()
    }

    /// The next thing that happened on the connection. After `Close` or an
    /// error, every call answers `Error::Closed`.
    pub async fn next(&mut self) -> Result<Event, Error> {
        if self.finished {
            return Err(Error::Closed);
        }
        match self.read_event().await {
            Ok(event) => {
                if matches!(event, Event::Close(_)) {
                    self.finished = true;
                }
                Ok(event)
            }
            Err(error) => {
                self.finished = true;
                let code = match &error {
                    Error::Protocol(_) => Some(status::PROTOCOL_ERROR),
                    Error::InvalidUtf8 => Some(status::INVALID_DATA),
                    Error::MessageTooBig => Some(status::TOO_BIG),
                    Error::Io(_) | Error::Closed => None,
                };
                if let Some(code) = code {
                    // The peer may be gone; the close is a courtesy.
                    let _ = self.sender.close(code, "").await;
                }
                Err(error)
            }
        }
    }

    async fn fill(&mut self) -> Result<(), Error> {
        self.buf.reserve(self.config.read_buffer);
        if self.reader.read_buf(&mut self.buf).await? == 0 {
            return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
        }
        Ok(())
    }

    async fn read_event(&mut self) -> Result<Event, Error> {
        loop {
            let Some(state) = self.frame.as_mut() else {
                match frame::parse_header(&self.buf) {
                    Err(error) => return Err(Error::Protocol(error.describe())),
                    Ok(None) => self.fill().await?,
                    Ok(Some((header, used))) => {
                        self.buf.advance(used);
                        if let Some(event) = self.begin_frame(header)? {
                            return Ok(event);
                        }
                    }
                }
                continue;
            };
            let header = state.header;
            if header.is_control() {
                let need = header.len as usize;
                if self.buf.len() < need {
                    self.fill().await?;
                    continue;
                }
                let mut payload = self.buf.split_to(need);
                frame::apply_mask(&mut payload, header.mask, 0);
                self.frame = None;
                return match header.opcode {
                    OP_PING => {
                        // After our own close the pong has nowhere to go.
                        let _ = self.sender.send_pong(&payload).await;
                        Ok(Event::Ping)
                    }
                    OP_PONG => Ok(Event::Pong),
                    _ => self.on_close(&payload).await,
                };
            }
            if self.buf.is_empty() {
                self.fill().await?;
                continue;
            }
            let take = (self.buf.len() as u64).min(state.remaining) as usize;
            let mut chunk = self.buf.split_to(take);
            frame::apply_mask(&mut chunk, header.mask, state.consumed);
            state.consumed += take;
            state.remaining -= take as u64;
            let last_of_frame = state.remaining == 0;
            if last_of_frame {
                self.frame = None;
            }
            return self
                .deliver(&chunk, header.fin && last_of_frame)
                .map(|(kind, end)| Event::Data {
                    kind,
                    bytes: chunk.freeze(),
                    end,
                });
        }
    }

    /// Checks a new frame against the message in progress and queues its
    /// payload. An empty data frame has no payload to wait for, so it is
    /// answered at once.
    fn begin_frame(&mut self, header: Header) -> Result<Option<Event>, Error> {
        if !header.is_control() {
            match (header.opcode, self.message.is_some()) {
                (OP_CONTINUATION, false) => {
                    return Err(Error::Protocol(
                        "a continuation without a message to continue",
                    ));
                }
                (OP_TEXT | OP_BINARY, true) => {
                    return Err(Error::Protocol(
                        "a new message began before the last one ended",
                    ));
                }
                (OP_TEXT | OP_BINARY, false) => {
                    let kind = if header.opcode == OP_TEXT {
                        MessageKind::Text
                    } else {
                        MessageKind::Binary
                    };
                    let utf8 = (kind == MessageKind::Text).then(Utf8Check::default);
                    self.message = Some(MessageState {
                        kind,
                        size: 0,
                        utf8,
                    });
                }
                _ => {}
            }
            // Refuse a frame that cannot fit before reading any of it.
            let size = self.message.as_ref().map_or(0, |m| m.size);
            if header.len
                > (self.config.max_message_size - size.min(self.config.max_message_size)) as u64
            {
                return Err(Error::MessageTooBig);
            }
            if header.len == 0 {
                let (kind, end) = self.deliver(&[], header.fin)?;
                return Ok(Some(Event::Data {
                    kind,
                    bytes: Bytes::new(),
                    end,
                }));
            }
        }
        self.frame = Some(FrameState {
            header,
            remaining: header.len,
            consumed: 0,
        });
        Ok(None)
    }

    /// Accounts for a piece of the message: its size, its UTF-8, its end.
    fn deliver(&mut self, chunk: &[u8], fin: bool) -> Result<(MessageKind, bool), Error> {
        let message = self
            .message
            .as_mut()
            .expect("a data frame always has a message");
        message.size += chunk.len();
        if message.size > self.config.max_message_size {
            return Err(Error::MessageTooBig);
        }
        if let Some(utf8) = message.utf8.as_mut()
            && (!utf8.feed(chunk) || (fin && !utf8.finish()))
        {
            return Err(Error::InvalidUtf8);
        }
        let kind = message.kind;
        if fin {
            self.message = None;
        }
        Ok((kind, fin))
    }

    async fn on_close(&mut self, payload: &[u8]) -> Result<Event, Error> {
        let frame = match payload {
            [] => CloseFrame {
                code: None,
                reason: String::new(),
            },
            [_] => return Err(Error::Protocol("a close frame with half a status")),
            [high, low, reason @ ..] => {
                let code = u16::from_be_bytes([*high, *low]);
                if !valid_close_code(code) {
                    return Err(Error::Protocol(
                        "a close frame with a status that may not be sent",
                    ));
                }
                let reason = std::str::from_utf8(reason)
                    .map_err(|_| Error::Protocol("a close reason that is not UTF-8"))?;
                CloseFrame {
                    code: Some(code),
                    reason: reason.to_owned(),
                }
            }
        };
        // Answer with a plain normal closure when the peer began, whatever
        // status it chose. When we began, this is the answer to ours.
        let _ = self.sender.close(status::NORMAL, "").await;
        Ok(Event::Close(frame))
    }
}

#[cfg(test)]
mod tests;
