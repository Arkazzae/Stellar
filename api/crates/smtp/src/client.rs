//! The SMTP client: one message to its recipients over one connection.
//!
//! The exchange is greeting, EHLO, the security the settings ask for
//! (implicit TLS, STARTTLS or nothing), authentication when there is a user
//! name, MAIL FROM, RCPT TO, DATA and QUIT. Every read and write has a
//! timeout, and the whole exchange has a deadline.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, lookup_host};
use tokio::time::{Instant, timeout};
use tokio_rustls::TlsConnector;

use crate::address::Mailbox;
use crate::auth::{self, Mechanism, Scram};
use crate::encode::{base64, base64_decode};
use crate::error::{Error, ErrorKind, Stage};
use crate::message::Message;

/// A server may send a long reply line (a list of extensions, a banner), but
/// not an unbounded one.
const MAX_REPLY_LINE: usize = 4096;
const MAX_REPLY_LINES: usize = 128;

/// How the connection is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// A plain connection upgraded with STARTTLS, which must succeed; the
    /// usual port is 587.
    StartTls,
    /// TLS from the first byte; the usual port is 465.
    Tls,
    /// No encryption, for a relay on the same host or network.
    None,
}

impl Security {
    /// The setting's name as the operator writes it: `starttls`, `tls` or
    /// `none`, in any case. Anything else, and no setting at all, is
    /// STARTTLS: the safe choice that works on the usual submission port.
    pub fn parse(name: Option<&str>) -> Security {
        match name.map(|n| n.trim().to_ascii_lowercase()).as_deref() {
            Some("tls") => Security::Tls,
            Some("none") => Security::None,
            _ => Security::StartTls,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// Empty means the server is not logged in to.
    pub username: String,
    pub password: String,
}

// Printing a connection must never print the password.
impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// The longest one read, write, handshake or connect attempt may take.
    pub step_timeout: Duration,
    /// The longest the whole exchange may take. Password recovery waits on
    /// it, so a server that hangs must not hold the request for long.
    pub total_timeout: Duration,
    /// The name given in EHLO. Without it the client announces the local
    /// address of the connection, which is what a host without a name may do.
    pub local_name: Option<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            step_timeout: Duration::from_secs(15),
            total_timeout: Duration::from_secs(15),
            local_name: None,
        }
    }
}

/// What the server said about a message it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub message_id: String,
    /// The server's last reply, often with its queue id.
    pub reply: String,
}

/// Connects and sends the message.
pub async fn send(
    connection: &Connection,
    tls: Arc<ClientConfig>,
    options: &Options,
    message: &Message,
) -> Result<Delivery, Error> {
    let at = |error: Error| error.with_endpoint(&connection.host, connection.port);
    let prepared = Prepared::new(message).map_err(at)?;
    let clock = Clock::new(options);
    let stream = connect(connection, &clock).await.map_err(at)?;
    let local = match &options.local_name {
        Some(name) => name.clone(),
        None => stream
            .local_addr()
            .map(|a| address_literal(a.ip()))
            .unwrap_or_else(|_| "localhost".into()),
    };
    run(Box::new(stream), connection, tls, clock, &prepared, &local)
        .await
        .map_err(at)
}

/// Sends the message over a stream that is already connected. This is the
/// whole client minus the TCP connect, for a connection the caller made
/// itself and for tests.
pub async fn send_over<S>(
    stream: S,
    connection: &Connection,
    tls: Arc<ClientConfig>,
    options: &Options,
    message: &Message,
) -> Result<Delivery, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let at = |error: Error| error.with_endpoint(&connection.host, connection.port);
    let prepared = Prepared::new(message).map_err(at)?;
    let clock = Clock::new(options);
    let local = options
        .local_name
        .clone()
        .unwrap_or_else(|| "localhost".into());
    run(Box::new(stream), connection, tls, clock, &prepared, &local)
        .await
        .map_err(at)
}

fn address_literal(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => format!("[{v4}]"),
        IpAddr::V6(v6) => format!("[IPv6:{v6}]"),
    }
}

/// The message in its wire form and the envelope it goes with.
struct Prepared {
    bytes: Vec<u8>,
    sender: String,
    recipients: Vec<String>,
    message_id: String,
    utf8: bool,
}

impl Prepared {
    fn new(message: &Message) -> Result<Prepared, Error> {
        let encoded = message.encode()?;
        let recipients: Vec<String> = message.to.iter().map(|m| m.address().to_owned()).collect();
        let sender = message.from.address().to_owned();
        let utf8 = encoded.needs_utf8
            || message.from.needs_utf8()
            || message.to.iter().any(Mailbox::needs_utf8);
        Ok(Prepared {
            bytes: encoded.bytes,
            sender,
            recipients,
            message_id: message.message_id.clone(),
            utf8,
        })
    }
}

async fn connect(connection: &Connection, clock: &Clock) -> Result<TcpStream, Error> {
    if connection.host.trim().is_empty() {
        return Err(Error::new(
            ErrorKind::Connect,
            Stage::Connect,
            false,
            "no SMTP host is set",
        ));
    }
    let limit = clock.limit(Stage::Connect)?;
    let found = timeout(
        limit,
        lookup_host((connection.host.as_str(), connection.port)),
    )
    .await
    .map_err(|_| clock.late(Stage::Connect, limit))?
    .map_err(|e| {
        Error::new(
            ErrorKind::Connect,
            Stage::Connect,
            true,
            format!("the host name did not resolve: {e}"),
        )
    })?;
    let addresses: Vec<SocketAddr> = found.collect();
    let mut last = Error::new(
        ErrorKind::Connect,
        Stage::Connect,
        true,
        "the host has no address",
    );
    for (index, address) in addresses.iter().enumerate() {
        // Each address gets its share of what is left, so that a first one
        // that does not answer cannot use up the others' time.
        let left = (addresses.len() - index) as u32;
        let limit = clock.limit_within(clock.left() / left, Stage::Connect)?;
        match timeout(limit, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                // Mail is a conversation of short lines.
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(e)) => {
                last = Error::new(
                    ErrorKind::Connect,
                    Stage::Connect,
                    true,
                    format!("{address}: {e}"),
                );
            }
            Err(_) => last = clock.late(Stage::Connect, limit),
        }
    }
    Err(last)
}

/// The two bounds on every step: how long one step may take, and when the
/// whole exchange must be over.
#[derive(Clone, Copy)]
struct Clock {
    step: Duration,
    total: Duration,
    deadline: Instant,
}

impl Clock {
    fn new(options: &Options) -> Clock {
        Clock {
            step: options.step_timeout,
            total: options.total_timeout,
            deadline: Instant::now() + options.total_timeout,
        }
    }

    fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// How long the next step may take.
    fn limit(&self, stage: Stage) -> Result<Duration, Error> {
        self.limit_within(self.step, stage)
    }

    fn limit_within(&self, cap: Duration, stage: Stage) -> Result<Duration, Error> {
        let limit = cap.min(self.step).min(self.left());
        if limit.is_zero() {
            return Err(self.late(stage, limit));
        }
        Ok(limit)
    }

    /// The error for a step that did not finish in `limit`.
    fn late(&self, stage: Stage, limit: Duration) -> Error {
        if limit.is_zero() {
            return Error::new(
                ErrorKind::Timeout,
                stage,
                true,
                format!(
                    "the exchange did not finish within the {:.0} seconds allowed",
                    self.total.as_secs_f64()
                ),
            );
        }
        Error::timeout(stage, limit.as_secs_f64())
    }
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn run(
    io: Box<dyn Io>,
    connection: &Connection,
    tls: Arc<ClientConfig>,
    clock: Clock,
    mail: &Prepared,
    local_name: &str,
) -> Result<Delivery, Error> {
    if local_name.is_empty()
        || !local_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.[]:".contains(&b))
    {
        return Err(Error::invalid(format!(
            "{local_name:?} is not a name to announce in EHLO"
        )));
    }
    // A name is only needed, and only has to be a valid one, when TLS is.
    let server_name = || {
        ServerName::try_from(connection.host.trim().to_owned()).map_err(|_| {
            Error::unsupported(
                Stage::Handshake,
                "the host is not a valid server name for TLS",
            )
        })
    };
    let connector = TlsConnector::from(tls);
    let mut session = Session {
        io,
        buffer: Vec::new(),
        clock,
    };

    if connection.security == Security::Tls {
        session.handshake(&connector, server_name()?).await?;
    }
    let greeting = session.read_reply(Stage::Greeting).await?;
    session.expect(Stage::Greeting, &greeting, &[220])?;

    let mut caps = session.hello(local_name).await?;
    if connection.security == Security::StartTls {
        if !caps.starttls {
            return Err(Error::unsupported(
                Stage::StartTls,
                "the server does not offer STARTTLS; set the connection security to match the server",
            ));
        }
        let reply = session.command(Stage::StartTls, "STARTTLS").await?;
        session.expect(Stage::StartTls, &reply, &[220])?;
        if !session.buffer.is_empty() {
            // Bytes that arrived before the handshake were sent in the clear
            // and would otherwise be read as if they came through TLS.
            return Err(Error::protocol(
                Stage::StartTls,
                "the server sent data before the TLS handshake",
            ));
        }
        session.handshake(&connector, server_name()?).await?;
        caps = session.hello(local_name).await?;
    }

    if !connection.username.is_empty() {
        session
            .authenticate(&caps, &connection.username, &connection.password)
            .await?;
    }

    if mail.utf8 && !caps.smtputf8 {
        return Err(Error::unsupported(
            Stage::Sender,
            "an address has non-ASCII letters before the @ and the server does not offer SMTPUTF8",
        ));
    }
    if let Some(limit) = caps.size.filter(|&l| l > 0)
        && mail.bytes.len() as u64 > limit
    {
        return Err(Error::unsupported(
            Stage::Data,
            format!(
                "the message is {} bytes and the server accepts at most {limit}",
                mail.bytes.len()
            ),
        ));
    }
    let mut from = format!("MAIL FROM:<{}>", mail.sender);
    if caps.size.is_some() {
        from.push_str(&format!(" SIZE={}", mail.bytes.len()));
    }
    if mail.utf8 {
        from.push_str(" SMTPUTF8");
    }
    let reply = session.command(Stage::Sender, &from).await?;
    session.expect(Stage::Sender, &reply, &[250])?;
    for recipient in &mail.recipients {
        let reply = session
            .command(Stage::Recipient, &format!("RCPT TO:<{recipient}>"))
            .await?;
        if reply.code != 250 && reply.code != 251 {
            return Err(Error::rejected(
                Stage::Recipient,
                reply.code,
                &format!("{} (for {recipient})", reply.text()),
            ));
        }
    }
    let reply = session.command(Stage::Data, "DATA").await?;
    session.expect(Stage::Data, &reply, &[354])?;
    session.write(Stage::Data, &dot_stuff(&mail.bytes)).await?;
    // From here on the server has the whole message: a failure to hear its
    // answer leaves it unknown whether the message was taken.
    let end = match session.read_reply(Stage::EndOfData).await {
        Ok(reply) => reply,
        Err(error) => return Err(error.delivered_maybe()),
    };
    session.expect(Stage::EndOfData, &end, &[250])?;
    tracing::debug!(
        recipients = mail.recipients.len(),
        "the server took the message"
    );

    session.quit().await;
    Ok(Delivery {
        message_id: mail.message_id.clone(),
        reply: end.text(),
    })
}

/// The message as DATA carries it: a line that starts with a dot gets a
/// second one, and a line of a single dot ends it.
fn dot_stuff(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 16);
    let mut line_start = true;
    for &b in message {
        if line_start && b == b'.' {
            out.push(b'.');
        }
        out.push(b);
        line_start = b == b'\n';
    }
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

#[derive(Debug, Default)]
struct Capabilities {
    starttls: bool,
    auth: Vec<String>,
    size: Option<u64>,
    smtputf8: bool,
}

impl Capabilities {
    /// The lines of an EHLO answer after the greeting line.
    fn parse(lines: &[String]) -> Capabilities {
        let mut caps = Capabilities::default();
        for line in lines.iter().skip(1) {
            let mut words = line.split_whitespace();
            let Some(keyword) = words.next() else {
                continue;
            };
            let keyword = keyword.to_ascii_uppercase();
            match keyword.as_str() {
                "STARTTLS" => caps.starttls = true,
                "SMTPUTF8" => caps.smtputf8 = true,
                "SIZE" => caps.size = words.next().and_then(|n| n.parse().ok()),
                "AUTH" => caps.auth.extend(words.map(|w| w.to_ascii_uppercase())),
                // The form some old servers still announce.
                _ => {
                    if let Some(first) = keyword.strip_prefix("AUTH=") {
                        caps.auth.push(first.to_owned());
                        caps.auth.extend(words.map(|w| w.to_ascii_uppercase()));
                    }
                }
            }
        }
        caps
    }
}

#[derive(Debug)]
struct Reply {
    code: u16,
    lines: Vec<String>,
}

impl Reply {
    fn text(&self) -> String {
        self.lines.join(" ")
    }
}

struct Session {
    io: Box<dyn Io>,
    /// Bytes read from the server and not yet taken as a line.
    buffer: Vec<u8>,
    clock: Clock,
}

impl Session {
    fn limit(&self, stage: Stage) -> Result<Duration, Error> {
        self.clock.limit(stage)
    }

    async fn write(&mut self, stage: Stage, data: &[u8]) -> Result<(), Error> {
        for chunk in data.chunks(16 * 1024) {
            let limit = self.limit(stage)?;
            match timeout(limit, self.io.write_all(chunk)).await {
                Err(_) => return Err(self.clock.late(stage, limit)),
                Ok(Err(e)) => {
                    return Err(Error::io(
                        stage,
                        format!("writing to the server failed: {e}"),
                    ));
                }
                Ok(Ok(())) => {}
            }
        }
        let limit = self.limit(stage)?;
        match timeout(limit, self.io.flush()).await {
            Err(_) => Err(self.clock.late(stage, limit)),
            Ok(Err(e)) => Err(Error::io(
                stage,
                format!("writing to the server failed: {e}"),
            )),
            Ok(Ok(())) => Ok(()),
        }
    }

    /// One line from the server without its break.
    async fn read_line(&mut self, stage: Stage) -> Result<Vec<u8>, Error> {
        loop {
            if let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
                if end > MAX_REPLY_LINE {
                    return Err(Error::protocol(
                        stage,
                        "the server sent a reply line that is too long",
                    ));
                }
                let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(line);
            }
            if self.buffer.len() > MAX_REPLY_LINE {
                return Err(Error::protocol(
                    stage,
                    "the server sent a reply line that is too long",
                ));
            }
            let limit = self.limit(stage)?;
            let mut chunk = [0u8; 2048];
            match timeout(limit, self.io.read(&mut chunk)).await {
                Err(_) => return Err(self.clock.late(stage, limit)),
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(Error::io(stage, "the server closed the connection"));
                }
                Ok(Err(e)) => {
                    return Err(Error::io(
                        stage,
                        format!("reading from the server failed: {e}"),
                    ));
                }
                Ok(Ok(0)) => return Err(Error::io(stage, "the server closed the connection")),
                Ok(Ok(n)) => self.buffer.extend_from_slice(&chunk[..n]),
            }
        }
    }

    async fn read_reply(&mut self, stage: Stage) -> Result<Reply, Error> {
        let mut code = None;
        let mut lines = Vec::new();
        loop {
            let line = self.read_line(stage).await?;
            let (line_code, more, text) = parse_reply_line(&line)
                .ok_or_else(|| Error::protocol(stage, "the server's reply is not SMTP"))?;
            if *code.get_or_insert(line_code) != line_code {
                return Err(Error::protocol(
                    stage,
                    "the lines of a reply have different codes",
                ));
            }
            lines.push(text);
            if !more {
                break;
            }
            if lines.len() >= MAX_REPLY_LINES {
                return Err(Error::protocol(
                    stage,
                    "the server's reply has too many lines",
                ));
            }
        }
        let code = code.unwrap_or(0);
        tracing::debug!(code, ?stage, "smtp reply");
        Ok(Reply { code, lines })
    }

    async fn command(&mut self, stage: Stage, line: &str) -> Result<Reply, Error> {
        if line.contains(['\r', '\n']) {
            return Err(Error::invalid("a command holds a line break"));
        }
        let mut data = Vec::with_capacity(line.len() + 2);
        data.extend_from_slice(line.as_bytes());
        data.extend_from_slice(b"\r\n");
        self.write(stage, &data).await?;
        self.read_reply(stage).await
    }

    fn expect(&self, stage: Stage, reply: &Reply, codes: &[u16]) -> Result<(), Error> {
        if codes.contains(&reply.code) {
            Ok(())
        } else {
            Err(Error::rejected(stage, reply.code, &reply.text()))
        }
    }

    async fn handshake(
        &mut self,
        connector: &TlsConnector,
        name: ServerName<'static>,
    ) -> Result<(), Error> {
        let limit = self.limit(Stage::Handshake)?;
        let plain = std::mem::replace(
            &mut self.io,
            Box::new(tokio::io::join(tokio::io::empty(), tokio::io::sink())),
        );
        match timeout(limit, connector.connect(name, plain)).await {
            Err(_) => Err(self.clock.late(Stage::Handshake, limit)),
            Ok(Err(e)) => {
                // A verdict of the TLS library (an untrusted or wrong
                // certificate, no common protocol) will not change on a
                // second try; a connection that broke might.
                let verdict = e.get_ref().is_some_and(|inner| inner.is::<rustls::Error>());
                Err(Error::new(
                    ErrorKind::Tls,
                    Stage::Handshake,
                    !verdict,
                    e.to_string(),
                ))
            }
            Ok(Ok(stream)) => {
                self.io = Box::new(stream);
                Ok(())
            }
        }
    }

    /// EHLO, with HELO for a server that does not know it.
    async fn hello(&mut self, name: &str) -> Result<Capabilities, Error> {
        let reply = self.command(Stage::Hello, &format!("EHLO {name}")).await?;
        if reply.code == 250 {
            return Ok(Capabilities::parse(&reply.lines));
        }
        if (500..=504).contains(&reply.code) {
            let reply = self.command(Stage::Hello, &format!("HELO {name}")).await?;
            self.expect(Stage::Hello, &reply, &[250])?;
            return Ok(Capabilities::default());
        }
        Err(Error::rejected(Stage::Hello, reply.code, &reply.text()))
    }

    async fn authenticate(
        &mut self,
        caps: &Capabilities,
        user: &str,
        password: &str,
    ) -> Result<(), Error> {
        if caps.auth.is_empty() {
            return Err(Error::unsupported(
                Stage::Auth,
                "a user name is set and the server does not offer authentication",
            ));
        }
        let candidates = Mechanism::candidates(&caps.auth, user, password);
        if candidates.is_empty() {
            return Err(Error::unsupported(
                Stage::Auth,
                format!(
                    "the server offers no authentication this client can use (it offers {})",
                    caps.auth.join(", ")
                ),
            ));
        }
        let mut refusal = None;
        for mechanism in candidates {
            tracing::debug!(mechanism = mechanism.name(), "authenticating");
            match self.authenticate_with(mechanism, user, password).await {
                Ok(()) => return Ok(()),
                // A refusal may be the mechanism's and not the login's; the
                // next one decides. Anything else ends it.
                Err(error) if error.kind() == ErrorKind::Rejected && !error.is_transient() => {
                    refusal = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(refusal.unwrap_or_else(|| Error::protocol(Stage::Auth, "no mechanism was tried")))
    }

    async fn authenticate_with(
        &mut self,
        mechanism: Mechanism,
        user: &str,
        password: &str,
    ) -> Result<(), Error> {
        let stage = Stage::Auth;
        let start = self
            .command(stage, &format!("AUTH {}", mechanism.name()))
            .await?;
        self.expect(stage, &start, &[334])?;
        match mechanism {
            Mechanism::Plain => {
                let reply = self
                    .command(stage, &base64(&auth::plain(user, password)))
                    .await?;
                self.expect(stage, &reply, &[235])
            }
            Mechanism::Login => {
                let reply = self.command(stage, &base64(user.as_bytes())).await?;
                self.expect(stage, &reply, &[334])?;
                let reply = self.command(stage, &base64(password.as_bytes())).await?;
                self.expect(stage, &reply, &[235])
            }
            Mechanism::ScramSha256 | Mechanism::ScramSha1 => {
                let (mut scram, first) = Scram::new(mechanism, user, password)
                    .ok_or_else(|| Error::protocol(stage, "no random nonce could be made"))?;
                let reply = self.command(stage, &base64(&first)).await?;
                self.expect(stage, &reply, &[334])?;
                let challenge = decode_challenge(stage, &reply)?;
                let last = scram
                    .respond(&challenge)
                    .map_err(|why| Error::protocol(stage, why))?;
                let reply = self.command(stage, &base64(&last)).await?;
                match reply.code {
                    334 => {
                        scram
                            .verify(&decode_challenge(stage, &reply)?)
                            .map_err(|why| Error::protocol(stage, why))?;
                        let done = self.command(stage, "").await?;
                        self.expect(stage, &done, &[235])
                    }
                    235 => {
                        // A server may carry its final message in the success
                        // reply; when it does, it must check out.
                        if let Some(data) =
                            base64_decode(&reply.text()).filter(|d| d.starts_with(b"v="))
                        {
                            scram
                                .verify(&data)
                                .map_err(|why| Error::protocol(stage, why))?;
                        }
                        Ok(())
                    }
                    code => Err(Error::rejected(stage, code, &reply.text())),
                }
            }
        }
    }

    /// QUIT, and no complaint when it fails: the message is taken already.
    async fn quit(&mut self) {
        let Ok(limit) = self.limit(Stage::EndOfData) else {
            return;
        };
        let _ = timeout(limit.min(Duration::from_secs(2)), async {
            self.io.write_all(b"QUIT\r\n").await?;
            self.io.flush().await?;
            let mut sink = [0u8; 256];
            let _ = self.io.read(&mut sink).await;
            self.io.shutdown().await
        })
        .await;
    }
}

fn decode_challenge(stage: Stage, reply: &Reply) -> Result<Vec<u8>, Error> {
    base64_decode(reply.lines.first().map_or("", String::as_str).trim())
        .ok_or_else(|| Error::protocol(stage, "the server's challenge is not base64"))
}

/// `250-text`, `250 text` or a bare `250`: the code, whether more lines
/// follow, and the text, with control characters taken out.
fn parse_reply_line(line: &[u8]) -> Option<(u16, bool, String)> {
    if line.len() < 3 || !line[..3].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let code = u16::from(line[0] - b'0') * 100
        + u16::from(line[1] - b'0') * 10
        + u16::from(line[2] - b'0');
    if !(100..600).contains(&code) {
        return None;
    }
    let more = match line.get(3) {
        None | Some(b' ') => false,
        Some(b'-') => true,
        Some(_) => return None,
    };
    let text: String = String::from_utf8_lossy(line.get(4..).unwrap_or(&[]))
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    Some((code, more, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_security_setting_is_read_like_the_operator_writes_it() {
        assert_eq!(Security::parse(Some("tls")), Security::Tls);
        assert_eq!(Security::parse(Some(" TLS ")), Security::Tls);
        assert_eq!(Security::parse(Some("None")), Security::None);
        assert_eq!(Security::parse(Some("starttls")), Security::StartTls);
        assert_eq!(Security::parse(Some("ssl")), Security::StartTls);
        assert_eq!(Security::parse(Some("")), Security::StartTls);
        assert_eq!(Security::parse(None), Security::StartTls);
    }

    #[test]
    fn a_connection_never_prints_its_password() {
        let c = Connection {
            host: "h".into(),
            port: 25,
            security: Security::None,
            username: "u".into(),
            password: "hunter2".into(),
        };
        assert!(!format!("{c:?}").contains("hunter2"));
    }

    #[test]
    fn dots_at_the_start_of_lines_are_doubled() {
        assert_eq!(
            dot_stuff(b"a\r\n.b\r\n..\r\n.\r\n"),
            b"a\r\n..b\r\n...\r\n..\r\n.\r\n"
        );
        assert_eq!(dot_stuff(b".x"), b"..x\r\n.\r\n");
        assert_eq!(dot_stuff(b""), b"\r\n.\r\n");
        assert_eq!(dot_stuff(b"a.b\r\n"), b"a.b\r\n.\r\n");
    }

    #[test]
    fn reply_lines_are_parsed_strictly() {
        assert_eq!(parse_reply_line(b"250 ok"), Some((250, false, "ok".into())));
        assert_eq!(
            parse_reply_line(b"250-SIZE 100"),
            Some((250, true, "SIZE 100".into()))
        );
        assert_eq!(parse_reply_line(b"354"), Some((354, false, String::new())));
        for bad in [
            &b""[..],
            b"25",
            b"abc def",
            b"2500 x",
            b"250x",
            b"099 x",
            b"600 x",
            b"HTTP/1.1 400",
        ] {
            assert_eq!(
                parse_reply_line(bad),
                None,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        assert_eq!(
            parse_reply_line(b"250 a\x07b\x1b[0m"),
            Some((250, false, "ab[0m".into()))
        );
    }

    #[test]
    fn extensions_are_read_from_the_ehlo_lines() {
        let lines: Vec<String> = [
            "mail.test hello",
            "size 35882577",
            "STARTTLS",
            "AUTH PLAIN login",
            "AUTH=CRAM-MD5",
            "SMTPUTF8",
            "8BITMIME",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let caps = Capabilities::parse(&lines);
        assert!(caps.starttls && caps.smtputf8);
        assert_eq!(caps.size, Some(35_882_577));
        assert_eq!(caps.auth, ["PLAIN", "LOGIN", "CRAM-MD5"]);
    }

    #[test]
    fn address_literals_follow_rfc_5321() {
        assert_eq!(address_literal("10.0.0.1".parse().unwrap()), "[10.0.0.1]");
        assert_eq!(address_literal("::1".parse().unwrap()), "[IPv6:::1]");
    }
}
