//! How the instance backs itself up and tidies itself: the `Backups`,
//! `Cloudflare:R2` and `Maintenance` sections.
//!
//! What the operator saves in the panel (the schedule, the offsite
//! destination) wins over these, which are what an instance starts with. The
//! credentials of the offsite destination are secrets and live in
//! [`crate::secrets`], never here.

use super::SettingError;
use super::config::Config;

/// The `Backups` section: whether the instance backs itself up on a schedule,
/// and how often.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSettings {
    pub enabled: bool,
    pub interval_hours: i64,
    pub retry_minutes: i64,
    pub initial_delay_minutes: i64,
}

impl BackupSettings {
    pub const INTERVAL_HOURS: (i64, i64) = (1, 8760);
    pub const RETRY_MINUTES: (i64, i64) = (1, 1440);
    pub const INITIAL_DELAY_MINUTES: (i64, i64) = (0, 1440);

    pub(super) fn load(config: &Config) -> Result<BackupSettings, SettingError> {
        let ranged = |key: &str, default: i64, (low, high): (i64, i64)| {
            let value = config.number(key)?.unwrap_or(default);
            if (low..=high).contains(&value) {
                Ok(value)
            } else {
                Err(SettingError::new(format!(
                    "{key} must be between {low} and {high}, but is {value}."
                )))
            }
        };
        Ok(BackupSettings {
            enabled: config.flag("Backups:Enabled")?.unwrap_or(false),
            interval_hours: ranged("Backups:IntervalHours", 24, Self::INTERVAL_HOURS)?,
            retry_minutes: ranged("Backups:RetryMinutes", 15, Self::RETRY_MINUTES)?,
            initial_delay_minutes: ranged(
                "Backups:InitialDelayMinutes",
                5,
                Self::INITIAL_DELAY_MINUTES,
            )?,
        })
    }
}

/// How the destination is reached: with S3 credentials, or through the relay
/// Worker with a token.
pub const R2_MODE_S3: &str = "s3";
pub const R2_MODE_WORKER: &str = "worker";

pub fn is_known_r2_mode(mode: &str) -> bool {
    mode == R2_MODE_S3 || mode == R2_MODE_WORKER
}

/// The `Cloudflare:R2` section without its credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudflareR2Settings {
    pub enabled: bool,
    pub mode: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
}

impl CloudflareR2Settings {
    pub(super) fn load(config: &Config) -> Result<CloudflareR2Settings, SettingError> {
        let text = |key: &str, default: &str| config.raw(key).unwrap_or(default).to_string();
        let settings = CloudflareR2Settings {
            enabled: config.flag("Cloudflare:R2:Enabled")?.unwrap_or(false),
            mode: text("Cloudflare:R2:Mode", R2_MODE_S3),
            endpoint: text("Cloudflare:R2:Endpoint", ""),
            region: text("Cloudflare:R2:Region", "auto"),
            bucket: text("Cloudflare:R2:Bucket", ""),
            prefix: text("Cloudflare:R2:Prefix", "stellarr"),
        };
        if !settings.is_valid() {
            return Err(SettingError::new(
                "Cloudflare R2 requires HTTPS, a safe prefix and the credentials of its connection mode.",
            ));
        }
        Ok(settings)
    }

    /// Whether the destination could work, apart from its credentials. One
    /// that is off is always valid.
    pub fn is_valid(&self) -> bool {
        if !self.enabled {
            return true;
        }
        match self.mode.as_str() {
            R2_MODE_S3 => {
                is_plain_https_url(&self.endpoint)
                    && !self.bucket.trim().is_empty()
                    && !self.region.trim().is_empty()
                    && has_safe_prefix(&self.prefix)
            }
            R2_MODE_WORKER => is_plain_https_url(&self.endpoint) && has_safe_prefix(&self.prefix),
            _ => false,
        }
    }
}

/// An absolute `https` address with no credentials, query or fragment.
pub fn is_plain_https_url(endpoint: &str) -> bool {
    let Some((scheme, rest)) = endpoint.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty()
        && !authority.contains('@')
        && !rest.contains(['?', '#'])
        && !endpoint
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
}

fn has_safe_prefix(prefix: &str) -> bool {
    !prefix.trim().is_empty()
        && !prefix
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."))
}

/// The `Maintenance` section: the hourly housekeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaintenanceSettings {
    pub enabled: bool,
    pub initial_delay_minutes: i64,
    pub interval_hours: i64,
    pub batch_size: i64,
    pub message_change_retention_days: i64,
}

impl MaintenanceSettings {
    pub(super) fn load(config: &Config) -> Result<MaintenanceSettings, SettingError> {
        let number =
            |key: &str, default: i64| Ok::<_, SettingError>(config.number(key)?.unwrap_or(default));
        Ok(MaintenanceSettings {
            enabled: config.flag("Maintenance:Enabled")?.unwrap_or(true),
            initial_delay_minutes: number("Maintenance:InitialDelayMinutes", 5)?,
            interval_hours: number("Maintenance:IntervalHours", 1)?,
            batch_size: number("Maintenance:BatchSize", 1000)?,
            message_change_retention_days: number("Maintenance:MessageChangeRetentionDays", 30)?,
        })
    }

    /// How many orphans and queued deletions one run handles, 1 to 1000.
    pub fn batch(&self) -> usize {
        self.batch_size.clamp(1, 1000) as usize
    }

    /// How long the message journal keeps its entries, 7 to 365 days.
    pub fn journal_days(&self) -> i64 {
        self.message_change_retention_days.clamp(7, 365)
    }
}
