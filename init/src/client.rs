//! The other end of the control socket: what the subcommands send, and how
//! they show the answer.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::State;
use crate::protocol::{Request, ServiceLine, StatusReply, parse_reply, parse_status, phase_name};
use crate::sys;

/// How long a client waits for each step of the exchange.
const STEP_TIMEOUT: Duration = Duration::from_secs(2);

/// After this many seconds the client exits with status 1 whatever it is
/// doing, well inside the five seconds Docker gives a health check.
pub const HARD_DEADLINE_SECS: u32 = 3;

/// The longest reply read.
const MAX_REPLY: u64 = 64 * 1024;

/// Sets the process to exit with status 1 after [`HARD_DEADLINE_SECS`].
/// The subcommands call it first: a connect to a manager that is alive but
/// stuck can wait for ever once its backlog is full, and no timeout of the
/// standard library covers that.
pub fn set_hard_deadline() -> Result<(), String> {
    sys::exit_after(HARD_DEADLINE_SECS).map_err(|e| format!("cannot set an alarm: {e}"))
}

/// Sends `request` and returns the verdict and the lines after it. Each
/// step has a timeout; the connect has none, see [`set_hard_deadline`].
pub fn ask(
    socket: &Path,
    request: &Request,
) -> Result<(Result<String, String>, Vec<String>), String> {
    let unreachable =
        |e: std::io::Error| format!("cannot reach the manager at {}: {e}", socket.display());
    let mut stream = UnixStream::connect(socket).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(STEP_TIMEOUT))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(STEP_TIMEOUT))
        .map_err(unreachable)?;
    // A manager that turns the connection away writes why and closes it,
    // perhaps before the request is sent, so the reply is read even when
    // sending failed.
    let sent = stream.write_all(request.line().as_bytes());
    let mut text = String::new();
    let read = (&stream).take(MAX_REPLY).read_to_string(&mut text);
    if text.is_empty() {
        sent.map_err(|e| format!("cannot send the request: {e}"))?;
        read.map_err(|e| format!("no answer from the manager: {e}"))?;
    }
    let (verdict, lines) = parse_reply(&text)?;
    Ok((verdict, lines.into_iter().map(str::to_string).collect()))
}

/// Reads a `status` reply.
pub fn status(socket: &Path) -> Result<StatusReply, String> {
    let (verdict, lines) = ask(socket, &Request::Status)?;
    verdict?;
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    parse_status(&lines)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// A table for people.
pub fn render_table(reply: &StatusReply) -> String {
    let now = now_ms();
    let header = ["SERVICE", "STATE", "PID", "RESTARTS", "UP", "CRITICAL", ""];
    let mut rows: Vec<[String; 7]> = vec![header.map(str::to_string)];
    for s in &reply.services {
        let up = s
            .up_since
            .map_or_else(|| "-".to_string(), |at| span(now.saturating_sub(at)));
        let note = match (s.state, s.next_try) {
            (State::Backoff, Some(at)) => format!("next try in {}", span(at.saturating_sub(now))),
            _ => String::new(),
        };
        rows.push([
            s.name.clone(),
            s.state.name().to_string(),
            s.pid.map_or_else(|| "-".to_string(), |p| p.to_string()),
            s.restarts.to_string(),
            up,
            if s.critical { "yes" } else { "no" }.to_string(),
            note,
        ]);
    }
    let mut widths = [0usize; 7];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.len());
        }
    }
    let mut out = format!(
        "manager pid {}, {}\n",
        reply.manager.pid,
        phase_name(reply.manager.phase)
    );
    for row in &rows {
        let mut line = String::new();
        for (i, (cell, w)) in row.iter().zip(widths).enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            line.push_str(&format!("{cell:<w$}"));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// A duration in milliseconds, roughly, the way people read an uptime.
pub fn span(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0 => format!("{:.1}s", ms as f64 / 1000.0),
        1..60 => format!("{s}s"),
        60..3600 => format!("{}m{}s", s / 60, s % 60),
        3600..86400 => format!("{}h{}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d{}h", s / 86400, s % 86400 / 3600),
    }
}

/// JSON for programs: times in Unix milliseconds, `null` for none.
pub fn render_json(reply: &StatusReply) -> String {
    let services: Vec<String> = reply.services.iter().map(service_json).collect();
    format!(
        "{{\"manager\":{{\"phase\":\"{}\",\"pid\":{}}},\"services\":[{}]}}\n",
        phase_name(reply.manager.phase),
        reply.manager.pid,
        services.join(",")
    )
}

fn service_json(s: &ServiceLine) -> String {
    fn number(value: Option<impl ToString>) -> String {
        value.map_or_else(|| "null".to_string(), |v| v.to_string())
    }
    // Names are lower-case letters, digits and dashes, and states are fixed
    // words, so nothing needs escaping.
    format!(
        "{{\"name\":\"{}\",\"state\":\"{}\",\"pid\":{},\"restarts\":{},\"upSince\":{},\"nextTry\":{},\"critical\":{}}}",
        s.name,
        s.state.name(),
        number(s.pid),
        s.restarts,
        number(s.up_since),
        number(s.next_try),
        s.critical
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Phase;
    use crate::protocol::ManagerLine;

    fn reply() -> StatusReply {
        StatusReply {
            manager: ManagerLine {
                phase: Phase::Running,
                pid: 1,
            },
            services: vec![
                ServiceLine {
                    name: "media".into(),
                    state: State::Ready,
                    pid: Some(41),
                    restarts: 2,
                    up_since: Some(1_000),
                    next_try: None,
                    critical: true,
                },
                ServiceLine {
                    name: "tls-maintenance".into(),
                    state: State::Backoff,
                    pid: None,
                    restarts: 0,
                    up_since: None,
                    next_try: Some(u64::MAX),
                    critical: false,
                },
            ],
        }
    }

    #[test]
    fn json_is_exact() {
        assert_eq!(
            render_json(&reply()),
            concat!(
                "{\"manager\":{\"phase\":\"running\",\"pid\":1},\"services\":[",
                "{\"name\":\"media\",\"state\":\"ready\",\"pid\":41,\"restarts\":2,",
                "\"upSince\":1000,\"nextTry\":null,\"critical\":true},",
                "{\"name\":\"tls-maintenance\",\"state\":\"backoff\",\"pid\":null,\"restarts\":0,",
                "\"upSince\":null,\"nextTry\":18446744073709551615,\"critical\":false}]}\n"
            )
        );
    }

    #[test]
    fn the_table_lines_up() {
        let table = render_table(&reply());
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "manager pid 1, running");
        assert!(lines[1].starts_with("SERVICE          STATE    PID  RESTARTS"));
        assert!(lines[2].starts_with("media            ready    41   2"));
        assert!(lines[3].starts_with("tls-maintenance  backoff  -    0         -"));
        assert!(lines[3].contains("next try in"));
    }

    #[test]
    fn spans_read_like_uptimes() {
        assert_eq!(span(400), "0.4s");
        assert_eq!(span(59_999), "59s");
        assert_eq!(span(61_000), "1m1s");
        assert_eq!(span(3_600_000 + 120_000), "1h2m");
        assert_eq!(span(90_000_000), "1d1h");
    }
}
