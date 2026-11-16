//! The system calls the standard library does not offer. Every `unsafe`
//! block of the crate is here, each with the reason it is sound.

use std::io;
use std::mem::MaybeUninit;
use std::net::{SocketAddr, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Duration;

use crate::core::Exit;

fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn signal_set(signals: &[i32]) -> io::Result<libc::sigset_t> {
    let mut set = MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: sigemptyset initialises the set it is given; the pointer is to
    // a local of the right type.
    check(unsafe { libc::sigemptyset(set.as_mut_ptr()) })?;
    // SAFETY: sigemptyset succeeded, so the set is initialised.
    let mut set = unsafe { set.assume_init() };
    for &signal in signals {
        // SAFETY: the set is initialised; sigaddset rejects a bad number
        // with EINVAL rather than writing out of bounds.
        check(unsafe { libc::sigaddset(&mut set, signal) })?;
    }
    Ok(set)
}

/// Blocks `signals` in the calling thread and in every thread it starts
/// later, so that they wait for a [`SignalFd`] instead of being delivered.
/// For PID 1 this is also what keeps them from being dropped: the kernel
/// discards a signal PID 1 neither handles nor blocks.
///
/// The mask survives fork and exec, and `std::process::Command` passes it on
/// as it is, so every child must be started through
/// [`unblock_signals_on_exec`].
pub fn block_signals(signals: &[i32]) -> io::Result<()> {
    let set = signal_set(signals)?;
    // SAFETY: the set is initialised and the old mask is not asked for.
    let result = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    Ok(())
}

/// Makes `command` start its process with no signal blocked. Without this a
/// service would inherit the manager's mask and never see SIGTERM.
pub fn unblock_signals_on_exec(command: &mut Command) {
    let clear = || {
        let mut set = MaybeUninit::<libc::sigset_t>::uninit();
        // SAFETY: both calls are async-signal-safe and only touch the local
        // set, which sigemptyset initialises before sigprocmask reads it.
        let result = unsafe {
            libc::sigemptyset(set.as_mut_ptr());
            libc::sigprocmask(libc::SIG_SETMASK, set.as_ptr(), std::ptr::null_mut())
        };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    };
    // SAFETY: the closure runs in the child between fork and exec, where
    // only async-signal-safe functions may be called. It calls nothing else,
    // allocates nothing and captures nothing.
    unsafe { command.pre_exec(clear) };
}

/// A file descriptor that becomes readable when one of the signals it was
/// made for is pending.
pub struct SignalFd {
    fd: OwnedFd,
}

impl SignalFd {
    /// The signals must be blocked already, see [`block_signals`].
    pub fn new(signals: &[i32]) -> io::Result<SignalFd> {
        let set = signal_set(signals)?;
        // SAFETY: -1 asks for a new descriptor; the set is initialised.
        let fd =
            check(unsafe { libc::signalfd(-1, &set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC) })?;
        // SAFETY: signalfd returned a new descriptor that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(SignalFd { fd })
    }

    /// The next pending signal, or `None` when there is none.
    pub fn read(&self) -> io::Result<Option<i32>> {
        let mut info = MaybeUninit::<libc::signalfd_siginfo>::uninit();
        let size = std::mem::size_of::<libc::signalfd_siginfo>();
        // SAFETY: the buffer is a local of exactly `size` bytes, and the
        // kernel writes whole records into it.
        let n = unsafe { libc::read(self.fd.as_raw_fd(), info.as_mut_ptr().cast(), size) };
        if n == -1 {
            let error = io::Error::last_os_error();
            return match error.kind() {
                io::ErrorKind::WouldBlock => Ok(None),
                io::ErrorKind::Interrupted => Ok(None),
                _ => Err(error),
            };
        }
        if n as usize != size {
            return Err(io::Error::other("a short read from the signalfd"));
        }
        // SAFETY: the kernel filled the whole record.
        let info = unsafe { info.assume_init() };
        Ok(Some(info.ssi_signo as i32))
    }

    /// Waits up to `timeout` for a signal.
    pub fn wait(&self, timeout: Duration) -> io::Result<Option<i32>> {
        let mut fds = [libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        poll(&mut fds, Some(timeout))?;
        self.read()
    }
}

impl AsRawFd for SignalFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Waits until one of `fds` is ready or `timeout` passes; `None` waits for
/// ever. An interrupted wait counts as a timeout.
pub fn poll(fds: &mut [libc::pollfd], timeout: Option<Duration>) -> io::Result<usize> {
    let timeout_ms = match timeout {
        None => -1,
        // Rounded up, so that a deadline a fraction of a millisecond away
        // does not turn into a busy loop of zero-length waits.
        Some(t) => t.as_micros().div_ceil(1000).min(libc::c_int::MAX as u128) as libc::c_int,
    };
    // SAFETY: the pointer and length describe a live, writable slice of
    // pollfd records.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
    if n == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(0);
        }
        return Err(error);
    }
    Ok(n as usize)
}

/// Collects one child that has ended, any child, orphans included, without
/// waiting. `None` when no child has ended or there is no child at all.
pub fn reap() -> io::Result<Option<(u32, Exit)>> {
    let mut status: libc::c_int = 0;
    // SAFETY: the status pointer is to a local; WNOHANG keeps the call from
    // blocking.
    let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    if pid == -1 {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::ECHILD) => Ok(None),
            Some(libc::EINTR) => Ok(None),
            _ => Err(error),
        };
    }
    if pid == 0 {
        return Ok(None);
    }
    let exit = if libc::WIFSIGNALED(status) {
        Exit::Signal(libc::WTERMSIG(status))
    } else {
        Exit::Code(libc::WEXITSTATUS(status))
    };
    Ok(Some((pid as u32, exit)))
}

/// Sends `signal` to process `pid`, or to the process group it leads.
pub fn kill(pid: u32, signal: i32, group: bool) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| io::Error::from_raw_os_error(libc::ESRCH))?;
    // PID 0 and 1 would mean the caller's own group, or every process.
    if pid <= 1 {
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    let target = if group { -pid } else { pid };
    // SAFETY: kill takes plain integers and touches no memory of ours.
    check(unsafe { libc::kill(target, signal) })?;
    Ok(())
}

/// Makes orphaned descendants come to this process instead of to PID 1, so
/// that the manager reaps them when it is not PID 1 itself.
pub fn become_subreaper() -> io::Result<()> {
    // SAFETY: PR_SET_CHILD_SUBREAPER takes an integer flag and touches no
    // memory of ours.
    check(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1 as libc::c_ulong) })?;
    Ok(())
}

pub fn effective_uid() -> u32 {
    // SAFETY: geteuid cannot fail and touches no memory of ours.
    unsafe { libc::geteuid() }
}

/// The user ID of the process at the other end of a Unix socket.
pub fn peer_uid(socket: &impl AsRawFd) -> io::Result<u32> {
    let mut cred = MaybeUninit::<libc::ucred>::uninit();
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the buffer is a local ucred and `len` is its size, which the
    // kernel does not exceed.
    check(unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            cred.as_mut_ptr().cast(),
            &mut len,
        )
    })?;
    if len as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::other("a short SO_PEERCRED answer"));
    }
    // SAFETY: the kernel filled the whole record.
    Ok(unsafe { cred.assume_init() }.uid)
}

/// Starts a TCP connection without waiting for it. The stream is
/// nonblocking; it is connected once it polls writable and
/// `take_error` finds nothing.
pub fn connect_nonblocking(addr: SocketAddr) -> io::Result<TcpStream> {
    let (family, storage, len) = sockaddr(addr);
    // SAFETY: socket takes plain integers.
    let fd = check(unsafe {
        libc::socket(
            family,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    })?;
    // SAFETY: socket returned a new descriptor that nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: `storage` holds a sockaddr of `len` bytes for this family.
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&storage as *const libc::sockaddr_storage).cast(),
            len,
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error);
        }
    }
    Ok(TcpStream::from(fd))
}

fn sockaddr(addr: SocketAddr) -> (libc::c_int, libc::sockaddr_storage, libc::socklen_t) {
    // SAFETY: sockaddr_storage is plain data for which all zeroes is valid.
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    match addr {
        SocketAddr::V4(v4) => {
            let sin = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            // SAFETY: sockaddr_storage is larger than sockaddr_in and
            // suitably aligned for it.
            unsafe {
                std::ptr::write(
                    (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in>(),
                    sin,
                )
            };
            (
                libc::AF_INET,
                storage,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        }
        SocketAddr::V6(v6) => {
            let sin6 = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
            };
            // SAFETY: sockaddr_storage is larger than sockaddr_in6 and
            // suitably aligned for it.
            unsafe {
                std::ptr::write(
                    (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in6>(),
                    sin6,
                )
            };
            (
                libc::AF_INET6,
                storage,
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
            )
        }
    }
}

const ALARM_MESSAGE: &[u8] = b"stellarr-init: the manager did not answer in time\n";

extern "C" fn exit_on_alarm(_: libc::c_int) {
    // SAFETY: write and _exit are async-signal-safe, and the message is a
    // static byte string.
    unsafe {
        libc::write(2, ALARM_MESSAGE.as_ptr().cast(), ALARM_MESSAGE.len());
        libc::_exit(1);
    }
}

/// Ends the process with status 1 after `seconds`, whatever it is doing
/// then, even waiting in a `connect` the kernel would let hang.
pub fn exit_after(seconds: u32) -> io::Result<()> {
    // SAFETY: an all-zero sigaction is a valid starting point: no flags and
    // an empty mask.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = exit_on_alarm as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only calls async-signal-safe functions, and the
    // old action is not asked for.
    check(unsafe { libc::sigaction(libc::SIGALRM, &action, std::ptr::null_mut()) })?;
    // SAFETY: alarm takes a plain integer.
    unsafe { libc::alarm(seconds) };
    Ok(())
}

/// Sets `signal` to be ignored, as a test child that will not stop does.
/// Unlike a blocked signal, an ignored one stays ignored across exec.
pub fn ignore_signal(signal: i32) -> io::Result<()> {
    // SAFETY: an all-zero sigaction is a valid starting point: no flags and
    // an empty mask.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_IGN;
    // SAFETY: SIG_IGN installs no code, and the old action is not asked for.
    check(unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) })?;
    Ok(())
}
