//! Everything the API reads when it starts, in one place.
//!
//! `Settings::load` takes the process environment, builds the layered keys of
//! [`Config`] and reads each section from them. A value the API cannot use is
//! an error that names the setting, so that the start fails there and not at
//! the first request that needs it.
//!
//! To add a setting: put its default in `defaults.rs`, give its `STELLARR_*`
//! name a row in `variables.rs` if operators set it, add a field to the
//! section it belongs to (or a new section struct) and read it in that
//! section's `load`. A section the API does not use yet needs no struct: its
//! defaults can already be in the table.

mod config;
mod defaults;
mod trusted_proxies;
mod variables;

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use stellarr_messages::{Language, Messages};

pub use config::Config;
pub use trusted_proxies::{Network, TrustedProxies, normalize};
pub use variables::{VARIABLES, Variable};

/// A setting the API cannot start with. The text names the setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingError(String);

impl SettingError {
    pub fn new(message: impl Into<String>) -> SettingError {
        SettingError(message.into())
    }
}

impl fmt::Display for SettingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SettingError {}

/// The name of the environment the API runs in: `Development`, `Production`
/// or `Testing`, in any letter case. Anything else behaves like production,
/// as it does for the container's own scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment(String);

impl Environment {
    pub fn new(name: &str) -> Environment {
        Environment(name.to_string())
    }

    pub fn name(&self) -> &str {
        &self.0
    }

    pub fn is_development(&self) -> bool {
        self.0.eq_ignore_ascii_case("Development")
    }

    /// The environment the integration tests start the binary in: limits that
    /// tests would run into are lifted and the session cookie works over
    /// plain HTTP.
    pub fn is_testing(&self) -> bool {
        self.0.eq_ignore_ascii_case("Testing")
    }

    /// Whether the session cookie may be sent over plain HTTP.
    pub fn allows_plain_http(&self) -> bool {
        self.is_development() || self.is_testing()
    }
}

/// The addresses to listen on, from `ASPNETCORE_URLS`, the name the
/// container's entrypoint sets it under, such as `http://127.0.0.1:5000`. TLS ends at nginx, so
/// only `http` is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listen(pub Vec<SocketAddr>);

impl Listen {
    const DEFAULT: &'static str = "http://localhost:5000";

    fn parse(value: &str) -> Result<Listen, SettingError> {
        let mut addresses = Vec::new();
        for url in value
            .split(';')
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            let bad = || {
                SettingError::new(format!(
                    "ASPNETCORE_URLS lists \"{url}\", which is not an http address such as http://127.0.0.1:5000."
                ))
            };
            let rest = url.strip_prefix("http://").ok_or_else(bad)?;
            let authority = rest.trim_end_matches('/');
            let (host, port) = authority.rsplit_once(':').ok_or_else(bad)?;
            let port: u16 = port.parse().map_err(|_| bad())?;
            let ip = match host {
                "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
                "+" | "*" | "0.0.0.0" => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                other => other
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .map_err(|_| bad())?,
            };
            addresses.push(SocketAddr::new(ip, port));
        }
        if addresses.is_empty() {
            return Err(SettingError::new("ASPNETCORE_URLS lists no address."));
        }
        Ok(Listen(addresses))
    }
}

/// How the database file is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSettings {
    pub path: PathBuf,
    /// How long a statement waits for another connection's lock before it
    /// fails as busy.
    pub busy_timeout: Duration,
}

impl DatabaseSettings {
    const CONNECTION_STRING: &'static str = "ConnectionStrings:Database";

    /// The connection string names the file as `Data Source=...`, and may set
    /// `Default Timeout` in seconds, 30 when it does not. Keywords that only
    /// tune a pool, and `Foreign Keys`, which is always on, are accepted and
    /// ignored. A shared cache is refused: the database runs in write-ahead
    /// logging, which a shared cache defeats within one process.
    fn load(config: &Config) -> Result<DatabaseSettings, SettingError> {
        let text = config
            .text(Self::CONNECTION_STRING)
            .ok_or_else(|| SettingError::new("ConnectionStrings:Database is required."))?;
        let mut path = None;
        let mut timeout = Duration::from_secs(30);
        for part in text
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let (keyword, value) = part.split_once('=').ok_or_else(|| {
                SettingError::new(format!(
                    "ConnectionStrings:Database has \"{part}\", which is not keyword=value."
                ))
            })?;
            let keyword = keyword.trim().to_ascii_lowercase().replace([' ', '_'], "");
            let value = value.trim();
            match keyword.as_str() {
                "datasource" | "data source" | "filename" => path = Some(PathBuf::from(value)),
                "cache" if value.eq_ignore_ascii_case("shared") => {
                    return Err(SettingError::new(
                        "The database connection string sets Cache=Shared. The database runs in \
                         write-ahead logging, which a shared cache defeats within this process; \
                         remove Cache=Shared.",
                    ));
                }
                "defaulttimeout" | "commandtimeout" => {
                    let seconds: u64 = value.parse().map_err(|_| {
                        SettingError::new(format!(
                            "ConnectionStrings:Database has Default Timeout \"{value}\", which is not a number of seconds."
                        ))
                    })?;
                    timeout = Duration::from_secs(seconds);
                }
                "pooling" | "cache" | "foreignkeys" | "mode" => {}
                _ => {
                    return Err(SettingError::new(format!(
                        "ConnectionStrings:Database has the keyword \"{}\", which the API does not know.",
                        part.split('=').next().unwrap_or(part).trim()
                    )));
                }
            }
        }
        let path = path
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| SettingError::new("ConnectionStrings:Database has no Data Source."))?;
        Ok(DatabaseSettings {
            path,
            busy_timeout: timeout,
        })
    }
}

/// Where the API finds Stellarr Media, the media server every voice room goes
/// through: the address and secret of its control API, and the public UDP
/// port browsers send media to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaServerSettings {
    /// Absolute, with a trailing slash so that relative routes append to it.
    pub control_url: String,
    /// The bearer secret of the control API; empty when it runs without one.
    pub control_secret: String,
    pub media_port: u16,
}

impl MediaServerSettings {
    fn load(config: &Config) -> Result<MediaServerSettings, SettingError> {
        let url = config
            .text("Rtc:MediaControlUrl")
            .unwrap_or("http://127.0.0.1:5200");
        let usable = (url.starts_with("http://") && url.len() > "http://".len())
            || (url.starts_with("https://") && url.len() > "https://".len());
        if !usable || url.contains(char::is_whitespace) {
            return Err(SettingError::new(
                "Rtc:MediaControlUrl must be an absolute http or https address.",
            ));
        }
        let port = config.number("Rtc:MediaPort")?.unwrap_or(49000);
        let media_port = u16::try_from(port)
            .ok()
            .filter(|port| *port >= 1)
            .ok_or_else(|| SettingError::new("Rtc:MediaPort must be between 1 and 65535."))?;
        Ok(MediaServerSettings {
            control_url: if url.ends_with('/') {
                url.to_string()
            } else {
                format!("{url}/")
            },
            control_secret: config
                .text("Rtc:MediaControlSecret")
                .unwrap_or("")
                .to_string(),
            media_port,
        })
    }
}

/// Voice: the host browsers are told to reach, the secret that signs TURN
/// credentials, and the media server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcSettings {
    pub public_host: String,
    pub turn_shared_secret: Option<String>,
    pub media: MediaServerSettings,
}

impl RtcSettings {
    fn load(config: &Config, environment: &Environment) -> Result<RtcSettings, SettingError> {
        let turn_shared_secret = config.text("Rtc:TurnSharedSecret").map(str::to_string);
        // Without a secret of its own the relay would accept credentials that
        // anyone can compute from the published development secret.
        if turn_shared_secret.is_none()
            && !environment.is_development()
            && !environment.is_testing()
        {
            return Err(SettingError::new(
                "Rtc:TurnSharedSecret is required outside development. In the container the \
                 entrypoint sets it from the secret it keeps in the data volume.",
            ));
        }
        Ok(RtcSettings {
            public_host: config
                .raw("Rtc:PublicHost")
                .unwrap_or("localhost")
                .to_string(),
            turn_shared_secret,
            media: MediaServerSettings::load(config)?,
        })
    }
}

/// How the instance is run: its public address, its time zone and the
/// proxies in front of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsSettings {
    /// The address people reach the instance at, without a trailing slash;
    /// empty when the instance has none.
    pub public_url: String,
    pub time_zone: Option<String>,
    pub trusted_proxies: TrustedProxies,
}

impl OperationsSettings {
    fn load(config: &Config) -> Result<OperationsSettings, SettingError> {
        Ok(OperationsSettings {
            public_url: config
                .raw("Operations:PublicUrl")
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .to_string(),
            time_zone: config.text("Operations:TimeZone").map(str::to_string),
            trusted_proxies: TrustedProxies::parse(
                config.raw(trusted_proxies::SETTING_NAME).unwrap_or(""),
            )?,
        })
    }
}

/// Which clients the instance serves (the client gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSettings {
    /// False lets a caller that does not name its client through, for an
    /// instance that wants to be reachable from a script.
    pub require_identification: bool,
    /// The `X-Stellarr-Client` values served. Kinds listed under
    /// `Clients:Allowed` are admitted next to the built-in ones, never
    /// instead of them.
    pub allowed: Vec<String>,
    /// Where the current clients can be got; the refusal names it.
    pub download_path: Option<String>,
}

impl ClientSettings {
    const BUILT_IN: [&'static str; 4] = ["web", "cockpit", "desktop-linux", "mobile-android"];

    fn load(config: &Config) -> Result<ClientSettings, SettingError> {
        let mut allowed: Vec<String> = Self::BUILT_IN.iter().map(|kind| kind.to_string()).collect();
        allowed.extend(config.list("Clients:Allowed"));
        Ok(ClientSettings {
            require_identification: config
                .flag("Clients:RequireIdentification")?
                .unwrap_or(true),
            allowed,
            // Unset means the front page; set to nothing means no page.
            download_path: match config.raw("Clients:DownloadPath") {
                None => Some("/".to_string()),
                Some(path) if path.trim().is_empty() => None,
                Some(path) => Some(path.to_string()),
            },
        })
    }
}

/// The desktop client's version floor and recommendation, as configured. The
/// operator can save another floor in the database, which wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSettings {
    pub minimum_version: Option<String>,
    pub recommended_version: Option<String>,
    pub canonical_origin: String,
}

/// How many requests each limit allows in its window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitSettings {
    pub login: u32,
    pub login_poll: u32,
    pub auth_write: u32,
    pub security_write: u32,
    pub invite_preview: u32,
    pub uploads_per_minute: u32,
    pub pin_unlock: u32,
    /// Uploads that run at once; more wait in a queue four times as long.
    pub concurrent_uploads: u32,
}

impl RateLimitSettings {
    fn load(config: &Config, environment: &Environment) -> Result<RateLimitSettings, SettingError> {
        // Tests sign in over and over from one address.
        let permit = |name: &str, fallback: u32| -> Result<u32, SettingError> {
            if environment.is_testing() {
                return Ok(10_000);
            }
            let configured = config.number(&format!("RateLimiting:{name}:PermitLimit"))?;
            Ok(configured.map_or(fallback, |value| value.clamp(1, i64::from(u32::MAX)) as u32))
        };
        let concurrent_uploads = if environment.is_testing() {
            100
        } else {
            config
                .number("Uploads:MaxConcurrentUploads")?
                .map_or(4, |n| n.clamp(1, 32) as u32)
        };
        Ok(RateLimitSettings {
            login: permit("Login", 10)?,
            // The desktop client polls for the browser's approval every second
            // or two, so polling counts apart and wider.
            login_poll: permit("LoginPoll", 120)?,
            auth_write: permit("AuthWrite", 5)?,
            security_write: permit("SecurityWrite", 30)?,
            invite_preview: permit("InvitePreview", 30)?,
            uploads_per_minute: permit("Uploads", 30)?,
            pin_unlock: permit("PinUnlock", 5)?,
            concurrent_uploads,
        })
    }
}

/// What the log shows: a default level and the levels of single categories,
/// both as `Logging:LogLevel:*` keys, whose names are tracing targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSettings {
    pub default_level: String,
    pub categories: Vec<(String, String)>,
}

impl LogSettings {
    const PREFIX: &'static str = "Logging:LogLevel:";

    fn load(config: &Config) -> LogSettings {
        let default_level = config
            .text("Logging:LogLevel:Default")
            .unwrap_or("Information")
            .to_string();
        let mut categories = config.prefixed(Self::PREFIX);
        categories.retain(|(category, _)| !category.eq_ignore_ascii_case("Default"));
        categories.sort();
        LogSettings {
            default_level,
            categories,
        }
    }

    /// The same levels as a `tracing` filter directive. Levels keep the names
    /// operators know: Trace, Debug, Information, Warning, Error, Critical,
    /// None.
    pub fn directives(&self) -> String {
        let mut directives = vec![level_directive(&self.default_level)];
        for (category, level) in &self.categories {
            directives.push(format!("{category}={}", level_directive(level)));
        }
        directives.join(",")
    }
}

fn level_directive(level: &str) -> String {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "warning" => "warn",
        "error" | "critical" => "error",
        "none" => "off",
        _ => "info",
    }
    .to_string()
}

/// Everything the API reads at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub environment: Environment,
    pub listen: Listen,
    /// The folder every file of the instance lives in; the database and the
    /// keys folder are inside it unless the connection string says otherwise.
    pub data_root: PathBuf,
    pub database: DatabaseSettings,
    pub language: Language,
    pub rtc: RtcSettings,
    pub operations: OperationsSettings,
    pub clients: ClientSettings,
    pub desktop: DesktopSettings,
    pub rate_limits: RateLimitSettings,
    pub log: LogSettings,
    /// Tests only: a file the process appends every published event to, one
    /// JSON object per line. Honoured in the Testing environment alone.
    pub event_log: Option<PathBuf>,
    /// Tests only: the names of the background workers that run, from
    /// `Testing:Workers` (comma-separated; none when it is not set). `None`
    /// outside the Testing environment, where every worker runs.
    pub testing_workers: Option<Vec<String>>,
}

impl Settings {
    /// Reads the settings from the process environment, given as `(name,
    /// value)` pairs.
    pub fn load(variables: &[(String, String)]) -> Result<Settings, SettingError> {
        let lookup = |name: &str| {
            variables
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
        };
        // The container's entrypoint sets both of these names.
        let environment =
            Environment::new(lookup("ASPNETCORE_ENVIRONMENT").unwrap_or("Production"));
        let config = Config::layered(environment.name(), variables);
        Settings::from_config(&config, environment, lookup("ASPNETCORE_URLS"))
    }

    /// Reads the sections from layered keys. Tests use this to set single
    /// keys.
    pub fn from_config(
        config: &Config,
        environment: Environment,
        urls: Option<&str>,
    ) -> Result<Settings, SettingError> {
        let language_text = config.raw("Stellarr:Language").unwrap_or("en");
        let language = Language::parse(language_text).map_err(|refused| {
            SettingError::new(format!(
                "Stellarr:Language is \"{}\" but must be \"en\" or \"pl\".",
                refused.0
            ))
        })?;
        let rtc = RtcSettings::load(config, &environment)?;
        let event_log = environment
            .is_testing()
            .then(|| config.text("Testing:EventLog").map(PathBuf::from))
            .flatten();
        let testing_workers = environment.is_testing().then(|| {
            config
                .text("Testing:Workers")
                .into_iter()
                .flat_map(|names| names.split(','))
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        });
        Ok(Settings {
            listen: Listen::parse(urls.unwrap_or(Listen::DEFAULT))?,
            data_root: PathBuf::from(config.text("DataRoot").unwrap_or("data")),
            database: DatabaseSettings::load(config)?,
            language,
            rtc,
            operations: OperationsSettings::load(config)?,
            clients: ClientSettings::load(config)?,
            desktop: DesktopSettings {
                minimum_version: config.text("Desktop:MinimumVersion").map(str::to_string),
                recommended_version: config
                    .text("Desktop:RecommendedVersion")
                    .map(str::to_string),
                canonical_origin: config
                    .text("Desktop:CanonicalOrigin")
                    .unwrap_or("")
                    .to_string(),
            },
            rate_limits: RateLimitSettings::load(config, &environment)?,
            log: LogSettings::load(config),
            event_log,
            testing_workers,
            environment,
        })
    }

    /// The catalogue of messages in the language of the instance.
    pub fn messages(&self) -> Messages {
        Messages::new(self.language)
    }
}

#[cfg(test)]
mod tests;
