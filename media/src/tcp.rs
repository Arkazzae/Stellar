//! The passive ICE-TCP listener and RFC 4571 packet framing. Socket tasks
//! do their own I/O; the task driving ICE only enqueues complete packets.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use str0m::net::{DatagramSend, Protocol};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinError, JoinSet};
use tokio::time::timeout;
use tracing::debug;

use crate::core::{Core, Endpoint, MAX_DATAGRAM, Outgoing, SessionKey};

const MAX_CONNECTIONS: usize = 512;
const MAX_PENDING: usize = 64;
const MAX_CONNECTIONS_PER_IP: usize = 8;
const MAX_PENDING_PER_IP: usize = 4;
const RECEIVE_QUEUE: usize = 256;
const SEND_QUEUE: usize = 64;
const AUTH_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy)]
pub(crate) struct SocketId {
    endpoint: Endpoint,
    generation: u64,
}

pub(crate) struct Frame {
    socket: SocketId,
    data: Vec<u8>,
    authorize: Option<oneshot::Sender<bool>>,
}

pub(crate) enum Event {
    Accepted(io::Result<(TcpStream, SocketAddr)>),
    Frame(Frame),
    Finished(Result<SocketId, JoinError>),
}

struct Peer {
    generation: u64,
    session: Option<SessionKey>,
    send: mpsc::Sender<DatagramSend>,
    abort: AbortHandle,
}

#[derive(Default)]
struct PeerCounts {
    total: usize,
    pending: usize,
}

pub(crate) struct TcpMedia {
    listener: TcpListener,
    candidates: Vec<SocketAddr>,
    peers: HashMap<Endpoint, Peer>,
    per_ip: HashMap<IpAddr, PeerCounts>,
    pending: usize,
    next_generation: u64,
    next_accept: tokio::time::Instant,
    tasks: JoinSet<SocketId>,
    frame_tx: mpsc::Sender<Frame>,
    frame_rx: mpsc::Receiver<Frame>,
}

impl TcpMedia {
    pub(crate) fn new(listener: TcpListener, candidates: Vec<SocketAddr>) -> Self {
        let (frame_tx, frame_rx) = mpsc::channel(RECEIVE_QUEUE);
        Self {
            listener,
            candidates,
            peers: HashMap::new(),
            per_ip: HashMap::new(),
            pending: 0,
            next_generation: 0,
            next_accept: tokio::time::Instant::now(),
            tasks: JoinSet::new(),
            frame_tx,
            frame_rx,
        }
    }

    pub(crate) async fn next(&mut self) -> Event {
        tokio::select! {
            accepted = async {
                tokio::time::sleep_until(self.next_accept).await;
                self.listener.accept().await
            }, if self.pending < MAX_PENDING && self.tasks.len() < MAX_CONNECTIONS => {
                Event::Accepted(accepted)
            }
            frame = self.frame_rx.recv() => Event::Frame(frame.expect("the listener owns a sender")),
            joined = self.tasks.join_next(), if !self.tasks.is_empty() => {
                Event::Finished(joined.expect("there is a socket task"))
            }
        }
    }

    pub(crate) fn handle(&mut self, core: &mut Core, event: Event) {
        match event {
            Event::Accepted(Ok((stream, remote))) => self.accept(stream, remote),
            Event::Accepted(Err(error)) => {
                self.next_accept = tokio::time::Instant::now() + ACCEPT_RETRY;
                debug!(%error, "accepting a media socket failed");
            }
            Event::Frame(frame) => self.receive(core, frame),
            Event::Finished(Ok(socket)) => self.remove(socket),
            Event::Finished(Err(error)) if !error.is_cancelled() => {
                // A panicked worker must never leave a reservation behind.
                let failed = self
                    .peers
                    .iter()
                    .find(|(_, peer)| peer.abort.id() == error.id());
                if let Some((&endpoint, peer)) = failed {
                    self.remove(SocketId {
                        endpoint,
                        generation: peer.generation,
                    });
                }
                debug!(%error, "a media socket task failed");
            }
            Event::Finished(Err(_)) => {}
        }
    }

    fn accept(&mut self, stream: TcpStream, remote: SocketAddr) {
        let ip = remote.ip().to_canonical();
        if self.per_ip.get(&ip).is_some_and(|count| {
            count.total >= MAX_CONNECTIONS_PER_IP || count.pending >= MAX_PENDING_PER_IP
        }) {
            return;
        }
        let Ok(local) = stream.local_addr() else {
            return;
        };
        if stream.set_nodelay(true).is_err() {
            return;
        }
        // A connection through NAT or a TCP forwarding proxy arrives at the
        // listener's own address. Give str0m an address it has advertised.
        let local = self
            .candidates
            .iter()
            .copied()
            .find(|&addr| addr == local)
            .unwrap_or(self.candidates[0]);
        let endpoint = Endpoint {
            protocol: Protocol::Tcp,
            remote,
            local,
        };
        // Different OS socket tuples can collapse to the same advertised
        // tuple. Never let a second stream take over that first stream.
        if self.peers.contains_key(&endpoint) {
            return;
        }
        let socket = SocketId {
            endpoint,
            generation: self.next_generation,
        };
        self.next_generation = self.next_generation.wrapping_add(1);
        let (send, outgoing) = mpsc::channel(SEND_QUEUE);
        let abort = self
            .tasks
            .spawn(run(stream, socket, self.frame_tx.clone(), outgoing));
        self.peers.insert(
            endpoint,
            Peer {
                generation: socket.generation,
                session: None,
                send,
                abort,
            },
        );
        self.pending += 1;
        let count = self.per_ip.entry(ip).or_default();
        count.total += 1;
        count.pending += 1;
    }

    fn receive(&mut self, core: &mut Core, frame: Frame) {
        let Some(peer) = self.peers.get_mut(&frame.socket.endpoint) else {
            return;
        };
        if peer.generation != frame.socket.generation {
            return;
        }
        let session = core.handle_tcp(
            Instant::now(),
            frame.socket.endpoint,
            &frame.data,
            peer.session,
        );
        if let Some(authorize) = frame.authorize {
            if peer.session.is_none()
                && let Some(session) = session
            {
                peer.session = Some(session);
                self.pending -= 1;
                self.per_ip
                    .get_mut(&frame.socket.endpoint.remote.ip().to_canonical())
                    .expect("every peer has a reservation")
                    .pending -= 1;
            }
            let _ = authorize.send(session.is_some());
        }
    }

    pub(crate) fn send(&mut self, packet: Outgoing) {
        let endpoint = Endpoint {
            protocol: packet.protocol,
            remote: packet.destination,
            local: packet.source,
        };
        let Some(peer) = self.peers.get(&endpoint) else {
            return;
        };
        if peer.session != Some(packet.session) {
            return;
        }
        if peer.send.try_send(packet.contents).is_err() {
            // A slow stream cannot hold up ICE timers or other transports.
            debug!(remote = %endpoint.remote, "closing a backed-up TCP media socket");
            self.remove(SocketId {
                endpoint,
                generation: peer.generation,
            });
        }
    }

    pub(crate) fn remove_sessions(&mut self, closed: &[SessionKey]) {
        if closed.is_empty() {
            return;
        }
        let sockets: Vec<_> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.session.is_some_and(|key| closed.contains(&key)))
            .map(|(&endpoint, peer)| SocketId {
                endpoint,
                generation: peer.generation,
            })
            .collect();
        for socket in sockets {
            self.remove(socket);
        }
    }

    fn remove(&mut self, socket: SocketId) {
        if self
            .peers
            .get(&socket.endpoint)
            .is_none_or(|p| p.generation != socket.generation)
        {
            return;
        }
        let peer = self
            .peers
            .remove(&socket.endpoint)
            .expect("socket still present");
        peer.abort.abort();
        let ip = socket.endpoint.remote.ip().to_canonical();
        let count = self
            .per_ip
            .get_mut(&ip)
            .expect("every peer has a reservation");
        count.total -= 1;
        if peer.session.is_none() {
            self.pending -= 1;
            count.pending -= 1;
        }
        if count.total == 0 {
            self.per_ip.remove(&ip);
        }
        // The Rtc can still nominate another path. ICE's own timers and
        // checks decide when there is no viable connection left.
    }
}

async fn run(
    mut stream: TcpStream,
    socket: SocketId,
    frames: mpsc::Sender<Frame>,
    mut outgoing: mpsc::Receiver<DatagramSend>,
) -> SocketId {
    let (rx, mut tx) = stream.split();
    let mut rx = BufReader::new(rx);
    let result = tokio::select! {
        result = receive_frames(&mut rx, socket, &frames) => result,
        result = send_frames(&mut tx, &mut outgoing) => result,
    };
    if let Err(error) = result {
        debug!(%error, remote = %socket.endpoint.remote, "TCP media socket closed");
    }
    socket
}

async fn receive_frames(
    rx: &mut (impl AsyncRead + Unpin),
    socket: SocketId,
    frames: &mpsc::Sender<Frame>,
) -> io::Result<()> {
    let allowed = timeout(AUTH_TIMEOUT, async {
        let data = read_frame(rx, false).await?;
        let (approve, decision) = oneshot::channel();
        frames
            .send(Frame {
                socket,
                data,
                authorize: Some(approve),
            })
            .await
            .map_err(io::Error::other)?;
        decision.await.map_err(io::Error::other)
    })
    .await??;
    if !allowed {
        return Ok(());
    }

    loop {
        let data = timeout(READ_TIMEOUT, read_frame(rx, true)).await??;
        frames
            .send(Frame {
                socket,
                data,
                authorize: None,
            })
            .await
            .map_err(io::Error::other)?;
    }
}

async fn read_frame(rx: &mut (impl AsyncRead + Unpin), authenticated: bool) -> io::Result<Vec<u8>> {
    loop {
        let len = usize::from(rx.read_u16().await?);
        if len == 0 {
            if !authenticated {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "null frame before ICE authentication",
                ));
            }
            // RFC 4571 null packets carry no media.
            continue;
        }
        if len > MAX_DATAGRAM {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized TCP media frame",
            ));
        }
        let mut frame = vec![0; len];
        rx.read_exact(&mut frame).await?;
        return Ok(frame);
    }
}

async fn send_frames(
    tx: &mut (impl AsyncWrite + Unpin),
    outgoing: &mut mpsc::Receiver<DatagramSend>,
) -> io::Result<()> {
    while let Some(frame) = outgoing.recv().await {
        timeout(WRITE_TIMEOUT, write_frame(tx, &frame)).await??;
    }
    Ok(())
}

async fn write_frame(tx: &mut (impl AsyncWrite + Unpin), frame: &[u8]) -> io::Result<()> {
    let len = u16::try_from(frame.len()).map_err(io::Error::other)?;
    tx.write_u16(len).await?;
    tx.write_all(frame).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn reads_partial_and_consecutive_frames_and_ignores_null_frames() {
        let (mut tx, mut rx) = duplex(4);
        let writer = tokio::spawn(async move {
            for chunk in [&[0][..], &[3, 11], &[22, 33, 0, 0], &[0, 1, 44]] {
                tx.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert_eq!(read_frame(&mut rx, true).await.unwrap(), [11, 22, 33]);
        assert_eq!(read_frame(&mut rx, true).await.unwrap(), [44]);
        assert_eq!(
            read_frame(&mut rx, true).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_null_frames_before_authentication() {
        let (mut tx, mut rx) = duplex(8);
        tx.write_all(&[0, 0, 0, 1, 42]).await.unwrap();
        assert_eq!(
            read_frame(&mut rx, false).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn rejects_oversized_and_truncated_frames() {
        let (mut tx, mut rx) = duplex(4);
        tx.write_u16((MAX_DATAGRAM + 1).try_into().unwrap())
            .await
            .unwrap();
        assert_eq!(
            read_frame(&mut rx, true).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        tx.write_all(&[0, 2, 1]).await.unwrap();
        drop(tx);
        assert_eq!(
            read_frame(&mut rx, true).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
