//! The limits and requirements authentication reads: whether a second step is
//! asked for, how long challenges, codes and remembered devices live and how
//! often a code may be asked for. An installation with no settings row yet
//! gets the documented defaults, which keep the second step optional.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};

use crate::instance::access;
use crate::values::Id;

pub const OPTIONAL: &str = "optional";
pub const ADMINS: &str = "admins";
pub const EVERYONE: &str = "everyone";
pub const COCKPIT: &str = "cockpit";

/// How recent a real second step must be for a change that could hand an
/// account or the instance to somebody else.
pub const FRESH_SECOND_STEP_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Failed second-step proofs an account may collect inside
/// [`FAILURE_WINDOW`], across every challenge, before further ones are refused.
pub const FAILURE_BUDGET: i64 = 10;

/// The span the account-wide failure budget counts over.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Whether the account has a second step that is confirmed and still in use.
pub fn has_confirmed_method(connection: &Connection, user: Id) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"MfaMethods\" WHERE \"UserId\" = ?1 AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL)",
        [user],
        |row| row.get(0),
    )
}

/// The requirement as one of the four known words, `optional` for anything else.
pub fn normalize_requirement(value: Option<&str>) -> &'static str {
    match value.map(|v| v.trim().to_lowercase()).as_deref() {
        Some("admins") => ADMINS,
        Some("everyone") => EVERYONE,
        Some("cockpit") => COCKPIT,
        _ => OPTIONAL,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityPolicy {
    pub mfa_requirement: &'static str,
    pub trusted_device_days: i64,
    pub auth_challenge_minutes: i64,
    pub email_code_minutes: i64,
    pub email_code_cooldown_seconds: i64,
    pub email_code_max_attempts: i64,
    pub email_code_sends_per_hour: i64,
    pub cli_recovery_minutes: i64,
}

impl Default for SecurityPolicy {
    fn default() -> SecurityPolicy {
        SecurityPolicy {
            mfa_requirement: OPTIONAL,
            trusted_device_days: 14,
            auth_challenge_minutes: 5,
            email_code_minutes: 8,
            email_code_cooldown_seconds: 45,
            email_code_max_attempts: 5,
            email_code_sends_per_hour: 10,
            cli_recovery_minutes: 10,
        }
    }
}

impl SecurityPolicy {
    /// What the instance server's settings say, or the defaults.
    pub fn load(connection: &Connection) -> rusqlite::Result<SecurityPolicy> {
        let Some(server) = access::instance_server(connection)? else {
            return Ok(SecurityPolicy::default());
        };
        let row = connection
            .query_row(
                "SELECT \"MfaRequirement\", \"TrustedDeviceDays\", \"AuthChallengeMinutes\", \"EmailCodeMinutes\",
                        \"EmailCodeCooldownSeconds\", \"EmailCodeMaxAttempts\", \"EmailCodeSendsPerHour\", \"CliRecoveryMinutes\"
                 FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
                [server],
                |row| {
                    Ok(SecurityPolicy {
                        mfa_requirement: normalize_requirement(row.get::<_, Option<String>>(0)?.as_deref()),
                        trusted_device_days: row.get(1)?,
                        auth_challenge_minutes: row.get(2)?,
                        email_code_minutes: row.get(3)?,
                        email_code_cooldown_seconds: row.get(4)?,
                        email_code_max_attempts: row.get(5)?,
                        email_code_sends_per_hour: row.get(6)?,
                        cli_recovery_minutes: row.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    pub fn challenge_lifetime(&self) -> Duration {
        Duration::from_secs(self.auth_challenge_minutes.clamp(1, 60) as u64 * 60)
    }

    pub fn email_code_lifetime(&self) -> Duration {
        Duration::from_secs(self.email_code_minutes.clamp(1, 60) as u64 * 60)
    }

    pub fn email_cooldown(&self) -> Duration {
        Duration::from_secs(self.email_code_cooldown_seconds.clamp(0, 600) as u64)
    }

    /// How many codes an account may be sent in an hour, and how many wrong
    /// guesses one code survives.
    pub fn email_code_sends_per_hour_clamped(&self) -> i64 {
        self.email_code_sends_per_hour.clamp(1, 60)
    }

    pub fn email_code_max_attempts_clamped(&self) -> i64 {
        self.email_code_max_attempts.clamp(1, 10)
    }

    pub fn trusted_device_lifetime(&self) -> Duration {
        Duration::from_secs(self.trusted_device_days_clamped() as u64 * 24 * 3600)
    }

    pub fn cli_recovery_lifetime(&self) -> Duration {
        Duration::from_secs(self.cli_recovery_minutes.clamp(1, 60) as u64 * 60)
    }

    /// Whether the operator's panel asks for a real second step at all.
    pub fn requires_cockpit_second_step(&self) -> bool {
        self.mfa_requirement != OPTIONAL
    }

    pub fn trusted_device_days_clamped(&self) -> i64 {
        self.trusted_device_days.clamp(1, 365)
    }

    /// Whether a new sign-in must complete a second step, or an enrolment,
    /// before it gets a session.
    pub fn is_required_at_login(
        &self,
        connection: &Connection,
        user: Id,
    ) -> rusqlite::Result<bool> {
        match self.mfa_requirement {
            EVERYONE => Ok(true),
            ADMINS => access::is_instance_admin(connection, user),
            _ => Ok(false),
        }
    }

    /// Whether a session that has passed `fresh_mfa` (a real second step) or
    /// has `evidence` of another kind meets the requirement. A remembered
    /// device is a real proof for an ordinary session, but never counts where
    /// the requirement is for the operator panel.
    pub fn is_satisfied(&self, fresh_mfa: bool, evidence: Option<&str>) -> bool {
        fresh_mfa || (evidence == Some("trusted_device") && self.mfa_requirement != COCKPIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_requirement_is_optional_and_a_known_one_is_read_in_any_case() {
        assert_eq!(normalize_requirement(Some(" Everyone ")), EVERYONE);
        assert_eq!(normalize_requirement(Some("ADMINS")), ADMINS);
        assert_eq!(normalize_requirement(Some("cockpit")), COCKPIT);
        assert_eq!(normalize_requirement(Some("sometimes")), OPTIONAL);
        assert_eq!(normalize_requirement(None), OPTIONAL);
    }

    #[test]
    fn lifetimes_are_held_inside_their_bounds() {
        let policy = SecurityPolicy {
            auth_challenge_minutes: 500,
            trusted_device_days: 0,
            ..SecurityPolicy::default()
        };

        assert_eq!(policy.challenge_lifetime(), Duration::from_secs(3600));
        assert_eq!(policy.trusted_device_days_clamped(), 1);
    }

    #[test]
    fn every_limit_of_a_code_is_held_inside_its_bounds() {
        let wild = |value| SecurityPolicy {
            email_code_minutes: value,
            email_code_cooldown_seconds: value,
            email_code_max_attempts: value,
            email_code_sends_per_hour: value,
            cli_recovery_minutes: value,
            trusted_device_days: value,
            ..SecurityPolicy::default()
        };
        let (low, high) = (wild(-5), wild(100_000));

        assert_eq!(low.email_code_lifetime(), Duration::from_secs(60));
        assert_eq!(high.email_code_lifetime(), Duration::from_secs(3600));
        assert_eq!(low.email_cooldown(), Duration::ZERO);
        assert_eq!(high.email_cooldown(), Duration::from_secs(600));
        assert_eq!(low.email_code_max_attempts_clamped(), 1);
        assert_eq!(high.email_code_max_attempts_clamped(), 10);
        assert_eq!(low.email_code_sends_per_hour_clamped(), 1);
        assert_eq!(high.email_code_sends_per_hour_clamped(), 60);
        assert_eq!(low.cli_recovery_lifetime(), Duration::from_secs(60));
        assert_eq!(high.cli_recovery_lifetime(), Duration::from_secs(3600));
        assert_eq!(
            low.trusted_device_lifetime(),
            Duration::from_secs(24 * 3600)
        );
        assert_eq!(
            high.trusted_device_lifetime(),
            Duration::from_secs(365 * 24 * 3600)
        );
    }

    #[test]
    fn the_settings_row_of_the_instance_server_is_read_and_a_missing_one_gives_the_defaults() {
        let (_folder, connection) = crate::auth::fixtures::open();
        assert_eq!(
            SecurityPolicy::load(&connection).unwrap(),
            SecurityPolicy::default()
        );
        let server = crate::auth::fixtures::add_instance_server(&connection);
        assert_eq!(
            SecurityPolicy::load(&connection).unwrap(),
            SecurityPolicy::default()
        );
        connection
            .execute(
                "INSERT INTO \"OperationsSettings\" (\"ServerId\", \"PublicUrl\", \"TimeZone\", \"MailingEnabled\", \"MailFromAddress\",
                     \"InviteMailEnabled\", \"RecoveryMailEnabled\", \"AlertMailEnabled\", \"BackupRetentionDays\", \"LogRetentionDays\",
                     \"OrphanObjectRetentionDays\", \"AutomaticCleanupEnabled\", \"UpdatedAt\", \"MfaRequirement\", \"TrustedDeviceDays\",
                     \"AuthChallengeMinutes\", \"EmailCodeMinutes\", \"EmailCodeCooldownSeconds\", \"EmailCodeMaxAttempts\",
                     \"EmailCodeSendsPerHour\", \"CliRecoveryMinutes\")
                 VALUES (?1, '', 'UTC', 0, '', 0, 0, 0, 7, 7, 7, 0, '2026-11-24 00:00:00', 'Admins', 30, 9, 3, 12, 4, 2, 6)",
                [server],
            )
            .unwrap();

        assert_eq!(
            SecurityPolicy::load(&connection).unwrap(),
            SecurityPolicy {
                mfa_requirement: ADMINS,
                trusted_device_days: 30,
                auth_challenge_minutes: 9,
                email_code_minutes: 3,
                email_code_cooldown_seconds: 12,
                email_code_max_attempts: 4,
                email_code_sends_per_hour: 2,
                cli_recovery_minutes: 6,
            }
        );
    }

    #[test]
    fn the_defaults_are_the_documented_ones() {
        let policy = SecurityPolicy::default();

        assert_eq!(policy.email_code_lifetime(), Duration::from_secs(8 * 60));
        assert_eq!(policy.email_cooldown(), Duration::from_secs(45));
        assert_eq!(policy.email_code_max_attempts_clamped(), 5);
        assert_eq!(policy.email_code_sends_per_hour_clamped(), 10);
        assert_eq!(policy.cli_recovery_lifetime(), Duration::from_secs(600));
        assert!(!policy.requires_cockpit_second_step());
    }

    #[test]
    fn a_remembered_device_satisfies_every_requirement_but_the_operator_panels() {
        let policy = |requirement| SecurityPolicy {
            mfa_requirement: requirement,
            ..SecurityPolicy::default()
        };

        assert!(policy(EVERYONE).is_satisfied(false, Some("trusted_device")));
        assert!(!policy(COCKPIT).is_satisfied(false, Some("trusted_device")));
        assert!(policy(COCKPIT).is_satisfied(true, None));
        assert!(!policy(EVERYONE).is_satisfied(false, None));
        assert!(!policy(EVERYONE).is_satisfied(false, Some("provider_mfa")));
    }
}
