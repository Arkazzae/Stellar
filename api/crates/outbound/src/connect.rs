//! Opening the connection: resolving the name, judging every address,
//! connecting to one that passed, and the TLS handshake.
//!
//! The socket is opened to the very address that was judged. The name is
//! never handed to the operating system a second time, so an answer that
//! changes between the check and the connect has nothing to change.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::address::AddressPolicy;
use crate::error::{Error, Result};
use crate::url::Url;

/// Turns a host name into addresses. The default asks the operating system;
/// a test can answer from a table.
pub trait Resolve: Send + Sync {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>>;
}

/// Resolves through the operating system's own resolver, which is the one
/// that knows `/etc/hosts` and the container's DNS.
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>> {
        Box::pin(async move {
            let found = tokio::net::lookup_host((host, port)).await?;
            Ok(found.map(|address| address.ip()).collect())
        })
    }
}

/// A connected stream, plain or encrypted.
pub(crate) enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            Stream::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            Stream::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(stream) => Pin::new(stream).poll_flush(cx),
            Stream::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            Stream::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            Stream::Tls(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Stream::Plain(stream) => stream.is_write_vectored(),
            Stream::Tls(stream) => stream.is_write_vectored(),
        }
    }
}

/// The addresses of a host that the policy allows, in the order the
/// resolver gave them and without repeats. An IP literal is not resolved.
///
/// Addresses that the policy refuses are left out and the rest are used,
/// so a name with one private address among public ones can be reached
/// only at the public ones.
pub(crate) async fn allowed_addresses(
    url: &Url,
    policy: AddressPolicy,
    resolver: &dyn Resolve,
) -> Result<Vec<IpAddr>> {
    let host = url.bare_host();
    let found = match host.parse::<IpAddr>() {
        Ok(literal) => vec![literal],
        Err(_) => resolver
            .resolve(host, url.port())
            .await
            .map_err(|source| Error::Resolve {
                host: host.to_owned(),
                source,
            })?,
    };
    let mut allowed: Vec<IpAddr> = Vec::new();
    for address in found {
        if policy.allows(address) && !allowed.contains(&address) {
            allowed.push(address);
        }
    }
    if allowed.is_empty() {
        tracing::debug!(host, "every address of the host is refused by the policy");
        return Err(Error::AddressRefused {
            host: host.to_owned(),
        });
    }
    Ok(allowed)
}

/// Resolves, judges and connects within `limit`, then does the TLS
/// handshake for an `https` URL, inside the same limit.
pub(crate) async fn open(
    url: &Url,
    policy: AddressPolicy,
    resolver: &dyn Resolve,
    tls: Option<&TlsConnector>,
    limit: Duration,
) -> Result<Stream> {
    let host = url.bare_host().to_owned();
    let deadline = Instant::now() + limit;
    let work = async {
        let addresses = allowed_addresses(url, policy, resolver).await?;
        let tcp = connect_any(&host, &addresses, url.port(), deadline).await?;
        match tls {
            None => Ok(Stream::Plain(tcp)),
            Some(connector) => {
                let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| {
                    Error::Tls {
                        host: host.clone(),
                        message: "the host is not a valid server name".into(),
                    }
                })?;
                let stream = connector
                    .connect(name, tcp)
                    .await
                    .map_err(|error| Error::Tls {
                        host: host.clone(),
                        message: tls_message(&error),
                    })?;
                Ok(Stream::Tls(Box::new(stream)))
            }
        }
    };
    match tokio::time::timeout_at(deadline, work).await {
        Ok(result) => result,
        Err(_) => Err(Error::ConnectTimeout { host }),
    }
}

/// What a failed handshake says, without anything the other end sent.
fn tls_message(error: &io::Error) -> String {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(reason)) => {
            format!("the certificate is not accepted ({reason:?})")
        }
        Some(other) => format!("{other}"),
        None => format!("{}", error.kind()),
    }
}

/// Tries the addresses one after the other. Each gets an equal share of
/// what is left, so one that swallows packets cannot use up the time of the
/// others.
async fn connect_any(
    host: &str,
    addresses: &[IpAddr],
    port: u16,
    deadline: Instant,
) -> Result<TcpStream> {
    let mut last: Option<io::Error> = None;
    for (index, address) in addresses.iter().enumerate() {
        let left = u32::try_from(addresses.len() - index).unwrap_or(1);
        let share = deadline.saturating_duration_since(Instant::now()) / left;
        let attempt = TcpStream::connect(SocketAddr::new(*address, port));
        match tokio::time::timeout(share, attempt).await {
            Ok(Ok(stream)) => {
                // A request is written in one go and the answer waited for:
                // Nagle's delay would only add to the wait.
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(error)) => last = Some(error),
            Err(_) => last = Some(io::ErrorKind::TimedOut.into()),
        }
    }
    Err(Error::Connect {
        host: host.to_owned(),
        source: last.unwrap_or_else(|| io::ErrorKind::NotFound.into()),
    })
}

/// A resolver of a fixed table, for tests.
pub struct TableResolver(pub Vec<(String, Vec<IpAddr>)>);

impl Resolve for TableResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        _port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>> {
        Box::pin(async move {
            self.0
                .iter()
                .find(|(name, _)| name == host)
                .map(|(_, addresses)| addresses.clone())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such host"))
        })
    }
}

/// Keeps the connector type out of the public interface.
pub(crate) fn connector(config: Arc<rustls::ClientConfig>) -> TlsConnector {
    TlsConnector::from(config)
}
