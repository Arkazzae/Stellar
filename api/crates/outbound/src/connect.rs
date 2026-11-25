//! Resolving and judging addresses, establishing TCP connections, and
//! negotiating TLS.
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

/// Connects to a validated address and returns it with the stream. The
/// caller enforces the deadline across DNS, TCP and the TLS handshake.
pub(crate) async fn open(
    url: &Url,
    addresses: &[IpAddr],
    tls: Option<&TlsConnector>,
    deadline: Instant,
) -> Result<(Stream, IpAddr)> {
    let host = url.bare_host();
    let (tcp, peer) = connect_any(host, addresses, url.port(), deadline).await?;
    let Some(connector) = tls else {
        return Ok((Stream::Plain(tcp), peer));
    };
    let name =
        rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|_| Error::Tls {
            host: host.to_owned(),
            message: "the host is not a valid server name".into(),
        })?;
    let stream = connector
        .connect(name, tcp)
        .await
        .map_err(|error| Error::Tls {
            host: host.to_owned(),
            message: tls_message(&error),
        })?;
    Ok((Stream::Tls(Box::new(stream)), peer))
}

/// What a failed handshake says, in words of this crate's own: nothing
/// that the other end wrote, not even the names in its certificate.
fn tls_message(error: &io::Error) -> String {
    use rustls::CertificateError as Certificate;
    let inner = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match inner {
        Some(rustls::Error::InvalidCertificate(reason)) => match reason {
            Certificate::UnknownIssuer => "the certificate's issuer is not trusted",
            Certificate::Expired | Certificate::ExpiredContext { .. } => {
                "the certificate has expired"
            }
            Certificate::NotValidYet | Certificate::NotValidYetContext { .. } => {
                "the certificate is not valid yet"
            }
            Certificate::NotValidForName | Certificate::NotValidForNameContext { .. } => {
                "the certificate is not valid for this host"
            }
            Certificate::BadSignature => "the certificate's signature is wrong",
            Certificate::Revoked => "the certificate was revoked",
            _ => "the certificate is not accepted",
        }
        .to_owned(),
        Some(rustls::Error::AlertReceived(_)) => "the server refused the handshake".to_owned(),
        Some(_) => "the handshake failed".to_owned(),
        None => format!("the handshake failed ({})", error.kind()),
    }
}

/// Tries the addresses one after the other. Each gets an equal share of
/// what is left, so one that swallows packets cannot use up the time of the
/// others. When every attempt ran out of time the connect timed out; when
/// some were refused, the last refusal is the error.
async fn connect_any(
    host: &str,
    addresses: &[IpAddr],
    port: u16,
    deadline: Instant,
) -> Result<(TcpStream, IpAddr)> {
    dial_any(host, addresses, deadline, |address| async move {
        let stream = TcpStream::connect(SocketAddr::new(address, port)).await?;
        // A request is written in one go and the answer waited for:
        // Nagle's delay would only add to the wait.
        let _ = stream.set_nodelay(true);
        Ok((stream, address))
    })
    .await
}

async fn dial_any<S, F, Fut>(
    host: &str,
    addresses: &[IpAddr],
    deadline: Instant,
    dial: F,
) -> Result<S>
where
    F: Fn(IpAddr) -> Fut,
    Fut: Future<Output = io::Result<S>>,
{
    let mut refused: Option<io::Error> = None;
    for (index, address) in addresses.iter().enumerate() {
        let left = u32::try_from(addresses.len() - index).unwrap_or(1);
        let share = deadline.saturating_duration_since(Instant::now()) / left;
        match tokio::time::timeout(share, dial(*address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => refused = Some(error),
            Err(_) => {}
        }
    }
    match refused {
        Some(source) => Err(Error::Connect {
            host: host.to_owned(),
            source,
        }),
        None => Err(Error::ConnectTimeout {
            host: host.to_owned(),
        }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn addresses(count: usize) -> Vec<IpAddr> {
        (1..=count)
            .map(|i| IpAddr::from([192, 0, 2, i as u8]))
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn a_connect_that_swallows_every_packet_is_a_connect_timeout() {
        let deadline = Instant::now() + Duration::from_secs(10);

        let result: Result<()> = dial_any("black.hole", &addresses(2), deadline, |_| {
            std::future::pending::<io::Result<()>>()
        })
        .await;

        let error = result.unwrap_err();
        assert!(matches!(error, Error::ConnectTimeout { .. }), "{error}");
        assert!(error.is_timeout());
    }

    #[tokio::test(start_paused = true)]
    async fn an_address_that_swallows_packets_leaves_time_for_the_next() {
        let deadline = Instant::now() + Duration::from_secs(10);

        let result = dial_any("half.hole", &addresses(2), deadline, |address| async move {
            if address == IpAddr::from([192, 0, 2, 1]) {
                std::future::pending::<io::Result<u8>>().await
            } else {
                Ok(7)
            }
        })
        .await;

        assert_eq!(result.unwrap(), 7);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_is_reported_as_one_even_when_another_address_timed_out() {
        let deadline = Instant::now() + Duration::from_secs(10);

        let result = dial_any(
            "mixed.test",
            &addresses(2),
            deadline,
            |address| async move {
                if address == IpAddr::from([192, 0, 2, 1]) {
                    std::future::pending::<io::Result<u8>>().await
                } else {
                    Err(io::ErrorKind::ConnectionRefused.into())
                }
            },
        )
        .await;

        let error = result.unwrap_err();
        assert!(matches!(error, Error::Connect { .. }), "{error}");
        assert!(!error.is_timeout());
    }
}
