//! What can go wrong, and whether trying again can help.
//!
//! The caller decides on a retry from [`Error::is_transient`] and, when the
//! failure came after the message was handed over, from
//! [`Error::may_have_been_delivered`].

use std::fmt;

/// The step of the exchange a failure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The message could not be put together; nothing was sent.
    Message,
    Connect,
    /// The TLS handshake, whether it opens the connection or follows STARTTLS.
    Handshake,
    Greeting,
    Hello,
    StartTls,
    Auth,
    Sender,
    Recipient,
    Data,
    EndOfData,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Message => "message",
            Stage::Connect => "connecting",
            Stage::Handshake => "TLS handshake",
            Stage::Greeting => "greeting",
            Stage::Hello => "EHLO",
            Stage::StartTls => "STARTTLS",
            Stage::Auth => "authentication",
            Stage::Sender => "sender",
            Stage::Recipient => "recipient",
            Stage::Data => "DATA",
            Stage::EndOfData => "end of message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The message or an address in it is not something that can be sent.
    InvalidMessage,
    /// The server could not be reached.
    Connect,
    /// A step did not finish in time.
    Timeout,
    /// The connection broke or was closed by the server.
    Io,
    /// The TLS handshake failed.
    Tls,
    /// The server said something that is not SMTP, or too much of it.
    Protocol,
    /// The server answered a command with a failure code.
    Rejected,
    /// The server lacks something the settings require.
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct Error {
    kind: ErrorKind,
    stage: Stage,
    code: Option<u16>,
    transient: bool,
    maybe_delivered: bool,
    endpoint: Option<String>,
    detail: String,
}

impl Error {
    pub(crate) fn new(
        kind: ErrorKind,
        stage: Stage,
        transient: bool,
        detail: impl Into<String>,
    ) -> Error {
        Error {
            kind,
            stage,
            code: None,
            transient,
            maybe_delivered: false,
            endpoint: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn invalid(detail: impl Into<String>) -> Error {
        Error::new(ErrorKind::InvalidMessage, Stage::Message, false, detail)
    }

    pub(crate) fn unsupported(stage: Stage, detail: impl Into<String>) -> Error {
        Error::new(ErrorKind::Unsupported, stage, false, detail)
    }

    pub(crate) fn protocol(stage: Stage, detail: impl Into<String>) -> Error {
        // A server that does not speak SMTP will not start to on a second try.
        Error::new(ErrorKind::Protocol, stage, false, detail)
    }

    pub(crate) fn io(stage: Stage, detail: impl Into<String>) -> Error {
        Error::new(ErrorKind::Io, stage, true, detail)
    }

    pub(crate) fn timeout(stage: Stage, seconds: f64) -> Error {
        Error::new(
            ErrorKind::Timeout,
            stage,
            true,
            format!("the server did not answer within {seconds:.1} seconds"),
        )
    }

    /// A reply with a code the step did not expect. 4xx means the server
    /// asks to try later; 5xx means it will not take this; anything else is
    /// a server that lost the thread.
    pub(crate) fn rejected(stage: Stage, code: u16, text: &str) -> Error {
        let detail = format!("{code} {text}");
        match code {
            400..=499 => Error {
                code: Some(code),
                ..Error::new(ErrorKind::Rejected, stage, true, detail)
            },
            500..=599 => Error {
                code: Some(code),
                ..Error::new(ErrorKind::Rejected, stage, false, detail)
            },
            _ => Error {
                code: Some(code),
                ..Error::protocol(stage, format!("unexpected reply {detail}"))
            },
        }
    }

    pub(crate) fn with_endpoint(mut self, host: &str, port: u16) -> Error {
        self.endpoint = Some(format!("{host}:{port}"));
        self
    }

    pub(crate) fn delivered_maybe(mut self) -> Error {
        self.maybe_delivered = true;
        self
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The server's reply code, when the failure is a reply.
    /// Inspect SMTP replies in protocol tests.
    #[doc(hidden)]
    pub fn reply_code(&self) -> Option<u16> {
        self.code
    }

    /// Whether the same message may be accepted on a later attempt: a
    /// connection that failed or timed out, a server asking to try again
    /// (4xx). A refusal, a bad address, a failed login, a certificate that
    /// does not verify or a server that does not speak SMTP stays the same.
    pub fn is_transient(&self) -> bool {
        self.transient
    }

    /// True when the connection failed after the whole message was sent and
    /// before the server's answer arrived. The server may have accepted it,
    /// and a retry may then send it twice.
    /// Inspect the delivery state in protocol tests.
    #[doc(hidden)]
    pub fn may_have_been_delivered(&self) -> bool {
        self.maybe_delivered
    }

    /// What the server or the system said, without the endpoint or stage.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(endpoint) = &self.endpoint {
            write!(f, "SMTP {endpoint}: ")?;
        }
        write!(f, "{}: {}", self.stage.name(), self.detail)
    }
}

impl std::error::Error for Error {}
