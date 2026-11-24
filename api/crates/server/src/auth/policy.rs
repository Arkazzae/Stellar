//! The limits and requirements authentication reads: whether a second step is
//! asked for, and for how long challenges and remembered devices live. An
//! installation with no settings row yet gets the documented defaults, which
//! keep the second step optional.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};

use crate::instance::access;

pub const OPTIONAL: &str = "optional";
pub const ADMINS: &str = "admins";
pub const EVERYONE: &str = "everyone";
pub const COCKPIT: &str = "cockpit";

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
}

impl Default for SecurityPolicy {
    fn default() -> SecurityPolicy {
        SecurityPolicy {
            mfa_requirement: OPTIONAL,
            trusted_device_days: 14,
            auth_challenge_minutes: 5,
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
                "SELECT \"MfaRequirement\", \"TrustedDeviceDays\", \"AuthChallengeMinutes\" FROM \"OperationsSettings\" WHERE \"ServerId\" = ?1",
                [server],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()?;
        Ok(
            row.map_or_else(SecurityPolicy::default, |(requirement, days, minutes)| {
                SecurityPolicy {
                    mfa_requirement: normalize_requirement(requirement.as_deref()),
                    trusted_device_days: days,
                    auth_challenge_minutes: minutes,
                }
            }),
        )
    }

    pub fn challenge_lifetime(&self) -> Duration {
        Duration::from_secs(self.auth_challenge_minutes.clamp(1, 60) as u64 * 60)
    }

    pub fn trusted_device_days_clamped(&self) -> i64 {
        self.trusted_device_days.clamp(1, 365)
    }

    /// Whether a new sign-in must complete a second step, or an enrolment,
    /// before it gets a session.
    pub fn is_required_at_login(
        &self,
        connection: &Connection,
        user: crate::values::Id,
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
