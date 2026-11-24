//! What happens after every proof of who somebody is: a password, a
//! registration, a provider or a recovery. It either gives them an ordinary
//! session or a challenge that is bound to their browser, never both, and an
//! account that must complete a second step, or enrol one, never gets a
//! session without it. The routes that complete a challenge belong to a later
//! group of endpoints; the answer to a sign-in that needs one is made here.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use super::cookies;
use super::policy::SecurityPolicy;
use super::security_cookies;
use super::session::NewSession;
use super::tokens;
use super::trusted_devices;
use crate::database::DbError;
use crate::instance::users::User;
use crate::values::{Id, Timestamp};

/// The challenge that a person waits on while they prove a second step.
pub const PURPOSE_MFA: &str = "mfa";
/// The challenge that only lets a person enrol a second step.
pub const PURPOSE_ENROLLMENT: &str = "enrollment";
/// The challenge of a session that is already running and is asked for a fresh
/// second step.
pub const PURPOSE_REVERIFY: &str = "reverify";
/// The challenge an operator's recovery credential is exchanged for.
pub const PURPOSE_RECOVERY: &str = "recovery";
/// Evidence of a second step that the identity provider asked for.
pub const PROVIDER_EVIDENCE: &str = "provider_mfa";

/// What the request the person signs in with says about it.
#[derive(Clone)]
pub struct Context<'a> {
    pub ip_address: &'a str,
    /// The browser's own name for itself, trimmed.
    pub user_agent: &'a str,
    /// Whether the request came over HTTPS, which names the cookies it sets.
    pub https: bool,
    pub trusted_device_cookie: Option<&'a str>,
    /// Whether the instance can offer passkeys.
    pub passkeys_available: bool,
}

/// Nothing but where it came from: the cookie of a remembered device is a
/// secret.
impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("ip_address", &self.ip_address)
            .field("https", &self.https)
            .finish_non_exhaustive()
    }
}

impl Context<'_> {
    /// The text a session is named by in the list of a person's devices.
    pub fn device(&self, messages: &Messages) -> Result<String, DbError> {
        if self.user_agent.is_empty() {
            return messages
                .get(MessageKey::UnknownDevice, &[])
                .map_err(|e| DbError::Task(e.to_string()));
        }
        Ok(self.user_agent.chars().take(300).collect())
    }
}

pub enum Start {
    /// The account must be recovered at the console before anybody signs in.
    RecoveryRequired,
    /// An ordinary session, to be written and signed in.
    Session(NewSession),
    /// A challenge: what to tell the client, and the cookie that binds it to
    /// this browser.
    Challenge { body: Challenge, cookie: String },
}

/// Which way it went, and nothing of the cookie that binds a challenge.
impl std::fmt::Debug for Start {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Start::RecoveryRequired => "Start::RecoveryRequired",
            Start::Session(_) => "Start::Session(..)",
            Start::Challenge { .. } => "Start::Challenge { .. }",
        })
    }
}

/// Options of a start that only some ways in use.
#[derive(Debug, Default, Clone, Copy)]
pub struct Options {
    /// Overrides what the policy says about whether a second step is needed.
    pub required_override: Option<bool>,
    pub short_lived: bool,
    /// The identity provider already asked for a second step and the instance
    /// trusts it.
    pub second_step_at_provider: bool,
    pub pending_identity: Option<Id>,
    /// The session the challenge was asked from, which has to be still good
    /// when the challenge is completed.
    pub parent_session: Option<Id>,
}

/// Decides what a proof of identity leads to, and writes the challenge when it
/// leads to one.
pub fn start(
    tx: &Transaction,
    context: &Context<'_>,
    messages: &Messages,
    user: &User,
    primary_source: &str,
    policy: &SecurityPolicy,
    options: Options,
) -> Result<Start, DbError> {
    let methods = confirmed_methods(tx, user.id)?;
    if user.security_recovery_required {
        return Ok(Start::RecoveryRequired);
    }
    let device = context.device(messages)?;
    let session = |primary: &str| {
        NewSession::new(
            user.id,
            device.clone(),
            context.ip_address.to_string(),
            primary,
            user.session_version,
            options.short_lived,
        )
    };
    if options.second_step_at_provider {
        let mut session = session(primary_source);
        session.mfa_completed_at = Some(session.created_at);
        session.mfa_evidence = Some(PROVIDER_EVIDENCE.to_string());
        return Ok(Start::Session(session));
    }
    let required = match options.required_override {
        Some(required) => required,
        None => policy.is_required_at_login(tx, user.id)?,
    };
    if methods.is_empty() && !required {
        return Ok(Start::Session(session(primary_source)));
    }
    let purpose = if methods.is_empty() {
        PURPOSE_ENROLLMENT
    } else {
        PURPOSE_MFA
    };
    let (body, cookie) =
        start_challenge(tx, context, user, primary_source, purpose, policy, options)?;
    Ok(Start::Challenge { body, cookie })
}

/// Writes a challenge of the purpose given for the account, and says what to
/// tell the client and which cookie binds it to this browser.
pub(super) fn start_challenge(
    tx: &Transaction,
    context: &Context<'_>,
    user: &User,
    primary_source: &str,
    purpose: &str,
    policy: &SecurityPolicy,
    options: Options,
) -> Result<(Challenge, String), DbError> {
    let methods = confirmed_methods(tx, user.id)?;
    let (challenge, secret) = create_challenge(tx, user, primary_source, purpose, policy, options)?;
    let body = describe(tx, context, user, &challenge, methods, policy)?;
    let cookie = preauth_cookie(context.https, challenge.id, &secret, challenge.expires_at);
    Ok((body, cookie))
}

pub(super) fn preauth_name(https: bool) -> &'static str {
    if https {
        "__Host-stellarr-preauth"
    } else {
        "stellarr-dev-preauth"
    }
}

/// The `Set-Cookie` line that takes the challenge's cookie away.
pub fn clear_preauth_cookie(https: bool) -> String {
    cookies::delete(preauth_name(https), https)
}

pub(super) fn preauth_cookie(https: bool, id: Id, secret: &str, expires: Timestamp) -> String {
    cookies::set(
        preauth_name(https),
        &security_cookies::bound_value(id, secret),
        Some(expires),
        https,
    )
}

pub(super) struct Created {
    id: Id,
    purpose: String,
    expires_at: Timestamp,
    pending_identity: Option<Id>,
}

impl Created {
    /// A challenge that was made earlier, to be described again.
    pub(super) fn of(challenge: &StoredChallenge) -> Created {
        Created {
            id: challenge.id,
            purpose: challenge.purpose.clone(),
            expires_at: challenge.expires_at,
            pending_identity: challenge.pending_identity,
        }
    }
}

pub(super) fn create_challenge(
    tx: &Transaction,
    user: &User,
    primary_source: &str,
    purpose: &str,
    policy: &SecurityPolicy,
    options: Options,
) -> Result<(Created, String), DbError> {
    let now = Timestamp::now();
    let secret = tokens::random(32);
    let id = Id::random();
    let expires_at = now.plus(policy.challenge_lifetime());
    tx.execute(
        "INSERT INTO \"AuthChallenges\" (\"Id\", \"UserId\", \"Purpose\", \"SecretHash\", \"PrimarySource\", \"ParentSessionId\",
             \"SecurityVersion\", \"ShortLived\", \"PendingIdentityId\", \"WebAuthnOptions\", \"Attempts\", \"MaxAttempts\",
             \"CreatedAt\", \"ExpiresAt\", \"ConsumedAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?11, ?6, ?7, ?8, NULL, 0, 5, ?9, ?10, NULL)",
        params![
            id,
            user.id,
            purpose,
            tokens::hash(&secret),
            primary_source,
            user.session_version,
            options.short_lived,
            options.pending_identity,
            now,
            expires_at,
            options.parent_session
        ],
    )?;
    // Challenges of the account that ran out unused are of no use to anybody.
    tx.execute(
        "DELETE FROM \"AuthChallenges\" WHERE \"UserId\" = ?1 AND \"ConsumedAt\" IS NULL AND \"ExpiresAt\" <= ?2",
        params![user.id, now],
    )?;
    Ok((
        Created {
            id,
            purpose: purpose.to_string(),
            expires_at,
            pending_identity: options.pending_identity,
        },
        secret,
    ))
}

/// A second-step method, as a client lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Method {
    id: Id,
    r#type: String,
    name: String,
    created_at: Timestamp,
    last_used_at: Option<Timestamp>,
    email_address: Option<String>,
    pending: bool,
}

/// The answer to a sign-in that needs a second step or an enrolment first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    status: &'static str,
    challenge_id: Id,
    purpose: String,
    expires_at: Timestamp,
    methods: Vec<Method>,
    trusted_device_available: bool,
    email_code_available: bool,
    account: ChallengeAccount,
    trusted_device_days: i64,
    passkey_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChallengeAccount {
    display_name: String,
    username: String,
    avatar_color: String,
    avatar_url: Option<String>,
}

pub(super) fn confirmed_methods(connection: &Connection, user: Id) -> Result<Vec<Method>, DbError> {
    let mut statement = connection.prepare(
        "SELECT \"Id\", \"Type\", \"Name\", \"CreatedAt\", \"LastUsedAt\", \"EmailAddress\", \"ConfirmedAt\" FROM \"MfaMethods\"
         WHERE \"UserId\" = ?1 AND \"ConfirmedAt\" IS NOT NULL AND \"RevokedAt\" IS NULL ORDER BY \"CreatedAt\"",
    )?;
    let methods = statement
        .query_map([user], |row| {
            let kind: String = row.get(1)?;
            let address: Option<String> = row.get(5)?;
            Ok(Method {
                id: row.get(0)?,
                email_address: if kind == "email" { address } else { None },
                r#type: kind,
                name: row.get(2)?,
                created_at: row.get(3)?,
                last_used_at: row.get(4)?,
                pending: row.get::<_, Option<Timestamp>>(6)?.is_none(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(methods)
}

pub(super) fn describe(
    connection: &Connection,
    context: &Context<'_>,
    user: &User,
    challenge: &Created,
    methods: Vec<Method>,
    policy: &SecurityPolicy,
) -> Result<Challenge, DbError> {
    let reverify = challenge.purpose == PURPOSE_REVERIFY;
    let status = if challenge.purpose == PURPOSE_MFA || reverify {
        "mfa_required"
    } else {
        "mfa_enrollment_required"
    };
    // A remembered device skips the step but proves nothing now, so a fresh
    // proof is never met by one, and a link of an identity asks for the proof
    // a link from the settings would.
    let trusted = !reverify
        && challenge.pending_identity.is_none()
        && trusted_device(connection, user.id, context.trusted_device_cookie)?;
    Ok(Challenge {
        status,
        challenge_id: challenge.id,
        purpose: challenge.purpose.clone(),
        expires_at: challenge.expires_at,
        trusted_device_available: trusted,
        email_code_available: methods.iter().any(|method| method.r#type == "email"),
        methods,
        account: ChallengeAccount {
            display_name: user.display_name.clone(),
            username: user.username.clone(),
            avatar_color: user.avatar_color.clone(),
            avatar_url: user.avatar_url.clone(),
        },
        trusted_device_days: policy.trusted_device_days_clamped(),
        passkey_available: context.passkeys_available,
    })
}

/// Whether the browser carries a remembered device of the account that is
/// still good.
fn trusted_device(
    connection: &Connection,
    user: Id,
    cookie: Option<&str>,
) -> Result<bool, DbError> {
    Ok(trusted_devices::find(connection, user, cookie, Timestamp::now())?.is_some())
}

/// Whether the instance can offer passkeys: its public address is HTTPS (or
/// HTTP on localhost) and names a host a browser reads as a name, not as an
/// address. It is read from the settings, never from a request, so a forged
/// `Host` header cannot move it. The routes that make a passkey work build the
/// relying party from the same address in the same way, so what a challenge
/// says about passkeys is never more than they can do.
pub fn passkeys_available(public_url: &str) -> bool {
    stellarr_webauthn::RelyingParty::from_public_url(public_url).is_some()
}

/// What looking a challenge up came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeStatus {
    Valid,
    /// There is no such challenge for this browser: no cookie, another
    /// challenge's, a wrong secret or another purpose.
    Missing,
    Expired,
    Consumed,
    AttemptLimit,
    /// The account changed since: disabled, a new session version, or in need
    /// of recovery (and the challenge not for it), or the session it was asked
    /// from is gone.
    SecurityChanged,
}

/// A challenge row.
#[derive(Debug, Clone)]
pub struct StoredChallenge {
    pub id: Id,
    pub user: Id,
    pub purpose: String,
    pub primary_source: String,
    pub parent_session: Option<Id>,
    pub short_lived: bool,
    pub pending_identity: Option<Id>,
    pub attempts: i64,
    pub max_attempts: i64,
    pub expires_at: Timestamp,
}

/// The challenge a browser holds, from the cookie that binds it: its id and
/// its secret.
pub fn read_preauth(headers: &axum::http::HeaderMap, https: bool) -> Option<(Id, String)> {
    security_cookies::parse_bound(&cookies::read(headers, preauth_name(https))?)
}

/// Looks a challenge up for the browser that holds `cookie`, as of `now`, and
/// says whether it can still be used. `purpose` limits it to one kind.
pub(super) fn validate(
    connection: &Connection,
    cookie: Option<&(Id, String)>,
    challenge_id: &str,
    purpose: Option<&str>,
    now: Timestamp,
) -> Result<(Option<StoredChallenge>, ChallengeStatus), DbError> {
    use ChallengeStatus::*;
    let (Some(id), Some((cookie_id, secret))) = (crate::values::parse_guid(challenge_id), cookie)
    else {
        return Ok((None, Missing));
    };
    if *cookie_id != id {
        return Ok((None, Missing));
    }
    let found = connection
        .query_row(
            "SELECT c.\"UserId\", c.\"Purpose\", c.\"SecretHash\", c.\"PrimarySource\", c.\"ParentSessionId\", c.\"SecurityVersion\",
                    c.\"ShortLived\", c.\"PendingIdentityId\", c.\"Attempts\", c.\"MaxAttempts\", c.\"ExpiresAt\", c.\"ConsumedAt\",
                    u.\"IsDisabled\", u.\"SessionVersion\", u.\"SecurityRecoveryRequired\"
             FROM \"AuthChallenges\" c JOIN \"Users\" u ON u.\"Id\" = c.\"UserId\" WHERE c.\"Id\" = ?1",
            [id],
            |row| {
                Ok((
                    StoredChallenge {
                        id,
                        user: row.get(0)?,
                        purpose: row.get(1)?,
                        primary_source: row.get(3)?,
                        parent_session: row.get(4)?,
                        short_lived: row.get(6)?,
                        pending_identity: row.get(7)?,
                        attempts: row.get(8)?,
                        max_attempts: row.get(9)?,
                        expires_at: row.get(10)?,
                    },
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Timestamp>(10)?,
                    row.get::<_, Option<Timestamp>>(11)?,
                    (row.get::<_, bool>(12)?, row.get::<_, i64>(13)?, row.get::<_, bool>(14)?),
                ))
            },
        )
        .optional()?;
    let Some((
        challenge,
        secret_hash,
        version,
        expires_at,
        consumed_at,
        (disabled, user_version, recovery),
    )) = found
    else {
        return Ok((None, Missing));
    };
    if !tokens::matches(secret, &secret_hash)
        || purpose.is_some_and(|wanted| challenge.purpose != wanted)
    {
        return Ok((None, Missing));
    }
    if consumed_at.is_some() {
        return Ok((None, Consumed));
    }
    if expires_at <= now {
        return Ok((None, Expired));
    }
    if challenge.attempts >= challenge.max_attempts {
        return Ok((None, AttemptLimit));
    }
    let for_recovery = challenge.purpose == PURPOSE_RECOVERY;
    if disabled
        || user_version != version
        || (recovery && !for_recovery)
        || (for_recovery && !recovery)
    {
        return Ok((None, SecurityChanged));
    }
    if let Some(parent) = challenge.parent_session {
        let active: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM \"UserSessions\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL
                            AND \"ExpiresAt\" > ?3 AND \"AbsoluteExpiresAt\" > ?3 AND \"SecurityVersion\" = ?4)",
            params![parent, challenge.user, now, user_version],
            |row| row.get(0),
        )?;
        if !active {
            return Ok((None, SecurityChanged));
        }
    }
    Ok((Some(challenge), Valid))
}

/// Uses a challenge up. True for the one request that did, so that two at
/// once cannot both complete it.
pub(super) fn consume(tx: &Transaction, challenge: Id, now: Timestamp) -> Result<bool, DbError> {
    let changed = tx.execute(
        "UPDATE \"AuthChallenges\" SET \"ConsumedAt\" = ?2 WHERE \"Id\" = ?1 AND \"ConsumedAt\" IS NULL AND \"ExpiresAt\" > ?2",
        params![challenge, now],
    )?;
    Ok(changed == 1)
}

/// Spends one attempt of a challenge, and closes it at its budget.
pub(super) fn fail_attempt(tx: &Transaction, challenge: Id, now: Timestamp) -> Result<(), DbError> {
    tx.execute(
        "UPDATE \"AuthChallenges\" SET \"Attempts\" = \"Attempts\" + 1,
             \"ConsumedAt\" = CASE WHEN \"Attempts\" + 1 >= \"MaxAttempts\" THEN ?2 ELSE \"ConsumedAt\" END
         WHERE \"Id\" = ?1 AND \"ConsumedAt\" IS NULL",
        params![challenge, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkeys_need_a_secure_address_with_a_name() {
        for (url, available) in [
            ("https://chat.example.test", true),
            ("https://chat.example.test:8443/x", true),
            ("http://localhost:3000", true),
            ("http://chat.example.test", false),
            ("https://203.0.113.5", false),
            ("https://[::1]", false),
            // A host a browser reads as an address.
            ("https://0x7f.0x1", false),
            ("", false),
            ("chat.example.test", false),
        ] {
            assert_eq!(passkeys_available(url), available, "{url}");
        }
    }

    #[test]
    fn the_cookie_that_binds_a_challenge_names_it_without_dashes() {
        let id = Id::parse("049083c7-a28f-4b91-b21b-2a0920dc1686").unwrap();
        let expires = Timestamp::parse("2026-11-24 01:23:38").unwrap();

        assert_eq!(
            preauth_cookie(true, id, "secret", expires),
            "__Host-stellarr-preauth=049083c7a28f4b91b21b2a0920dc1686.secret; expires=Tue, 24 Nov 2026 01:23:38 GMT; path=/; secure; samesite=lax; httponly"
        );
        assert!(preauth_cookie(false, id, "s", expires).starts_with("stellarr-dev-preauth="));
    }

    use crate::testing::new_database;

    /// A challenge in a database of its own, written by `start`, with the
    /// cookie that binds it.
    fn challenge_in(database: &crate::database::Database) -> (Id, Id, (Id, String)) {
        let user = Id::random();
        let secret = tokens::random(32);
        let id = Id::random();
        let connection = rusqlite::Connection::open(database.path()).unwrap();
        let now = Timestamp::now();
        connection
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"PasswordHash\", \"IsDisabled\",
                     \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\")
                 VALUES (?1, ?1, ?1, 'U', NULL, 0, 3, 0, 'c', ?2)",
                params![user, now],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO \"AuthChallenges\" VALUES (?1, ?2, 'mfa', ?3, 'password', NULL, 3, 0, NULL, NULL, 0, 2, ?4, ?5, NULL)",
                params![id, user, tokens::hash(&secret), now, now.plus(std::time::Duration::from_secs(300))],
            )
            .unwrap();
        (id, user, (id, secret))
    }

    fn status(
        database: &crate::database::Database,
        cookie: Option<&(Id, String)>,
        id: &str,
        purpose: Option<&str>,
        now: Timestamp,
    ) -> ChallengeStatus {
        let connection = rusqlite::Connection::open(database.path()).unwrap();
        validate(&connection, cookie, id, purpose, now).unwrap().1
    }

    #[test]
    fn a_challenge_is_found_by_the_browser_it_was_made_for_and_by_nobody_else() {
        let (_folder, database) = new_database();
        let (id, _, cookie) = challenge_in(&database);
        let now = Timestamp::now();

        assert_eq!(
            status(&database, Some(&cookie), &id.to_string(), Some("mfa"), now),
            ChallengeStatus::Valid
        );
        assert_eq!(
            status(&database, Some(&cookie), &id.to_string(), None, now),
            ChallengeStatus::Valid
        );
        assert_eq!(
            status(&database, None, &id.to_string(), None, now),
            ChallengeStatus::Missing
        );
        assert_eq!(
            status(
                &database,
                Some(&(id, "wrong".into())),
                &id.to_string(),
                None,
                now
            ),
            ChallengeStatus::Missing
        );
        assert_eq!(
            status(
                &database,
                Some(&(Id::random(), cookie.1.clone())),
                &id.to_string(),
                None,
                now
            ),
            ChallengeStatus::Missing
        );
        assert_eq!(
            status(
                &database,
                Some(&cookie),
                &Id::random().to_string(),
                None,
                now
            ),
            ChallengeStatus::Missing
        );
        assert_eq!(
            status(&database, Some(&cookie), "not an id", None, now),
            ChallengeStatus::Missing
        );
        assert_eq!(
            status(
                &database,
                Some(&cookie),
                &id.to_string(),
                Some("enrollment"),
                now
            ),
            ChallengeStatus::Missing
        );
    }

    #[test]
    fn a_challenge_that_ran_out_was_used_or_has_no_attempts_left_says_which() {
        let (_folder, database) = new_database();
        let (id, _, cookie) = challenge_in(&database);
        let connection = rusqlite::Connection::open(database.path()).unwrap();
        let later = Timestamp::now().plus(std::time::Duration::from_secs(301));

        assert_eq!(
            status(&database, Some(&cookie), &id.to_string(), None, later),
            ChallengeStatus::Expired
        );
        connection
            .execute("UPDATE \"AuthChallenges\" SET \"Attempts\" = 2", [])
            .unwrap();
        assert_eq!(
            status(
                &database,
                Some(&cookie),
                &id.to_string(),
                None,
                Timestamp::now()
            ),
            ChallengeStatus::AttemptLimit
        );
        connection
            .execute(
                "UPDATE \"AuthChallenges\" SET \"ConsumedAt\" = ?1",
                [Timestamp::now()],
            )
            .unwrap();
        assert_eq!(
            status(
                &database,
                Some(&cookie),
                &id.to_string(),
                None,
                Timestamp::now()
            ),
            ChallengeStatus::Consumed
        );
    }

    #[test]
    fn a_challenge_does_not_outlive_a_change_of_the_account() {
        for change in [
            "UPDATE \"Users\" SET \"IsDisabled\" = 1",
            "UPDATE \"Users\" SET \"SessionVersion\" = 4",
            "UPDATE \"Users\" SET \"SecurityRecoveryRequired\" = 1",
        ] {
            let (_folder, database) = new_database();
            let (id, _, cookie) = challenge_in(&database);
            rusqlite::Connection::open(database.path())
                .unwrap()
                .execute(change, [])
                .unwrap();

            assert_eq!(
                status(
                    &database,
                    Some(&cookie),
                    &id.to_string(),
                    None,
                    Timestamp::now()
                ),
                ChallengeStatus::SecurityChanged,
                "{change}"
            );
        }
    }

    #[test]
    fn a_challenge_asked_from_a_session_needs_that_session_to_be_running() {
        let (_folder, database) = new_database();
        let (id, user, cookie) = challenge_in(&database);
        let connection = rusqlite::Connection::open(database.path()).unwrap();
        let parent = Id::random();
        let now = Timestamp::now();
        let end = now.plus(std::time::Duration::from_secs(3600));
        connection
            .execute(
                "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\", \"AbsoluteExpiresAt\",
                     \"PrimarySource\", \"SecurityVersion\", \"ShortLived\") VALUES (?1, ?2, 'd', 'i', ?3, ?3, ?4, ?4, 'password', 3, 0)",
                params![parent, user, now, end],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE \"AuthChallenges\" SET \"ParentSessionId\" = ?1",
                [parent],
            )
            .unwrap();

        assert_eq!(
            status(&database, Some(&cookie), &id.to_string(), None, now),
            ChallengeStatus::Valid
        );
        connection
            .execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?1", [now])
            .unwrap();
        assert_eq!(
            status(&database, Some(&cookie), &id.to_string(), None, now),
            ChallengeStatus::SecurityChanged
        );
    }

    #[tokio::test]
    async fn only_one_of_two_requests_uses_a_challenge_up_and_failures_close_it_at_its_budget() {
        let (_folder, database) = new_database();
        let (id, _, _) = challenge_in(&database);
        let now = Timestamp::now();

        let used = database
            .write(move |tx| -> Result<(bool, bool), DbError> {
                Ok((consume(tx, id, now)?, consume(tx, id, now)?))
            })
            .await
            .unwrap();
        assert_eq!(used, (true, false));

        let (second, _, _) = challenge_in(&database);
        database
            .write(move |tx| -> Result<(), DbError> {
                fail_attempt(tx, second, now)?;
                let (attempts, consumed): (i64, Option<Timestamp>) = tx.query_row(
                    "SELECT \"Attempts\", \"ConsumedAt\" FROM \"AuthChallenges\" WHERE \"Id\" = ?1",
                    [second],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!((attempts, consumed), (1, None));
                fail_attempt(tx, second, now)?;
                let consumed: Option<Timestamp> = tx.query_row(
                    "SELECT \"ConsumedAt\" FROM \"AuthChallenges\" WHERE \"Id\" = ?1",
                    [second],
                    |r| r.get(0),
                )?;
                assert!(consumed.is_some());
                Ok(())
            })
            .await
            .unwrap();
    }

    #[test]
    fn the_cookie_that_binds_a_challenge_is_read_back() {
        let id = Id::parse("049083c7-a28f-4b91-b21b-2a0920dc1686").unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "stellarr-dev-preauth=049083c7a28f4b91b21b2a0920dc1686.the-secret"
                .parse()
                .unwrap(),
        );

        assert_eq!(
            read_preauth(&headers, false),
            Some((id, "the-secret".to_string()))
        );
        assert_eq!(read_preauth(&headers, true), None);
        headers.insert(
            axum::http::header::COOKIE,
            "stellarr-dev-preauth=short.secret".parse().unwrap(),
        );
        assert_eq!(read_preauth(&headers, false), None);
    }

    #[test]
    fn a_fresh_proof_is_asked_as_a_second_step_and_never_offers_a_remembered_device() {
        let (_folder, database) = new_database();
        let (_, user_id, _) = challenge_in(&database);
        let connection = rusqlite::Connection::open(database.path()).unwrap();
        let user = User::find(&connection, user_id).unwrap().unwrap();
        let now = Timestamp::now();
        connection
            .execute(
                "INSERT INTO \"TrustedDevices\" VALUES ('11111111-1111-1111-1111-111111111111', ?1, ?2, 'n', 'd', 'i', ?3, ?4, NULL, NULL)",
                params![user_id, tokens::hash("remembered"), now, now.plus(std::time::Duration::from_secs(3600))],
            )
            .unwrap();
        let context = Context {
            ip_address: "i",
            user_agent: "",
            https: false,
            trusted_device_cookie: Some("remembered"),
            passkeys_available: false,
        };
        let describe_as = |purpose: &str| {
            let created = Created {
                id: Id::random(),
                purpose: purpose.to_string(),
                expires_at: now,
                pending_identity: None,
            };
            let body = describe(
                &connection,
                &context,
                &user,
                &created,
                Vec::new(),
                &SecurityPolicy::default(),
            )
            .unwrap();
            let json = serde_json::to_value(&body).unwrap();
            (
                json["status"].as_str().unwrap().to_string(),
                json["trustedDeviceAvailable"].as_bool().unwrap(),
            )
        };

        assert_eq!(describe_as("mfa"), ("mfa_required".to_string(), true));
        assert_eq!(describe_as("reverify"), ("mfa_required".to_string(), false));
        assert_eq!(
            describe_as("enrollment"),
            ("mfa_enrollment_required".to_string(), true)
        );
    }

    #[test]
    fn what_is_printed_of_a_request_or_a_start_holds_no_secret() {
        let context = Context {
            ip_address: "1.2.3.4",
            user_agent: "agent",
            https: true,
            trusted_device_cookie: Some("sekrit-token"),
            passkeys_available: false,
        };

        assert!(!format!("{context:?}").contains("sekrit"));
        assert_eq!(
            format!("{:?}", Start::RecoveryRequired),
            "Start::RecoveryRequired"
        );
    }
}
