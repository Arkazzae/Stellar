//! What browsers are told of the relay (TURN): its two ports and whether it
//! also answers over TCP and TLS.

use super::SettingError;
use super::config::Config;

/// The relay's ports and switches. A value that cannot be right stops the
/// start, like any other setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySettings {
    pub arrangement: Relay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relay {
    pub enabled: bool,
    pub turn_port: u16,
    pub turns_port: u16,
    pub turns_enabled: bool,
    pub tcp_enabled: bool,
}

impl RelaySettings {
    pub(super) fn load(config: &Config) -> Result<RelaySettings, SettingError> {
        Ok(RelaySettings {
            arrangement: arrange(config)?,
        })
    }
}

fn arrange(config: &Config) -> Result<Relay, SettingError> {
    let turn_port = number(config, "Rtc:TurnPort", 3478)?;
    let turns_port = number(config, "Rtc:TurnsPort", 5349)?;
    let turns_enabled = flag(config, "Rtc:TurnsEnabled")?;
    if !(1..=65535).contains(&turn_port) {
        return Err(SettingError::new(
            "Rtc:TurnPort must be between 1 and 65535.",
        ));
    }
    if turns_enabled && !(1..=65535).contains(&turns_port) {
        return Err(SettingError::new(
            "Rtc:TurnsPort must be between 1 and 65535.",
        ));
    }
    let tcp_enabled = flag(config, "Rtc:TurnTcpEnabled")?;
    Ok(Relay {
        enabled: flag(config, "Rtc:TurnEnabled")?,
        turn_port: u16::try_from(turn_port)
            .map_err(|_| SettingError::new("Rtc:TurnPort must be between 1 and 65535."))?,
        // Only used when the TLS port is on, and then checked above.
        turns_port: u16::try_from(turns_port).unwrap_or(0),
        turns_enabled,
        tcp_enabled,
    })
}

fn number(config: &Config, key: &str, default: i32) -> Result<i32, SettingError> {
    match config.raw(key) {
        None => Ok(default),
        Some(text) => int(text).ok_or_else(|| SettingError::new(format!("{key} is not a number."))),
    }
}

fn flag(config: &Config, key: &str) -> Result<bool, SettingError> {
    match config.raw(key) {
        None => Ok(false),
        Some(text) => {
            boolean(text).ok_or_else(|| SettingError::new(format!("{key} is not true or false.")))
        }
    }
}

/// A whole number: the ordinary integer, with white space around it allowed.
pub(super) fn int(text: &str) -> Option<i32> {
    text.trim_matches(char::is_whitespace).parse().ok()
}

fn boolean(text: &str) -> Option<bool> {
    let text = text.trim_matches(char::is_whitespace);
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arranged(settings: &[(&str, &str)]) -> Result<Relay, SettingError> {
        let mut config = Config::layered("Production", &[]);
        for (key, value) in settings {
            config.set(key, value);
        }
        RelaySettings::load(&config).map(|loaded| loaded.arrangement)
    }

    #[test]
    fn a_port_is_a_plain_whole_number() {
        for (text, port) in [
            ("3478", Some(3478)),
            (" 3478 ", Some(3478)),
            ("+3478", Some(3478)),
            ("01", Some(1)),
            ("1", Some(1)),
            ("65535", Some(65535)),
            ("0x10", None),
            ("&h10", None),
            ("#10", None),
            ("0", None),
            ("65536", None),
            ("-1", None),
            ("99999999999", None),
            ("abc", None),
            ("", None),
            ("3478.0", None),
            ("1_0", None),
            ("1,000", None),
            ("3 478", None),
            ("٣٤٧٨", None),
        ] {
            let got = arranged(&[("Rtc:TurnPort", text)]).map(|relay| relay.turn_port);
            assert_eq!(got.ok(), port, "{text:?}");
        }
    }

    #[test]
    fn the_tls_port_is_looked_at_only_while_tls_is_on_but_must_always_be_a_number() {
        assert!(arranged(&[("Rtc:TurnsEnabled", "false"), ("Rtc:TurnsPort", "0")]).is_ok());
        assert!(arranged(&[("Rtc:TurnsEnabled", "false"), ("Rtc:TurnsPort", "70000")]).is_ok());
        assert!(arranged(&[("Rtc:TurnsEnabled", "false"), ("Rtc:TurnsPort", "abc")]).is_err());
        assert!(arranged(&[("Rtc:TurnsEnabled", "false"), ("Rtc:TurnsPort", "")]).is_err());
        assert!(arranged(&[("Rtc:TurnsEnabled", "true"), ("Rtc:TurnsPort", "0")]).is_err());
        assert!(arranged(&[("Rtc:TurnsEnabled", "true"), ("Rtc:TurnsPort", "65536")]).is_err());
        assert_eq!(
            arranged(&[("Rtc:TurnsEnabled", "true"), ("Rtc:TurnsPort", "443")])
                .unwrap()
                .turns_port,
            443
        );
        assert!(arranged(&[("Rtc:TurnsEnabled", "true"), ("Rtc:TurnsPort", "0x1BB")]).is_err());
    }

    #[test]
    fn a_switch_is_true_or_false_in_any_letter_case_with_spaces_around_and_nothing_else() {
        for (text, on) in [
            ("true", Some(true)),
            ("TRUE", Some(true)),
            (" true ", Some(true)),
            ("False", Some(false)),
            ("1", None),
            ("yes", None),
            ("", None),
            ("abc", None),
        ] {
            let got = arranged(&[("Rtc:TurnTcpEnabled", text)]).map(|relay| relay.tcp_enabled);
            assert_eq!(got.ok(), on, "{text:?}");
            let got = arranged(&[("Rtc:TurnsEnabled", text)]).map(|relay| relay.turns_enabled);
            assert_eq!(got.ok(), on, "{text:?}");
        }
    }

    #[test]
    fn what_is_not_set_is_the_default_and_a_wrong_value_stops_the_start() {
        let relay = arranged(&[]).unwrap();
        assert_eq!(
            (
                relay.turn_port,
                relay.turns_port,
                relay.turns_enabled,
                relay.tcp_enabled
            ),
            (3478, 5349, false, false)
        );
        for (key, value, said) in [
            ("Rtc:TurnPort", "abc", "Rtc:TurnPort is not a number."),
            (
                "Rtc:TurnPort",
                "0",
                "Rtc:TurnPort must be between 1 and 65535.",
            ),
            (
                "Rtc:TurnPort",
                "65536",
                "Rtc:TurnPort must be between 1 and 65535.",
            ),
            (
                "Rtc:TurnTcpEnabled",
                "maybe",
                "Rtc:TurnTcpEnabled is not true or false.",
            ),
        ] {
            let refused = arranged(&[(key, value)]).unwrap_err();
            assert_eq!(refused.to_string(), said);
        }
    }

    #[test]
    fn only_the_plain_integer_form_is_a_number() {
        assert_eq!(int("3478"), Some(3478));
        assert_eq!(int(" 3478 "), Some(3478));
        for text in ["0x10", "&h10", "#10", "", "0x"] {
            assert_eq!(int(text), None, "{text:?}");
        }
    }
}
