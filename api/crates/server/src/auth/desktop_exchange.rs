//! The desktop application's sign-in through the browser, as the application
//! sees it: it holds the secret of a request that the person approved in
//! their browser, and exchanges it for a session of its own.
//!
//! The session carries what the approving browser's session had when it
//! approved, which the approval copied onto the request: when it passed a
//! second step, and by what, and how it first signed in. The caller cannot
//! give it any of these; they are taken from the row that was validated.

use rusqlite::{OptionalExtension, Transaction, params};

use super::events;
use super::flow::Context;
use super::policy::{SecurityPolicy, has_confirmed_method};
use super::session::{self, NewSession};
use super::tokens;
use crate::database::DbError;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

/// The shortest and the longest secret that is looked at, counted in UTF-16
/// units as the first server counted them.
const SECRET_UNITS: std::ops::RangeInclusive<usize> = 16..=512;

/// The session an exchange made.
pub struct DesktopSession {
    pub user: Id,
    pub session: Id,
    /// The `Set-Cookie` line of the session. It is not short-lived: the
    /// application keeps its cookies across restarts.
    pub cookie: String,
}

impl std::fmt::Debug for DesktopSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopSession")
            .field("user", &self.user)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// What an exchange came to.
#[must_use]
#[derive(Debug)]
pub enum Exchanged {
    /// Not approved yet (a 202 with `{"status":"pending"}`). Nothing was written.
    Pending,
    /// No such request, not for this secret, out of date or already used: one
    /// answer for all (a 400, `BrowserLoginLinkInvalid`).
    Invalid,
    /// The approval no longer stands: the account is disabled or waiting for
    /// recovery, the session that approved it ended, the account changed since,
    /// or the instance now asks for a second step the approval did not pass (a
    /// 403).
    Forbidden,
    SignedIn(Box<DesktopSession>),
}

struct Row {
    secret_hash: String,
    expires_at: Timestamp,
    redeemed: bool,
    user: Option<Id>,
    approved: bool,
    approved_session: Option<Id>,
    primary_source: Option<String>,
    mfa_completed_at: Option<Timestamp>,
    mfa_evidence: Option<String>,
    security_version: i64,
}

fn row_of(connection: &rusqlite::Connection, id: Id) -> rusqlite::Result<Option<Row>> {
    connection
        .query_row(
            "SELECT \"SecretHash\", \"ExpiresAt\", \"RedeemedAt\" IS NOT NULL, \"UserId\", \"ApprovedAt\" IS NOT NULL,
                    \"ApprovedSessionId\", \"PrimarySource\", \"MfaCompletedAt\", \"MfaEvidence\", \"SecurityVersion\"
             FROM \"DesktopBrowserLogins\" WHERE \"Id\" = ?1",
            [id],
            |row| {
                Ok(Row {
                    secret_hash: row.get(0)?,
                    expires_at: row.get(1)?,
                    redeemed: row.get(2)?,
                    user: row.get(3)?,
                    approved: row.get(4)?,
                    approved_session: row.get(5)?,
                    primary_source: row.get(6)?,
                    mfa_completed_at: row.get(7)?,
                    mfa_evidence: row.get(8)?,
                    security_version: row.get(9)?,
                })
            },
        )
        .optional()
}

/// Whether the row can still be exchanged by the one who holds the secret.
fn open(row: &Row, secret: &str, now: Timestamp) -> bool {
    row.expires_at > now && !row.redeemed && tokens::matches(secret, &row.secret_hash)
}

/// Exchanges the approved request for a session. `request_id` and `secret` are
/// what the application sent, as text. A request that is not approved yet is
/// answered from one read and writes nothing; an approved one is read again in
/// the transaction that redeems it and makes the session, so that two
/// exchanges of one request make one session.
pub async fn exchange_browser_login(
    state: &AppState,
    context: &Context<'_>,
    request_id: &str,
    secret: &str,
) -> Result<Exchanged, DbError> {
    let Some(id) = Id::parse(request_id) else {
        return Ok(Exchanged::Invalid);
    };
    if !SECRET_UNITS.contains(&secret.encode_utf16().count()) {
        return Ok(Exchanged::Invalid);
    }
    let owned_secret = secret.to_string();
    let first = state
        .database()
        .read(move |connection| -> Result<Option<bool>, DbError> {
            let Some(row) = row_of(connection, id)? else {
                return Ok(None);
            };
            Ok(open(&row, &owned_secret, Timestamp::now())
                .then_some(row.user.is_some() && row.approved))
        })
        .await?;
    match first {
        None => return Ok(Exchanged::Invalid),
        Some(false) => return Ok(Exchanged::Pending),
        Some(true) => {}
    }
    let state_in = state.clone();
    let (ip_address, user_agent, https) = (
        context.ip_address.to_string(),
        context.user_agent.to_string(),
        context.https,
    );
    let secret = secret.to_string();
    state
        .database()
        .write(move |tx| -> Result<Exchanged, DbError> {
            let context = Context {
                ip_address: &ip_address,
                user_agent: &user_agent,
                https,
                trusted_device_cookie: None,
                passkeys_available: false,
            };
            redeem(tx, &state_in, &context, id, &secret)
        })
        .await
}

fn redeem(
    tx: &Transaction,
    state: &AppState,
    context: &Context<'_>,
    id: Id,
    secret: &str,
) -> Result<Exchanged, DbError> {
    let now = Timestamp::now();
    let Some(row) = row_of(tx, id)? else {
        return Ok(Exchanged::Invalid);
    };
    if !open(&row, secret, now) {
        return Ok(Exchanged::Invalid);
    }
    let (Some(user_id), true) = (row.user, row.approved) else {
        return Ok(Exchanged::Pending);
    };
    let account: Option<(bool, bool, i64)> = tx
        .query_row(
            "SELECT \"IsDisabled\", \"SecurityRecoveryRequired\", \"SessionVersion\" FROM \"Users\" WHERE \"Id\" = ?1",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((false, false, version)) = account else {
        return Ok(Exchanged::Forbidden);
    };
    let source: Option<(Timestamp, bool)> = match row.approved_session {
        Some(source) => tx
            .query_row(
                "SELECT COALESCE(\"AuthenticatedAt\", \"CreatedAt\"),
                        \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?3 AND COALESCE(\"AbsoluteExpiresAt\" > ?3, 0) AND \"SecurityVersion\" = ?4
                 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2",
                params![source, user_id, now, version],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?,
        None => None,
    };
    let Some((authenticated_at, true)) = source else {
        return Ok(Exchanged::Forbidden);
    };
    if row.security_version != version {
        return Ok(Exchanged::Forbidden);
    }
    let policy = SecurityPolicy::load(tx)?;
    if (has_confirmed_method(tx, user_id)? || policy.is_required_at_login(tx, user_id)?)
        && !policy.is_satisfied(row.mfa_completed_at.is_some(), row.mfa_evidence.as_deref())
    {
        return Ok(Exchanged::Forbidden);
    }
    let redeemed = tx.execute(
        "UPDATE \"DesktopBrowserLogins\" SET \"RedeemedAt\" = ?2 WHERE \"Id\" = ?1 AND \"RedeemedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![id, now],
    )?;
    if redeemed != 1 {
        return Ok(Exchanged::Invalid);
    }
    let mut session = NewSession::new(
        user_id,
        context.device(state.messages())?,
        if context.ip_address.is_empty() {
            "unknown".to_string()
        } else {
            context.ip_address.to_string()
        },
        row.primary_source.as_deref().unwrap_or("desktop"),
        version,
        false,
    );
    session.authenticated_at = authenticated_at;
    session.mfa_completed_at = row.mfa_completed_at;
    session.mfa_evidence = row.mfa_evidence;
    session.insert(tx)?;
    let alert = events::verified_email(tx, user_id)?;
    events::record_from(
        tx,
        context,
        user_id,
        events::LOGIN_SUCCEEDED,
        "desktop",
        alert.as_deref(),
    )?;
    let cookie = session::sign_in_cookie(
        state.sealer(),
        &state.settings().environment,
        context.https,
        user_id,
        version,
        session.id,
        false,
    );
    Ok(Exchanged::SignedIn(Box::new(DesktopSession {
        user: user_id,
        session: session.id,
        cookie,
    })))
}
