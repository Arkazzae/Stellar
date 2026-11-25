//! The options and the environment variables that stand in for them.

use crate::checks::{
    is_address, is_http_binding, is_ipv4, is_port, is_ref, is_tag, normalize_address,
};

/// The settings the installer writes to `.env`, in the order it handles them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    TurnEnabled,
    Domain,
    PublicIp,
    Language,
    HttpsPort,
    PublicPort,
    HttpPort,
    MediaPort,
    TurnPort,
    TurnsPort,
}

pub const KEYS: [Key; 10] = [
    Key::TurnEnabled,
    Key::Domain,
    Key::PublicIp,
    Key::Language,
    Key::HttpsPort,
    Key::PublicPort,
    Key::HttpPort,
    Key::MediaPort,
    Key::TurnPort,
    Key::TurnsPort,
];

impl Key {
    pub fn name(self) -> &'static str {
        match self {
            Key::TurnEnabled => "STELLARR_TURN_ENABLED",
            Key::Domain => "STELLARR_DOMAIN",
            Key::PublicIp => "STELLARR_PUBLIC_IP",
            Key::Language => "STELLARR_LANGUAGE",
            Key::HttpsPort => "STELLARR_HTTPS_PORT",
            Key::PublicPort => "STELLARR_PUBLIC_PORT",
            Key::HttpPort => "STELLARR_HTTP_PORT",
            Key::MediaPort => "STELLARR_MEDIA_PORT",
            Key::TurnPort => "STELLARR_TURN_PORT",
            Key::TurnsPort => "STELLARR_TURNS_PORT",
        }
    }

    /// The option that sets the key, for messages that say what to type.
    pub fn option(self) -> &'static str {
        match self {
            Key::TurnEnabled => "--enable-turn",
            Key::Domain => "--domain",
            Key::PublicIp => "--public-ip",
            Key::Language => "--language",
            Key::HttpsPort => "--https-port",
            Key::PublicPort => "--public-port",
            Key::HttpPort => "--http-port",
            Key::MediaPort => "--media-port",
            Key::TurnPort => "--turn-port",
            Key::TurnsPort => "--turns-port",
        }
    }

    fn from_value_option(option: &str) -> Option<Key> {
        KEYS.into_iter()
            .find(|key| *key != Key::TurnEnabled && key.option() == option)
    }
}

/// One setting: what the run was given, and from where, and later the value
/// it settles on.
#[derive(Clone, Default, Debug)]
pub struct Setting {
    pub given: bool,
    pub value: String,
    pub from: String,
    pub set: String,
}

#[derive(Debug)]
pub struct Options {
    pub settings: [Setting; 10],
    pub assume_yes: bool,
    pub build: bool,
    pub start: bool,
    pub install_docker: bool,
    pub force: bool,
    pub skip_port_check: bool,
    pub color: bool,
    pub dir: String,
    pub version: String,
    pub version_from: String,
    pub reference: String,
    pub ref_from: String,
    pub start_timeout: String,
    pub start_timeout_secs: u64,
    /// Where the answers are read. STELLARR_INSTALLER_TTY names another file
    /// or FIFO, from which the tests feed them.
    pub tty: String,
    /// The person's locale, which the interface language's default follows.
    pub locale_language: String,
}

pub fn env(name: &str) -> String {
    std::env::var_os(name)
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn first_set(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| env(name))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

pub enum Parsed {
    Run,
    Help,
}

impl Options {
    /// Each setting is read from the environment variable of the same name
    /// first, as Compose itself would.
    pub fn from_env() -> Options {
        let settings = KEYS.map(|key| {
            let value = env(key.name());
            Setting {
                given: !value.is_empty(),
                value,
                from: key.name().to_string(),
                set: String::new(),
            }
        });
        let start_timeout = Some(env("STELLARR_START_TIMEOUT"))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "300".to_string());
        Options {
            settings,
            assume_yes: false,
            build: false,
            start: true,
            install_docker: false,
            force: false,
            skip_port_check: false,
            color: true,
            dir: env("STELLARR_INSTALL_DIR"),
            version: env("STELLARR_VERSION"),
            version_from: "STELLARR_VERSION".to_string(),
            reference: env("STELLARR_REF"),
            ref_from: "STELLARR_REF".to_string(),
            start_timeout,
            start_timeout_secs: 300,
            tty: Some(env("STELLARR_INSTALLER_TTY"))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "/dev/tty".to_string()),
            locale_language: first_set(&["LC_ALL", "LC_MESSAGES", "LANG"]),
        }
    }

    pub fn setting(&self, key: Key) -> &Setting {
        &self.settings[key as usize]
    }

    pub fn setting_mut(&mut self, key: Key) -> &mut Setting {
        &mut self.settings[key as usize]
    }

    fn give(&mut self, key: Key, value: &str, from: &str) {
        let setting = self.setting_mut(key);
        setting.given = true;
        setting.value = value.to_string();
        setting.from = from.to_string();
    }

    /// Reads the arguments. A mistake is one line of message, for exit
    /// status 2.
    pub fn parse(&mut self, args: &[String]) -> Result<Parsed, String> {
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let (option, inline) = match arg.split_once('=') {
                Some((option, value)) if arg.starts_with("--") => (option, Some(value)),
                _ => (arg.as_str(), None),
            };
            match option {
                "--dir" | "--domain" | "--public-ip" | "--language" | "--https-port"
                | "--public-port" | "--http-port" | "--media-port" | "--turn-port"
                | "--turns-port" | "--version" | "--ref" => {
                    let value = match inline {
                        Some(value) => value.to_string(),
                        None => {
                            i += 1;
                            args.get(i)
                                .cloned()
                                .ok_or_else(|| format!("{option} needs a value; see --help"))?
                        }
                    };
                    match option {
                        "--dir" => self.dir = value,
                        "--version" => {
                            self.version = value;
                            self.version_from = option.to_string();
                        }
                        "--ref" => {
                            self.reference = value;
                            self.ref_from = option.to_string();
                        }
                        _ => {
                            let key = Key::from_value_option(option)
                                .expect("every value option has a key");
                            self.give(key, &value, option);
                        }
                    }
                }
                "--enable-turn" | "--disable-turn" => {
                    if inline.is_some() {
                        return Err(format!("{option} takes no value; see --help"));
                    }
                    let value = if option == "--enable-turn" {
                        "true"
                    } else {
                        "false"
                    };
                    self.give(Key::TurnEnabled, value, option);
                }
                "-y" | "--yes" | "--build" | "--no-start" | "--install-docker" | "--force"
                | "--skip-port-check" | "--no-color" | "-h" | "--help" => {
                    if inline.is_some() {
                        return Err(format!("{option} takes no value; see --help"));
                    }
                    match option {
                        "-y" | "--yes" => self.assume_yes = true,
                        "--build" => self.build = true,
                        "--no-start" => self.start = false,
                        "--install-docker" => self.install_docker = true,
                        "--force" => self.force = true,
                        "--skip-port-check" => self.skip_port_check = true,
                        "--no-color" => self.color = false,
                        _ => return Ok(Parsed::Help),
                    }
                }
                _ => return Err(format!("Unknown option {option}; see --help")),
            }
            i += 1;
        }
        Ok(Parsed::Run)
    }

    /// Checks the values given before anything else happens, so that a
    /// mistyped option stops the installer with nothing written.
    pub fn validate(&mut self) -> Result<(), String> {
        for key in KEYS {
            let setting = self.setting(key).clone();
            if !setting.given {
                continue;
            }
            let (value, from) = (&setting.value, &setting.from);
            match key {
                Key::Domain => {
                    let address = normalize_address(value);
                    if !is_address(&address) {
                        return Err(format!(
                            "{from} \"{value}\" is not a domain or an IPv4 address"
                        ));
                    }
                    self.setting_mut(key).value = address;
                }
                Key::PublicIp => {
                    if !value.is_empty() && value != "auto" && !is_ipv4(value) {
                        return Err(format!("{from} \"{value}\" is not an IPv4 address or auto"));
                    }
                }
                Key::Language => {
                    if value != "en" && value != "pl" {
                        return Err(format!("{from} \"{value}\" is not en or pl"));
                    }
                }
                Key::TurnEnabled => {
                    if value != "true" && value != "false" {
                        return Err(format!("{from} needs true or false"));
                    }
                }
                Key::HttpPort => {
                    if !is_http_binding(value) {
                        return Err(format!("{from} needs a port or an IP address and port"));
                    }
                }
                _ => {
                    if !is_port(value) {
                        return Err(format!("{from} \"{value}\" is not a port from 1 to 65535"));
                    }
                }
            }
        }
        if !self.version.is_empty() && !is_tag(&self.version) {
            return Err(format!(
                "{} \"{}\" is not a release such as 1.4.0, or latest",
                self.version_from, self.version
            ));
        }
        if !self.reference.is_empty() && !is_ref(&self.reference) {
            return Err(format!(
                "{} \"{}\" is not a git tag or branch",
                self.ref_from, self.reference
            ));
        }
        match self.start_timeout.parse::<u64>() {
            Ok(secs) if secs > 0 && self.start_timeout.bytes().all(|b| b.is_ascii_digit()) => {
                self.start_timeout_secs = secs;
            }
            _ => {
                return Err(format!(
                    "STELLARR_START_TIMEOUT \"{}\" is not a number of seconds",
                    self.start_timeout
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        let mut options = Options::from_env();
        for setting in &mut options.settings {
            *setting = Setting {
                from: setting.from.clone(),
                ..Setting::default()
            };
        }
        options.version.clear();
        options.reference.clear();
        options.start_timeout = "300".to_string();
        options
    }

    fn parse(args: &[&str]) -> Result<Options, String> {
        let mut options = options();
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        options.parse(&args)?;
        options.validate()?;
        Ok(options)
    }

    #[test]
    fn reads_values_inline_and_apart() {
        let options = parse(&[
            "--yes",
            "--domain=Chat.Example.com",
            "--https-port",
            "9443",
            "--enable-turn",
        ])
        .unwrap();
        assert!(options.assume_yes);
        assert_eq!(options.setting(Key::Domain).value, "chat.example.com");
        assert_eq!(options.setting(Key::Domain).from, "--domain");
        assert_eq!(options.setting(Key::HttpsPort).value, "9443");
        assert_eq!(options.setting(Key::TurnEnabled).value, "true");
    }

    #[test]
    fn refuses_bad_values() {
        for args in [
            &["--domain", "bad_host"][..],
            &["--domain", "1.2.3.999"],
            &["--public-ip", "300.1.1.1"],
            &["--https-port", "0"],
            &["--turns-port=08443"],
            &["--language", "de"],
            &["--version", "bad!tag"],
            &["--ref", "../main"],
            &["--dir"],
            &["--frobnicate"],
            &["--yes=1"],
            &["--enable-turn=yes"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn help_stops_reading() {
        let mut options = options();
        let args = vec!["--help".to_string(), "--frobnicate".to_string()];
        assert!(matches!(options.parse(&args), Ok(Parsed::Help)));
    }
}
