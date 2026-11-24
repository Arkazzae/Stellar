//! The one place an identity at a provider is linked to an account, whichever
//! way the person got here: a link from the settings, a password sign-in that
//! asked to link the identity the provider just confirmed, or an account made
//! through the provider. The rules are the same everywhere: an identity
//! belongs to one account, and an account has one identity per issuer.

use rusqlite::{Connection, Transaction, params};
use stellarr_oidc::options::Provider;

use super::names;
use super::pending::{self, Held};
use crate::auth::events;
use crate::database::DbError;
use crate::values::{Id, Timestamp};

/// How a link came out, in the words the web's link page reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linked {
    Linked,
    /// The identity belongs to another account.
    Taken,
    /// The account already has another identity at this provider.
    Other,
    /// Nothing could be linked: the round trip, the session or the waiting
    /// identity was gone.
    Failed,
}

impl Linked {
    pub fn word(self) -> &'static str {
        match self {
            Linked::Linked => "linked",
            Linked::Taken => "taken",
            Linked::Other => "other",
            Linked::Failed => "failed",
        }
    }
}

/// What a request says about itself, for the event a link writes.
#[derive(Debug, Clone)]
pub struct Requester {
    pub ip_address: String,
    pub user_agent: String,
}

/// Links the identity and writes it down, or says why not. A link that loses
/// a race to the same identity reads as taken.
pub fn link(
    tx: &Transaction,
    requester: &Requester,
    user: Id,
    provider: &Provider,
    issuer: &str,
    subject: &str,
) -> Result<Linked, DbError> {
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
        .any(|(owner, i, s)| i == issuer && s == subject && *owner == user)
    {
        return Ok(Linked::Linked);
    }
    if !existing.is_empty() {
        return Ok(if existing.iter().any(|(owner, ..)| *owner != user) {
            Linked::Taken
        } else {
            Linked::Other
        });
    }
    match stage(tx, requester, user, provider, issuer, subject, true) {
        Ok(()) => Ok(Linked::Linked),
        // Linked by a request that won the race.
        Err(error) if error.is_unique_violation() => Ok(Linked::Taken),
        Err(error) => Err(error),
    }
}

/// Adds the link and its event: the account may have been made a moment ago
/// and told nothing by mail (`alert` off).
pub fn stage(
    tx: &Transaction,
    requester: &Requester,
    user: Id,
    provider: &Provider,
    issuer: &str,
    subject: &str,
    alert: bool,
) -> Result<(), DbError> {
    let now = Timestamp::now();
    tx.execute(
        "INSERT INTO \"ExternalIdentities\" (\"Id\", \"UserId\", \"Issuer\", \"Subject\", \"ProviderId\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![Id::random(), user, issuer, subject, provider.id, now],
    )?;
    let email = if alert {
        events::verified_email(tx, user)?
    } else {
        None
    };
    events::record(
        tx,
        user,
        events::IDENTITY_LINKED,
        &requester.ip_address,
        &requester.user_agent,
        &names::recorded_name(provider),
        email.as_deref(),
    )?;
    Ok(())
}

/// Links the identity waiting for this browser to the account that has just
/// signed in, as the sign-in asked. It runs once the session exists, so a
/// refusal never undoes the sign-in; the waiting identity is used up either
/// way. `saw` is the identity the sign-in saw: one that replaced it since is
/// not linked in its place.
pub fn link_pending(
    tx: &Transaction,
    requester: &Requester,
    user: Id,
    saw: Option<Id>,
    held: Option<&Held>,
    configured: impl Fn(&str) -> Option<Provider>,
) -> Result<Linked, DbError> {
    let Some(saw) = saw else {
        return Ok(Linked::Failed);
    };
    let Some(waiting) = pending::take(tx, held, Some(saw))? else {
        return Ok(Linked::Failed);
    };
    let Some(provider) = configured(&waiting.provider_id) else {
        return Ok(Linked::Failed);
    };
    link(tx, requester, user, &provider, &waiting.issuer, &waiting.subject)
}

/// The ids of the providers that may create accounts, as the owner set it in
/// the panel. Read when it is needed, never kept, so turning it off stops the
/// next sign-up at once.
pub fn sign_up_allowed(connection: &Connection, provider: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"IdentityProviderSettings\" WHERE \"ProviderId\" = ?1 AND \"SignUpAllowed\" = 1)",
        [provider],
        |row| row.get(0),
    )
}

pub fn providers_allowing(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT \"ProviderId\" FROM \"IdentityProviderSettings\" WHERE \"SignUpAllowed\" = 1")?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()
}
