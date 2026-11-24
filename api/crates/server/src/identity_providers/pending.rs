//! The identity a provider confirmed that no account is linked to, waiting
//! for the browser it came back to.
//!
//! It lasts ten minutes and one use. The row is bound to the browser by a
//! secret in an HttpOnly cookie, and it is deleted when it is taken or
//! dropped, so a copy of the cookie carried elsewhere opens nothing.

use std::time::Duration;

use axum::http::HeaderMap;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::plain_login::plain_login;
use crate::auth::{cookies, security_cookies, tokens};
use crate::database::DbError;
use crate::values::{Id, Timestamp};

pub const LIFETIME: Duration = Duration::from_secs(10 * 60);

pub fn cookie_name(https: bool) -> &'static str {
    if https {
        "__Host-stellarr-pending-identity"
    } else {
        "stellarr-dev-pending-identity"
    }
}

/// What the browser holds for the identity waiting for it.
pub type Held = (Id, String);

pub fn held(headers: &HeaderMap, https: bool) -> Option<Held> {
    security_cookies::parse_bound(&cookies::read(headers, cookie_name(https))?)
}

/// The line that takes the cookie of the waiting identity away.
pub fn clear_cookie(https: bool) -> String {
    cookies::delete(cookie_name(https), https)
}

/// What a provider said about a person, as it is kept.
#[derive(Debug, Clone)]
pub struct Pending {
    pub id: Id,
    pub provider_id: String,
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub preferred_username: Option<String>,
    pub amr: String,
    pub auth_time: Option<String>,
    pub short_lived: bool,
}

impl Pending {
    /// A login to offer for an account made from the identity: the provider's
    /// own login name when it makes one, else the name, else the part of the
    /// address before the `@`. Whether it is free is not asked, as that would
    /// tell anyone who has an account; a taken one is refused when sent.
    pub fn suggested_username(&self) -> String {
        let local = self
            .email
            .as_deref()
            .map(|email| email.split('@').next().unwrap_or(""));
        [
            self.preferred_username.as_deref(),
            self.display_name.as_deref(),
            local,
        ]
        .into_iter()
        .map(plain_login)
        .find(|login| login.len() >= 3)
        .unwrap_or_default()
    }
}

/// What [`keep`] gives the caller to answer with.
pub struct Kept {
    pub cookie: String,
}

/// Keeps the identity for this browser, in place of anything it waited with
/// before, and clears out identities nobody came back for.
pub fn keep(
    tx: &Transaction,
    identity: &Pending,
    previous: Option<Id>,
    https: bool,
) -> Result<Kept, DbError> {
    let now = Timestamp::now();
    tx.execute(
        "DELETE FROM \"PendingExternalIdentities\" WHERE \"ExpiresAt\" <= ?1 OR \"Id\" = ?2",
        params![now, previous],
    )?;
    let secret = tokens::random(32);
    let id = Id::random();
    let expires = now.plus(LIFETIME);
    tx.execute(
        "INSERT INTO \"PendingExternalIdentities\" (\"Id\", \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"Email\",
             \"EmailVerified\", \"DisplayName\", \"PreferredUsername\", \"Amr\", \"AuthTime\", \"ShortLived\", \"CreatedAt\", \"ExpiresAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            id,
            tokens::hash(&secret),
            identity.provider_id,
            identity.issuer,
            identity.subject,
            identity.email,
            identity.email.is_some() && identity.email_verified,
            identity.display_name,
            identity.preferred_username,
            identity.amr,
            identity.auth_time,
            identity.short_lived,
            now,
            expires
        ],
    )?;
    Ok(Kept {
        cookie: cookies::set(
            cookie_name(https),
            &security_cookies::bound_value(id, &secret),
            Some(expires),
            https,
        ),
    })
}

/// The identity this browser waits with, left in place; none when there is
/// none, it ran out, or the secret is not its own.
pub fn find(connection: &Connection, held: Option<&Held>) -> Result<Option<Pending>, DbError> {
    let Some((id, secret)) = held else {
        return Ok(None);
    };
    let found: Option<(String, Pending)> = connection
        .query_row(
            "SELECT \"SecretHash\", \"ProviderId\", \"Issuer\", \"Subject\", \"Email\", \"EmailVerified\", \"DisplayName\",
                    \"PreferredUsername\", \"Amr\", \"AuthTime\", \"ShortLived\"
             FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1 AND \"ExpiresAt\" > ?2",
            params![id, Timestamp::now()],
            |row| {
                Ok((
                    row.get(0)?,
                    Pending {
                        id: *id,
                        provider_id: row.get(1)?,
                        issuer: row.get(2)?,
                        subject: row.get(3)?,
                        email: row.get(4)?,
                        email_verified: row.get(5)?,
                        display_name: row.get(6)?,
                        preferred_username: row.get(7)?,
                        amr: row.get(8)?,
                        auth_time: row.get(9)?,
                        short_lived: row.get(10)?,
                    },
                ))
            },
        )
        .optional()?;
    Ok(found
        .filter(|(hash, _)| tokens::matches(secret, hash))
        .map(|(_, identity)| identity))
}

/// The identity this browser waits with, taken: only one request can take it.
/// With `expected`, only that identity is taken, as a sign-in that saw it asks.
pub fn take(
    tx: &Transaction,
    held: Option<&Held>,
    expected: Option<Id>,
) -> Result<Option<Pending>, DbError> {
    let Some(identity) = find(tx, held)? else {
        return Ok(None);
    };
    if expected.is_some_and(|expected| identity.id != expected) {
        return Ok(None);
    }
    let taken = tx.execute(
        "DELETE FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1",
        [identity.id],
    )?;
    Ok((taken == 1).then_some(identity))
}

/// Forgets the identity this browser waits with, if any.
pub fn forget(tx: &Transaction, held: Option<&Held>) -> Result<(), DbError> {
    if let Some((id, _)) = held
        && find(tx, held)?.is_some()
    {
        tx.execute(
            "DELETE FROM \"PendingExternalIdentities\" WHERE \"Id\" = ?1",
            [id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(preferred: Option<&str>, name: Option<&str>, email: Option<&str>) -> Pending {
        Pending {
            id: Id::random(),
            provider_id: "default".into(),
            issuer: "https://id.test".into(),
            subject: "s".into(),
            email: email.map(str::to_string),
            email_verified: false,
            display_name: name.map(str::to_string),
            preferred_username: preferred.map(str::to_string),
            amr: String::new(),
            auth_time: None,
            short_lived: false,
        }
    }

    #[test]
    fn the_login_to_suggest_is_the_first_of_three_that_is_long_enough() {
        let suggest = |p, n, e| identity(p, n, e).suggested_username();
        assert_eq!(suggest(Some("ola"), Some("Ola Łęcka"), Some("o@x.test")), "ola");
        assert_eq!(suggest(Some("ol"), Some("Ola Łęcka"), None), "ola.lecka");
        assert_eq!(suggest(Some("ol"), Some("Ol"), Some("maya.lin@x.test")), "maya.lin");
        assert_eq!(suggest(None, None, Some("ab@x.test")), "");
        assert_eq!(suggest(None, None, None), "");
    }
}
