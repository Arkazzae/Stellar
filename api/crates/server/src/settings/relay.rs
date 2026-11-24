//! What browsers are told of the relay (TURN): its two ports and whether it
//! also answers over TCP and TLS.
//!
//! The first server read these when a bootstrap was asked for, not at start,
//! so a port that cannot be right did not stop it: it failed every bootstrap
//! with a 500 until the setting was mended. That is kept: a value that cannot
//! be right is held as the refusal it makes, and the bootstrap answers with it.

use super::config::Config;

/// The relay's ports and switches, or why they cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySettings {
    pub arrangement: Result<Relay, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relay {
    pub turn_port: u16,
    pub turns_port: u16,
    pub turns_enabled: bool,
    pub tcp_enabled: bool,
}

impl RelaySettings {
    pub(super) fn load(config: &Config) -> RelaySettings {
        RelaySettings {
            arrangement: arrange(config),
        }
    }
}

/// In the order the first server read and checked them, which decides which
/// refusal is reported (all of them are a 500 that says nothing of the cause).
fn arrange(config: &Config) -> Result<Relay, String> {
    let turn_port = number(config, "Rtc:TurnPort", 3478)?;
    let turns_port = number(config, "Rtc:TurnsPort", 5349)?;
    let turns_enabled = flag(config, "Rtc:TurnsEnabled")?;
    if !(1..=65535).contains(&turn_port) {
        return Err("Rtc:TurnPort must be between 1 and 65535.".to_string());
    }
    if turns_enabled && !(1..=65535).contains(&turns_port) {
        return Err("Rtc:TurnsPort must be between 1 and 65535.".to_string());
    }
    let tcp_enabled = flag(config, "Rtc:TurnTcpEnabled")?;
    Ok(Relay {
        turn_port: u16::try_from(turn_port).map_err(|_| "port".to_string())?,
        // Only used when the TLS port is on, and then checked above.
        turns_port: u16::try_from(turns_port).unwrap_or(0),
        turns_enabled,
        tcp_enabled,
    })
}

fn number(config: &Config, key: &str, default: i32) -> Result<i32, String> {
    match config.raw(key) {
        None => Ok(default),
        Some(text) => dotnet::int(text).ok_or_else(|| format!("{key} is not a number.")),
    }
}

fn flag(config: &Config, key: &str) -> Result<bool, String> {
    match config.raw(key) {
        None => Ok(false),
        Some(text) => dotnet::boolean(text).ok_or_else(|| format!("{key} is not true or false.")),
    }
}

/// Reading a configuration value as a number or a switch, as the first
/// server's binder did.
mod dotnet {
    pub fn int(text: &str) -> Option<i32> {
        let text = text.trim_matches(char::is_whitespace);
        crate::http::parse_int(text)
    }

    pub fn boolean(text: &str) -> Option<bool> {
        let text = text.trim_matches(char::is_whitespace);
        if text.eq_ignore_ascii_case("true") {
            Some(true)
        } else if text.eq_ignore_ascii_case("false") {
            Some(false)
        } else {
            None
        }
    }
}
