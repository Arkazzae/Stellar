//! The secrets an operator sets in the panel: the SMTP password, the Resend
//! key, the keys of the integrations.
//!
//! A value set in the panel wins; without one, the environment's value
//! stands. Values only go in: nothing here gives one back to a client, and
//! nothing logs one. A row holds the value sealed (see [`crate::seal`]) for the
//! purpose [`PURPOSE`] with the secret's name as associated data, so a row
//! moved under another name does not open.
//!
//! A row that cannot be opened (written under another master key, or altered)
//! is a damaged value like any other: reading the secrets fails with it, and
//! the operator clears the secret in the panel or restores the master key.
//!
//! The functions work on a connection or a transaction so that the caller
//! decides what else commits with a change (the operator panel writes its
//! audit entry in the same transaction).

use std::fmt;

use rusqlite::{Connection, params};

use crate::seal::Sealer;
use crate::settings::{EnvironmentSecrets, VARIABLES};
use crate::values::Timestamp;

#[cfg(test)]
mod tests;

/// The purpose every stored secret is sealed for.
const PURPOSE: &str = "instance-secret";

/// The longest value a secret may hold, counted as the clients count it, in
/// UTF-16 code units.
pub const MAX_LENGTH: usize = 4096;

/// A secret the panel can hold, and the environment key it falls back to.
/// Passwords keep their spaces; keys are trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretDefinition {
    pub name: &'static str,
    /// The key the environment's value is read under.
    pub setting: &'static str,
    pub trim: bool,
}

impl SecretDefinition {
    /// The `STELLARR_*` name an operator sets in `.env`.
    pub fn variable(&self) -> &'static str {
        VARIABLES
            .iter()
            .find(|variable| variable.key == Some(self.setting))
            .map_or("", |variable| variable.name)
    }
}

pub const RESEND_API_KEY: SecretDefinition = SecretDefinition {
    name: "mail.resend-api-key",
    setting: "Mail:ResendApiKey",
    trim: true,
};
pub const SMTP_PASSWORD: SecretDefinition = SecretDefinition {
    name: "mail.smtp-password",
    setting: "Mail:SmtpPassword",
    trim: false,
};
pub const GIPHY_API_KEY: SecretDefinition = SecretDefinition {
    name: "giphy.api-key",
    setting: "Giphy:ApiKey",
    trim: true,
};
pub const CLOUDFLARE_R2_ACCESS_KEY: SecretDefinition = SecretDefinition {
    name: "cloudflare.r2.access-key",
    setting: "Cloudflare:R2:AccessKey",
    trim: true,
};
pub const CLOUDFLARE_R2_SECRET_KEY: SecretDefinition = SecretDefinition {
    name: "cloudflare.r2.secret-key",
    setting: "Cloudflare:R2:SecretKey",
    trim: true,
};
pub const CLOUDFLARE_R2_WORKER_TOKEN: SecretDefinition = SecretDefinition {
    name: "cloudflare.r2.worker-token",
    setting: "Cloudflare:R2:WorkerToken",
    trim: true,
};

/// Every secret, in the order the panel lists them.
pub const ALL: [SecretDefinition; 6] = [
    RESEND_API_KEY,
    SMTP_PASSWORD,
    GIPHY_API_KEY,
    CLOUDFLARE_R2_ACCESS_KEY,
    CLOUDFLARE_R2_SECRET_KEY,
    CLOUDFLARE_R2_WORKER_TOKEN,
];

/// The secret called `name`, if there is one.
pub fn find(name: &str) -> Option<&'static SecretDefinition> {
    ALL.iter().find(|definition| definition.name == name)
}

/// The rule the panel and the operator's command line both apply before they
/// store a value: something other than blanks, at most [`MAX_LENGTH`] long.
pub fn is_valid_value(value: &str) -> bool {
    !value.trim().is_empty() && value.encode_utf16().count() <= MAX_LENGTH
}

/// Where the value of a secret comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    None,
    Panel,
    Environment,
}

impl Source {
    /// The word the panel shows.
    pub fn name(self) -> &'static str {
        match self {
            Source::None => "none",
            Source::Panel => "panel",
            Source::Environment => "environment",
        }
    }
}

/// A secret's value and where it came from; empty when it is set nowhere.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue {
    value: String,
    pub source: Source,
}

static MISSING: SecretValue = SecretValue {
    value: String::new(),
    source: Source::None,
};

impl SecretValue {
    pub fn is_set(&self) -> bool {
        self.source != Source::None
    }

    /// The value, for the code that uses it. Never for a response or the log.
    pub fn expose(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretValue {{ source: {:?} }}", self.source)
    }
}

/// What the panel is told of a secret: whether it is set and where it comes
/// from, never what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretStatus {
    pub name: &'static str,
    pub configured: bool,
    /// `panel`, `environment` or `none`.
    pub source: &'static str,
    /// The variable the environment's value is set under.
    pub variable: &'static str,
    /// When the panel's value was stored; none when it is not the panel's.
    pub updated_at: Option<Timestamp>,
}

/// Every secret, resolved.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedSecrets {
    values: Vec<(&'static str, SecretValue)>,
    panel_updated_at: Vec<(&'static str, Timestamp)>,
}

impl ResolvedSecrets {
    pub fn get(&self, definition: &SecretDefinition) -> &SecretValue {
        self.values
            .iter()
            .find(|(name, _)| *name == definition.name)
            .map_or(&MISSING, |(_, value)| value)
    }

    pub fn resend_api_key(&self) -> &SecretValue {
        self.get(&RESEND_API_KEY)
    }

    pub fn smtp_password(&self) -> &SecretValue {
        self.get(&SMTP_PASSWORD)
    }

    pub fn status(&self, definition: &SecretDefinition) -> SecretStatus {
        let value = self.get(definition);
        SecretStatus {
            name: definition.name,
            configured: value.is_set(),
            source: value.source.name(),
            variable: definition.variable(),
            updated_at: self
                .panel_updated_at
                .iter()
                .find(|(name, _)| *name == definition.name)
                .map(|(_, at)| *at),
        }
    }

    /// The status of every secret, in the panel's order.
    pub fn statuses(&self) -> Vec<SecretStatus> {
        ALL.iter()
            .map(|definition| self.status(definition))
            .collect()
    }
}

impl fmt::Debug for ResolvedSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.values.iter().map(|(name, value)| (name, value.source)))
            .finish()
    }
}

/// Resolves every secret: the panel's value where there is one that opens,
/// else the environment's.
pub fn read(
    connection: &Connection,
    environment: &EnvironmentSecrets,
    sealer: &Sealer,
) -> rusqlite::Result<ResolvedSecrets> {
    let mut statement = connection
        .prepare("SELECT \"Name\", \"ProtectedValue\", \"UpdatedAt\" FROM \"InstanceSecrets\"")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Timestamp>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let now = Timestamp::now();
    let mut values = Vec::new();
    let mut panel_updated_at = Vec::new();
    for definition in &ALL {
        let mut panel = None;
        if let Some((name, sealed, updated)) =
            rows.iter().find(|(name, _, _)| name == definition.name)
        {
            let opened = open(sealer, name, sealed, now)?;
            if !opened.is_empty() {
                panel = Some((opened, *updated));
            }
        }
        if let Some((value, updated)) = panel {
            values.push((
                definition.name,
                SecretValue {
                    value,
                    source: Source::Panel,
                },
            ));
            panel_updated_at.push((definition.name, updated));
            continue;
        }
        let from_environment = environment.get(definition.setting);
        let from_environment = if definition.trim {
            from_environment.trim()
        } else {
            from_environment
        };
        values.push((
            definition.name,
            if from_environment.is_empty() {
                MISSING.clone()
            } else {
                SecretValue {
                    value: from_environment.to_string(),
                    source: Source::Environment,
                }
            },
        ));
    }
    Ok(ResolvedSecrets {
        values,
        panel_updated_at,
    })
}

/// The plain value; a row that cannot be opened (a master key that is not the
/// one it was sealed under, or an altered row) is an error.
fn open(sealer: &Sealer, name: &str, sealed: &str, now: Timestamp) -> rusqlite::Result<String> {
    let bytes = sealer.open(PURPOSE, name.as_bytes(), sealed, now)?;
    String::from_utf8(bytes).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

/// Stores `value` for `definition`. The caller has checked [`is_valid_value`]
/// and commits.
pub fn set(
    connection: &Connection,
    sealer: &Sealer,
    definition: &SecretDefinition,
    value: &str,
) -> rusqlite::Result<()> {
    let stored = if definition.trim { value.trim() } else { value };
    let sealed = sealer.seal(PURPOSE, definition.name.as_bytes(), None, stored.as_bytes());
    connection.execute(
        "INSERT INTO \"InstanceSecrets\" (\"Name\", \"ProtectedValue\", \"UpdatedAt\") VALUES (?1, ?2, ?3)
         ON CONFLICT (\"Name\") DO UPDATE SET \"ProtectedValue\" = excluded.\"ProtectedValue\", \"UpdatedAt\" = excluded.\"UpdatedAt\"",
        params![definition.name, sealed, Timestamp::now()],
    )?;
    Ok(())
}

/// Removes the panel's value, so the environment's stands again. The caller
/// commits.
pub fn clear(connection: &Connection, definition: &SecretDefinition) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM \"InstanceSecrets\" WHERE \"Name\" = ?1",
        [definition.name],
    )?;
    Ok(())
}
