//! Sessions: what a signed-in person's cookie names, and what is checked on
//! every request that carries it.
//!
//! The cookie holds a sealed ticket naming the account, the account's session
//! version and the session row. The row, in `UserSessions`, is what decides:
//! a cookie alone is never trusted, so ending a session in the database ends
//! it everywhere at once.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::cookies;
use super::policy::SecurityPolicy;
use crate::database::DbError;
use crate::seal::Sealer;
use crate::settings::Environment;
use crate::values::{Id, Timestamp};

const PURPOSE: &str = "session-cookie";
/// How long a kept sign-in lasts, and how long the cookie of one does.
pub const LIFETIME: Duration = Duration::from_secs(30 * 24 * 3600);
/// The hard end of a sign-in that did not ask to stay signed in.
pub const SHORT_LIFETIME: Duration = Duration::from_secs(12 * 3600);
/// A session seen less than this ago is not written to again.
const SEEN_GRANULARITY: Duration = Duration::from_secs(5 * 60);
/// A session whose end is nearer than this is renewed when it is seen.
const RENEWAL_THRESHOLD: Duration = Duration::from_secs(15 * 24 * 3600);

/// The person a request is from, when its cookie names a session that is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub user: Id,
    pub session: Id,
}

/// The cookie's name and whether it is marked secure. In development and
/// testing, served over plain HTTP, the cookie has a name of its own and is
/// secure only on a request that came over HTTPS; elsewhere it is a `__Host-`
/// cookie, which a browser takes only when it is secure.
pub fn cookie_name(environment: &Environment) -> &'static str {
    if environment.allows_plain_http() {
        "stellarr-dev-session"
    } else {
        "__Host-stellarr-session"
    }
}

pub fn cookie_is_secure(environment: &Environment, request_is_https: bool) -> bool {
    !environment.allows_plain_http() || request_is_https
}

/// What a session cookie says, once opened.
#[derive(Serialize, Deserialize)]
pub struct Ticket {
    user: Id,
    version: i64,
    session: Id,
    issued: i64,
    expires: i64,
    persistent: bool,
}

impl Ticket {
    /// The session version the ticket was made for.
    pub fn version(&self) -> i64 {
        self.version
    }

    /// The ticket of a connection that is already open: the account, the
    /// version and the session it was admitted with. It has no life of its
    /// own, so checking it against the database can only fail because of what
    /// the row says, never because a cookie ran out.
    pub fn of_connection(user: Id, version: i64, session: Id) -> Ticket {
        let now = Timestamp::now().unix_seconds();
        Ticket {
            user,
            version,
            session,
            issued: now,
            expires: now + LIFETIME.as_secs() as i64,
            persistent: false,
        }
    }

    fn seal(&self, sealer: &Sealer) -> String {
        let payload = serde_json::to_vec(self).expect("a ticket is written as JSON");
        sealer.seal(
            PURPOSE,
            b"",
            Timestamp::from_unix_seconds(self.expires),
            &payload,
        )
    }

    /// Opens the cookie. Whatever is not a ticket this server sealed, or has
    /// run out, is none, and costs the database nothing.
    pub fn open(sealer: &Sealer, value: &str, now: Timestamp) -> Option<Ticket> {
        let payload = sealer.open(PURPOSE, b"", value, now).ok()?;
        serde_json::from_slice(&payload).ok()
    }
}

/// A row of `UserSessions`, as a sign-in makes it.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: Id,
    pub user: Id,
    pub device: String,
    pub ip_address: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub primary_source: String,
    pub authenticated_at: Timestamp,
    /// Only the second step's own calls say that a session passed one.
    pub(super) mfa_completed_at: Option<Timestamp>,
    pub(super) mfa_evidence: Option<String>,
    pub security_version: i64,
    pub short_lived: bool,
}

impl NewSession {
    /// A session for a sign-in arriving now. `device` is what names the
    /// browser, already shortened to fit.
    pub fn new(
        user: Id,
        device: String,
        ip_address: String,
        primary_source: &str,
        security_version: i64,
        short_lived: bool,
    ) -> NewSession {
        let now = Timestamp::now();
        NewSession {
            id: Id::random(),
            user,
            device,
            ip_address,
            created_at: now,
            expires_at: now.plus(if short_lived {
                SHORT_LIFETIME
            } else {
                LIFETIME
            }),
            primary_source: primary_source.to_string(),
            authenticated_at: now,
            mfa_completed_at: None,
            mfa_evidence: None,
            security_version,
            short_lived,
        }
    }

    pub fn insert(&self, tx: &Transaction) -> Result<(), DbError> {
        tx.execute(
            "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
                 \"AbsoluteExpiresAt\", \"PrimarySource\", \"AuthenticatedAt\", \"MfaCompletedAt\", \"MfaEvidence\",
                 \"SecurityVersion\", \"ShortLived\", \"RevokedAt\")
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)",
            params![
                self.id,
                self.user,
                self.device,
                self.ip_address,
                self.created_at,
                self.expires_at,
                self.primary_source,
                self.authenticated_at,
                self.mfa_completed_at,
                self.mfa_evidence,
                self.security_version,
                self.short_lived
            ],
        )?;
        Ok(())
    }
}

/// The `Set-Cookie` line that signs the person in: the cookie of a kept
/// sign-in outlives the browser, for thirty days, and its own expiry moves out
/// again when it is past half its life; the one of a short-lived sign-in is
/// dropped when the browser closes, and its session ends after twelve hours
/// either way. How long the session lasts is the row's business, not the
/// cookie's.
pub fn sign_in_cookie(
    sealer: &Sealer,
    environment: &Environment,
    request_is_https: bool,
    user: Id,
    security_version: i64,
    session: Id,
    short_lived: bool,
) -> String {
    let now = Timestamp::now();
    issue(
        sealer,
        environment,
        request_is_https,
        user,
        security_version,
        session,
        now,
        short_lived,
    )
}

#[allow(clippy::too_many_arguments)]
fn issue(
    sealer: &Sealer,
    environment: &Environment,
    request_is_https: bool,
    user: Id,
    version: i64,
    session: Id,
    issued: Timestamp,
    short_lived: bool,
) -> String {
    let lifetime = if short_lived {
        SHORT_LIFETIME
    } else {
        LIFETIME
    };
    renewed(
        sealer,
        environment,
        request_is_https,
        user,
        version,
        session,
        issued,
        issued.plus(lifetime),
        !short_lived,
    )
}

#[allow(clippy::too_many_arguments)]
fn renewed(
    sealer: &Sealer,
    environment: &Environment,
    request_is_https: bool,
    user: Id,
    version: i64,
    session: Id,
    issued: Timestamp,
    expires: Timestamp,
    persistent: bool,
) -> String {
    let ticket = Ticket {
        user,
        version,
        session,
        issued: issued.unix_seconds(),
        expires: expires.unix_seconds(),
        persistent,
    };
    cookies::set(
        cookie_name(environment),
        &ticket.seal(sealer),
        persistent.then_some(expires),
        cookie_is_secure(environment, request_is_https),
    )
}

/// The `Set-Cookie` line that takes the cookie away.
pub fn sign_out_cookie(environment: &Environment, request_is_https: bool) -> String {
    cookies::delete(
        cookie_name(environment),
        cookie_is_secure(environment, request_is_https),
    )
}

/// What the cookie of a request amounts to.
#[derive(Debug, PartialEq, Eq)]
pub enum Authentication {
    /// No cookie, or one that is not ours or has run out: nobody is signed in.
    Anonymous,
    /// A cookie for a session that is no longer valid, or an account that no
    /// longer allows it: nobody is signed in, and the cookie is taken away.
    Rejected,
    /// A valid session. `renewed_cookie` is the cookie to send in place of the
    /// one the request had, when it is past half its life.
    SignedIn {
        caller: Caller,
        renewed_cookie: Option<String>,
        touch: Option<Touch>,
    },
}

/// What a valid session is to be written back with: when it was seen, and
/// when it now ends.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Touch {
    session: Id,
    seen_at: Timestamp,
    expires_at: Timestamp,
}

impl Touch {
    pub fn write(&self, tx: &Transaction) -> Result<(), DbError> {
        tx.execute(
            "UPDATE \"UserSessions\" SET \"LastSeenAt\" = ?2, \"ExpiresAt\" = ?3 WHERE \"Id\" = ?1",
            params![self.session, self.seen_at, self.expires_at],
        )?;
        Ok(())
    }
}

/// Checks the cookie of a request against the database, as of `now`. A valid
/// session seen more than five minutes ago comes back with a [`Touch`] to
/// write: it is marked seen, and its end moved out to a full lifetime when it
/// is near. Reading and writing are apart so that a request that needs no
/// write never takes the write lock.
pub fn authenticate(
    connection: &Connection,
    sealer: &Sealer,
    environment: &Environment,
    request_is_https: bool,
    ticket: Ticket,
    now: Timestamp,
) -> Result<Authentication, DbError> {
    let Some(row) = connection
        .query_row(
            "SELECT s.\"LastSeenAt\", s.\"ExpiresAt\", s.\"AbsoluteExpiresAt\", s.\"RevokedAt\", s.\"SecurityVersion\",
                    s.\"MfaCompletedAt\", s.\"MfaEvidence\", u.\"SessionVersion\", u.\"IsDisabled\"
             FROM \"UserSessions\" s JOIN \"Users\" u ON u.\"Id\" = s.\"UserId\"
             WHERE s.\"Id\" = ?1 AND s.\"UserId\" = ?2",
            params![ticket.session, ticket.user],
            |row| {
                Ok(Row {
                    last_seen_at: row.get(0)?,
                    expires_at: row.get(1)?,
                    absolute_expires_at: row.get(2)?,
                    revoked_at: row.get(3)?,
                    security_version: row.get(4)?,
                    mfa_completed: row.get::<_, Option<Timestamp>>(5)?.is_some(),
                    mfa_evidence: row.get(6)?,
                    user_session_version: row.get(7)?,
                    user_is_disabled: row.get(8)?,
                })
            },
        )
        .optional()?
    else {
        return Ok(Authentication::Rejected);
    };
    let Some(absolute) = row.absolute_expires_at else {
        return Ok(Authentication::Rejected);
    };
    if row.user_is_disabled
        || row.security_version != ticket.version
        || row.user_session_version != ticket.version
        || row.revoked_at.is_some()
        || row.expires_at <= now
        || absolute <= now
    {
        return Ok(Authentication::Rejected);
    }
    let policy = SecurityPolicy::load(connection)?;
    if policy.is_required_at_login(connection, ticket.user)?
        && !policy.is_satisfied(row.mfa_completed, row.mfa_evidence.as_deref())
    {
        return Ok(Authentication::Rejected);
    }

    // Writing on every request would make each one a transaction; a session
    // seen minutes ago is fresh enough.
    let touch = (row.last_seen_at < now.minus(SEEN_GRANULARITY)).then(|| Touch {
        session: ticket.session,
        seen_at: now,
        expires_at: if row.expires_at < now.plus(RENEWAL_THRESHOLD) {
            now.plus(LIFETIME).min(absolute)
        } else {
            row.expires_at
        },
    });

    // The cookie itself renews once less of its life is left than has gone.
    let (issued, expires) = (ticket.issued, ticket.expires);
    let renewed_cookie = (expires - now.unix_seconds() < now.unix_seconds() - issued).then(|| {
        let length = u64::try_from(expires - issued).unwrap_or(0);
        renewed(
            sealer,
            environment,
            request_is_https,
            ticket.user,
            ticket.version,
            ticket.session,
            now,
            now.plus(Duration::from_secs(length)),
            ticket.persistent,
        )
    });
    Ok(Authentication::SignedIn {
        caller: Caller {
            user: ticket.user,
            session: ticket.session,
        },
        renewed_cookie,
        touch,
    })
}

struct Row {
    last_seen_at: Timestamp,
    expires_at: Timestamp,
    absolute_expires_at: Option<Timestamp>,
    revoked_at: Option<Timestamp>,
    security_version: i64,
    mfa_completed: bool,
    mfa_evidence: Option<String>,
    user_session_version: i64,
    user_is_disabled: bool,
}

/// Ends a session. True when it was running and now is not, so that the one
/// request that ends it is the one that records it.
pub fn revoke(tx: &Transaction, session: Id, now: Timestamp) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1 AND \"RevokedAt\" IS NULL",
        params![session, now],
    )?;
    Ok(changed == 1)
}
