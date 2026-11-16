//! Readiness checks that never block. Each one is a nonblocking socket the
//! main loop polls next to everything else, with a deadline after which it
//! counts as failed.

use std::hash::{BuildHasher, RandomState};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::os::fd::AsRawFd;
use std::time::Instant;

use crate::service::Readiness;
use crate::sys;

/// The most of an HTTP response read while looking for its status line.
const MAX_RESPONSE: usize = 1024;

const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_SUCCESS: u16 = 0x0101;
const STUN_MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];

/// One readiness check under way.
pub struct Probe {
    pub index: usize,
    pub run: u64,
    pub deadline: Instant,
    step: Step,
}

enum Step {
    /// Waiting for the TCP handshake; then sending `request`, or done if
    /// there is none.
    Connecting {
        stream: TcpStream,
        request: Option<Vec<u8>>,
    },
    Sending {
        stream: TcpStream,
        request: Vec<u8>,
        sent: usize,
    },
    Receiving {
        stream: TcpStream,
        response: Vec<u8>,
    },
    Stun {
        socket: UdpSocket,
        id: [u8; 12],
    },
    Spent,
}

impl Probe {
    /// Starts a check of run `run` of service `index`, which fails when it
    /// has no answer by `deadline`. An error means the check failed at once,
    /// such as a connection refused on the spot.
    pub fn start(
        index: usize,
        run: u64,
        check: &Readiness,
        deadline: Instant,
    ) -> io::Result<Probe> {
        let step = match check {
            Readiness::Running(_) => {
                return Err(io::Error::other("there is nothing to check"));
            }
            Readiness::Tcp(addr) => Step::Connecting {
                stream: sys::connect_nonblocking(*addr)?,
                request: None,
            },
            Readiness::Http {
                addr,
                path,
                headers,
            } => Step::Connecting {
                stream: sys::connect_nonblocking(*addr)?,
                request: Some(http_request(*addr, path, headers)),
            },
            Readiness::Stun(addr) => {
                let local = if addr.is_ipv4() {
                    "127.0.0.1:0"
                } else {
                    "[::1]:0"
                };
                let socket = UdpSocket::bind(local)?;
                socket.connect(addr)?;
                socket.set_nonblocking(true)?;
                let id = transaction_id();
                socket.send(&stun_request(&id))?;
                Step::Stun { socket, id }
            }
        };
        Ok(Probe {
            index,
            run,
            deadline,
            step,
        })
    }

    /// What to wait for on which descriptor.
    pub fn interest(&self) -> libc::pollfd {
        let (fd, events) = match &self.step {
            Step::Connecting { stream, .. } | Step::Sending { stream, .. } => {
                (stream.as_raw_fd(), libc::POLLOUT)
            }
            Step::Receiving { stream, .. } => (stream.as_raw_fd(), libc::POLLIN),
            Step::Stun { socket, .. } => (socket.as_raw_fd(), libc::POLLIN),
            Step::Spent => (-1, 0),
        };
        libc::pollfd {
            fd,
            events,
            revents: 0,
        }
    }

    /// Moves the check on after `poll` reported `revents` for it. `Some`
    /// once it has an answer.
    pub fn advance(&mut self, revents: i16) -> Option<bool> {
        let step = std::mem::replace(&mut self.step, Step::Spent);
        match next(step, revents) {
            Ok(step) => {
                self.step = step;
                None
            }
            Err(ready) => Some(ready),
        }
    }
}

/// The step after this one, or `Err` with the answer.
fn next(step: Step, revents: i16) -> Result<Step, bool> {
    match step {
        Step::Connecting { stream, request } => {
            if revents & (libc::POLLOUT | libc::POLLERR | libc::POLLHUP) == 0 {
                return Ok(Step::Connecting { stream, request });
            }
            if !matches!(stream.take_error(), Ok(None)) || stream.peer_addr().is_err() {
                return Err(false);
            }
            match request {
                None => Err(true),
                Some(request) => send(stream, request, 0),
            }
        }
        Step::Sending {
            stream,
            request,
            sent,
        } => send(stream, request, sent),
        Step::Receiving { stream, response } => receive(stream, response),
        Step::Stun { socket, id } => stun_receive(socket, id),
        Step::Spent => Err(false),
    }
}

fn http_request(addr: SocketAddr, path: &str, headers: &[(String, String)]) -> Vec<u8> {
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: stellarr-init\r\nConnection: close\r\n"
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.into_bytes()
}

fn send(mut stream: TcpStream, request: Vec<u8>, mut sent: usize) -> Result<Step, bool> {
    while sent < request.len() {
        match stream.write(&request[sent..]) {
            Ok(0) => return Err(false),
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(Step::Sending {
                    stream,
                    request,
                    sent,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(false),
        }
    }
    receive(stream, Vec::new())
}

fn receive(mut stream: TcpStream, mut response: Vec<u8>) -> Result<Step, bool> {
    let mut buf = [0u8; 512];
    loop {
        if let Some(code) = status_code(&response) {
            return Err((200..300).contains(&code));
        }
        if response.len() >= MAX_RESPONSE {
            return Err(false);
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err(false),
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(Step::Receiving { stream, response });
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(false),
        }
    }
}

/// The status code of an HTTP/1.x response, once its status line is
/// complete; 0 for a line that is not one.
pub fn status_code(response: &[u8]) -> Option<u16> {
    let end = response.iter().position(|&b| b == b'\n')?;
    let line = &response[..end];
    let parsed = (|| {
        let rest = line.strip_prefix(b"HTTP/1.")?;
        let (version, rest) = rest.split_first()?;
        if !version.is_ascii_digit() {
            return None;
        }
        let rest = rest.strip_prefix(b" ")?;
        let digits = rest.get(..3)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        if !matches!(rest.get(3), None | Some(b' ' | b'\r')) {
            return None;
        }
        std::str::from_utf8(digits).ok()?.parse().ok()
    })();
    Some(parsed.unwrap_or(0))
}

fn transaction_id() -> [u8; 12] {
    // Each RandomState has fresh keys from the system's random source, which
    // is plenty to tell this request's answer from a stray one.
    let a = RandomState::new().hash_one(0u8).to_ne_bytes();
    let b = RandomState::new().hash_one(1u8).to_ne_bytes();
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&a);
    id[8..].copy_from_slice(&b[..4]);
    id
}

/// A STUN binding request (RFC 8489) with no attributes.
pub fn stun_request(id: &[u8; 12]) -> [u8; 20] {
    let mut packet = [0u8; 20];
    packet[..2].copy_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    // The length of the attributes, of which there are none.
    packet[2..4].copy_from_slice(&0u16.to_be_bytes());
    packet[4..8].copy_from_slice(&STUN_MAGIC_COOKIE);
    packet[8..].copy_from_slice(id);
    packet
}

/// Whether `packet` is a binding success response to the request `id`.
pub fn is_stun_success(packet: &[u8], id: &[u8; 12]) -> bool {
    if packet.len() < 20 {
        return false;
    }
    let kind = u16::from_be_bytes([packet[0], packet[1]]);
    let length = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    kind == STUN_BINDING_SUCCESS
        && 20 + length <= packet.len()
        && packet[4..8] == STUN_MAGIC_COOKIE
        && &packet[8..20] == id
}

fn stun_receive(socket: UdpSocket, id: [u8; 12]) -> Result<Step, bool> {
    let mut buf = [0u8; 1500];
    loop {
        match socket.recv(&mut buf) {
            // Anything else, a late answer to an earlier check say, is
            // skipped.
            Ok(n) => {
                if is_stun_success(&buf[..n], &id) {
                    return Err(true);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(Step::Stun { socket, id });
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // Connection refused: nothing listens on the port.
            Err(_) => return Err(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_are_read_only_when_complete() {
        assert_eq!(status_code(b""), None);
        assert_eq!(status_code(b"HTTP/1.1 200 OK"), None);
        assert_eq!(status_code(b"HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(status_code(b"HTTP/1.0 204\r\n"), Some(204));
        assert_eq!(
            status_code(b"HTTP/1.1 503 Service Unavailable\r\nx"),
            Some(503)
        );
        assert_eq!(status_code(b"HTTP/1.1 2000 OK\r\n"), Some(0));
        assert_eq!(status_code(b"SSH-2.0-OpenSSH\r\n"), Some(0));
        assert_eq!(status_code(b"HTTP/2 200\r\n"), Some(0));
    }

    #[test]
    fn a_binding_success_must_answer_this_request() {
        let id = [7u8; 12];
        let request = stun_request(&id);
        assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);

        let mut answer = request;
        answer[..2].copy_from_slice(&STUN_BINDING_SUCCESS.to_be_bytes());
        assert!(is_stun_success(&answer, &id));
        assert!(!is_stun_success(&answer, &[8u8; 12]));
        assert!(!is_stun_success(&request, &id));
        assert!(!is_stun_success(&answer[..19], &id));

        // An error response, 0x0111, is no success.
        let mut error = answer;
        error[1] = 0x11;
        assert!(!is_stun_success(&error, &id));

        // A length that runs past the datagram.
        let mut long = answer;
        long[3] = 8;
        assert!(!is_stun_success(&long, &id));
    }

    #[test]
    fn transaction_ids_differ() {
        assert_ne!(transaction_id(), transaction_id());
    }

    #[test]
    fn an_http_check_sends_its_headers() {
        let addr: SocketAddr = "127.0.0.1:80".parse().unwrap();
        let headers = vec![("X-Forwarded-Proto".to_string(), "https".to_string())];
        assert_eq!(
            String::from_utf8(http_request(addr, "/index.html", &headers)).unwrap(),
            "GET /index.html HTTP/1.1\r\nHost: 127.0.0.1:80\r\nUser-Agent: stellarr-init\r\n\
             Connection: close\r\nX-Forwarded-Proto: https\r\n\r\n"
        );
    }
}
