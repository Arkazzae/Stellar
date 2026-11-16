//! A service that misbehaves on request, for tests/pid1.rs.
//!
//! Without options it runs until a signal ends it. The options combine:
//!
//! ```text
//! --tag NAME            only marks the command line, for the tests to find it
//! --report PATH         writes what the process was started with, then goes on
//! --fail-first N --count PATH
//!                       exits 1 at once on the first N starts, counted in PATH
//! --exit CODE           exits with CODE, at once or after --after
//! --after MS            the wait before --exit
//! --tcp PORT            accepts TCP connections on 127.0.0.1:PORT
//! --http PORT           answers every HTTP request on 127.0.0.1:PORT
//! --status CODE         the status the HTTP answers carry, 200 unless given
//! --stall-file PATH     answers no HTTP request while PATH exists
//! --stun PORT           answers STUN binding requests on 127.0.0.1:PORT
//! --ready-after MS      opens the TCP, HTTP or STUN port only this late
//! --ignore-term         ignores SIGTERM and SIGINT
//! --children N          starts N copies of itself that ignore SIGTERM too
//! --orphan              starts a grandchild that sleeps and outlives this one
//! --escape              puts that grandchild in a process group of its own
//! --sleep MS            exits 0 after MS; what the grandchild runs
//! --flood               writes to stdout and stderr without pause
//! --on-hup PATH         appends a line to PATH on each SIGHUP; exits 0 on SIGTERM
//! ```

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, exit};
use std::thread;
use std::time::Duration;

use stellarr_init::sys;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut options: HashMap<&str, &str> = HashMap::new();
    let mut flags: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let name = args[i].as_str();
        match name {
            "--ignore-term" | "--orphan" | "--escape" | "--flood" => flags.push(name),
            _ => {
                let value = args
                    .get(i + 1)
                    .unwrap_or_else(|| fail(&format!("{name} takes a value")));
                options.insert(name, value.as_str());
                i += 1;
            }
        }
        i += 1;
    }
    let number = |name: &str| -> Option<u64> {
        options.get(name).map(|v| {
            v.parse()
                .unwrap_or_else(|_| fail(&format!("{name} takes a number")))
        })
    };
    let flag = |name: &str| flags.contains(&name);

    // Before any thread starts, or one of them would take SIGHUP and die of
    // it.
    if options.contains_key("--on-hup") {
        sys::block_signals(&[libc::SIGHUP, libc::SIGTERM]).unwrap();
    }

    if let Some(ms) = number("--sleep") {
        thread::sleep(Duration::from_millis(ms));
        exit(0);
    }

    if let (Some(first), Some(path)) = (number("--fail-first"), options.get("--count")) {
        let count: u64 = fs::read_to_string(path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
            + 1;
        fs::write(path, count.to_string()).unwrap_or_else(|e| fail(&e.to_string()));
        if count <= first {
            exit(1);
        }
    }

    if flag("--ignore-term") {
        sys::ignore_signal(libc::SIGTERM).unwrap();
        sys::ignore_signal(libc::SIGINT).unwrap();
    }

    if let Some(path) = options.get("--report") {
        write_report(path);
    }

    if let Some(n) = number("--children") {
        let me = std::env::current_exe().unwrap();
        for _ in 0..n {
            // Never waited for: they are left for the manager to deal with.
            #[expect(clippy::zombie_processes)]
            Command::new(&me)
                .args(["--ignore-term", "--tag", "child"])
                .spawn()
                .unwrap_or_else(|e| fail(&e.to_string()));
        }
    }

    if flag("--orphan") {
        let me = std::env::current_exe().unwrap();
        let mut grandchild = Command::new(&me);
        grandchild.args(["--sleep", "1500", "--tag", "grandchild"]);
        if flag("--escape") {
            grandchild.process_group(0);
        }
        // Never waited for: it is to outlive this process.
        #[expect(clippy::zombie_processes)]
        grandchild.spawn().unwrap_or_else(|e| fail(&e.to_string()));
    }

    let ready_after = Duration::from_millis(number("--ready-after").unwrap_or(0));
    if let Some(port) = number("--tcp") {
        serve_after(ready_after, move || serve_tcp(port as u16));
    }
    if let Some(port) = number("--http") {
        let status = number("--status").unwrap_or(200);
        let stall = options.get("--stall-file").map(PathBuf::from);
        serve_after(ready_after, move || serve_http(port as u16, status, stall));
    }
    if let Some(port) = number("--stun") {
        serve_after(ready_after, move || serve_stun(port as u16));
    }

    if flag("--flood") {
        thread::spawn(|| flood(&mut std::io::stdout()));
        thread::spawn(|| flood(&mut std::io::stderr()));
    }

    if let Some(code) = number("--exit") {
        thread::sleep(Duration::from_millis(number("--after").unwrap_or(0)));
        exit(code as i32);
    }

    if let Some(path) = options.get("--on-hup") {
        let signals = sys::SignalFd::new(&[libc::SIGHUP, libc::SIGTERM]).unwrap();
        loop {
            match signals.wait(Duration::from_secs(1)) {
                Ok(Some(libc::SIGHUP)) => {
                    let mut file = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .unwrap();
                    writeln!(file, "hup").unwrap();
                }
                Ok(Some(_)) => exit(0),
                _ => {}
            }
        }
    }

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("misbehave: {message}");
    exit(2);
}

fn serve_after(delay: Duration, serve: impl FnOnce() + Send + 'static) {
    thread::spawn(move || {
        thread::sleep(delay);
        serve();
    });
}

fn serve_tcp(port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| fail(&e.to_string()));
    for stream in listener.incoming() {
        drop(stream);
    }
}

fn serve_http(port: u16, status: u64, stall: Option<PathBuf>) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| fail(&e.to_string()));
    // Connections taken while the stall file exists, held without an answer,
    // as a process that is alive but stuck would hold them.
    let mut held = Vec::new();
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        if stall.as_ref().is_some_and(|p| p.exists()) {
            held.push(stream);
            continue;
        }
        held.clear();
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut request = Vec::new();
        let mut buf = [0u8; 512];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => request.extend_from_slice(&buf[..n]),
            }
        }
        let _ = write!(
            stream,
            "HTTP/1.1 {status} Whatever\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
        );
    }
}

fn serve_stun(port: u16) {
    let socket = UdpSocket::bind(("127.0.0.1", port)).unwrap_or_else(|e| fail(&e.to_string()));
    let mut buf = [0u8; 1500];
    loop {
        let Ok((n, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        if n < 20 || buf[..2] != [0, 1] {
            continue;
        }
        let mut answer = [0u8; 20];
        answer.copy_from_slice(&buf[..20]);
        answer[..2].copy_from_slice(&[0x01, 0x01]);
        answer[2..4].copy_from_slice(&[0, 0]);
        let _ = socket.send_to(&answer, from);
    }
}

fn flood(out: &mut impl Write) {
    let chunk = vec![b'x'; 64 * 1024];
    loop {
        let _ = out.write_all(&chunk);
    }
}

/// What the tests check about how a service is started: identity, group,
/// signal state, descriptors, directory and environment.
fn write_report(path: &str) {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let line = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    // The fields after the command name, which is in parentheses.
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    let after = &stat[stat.rfind(')').unwrap() + 2..];
    let fields: Vec<&str> = after.split(' ').collect();
    let mut fds: Vec<String> = Vec::new();
    for entry in fs::read_dir("/proc/self/fd").unwrap().flatten() {
        let target = fs::read_link(entry.path())
            .map(|t| t.display().to_string())
            .unwrap_or_default();
        // The listing's own descriptor.
        if target.starts_with("/proc/") && target.ends_with("/fd") {
            continue;
        }
        fds.push(format!("{}={target}", entry.file_name().to_string_lossy()));
    }
    fds.sort();
    let env: Vec<String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("TEST_"))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let report = format!(
        "pid {}\npgid {}\nsid {}\nuid {}\ngid {}\ngroups {}\nsigblk {}\nsigign {}\nfds {}\ncwd {}\nenv {}\n",
        std::process::id(),
        fields[2],
        fields[3],
        line("Uid:"),
        line("Gid:"),
        line("Groups:"),
        line("SigBlk:"),
        line("SigIgn:"),
        fds.join(" "),
        fs::read_link("/proc/self/cwd").unwrap().display(),
        env.join(" "),
    );
    let pending = format!("{path}.pending");
    fs::write(&pending, report).unwrap();
    fs::rename(&pending, path).unwrap();
}
