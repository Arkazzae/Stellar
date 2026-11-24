//! Linking the identity a sign-in was asked to link, once the sign-in is
//! complete.
//!
//! A sign-in through a password can ask to link the identity a provider just
//! confirmed and no account had. The identity waits in a row bound to the
//! browser by a secret in a cookie; the sign-in is finished first and the link
//! comes after, inside the same transaction but apart from it, so that a link
//! that is refused never undoes a sign-in that was right.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::auth::ceremony::Completed;
use crate::auth::{cookies, events, security_cookies, tokens};
use crate::database::DbError;
use crate::http::Origin;
use crate::values::{Id, Timestamp};

/// The cookie that binds the waiting identity to the browser.
fn cookie_name(https: bool) -> &'static str {
    if https {
        "__Host-stellarr-pending-identity"
    } else {
        "stellarr-dev-pending-identity"
    }
}

/// The line that takes the cookie of the waiting identity away.
pub(super) fn clear_cookie(https: bool) -> String {
    cookies::delete(cookie_name(https), https)
}

/// How the link came out, in the words the web's link page reads.
pub(super) const LINKED: &str = "linked";
const TAKEN: &str = "taken";
const OTHER: &str = "other";
pub(super) const FAILED: &str = "failed";

/// What the browser holds for the identity waiting for it, and what the
/// request says about itself, for the event the link writes.
#[derive(Clone)]
pub(super) struct Waiting {
    cookie: Option<(Id, String)>,
    ip_address: String,
    user_agent: String,
}

impl Waiting {
    pub fn of(origin: &Origin) -> Waiting {
        Waiting {
            cookie: security_cookies::parse_bound(
                &cookies::read(&origin.headers, cookie_name(origin.https)).unwrap_or_default(),
            ),
            ip_address: origin.address.clone(),
            user_agent: origin.user_agent.clone(),
        }
    }
}

/// What the instance's providers are called by the people who see them, by
/// id. This build reads no provider from its settings yet (signing in through
/// one is another group's), so no waiting identity can name one that is
/// configured: a link is always refused here, as `failed`, and the identity
/// is used up.
fn provider_name(_provider: &str) -> Option<String> {
    None
}

/// The link that was asked for, as the caller's own writes of a completed
/// sign-in. Nothing at all for a sign-in that asked for none.
pub(super) fn pending(
    tx: &Transaction,
    completed: &Completed,
    waiting: &Waiting,
) -> Result<Option<&'static str>, DbError> {
    let Some(expected) = completed.pending_identity else {
        return Ok(None);
    };
    link(tx, completed.user, expected, waiting, &provider_name).map(Some)
}

/// Takes the identity waiting for this browser, if it is the one the sign-in
/// saw, and links it to the account if its provider is still configured.
fn link(
    tx: &Transaction,
    user: Id,
    expected: Id,
    waiting: &Waiting,
    provider_name: &dyn Fn(&str) -> Option<String>,
) -> Result<&'static str, DbError> {
    let now = Timestamp::now();
    let Some((id, secret)) = &waiting.cookie else {
        return Ok(FAILED);
    };
    let found: Option<(String, String, String, String)> = tx
        .query_row(
            "SELECT \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\" FROM \"PendingExternalIdentities\"
             WHERE \"Id\" = ?1 AND \"ExpiresAt\" > ?2",
            params![id, now],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((secret_hash, provider, issuer, subject)) = found else {
        return Ok(FAILED);
    };
    // Only one request can take it, and only the one the sign-in saw: one
    // that replaced it since is left where it is.
    if !tokens::matches(secret, &secret_hash) || *id != expected {
        return Ok(FAILED);
    }
    let taken = tx.execute(
        "DELETE FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1",
        [id],
    )?;
    let Some(name) = provider_name(&provider).filter(|_| taken == 1) else {
        return Ok(FAILED);
    };

    let mut statement = tx.prepare(
        "SELECT \"UserId\", \"Issuer\", \"Subject\" FROM \"ExternalIdentities\"
         WHERE (\"Issuer\" = ?1 AND \"Subject\" = ?2) OR (\"UserId\" = ?3 AND \"Issuer\" = ?1)",
    )?;
    let existing: Vec<(Id, String, String)> = statement
        .query_map(params![issuer, subject, user], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    if existing
        .iter()
        .any(|(owner, have_issuer, have_subject)| {
            *have_issuer == issuer && *have_subject == subject && *owner == user
        })
    {
        return Ok(LINKED);
    }
    if !existing.is_empty() {
        return Ok(if existing.iter().any(|(owner, _, _)| *owner != user) {
            TAKEN
        } else {
            OTHER
        });
    }
    let inserted = tx.execute(
        "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![Id::random(), user, issuer, subject, provider, now],
    );
    if let Err(error) = inserted {
        let error = DbError::Sqlite(error);
        // Linked by a request that won the race.
        if error.is_unique_violation() {
            return Ok(TAKEN);
        }
        return Err(error);
    }
    let alert = events::verified_email(tx, user)?;
    events::record(
        tx,
        user,
        events::IDENTITY_LINKED,
        &waiting.ip_address,
        &waiting.user_agent,
        &name,
        alert.as_deref(),
    )?;
    Ok(LINKED)
}
