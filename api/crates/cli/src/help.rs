//! What `stellarr help` says. The words are the first tool's, which scripts and
//! people have read: the text files hold them as they were printed.

const FULL: &str = include_str!("help/full.txt");
const STATUS: &str = include_str!("help/status.txt");
const DOCTOR: &str = include_str!("help/doctor.txt");
const CONFIG: &str = include_str!("help/config.txt");
const SECRETS: &str = include_str!("help/secrets.txt");
const USERS: &str = include_str!("help/users.txt");
const SESSIONS: &str = include_str!("help/sessions.txt");
const SECURITY: &str = include_str!("help/security.txt");
const BACKUP: &str = include_str!("help/backup.txt");

/// The version of the tool, which is the server's: the tool is built with it.
pub fn version() -> &'static str {
    stellarr_api::version::server()
}

pub fn full() -> String {
    FULL.replace("{version}", version())
}

/// Help for one command, or the full text for one that has none.
pub fn for_command(command: &str) -> String {
    match command {
        "status" => STATUS.to_string(),
        "doctor" => DOCTOR.to_string(),
        "config" => CONFIG.to_string(),
        "secrets" => SECRETS.to_string(),
        "users" => USERS.to_string(),
        "sessions" => SESSIONS.to_string(),
        "security" => SECURITY.to_string(),
        "backup" => BACKUP.to_string(),
        _ => full(),
    }
}
