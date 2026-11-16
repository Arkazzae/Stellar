//! The wire format of the control socket.
//!
//! A client connects, writes one request line and reads the reply until the
//! manager closes the connection. Every line ends with `\n` and is ASCII.
//!
//! Requests:
//!
//! ```text
//! status
//! health
//! restart <service>
//! reload <service>
//! ```
//!
//! The first line of a reply is `ok`, optionally followed by a space and a
//! message, or `error ` and a message. A `status` reply goes on with one
//! `manager` line and one `service` line per service, in table order, made of
//! space-separated `key=value` fields; `-` stands for no value, and times are
//! milliseconds since the Unix epoch:
//!
//! ```text
//! ok
//! manager phase=running pid=1
//! service media state=ready pid=41 restarts=0 up=1763300000123 next=- critical=yes
//! service api state=backoff pid=- restarts=3 up=- next=1763300012000 critical=yes
//! ```
//!
//! A reader skips lines and fields it does not know.

use crate::core::{Phase, State};

/// The longest request line the manager reads, newline included.
pub const MAX_REQUEST: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Status,
    Health,
    Restart(String),
    Reload(String),
}

impl Request {
    /// Parses one line, without its newline.
    pub fn parse(line: &str) -> Result<Request, String> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut words = line.split(' ');
        let verb = words.next().unwrap_or("");
        let argument = words.next();
        if words.next().is_some() {
            return Err("a request has at most two words".into());
        }
        match (verb, argument) {
            ("status", None) => Ok(Request::Status),
            ("health", None) => Ok(Request::Health),
            ("restart", Some(name)) if is_name(name) => Ok(Request::Restart(name.into())),
            ("reload", Some(name)) if is_name(name) => Ok(Request::Reload(name.into())),
            ("restart" | "reload", _) => Err(format!("{verb} takes the name of a service")),
            _ => Err(
                "unknown request; send status, health, restart <service> or reload <service>"
                    .into(),
            ),
        }
    }

    /// The request as the client writes it, newline included.
    pub fn line(&self) -> String {
        match self {
            Request::Status => "status\n".into(),
            Request::Health => "health\n".into(),
            Request::Restart(name) => format!("restart {name}\n"),
            Request::Reload(name) => format!("reload {name}\n"),
        }
    }
}

fn is_name(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 32
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The first line of a reply.
pub fn reply_line(result: &Result<String, String>) -> String {
    match result {
        Ok(message) if message.is_empty() => "ok\n".into(),
        Ok(message) => format!("ok {}\n", one_line(message)),
        Err(message) => format!("error {}\n", one_line(message)),
    }
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// The manager in a `status` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerLine {
    pub phase: Phase,
    pub pid: u32,
}

impl ManagerLine {
    pub fn format(&self) -> String {
        format!(
            "manager phase={} pid={}\n",
            phase_name(self.phase),
            self.pid
        )
    }
}

pub fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Running => "running",
        Phase::ShuttingDown => "shutting-down",
        Phase::Done => "done",
    }
}

fn parse_phase(name: &str) -> Option<Phase> {
    [Phase::Running, Phase::ShuttingDown, Phase::Done]
        .into_iter()
        .find(|p| phase_name(*p) == name)
}

/// One service in a `status` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceLine {
    pub name: String,
    pub state: State,
    pub pid: Option<u32>,
    pub restarts: u32,
    /// When the running process started, in Unix milliseconds.
    pub up_since: Option<u64>,
    /// When the next try is due, in Unix milliseconds.
    pub next_try: Option<u64>,
    pub critical: bool,
}

impl ServiceLine {
    pub fn format(&self) -> String {
        format!(
            "service {} state={} pid={} restarts={} up={} next={} critical={}\n",
            self.name,
            self.state.name(),
            dash(self.pid),
            self.restarts,
            dash(self.up_since),
            dash(self.next_try),
            if self.critical { "yes" } else { "no" }
        )
    }
}

fn dash<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "-".to_string(), |v| v.to_string())
}

/// A whole `status` reply, read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReply {
    pub manager: ManagerLine,
    pub services: Vec<ServiceLine>,
}

/// Splits a reply into its first line's verdict and message, and the lines
/// that follow.
pub fn parse_reply(text: &str) -> Result<(Result<String, String>, Vec<&str>), String> {
    let mut lines = text.lines();
    let first = lines
        .next()
        .ok_or("the manager closed the connection without a reply")?;
    let verdict = if first == "ok" {
        Ok(String::new())
    } else if let Some(message) = first.strip_prefix("ok ") {
        Ok(message.to_string())
    } else if let Some(message) = first.strip_prefix("error ") {
        Err(message.to_string())
    } else {
        return Err(format!(
            "the manager replied {first:?}, which is not a reply"
        ));
    };
    Ok((verdict, lines.collect()))
}

/// Reads the lines after the `ok` of a `status` reply.
pub fn parse_status(lines: &[&str]) -> Result<StatusReply, String> {
    let mut manager = None;
    let mut services = Vec::new();
    for line in lines {
        let mut words = line.split(' ');
        match words.next() {
            Some("manager") => {
                let fields = fields(words);
                let phase = field(&fields, "phase")?;
                manager = Some(ManagerLine {
                    phase: parse_phase(phase).ok_or_else(|| format!("unknown phase {phase:?}"))?,
                    pid: number(field(&fields, "pid")?)?,
                });
            }
            Some("service") => {
                let name = words.next().ok_or("a service line without a name")?;
                let fields = fields(words);
                let state = field(&fields, "state")?;
                services.push(ServiceLine {
                    name: name.to_string(),
                    state: State::parse(state).ok_or_else(|| format!("unknown state {state:?}"))?,
                    pid: optional(field(&fields, "pid")?)?,
                    restarts: number(field(&fields, "restarts")?)?,
                    up_since: optional(field(&fields, "up")?)?,
                    next_try: optional(field(&fields, "next")?)?,
                    critical: field(&fields, "critical")? == "yes",
                });
            }
            _ => {}
        }
    }
    Ok(StatusReply {
        manager: manager.ok_or("the reply has no manager line")?,
        services,
    })
}

fn fields<'a>(words: impl Iterator<Item = &'a str>) -> Vec<(&'a str, &'a str)> {
    words.filter_map(|w| w.split_once('=')).collect()
}

fn field<'a>(fields: &[(&str, &'a str)], key: &str) -> Result<&'a str, String> {
    fields
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .ok_or_else(|| format!("a status line without {key}"))
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{text:?} is not a number"))
}

fn optional<T: std::str::FromStr>(text: &str) -> Result<Option<T>, String> {
    if text == "-" {
        Ok(None)
    } else {
        number(text).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_and_print_back() {
        for request in [
            Request::Status,
            Request::Health,
            Request::Restart("api".into()),
            Request::Reload("nginx".into()),
        ] {
            let line = request.line();
            assert_eq!(Request::parse(line.trim_end_matches('\n')), Ok(request));
        }
        assert_eq!(Request::parse("status\r"), Ok(Request::Status));
    }

    #[test]
    fn malformed_requests_are_refused() {
        for line in [
            "",
            "STATUS",
            "status now",
            "restart",
            "restart ",
            "restart API",
            "restart a b",
            "reload ../x",
            "stop api",
            "\u{0}\u{1}",
        ] {
            assert!(Request::parse(line).is_err(), "{line:?}");
        }
    }

    #[test]
    fn a_status_reply_reads_back() {
        let services = vec![
            ServiceLine {
                name: "media".into(),
                state: State::Ready,
                pid: Some(41),
                restarts: 0,
                up_since: Some(1_763_300_000_123),
                next_try: None,
                critical: true,
            },
            ServiceLine {
                name: "tls-maintenance".into(),
                state: State::Backoff,
                pid: None,
                restarts: 7,
                up_since: None,
                next_try: Some(1_763_300_012_000),
                critical: false,
            },
        ];
        let manager = ManagerLine {
            phase: Phase::ShuttingDown,
            pid: 1,
        };
        let mut text = reply_line(&Ok(String::new()));
        text.push_str(&manager.format());
        // A line of a later version is skipped.
        text.push_str("timer next=5\n");
        for s in &services {
            text.push_str(&s.format());
        }
        let (verdict, lines) = parse_reply(&text).unwrap();
        assert_eq!(verdict, Ok(String::new()));
        let status = parse_status(&lines).unwrap();
        assert_eq!(status, StatusReply { manager, services });
    }

    #[test]
    fn a_reply_message_stays_on_one_line() {
        assert_eq!(reply_line(&Err("a\nb\tc".into())), "error a?b?c\n");
        assert_eq!(
            reply_line(&Ok("restarting api".into())),
            "ok restarting api\n"
        );
        let (verdict, _) = parse_reply("error no such service\n").unwrap();
        assert_eq!(verdict, Err("no such service".into()));
        assert!(parse_reply("").is_err());
        assert!(parse_reply("HTTP/1.1 200 OK\n").is_err());
    }
}
