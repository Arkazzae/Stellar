//! The control socket, as the manager serves it: nonblocking, with a
//! deadline for every connection, so that a client that sends nothing, sends
//! too much or never reads its reply costs a file descriptor for two seconds
//! and nothing else.

use std::fs::{self, DirBuilder, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::protocol::{MAX_REQUEST, Request};
use crate::sys;

/// How long a connection may take, from accept to the last byte of the
/// reply.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// Connections served at once; one more is turned away.
pub const MAX_CLIENTS: usize = 16;

pub struct Control {
    path: PathBuf,
    listener: UnixListener,
    clients: Vec<Client>,
}

struct Client {
    stream: UnixStream,
    input: Vec<u8>,
    output: Vec<u8>,
    written: usize,
    deadline: Instant,
    replying: bool,
}

/// What a connection is after a step.
enum Next {
    Keep,
    Close,
}

impl Control {
    /// Listens on `path`. Its folder is created for root alone when it does
    /// not exist; the socket is made for root alone too, and every
    /// connection is checked for the peer's user ID as well. A socket file
    /// left by an earlier run is replaced; one that a live manager answers
    /// on is not.
    pub fn bind(path: &Path) -> io::Result<Control> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
            && !dir.exists()
        {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        match fs::symlink_metadata(path) {
            Ok(meta) => {
                if !meta.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{} exists and is not a socket", path.display()),
                    ));
                }
                if UnixStream::connect(path).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another manager answers on {}", path.display()),
                    ));
                }
                fs::remove_file(path)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Control {
            path: path.to_path_buf(),
            listener,
            clients: Vec::new(),
        })
    }

    /// Appends what to poll: the listener, then each connection.
    pub fn interests(&self, fds: &mut Vec<libc::pollfd>) {
        fds.push(libc::pollfd {
            fd: self.listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        for client in &self.clients {
            fds.push(libc::pollfd {
                fd: client.stream.as_raw_fd(),
                events: if client.replying {
                    libc::POLLOUT
                } else {
                    libc::POLLIN
                },
                revents: 0,
            });
        }
    }

    /// Acts on what `poll` reported for the descriptors of
    /// [`interests`](Self::interests), in the same order. `answer` turns a
    /// request, or the reason it could not be read, into the reply.
    pub fn handle(
        &mut self,
        revents: &[i16],
        now: Instant,
        mut answer: impl FnMut(Result<Request, String>) -> String,
    ) {
        let (listener, clients) = revents.split_first().unwrap_or((&0, &[]));
        let mut k = 0;
        self.clients.retain_mut(|client| {
            let events = clients.get(k).copied().unwrap_or(0);
            k += 1;
            if events == 0 {
                return true;
            }
            matches!(client.step(&mut answer), Next::Keep)
        });
        if *listener != 0 {
            self.accept(now);
        }
    }

    /// Closes every connection whose time is up.
    pub fn expire(&mut self, now: Instant) {
        self.clients.retain(|c| c.deadline > now);
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.clients.iter().map(|c| c.deadline).min()
    }

    fn accept(&mut self, now: Instant) {
        loop {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                // WouldBlock ends the batch. Anything else, running out of
                // descriptors say, is left for the next round.
                Err(_) => return,
            };
            if stream.set_nonblocking(true).is_err() {
                continue;
            }
            let refusal = match sys::peer_uid(&stream) {
                Ok(uid) if uid == 0 || uid == sys::effective_uid() => None,
                Ok(_) => Some("error only root may use this socket\n"),
                Err(_) => Some("error cannot tell who is asking\n"),
            };
            let refusal = refusal.or(if self.clients.len() >= MAX_CLIENTS {
                Some("error busy; try again\n")
            } else {
                None
            });
            if let Some(text) = refusal {
                // One try; a client that cannot take a line now gets nothing.
                let _ = (&stream).write(text.as_bytes());
                continue;
            }
            self.clients.push(Client {
                stream,
                input: Vec::new(),
                output: Vec::new(),
                written: 0,
                deadline: now + CLIENT_TIMEOUT,
                replying: false,
            });
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Client {
    fn step(&mut self, answer: &mut impl FnMut(Result<Request, String>) -> String) -> Next {
        if !self.replying {
            match self.read_request() {
                None => return Next::Keep,
                Some(None) => return Next::Close,
                Some(Some(request)) => {
                    self.output = answer(request).into_bytes();
                    self.replying = true;
                }
            }
        }
        self.write_reply()
    }

    /// `None` while the line is incomplete; `Some(None)` when the client
    /// left without asking anything.
    fn read_request(&mut self) -> Option<Option<Result<Request, String>>> {
        let mut buf = [0u8; MAX_REQUEST];
        loop {
            if let Some(end) = self.input.iter().position(|&b| b == b'\n') {
                return Some(Some(parse(&self.input[..end])));
            }
            if self.input.len() >= MAX_REQUEST {
                return Some(Some(Err(format!(
                    "a request is one line of at most {MAX_REQUEST} bytes"
                ))));
            }
            match self.stream.read(&mut buf[..MAX_REQUEST - self.input.len()]) {
                // The client closed its end: what it sent, if anything, is
                // the request.
                Ok(0) if self.input.is_empty() => return Some(None),
                Ok(0) => return Some(Some(parse(&self.input))),
                Ok(n) => self.input.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Some(None),
            }
        }
    }

    fn write_reply(&mut self) -> Next {
        while self.written < self.output.len() {
            match self.stream.write(&self.output[self.written..]) {
                Ok(0) => return Next::Close,
                Ok(n) => self.written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Next::Keep,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Next::Close,
            }
        }
        Next::Close
    }
}

fn parse(line: &[u8]) -> Result<Request, String> {
    let line = std::str::from_utf8(line).map_err(|_| "a request is ASCII text".to_string())?;
    Request::parse(line)
}
