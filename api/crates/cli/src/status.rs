//! `status` and `doctor`: what the data volume, the database, the keys and the
//! local API look like, without changing any of it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use rusqlite::params;
use stellarr_api::values::Timestamp;
use stellarr_outbound::{AddressPolicy, Client, Request};

use crate::errors::{CliError, CliOutcome, CliResult, exit};
use crate::json::Json;
use crate::runtime::{Runtime, value_in, variable};
use crate::support::require_instance_server;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// What the local API answered to `/health/ready`.
pub struct ApiState {
    pub url: String,
    pub available: bool,
    pub status_code: Option<u16>,
    pub detail: String,
}

/// Asks the API on this machine whether it is ready: `STELLARR_API_URL`, or
/// `http://127.0.0.1:5000`.
pub fn probe_api(runtime: &Runtime) -> CliOutcome<ApiState> {
    let configured = value_in(&runtime.environment, variable("STELLARR_API_URL"));
    let url = configured.map_or_else(
        || "http://127.0.0.1:5000".to_string(),
        |url| url.trim().trim_end_matches('/').to_string(),
    );
    // A URL that cannot be asked at all is a mistake of the operator's, which
    // the first tool also met as an unexpected failure.
    let request = Request::get(&format!("{url}/health/ready"))
        .map_err(|_| CliError::unexpected("InvalidOperationException"))?
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_TIMEOUT)
        .max_redirects(50)
        .policy(AddressPolicy::Anywhere);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| CliError::unexpected("IOException"))?;
    Ok(
        match runtime.block_on(Client::new().send_collected(request)) {
            Ok(answer) => {
                let code = answer.status.as_u16();
                let available = answer.status.is_success();
                ApiState {
                    url,
                    available,
                    status_code: Some(code),
                    detail: if available {
                        "ready".to_string()
                    } else {
                        format!("HTTP {code}")
                    },
                }
            }
            Err(_) => ApiState {
                url,
                available: false,
                status_code: None,
                detail: "not reachable".to_string(),
            },
        },
    )
}

pub fn status(runtime: &mut Runtime) -> CliOutcome {
    let database = runtime.open_database(false)?;
    let connection = &database.connection;
    let server = require_instance_server(connection)?;
    let now = Timestamp::now();
    let users: i64 =
        connection.query_row("SELECT COUNT(*) FROM \"Users\"", [], |row| row.get(0))?;
    let active_sessions: i64 = connection.query_row(
        "SELECT COUNT(*) FROM \"UserSessions\" WHERE \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?1",
        params![now],
        |row| row.get(0),
    )?;
    let api = probe_api(runtime)?;
    let path = runtime.database_path();
    let size = std::fs::metadata(&path)?.len() as i64;
    let healthy = api.available;
    let data = Json::object([
        ("healthy", healthy.into()),
        ("dataRoot", Json::text(&runtime.data_root)),
        (
            "database",
            Json::object([
                ("path", Json::text(&path)),
                ("ready", true.into()),
                ("schemaCompatible", true.into()),
                ("sizeBytes", size.into()),
            ]),
        ),
        (
            "instance",
            Json::object([
                ("id", Json::text(server.id.to_string())),
                ("name", Json::text(&server.name)),
                ("users", users.into()),
                ("activeSessions", active_sessions.into()),
            ]),
        ),
        (
            "api",
            Json::object([
                ("url", Json::text(&api.url)),
                ("available", api.available.into()),
                (
                    "statusCode",
                    api.status_code
                        .map_or(Json::Null, |code| Json::Int(i64::from(code))),
                ),
                ("detail", Json::text(&api.detail)),
            ]),
        ),
    ]);
    let lines = vec![
        format!("Instance: {}", server.name),
        format!("Database: ready ({path}, {size} bytes)"),
        format!("Accounts: {users}; active sessions: {active_sessions}"),
        format!(
            "Local API: {} ({})",
            if api.available {
                "ready"
            } else {
                "not reachable"
            },
            api.url
        ),
    ];
    Ok(CliResult::new("status", data, lines).exit(if healthy {
        exit::SUCCESS
    } else {
        exit::FAILURE
    }))
}

struct Check {
    name: &'static str,
    status: &'static str,
    detail: String,
}

impl Check {
    fn new(name: &'static str, status: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            status,
            detail: detail.into(),
        }
    }

    fn json(&self) -> Json {
        Json::object([
            ("name", self.name.into()),
            ("status", self.status.into()),
            ("detail", Json::text(&self.detail)),
        ])
    }
}

pub fn doctor(runtime: &mut Runtime) -> CliOutcome {
    let mut checks = vec![
        check_directory("data-root", &runtime.data_root),
        check_keys(runtime),
        check_disk(&runtime.data_root),
        check_database(runtime),
    ];
    let api = probe_api(runtime)?;
    checks.push(Check::new(
        "local-api",
        if api.available { "ok" } else { "warn" },
        if api.available {
            format!("ready at {}", api.url)
        } else {
            format!(
                "not reachable at {}; expected when the instance is stopped or still starting",
                api.url
            )
        },
    ));
    let failed = checks.iter().any(|check| check.status == "fail");
    let exit = if !runtime.database_exists() {
        exit::NOT_FOUND
    } else if checks
        .iter()
        .any(|check| check.name == "schema" && check.status == "fail")
    {
        exit::CONFLICT
    } else if failed {
        exit::FAILURE
    } else {
        exit::SUCCESS
    };
    let lines = checks
        .iter()
        .map(|check| format!("[{}] {}: {}", check.status, check.name, check.detail))
        .collect();
    let data = Json::object([
        ("healthy", (!failed).into()),
        ("dataRoot", Json::text(&runtime.data_root)),
        ("checks", Json::array(checks.iter().map(Check::json))),
    ]);
    Ok(CliResult::new("doctor", data, lines).exit(exit))
}

fn check_directory(name: &'static str, path: &str) -> Check {
    if Path::new(path).is_dir() {
        Check::new(name, "ok", format!("{path} exists"))
    } else {
        Check::new(name, "fail", format!("{path} does not exist"))
    }
}

/// The keys folder holds one master key, `master.key`.
fn check_keys(runtime: &Runtime) -> Check {
    let keys = runtime.keys_directory();
    let shown = keys.display();
    if !keys.is_dir() {
        return Check::new(
            "keys",
            "warn",
            format!(
                "{shown} does not exist; read commands will not create it, and secrets cannot be written until the API creates it"
            ),
        );
    }
    if runtime.has_master_key() {
        return Check::new("keys", "ok", format!("{shown} has the master key"));
    }
    Check::new("keys", "warn", format!("{shown} has no master key yet"))
}

fn check_disk(data_root: &str) -> Check {
    let Some((free, root)) = free_space(Path::new(data_root)) else {
        return Check::new("disk", "warn", "free space could not be read");
    };
    let status = if free >= 1024 * 1024 * 1024 {
        "ok"
    } else if free >= 100 * 1024 * 1024 {
        "warn"
    } else {
        "fail"
    };
    Check::new(
        "disk",
        status,
        format!("{free} bytes free on {root} for {data_root}"),
    )
}

/// The bytes free on the volume that holds `path`, as `df` says; none when it
/// cannot say.
pub fn free_bytes(path: &Path) -> Option<u64> {
    free_space(path).map(|(free, _)| free)
}

/// The bytes free on the volume that holds `path` and where that volume is
/// mounted, as `df` says. The standard library cannot say, and this code may
/// not call into the system itself.
fn free_space(path: &Path) -> Option<(u64, String)> {
    let mut probe = std::path::absolute(path).ok()?;
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let output = Command::new("df")
        .arg("-Pk")
        .arg(&probe)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_df(&String::from_utf8_lossy(&output.stdout))
}

/// The Available column, in bytes, and the mount point, of the last line of
/// `df -P`. A device or a mount point may hold spaces, so the percentage marks
/// the place.
fn parse_df(text: &str) -> Option<(u64, String)> {
    let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let columns: Vec<&str> = line.split_whitespace().collect();
    let at = columns.iter().rposition(|column| column.ends_with('%'))?;
    let free = columns
        .get(at.checked_sub(1)?)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)?;
    Some((free, columns[at + 1..].join(" ")))
}

fn check_database(runtime: &Runtime) -> Check {
    if !runtime.database_exists() {
        return Check::new(
            "database",
            "fail",
            format!(
                "no database at {}; the CLI never creates one",
                runtime.database_path()
            ),
        );
    }
    match runtime.open_database(false) {
        Ok(_) => {
            let size = std::fs::metadata(runtime.database_path()).map_or(0, |meta| meta.len());
            Check::new(
                "schema",
                "ok",
                format!("database is readable ({size} bytes) and has the required schema"),
            )
        }
        Err(error) if error.code == "schema_incompatible" => {
            Check::new("schema", "fail", error.message)
        }
        Err(error) => Check::new(
            "database",
            "fail",
            match error.cause {
                // What the database driver said, as the first tool put it.
                Some(cause) => format!("the database cannot be read: {cause}"),
                None => error.message,
            },
        ),
    }
}

/// The filesystem with the longest mount path containing `path`, so that a
/// separately mounted data volume is checked instead of the root overlay.
pub fn longest_matching_root<'a>(path: &str, roots: &[&'a str]) -> Option<&'a str> {
    roots
        .iter()
        .copied()
        .filter(|root| is_within(path, root))
        .max_by_key(|root| root.len())
}

fn is_within(path: &str, root: &str) -> bool {
    if path == root {
        return true;
    }
    let prefix = if root.ends_with('/') {
        root.to_string()
    } else {
        format!("{root}/")
    };
    path.starts_with(&prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_mount_that_holds_the_path_is_the_one_checked() {
        let roots = ["/", "/data", "/data/stellarr"];
        assert_eq!(
            longest_matching_root("/data/stellarr/db/stellarr.db", &roots),
            Some("/data/stellarr")
        );
        assert_eq!(
            longest_matching_root("/data/stellarr-other/db/stellarr.db", &roots),
            Some("/data")
        );
        assert_eq!(
            longest_matching_root("/var/lib/stellarr/db/stellarr.db", &["/"]),
            Some("/")
        );
    }

    #[test]
    fn df_is_read_from_its_last_line() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s5 482344960 436207616 4194304 99% /System/Volumes/Data\n";
        assert_eq!(
            parse_df(text),
            Some((4_294_967_296, "/System/Volumes/Data".to_string()))
        );
    }
}
