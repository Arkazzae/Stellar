//! What can go wrong with a request.
//!
//! No message here contains a whole URL, a query string or a header value:
//! those carry API keys and tokens, and an error ends up in logs. At most a
//! host name appears.

use std::fmt;
use std::io;

/// Why a request failed.
#[derive(Debug)]
pub enum Error {
    /// The URL is not one this client will request.
    InvalidUrl(&'static str),
    /// A header the client sets itself, or one that would change how the
    /// message is framed, was given.
    ForbiddenHeader(String),
    /// The address policy refused every address of the host.
    AddressRefused { host: String },
    /// The name could not be resolved.
    Resolve { host: String, source: io::Error },
    /// No address of the host accepted the connection.
    Connect { host: String, source: io::Error },
    /// The TLS handshake failed or the certificate was not accepted.
    Tls { host: String, message: String },
    /// The system's trusted certificates could not be loaded.
    Roots(String),
    /// Connecting (resolving, connecting and the TLS handshake) took longer
    /// than the connect timeout.
    ConnectTimeout { host: String },
    /// The whole request, body included, took longer than its timeout.
    Timeout,
    /// The answer, headers and body together, is over the ceiling.
    TooLarge,
    /// More redirects than the request allows.
    TooManyRedirects,
    /// A redirect the client refuses, and why.
    RedirectRefused(&'static str),
    /// A redirect that needs the request body again, which was a stream.
    BodyNotReplayable,
    /// The request body did not have the length it was announced with.
    RequestBody(String),
    /// The other end did not speak HTTP/1.1.
    Protocol(String),
    /// The connection broke while the answer was being read.
    Body(String),
}

impl Error {
    /// Whether time ran out, in connecting or in the whole request.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Timeout | Error::ConnectTimeout { .. })
    }

    /// Whether the address policy is the reason: the host was refused, or a
    /// redirect led somewhere the policy does not allow.
    /// Inspect address-policy failures in tests.
    #[doc(hidden)]
    pub fn is_address_refused(&self) -> bool {
        matches!(self, Error::AddressRefused { .. })
    }

    /// Whether the answer was cut off for being over the ceiling.
    pub fn is_too_large(&self) -> bool {
        matches!(self, Error::TooLarge)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidUrl(why) => write!(f, "the URL is not usable: {why}"),
            Error::ForbiddenHeader(name) => write!(f, "the header {name} is set by the client"),
            Error::AddressRefused { host } => {
                write!(
                    f,
                    "refusing to connect to {host}: no address of it is allowed"
                )
            }
            Error::Resolve { host, source } => write!(f, "could not resolve {host}: {source}"),
            Error::Connect { host, source } => write!(f, "could not connect to {host}: {source}"),
            Error::Tls { host, message } => write!(f, "TLS with {host} failed: {message}"),
            Error::Roots(message) => write!(f, "no trusted certificates: {message}"),
            Error::ConnectTimeout { host } => write!(f, "connecting to {host} timed out"),
            Error::Timeout => f.write_str("the request timed out"),
            Error::TooLarge => f.write_str("the answer is over the size ceiling"),
            Error::TooManyRedirects => f.write_str("too many redirects"),
            Error::RedirectRefused(why) => write!(f, "redirect refused: {why}"),
            Error::BodyNotReplayable => {
                f.write_str("the redirect needs the request body again and it was a stream")
            }
            Error::RequestBody(message) => write!(f, "the request body failed: {message}"),
            Error::Protocol(message) => write!(f, "the answer is not valid HTTP/1.1: {message}"),
            Error::Body(message) => write!(f, "reading the answer failed: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Resolve { source, .. } | Error::Connect { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, Error>;
