//! What happens after every proof of who somebody is: a password, a
//! registration, a provider or a recovery. It either gives them an ordinary
//! session or a challenge that is bound to their browser, never both, and an
//! account that must complete a second step, or enrol one, never gets a
//! session without it. The routes that complete a challenge belong to a later
//! group of endpoints; the answer to a sign-in that needs one is made here.

use rusqlite::{Connection, Transaction, params};
use serde::Serialize;
use stellarr_messages::{MessageKey, Messages};

use super::cookies;
use super::policy::SecurityPolicy;
use super::session::NewSession;
use super::tokens;
use crate::database::DbError;
use crate::instance::users::User;
use crate::values::{Id, Timestamp};

/// The challenge that a person waits on while they prove a second step.
pub const PURPOSE_MFA: &str = "mfa";
/// The challenge that only lets a person enrol a second step.
pub const PURPOSE_ENROLLMENT: &str = "enrollment";
/// Evidence of a second step that the identity provider asked for.
pub const PROVIDER_EVIDENCE: &str = "provider_mfa";

/// What the request the person signs in with says about it.
#[derive(Debug, Clone)]
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

#[derive(Debug)]
pub enum Start {
    /// The account must be recovered at the console before anybody signs in.
    RecoveryRequired,
    /// An ordinary session, to be written and signed in.
    Session(NewSession),
    /// A challenge: what to tell the client, and the cookie that binds it to
    /// this browser.
    Challenge { body: Challenge, cookie: String },
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
    let (challenge, secret) = create_challenge(tx, user, primary_source, purpose, policy, options)?;
    let body = describe(tx, context, user, &challenge, methods, policy)?;
    let cookie = preauth_cookie(context.https, challenge.id, &secret, challenge.expires_at);
    Ok(Start::Challenge { body, cookie })
}

fn preauth_name(https: bool) -> &'static str {
    if https {
        "__Host-stellarr-preauth"
    } else {
        "stellarr-dev-preauth"
    }
}

fn preauth_cookie(https: bool, id: Id, secret: &str, expires: Timestamp) -> String {
    let compact: String = id.to_string().chars().filter(|c| *c != '-').collect();
    cookies::set(
        preauth_name(https),
        &format!("{compact}.{secret}"),
        Some(expires),
        https,
    )
}

struct Created {
    id: Id,
    purpose: String,
    expires_at: Timestamp,
}

fn create_challenge(
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
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, NULL, 0, 5, ?9, ?10, NULL)",
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
            expires_at
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

fn confirmed_methods(connection: &Connection, user: Id) -> Result<Vec<Method>, DbError> {
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

fn describe(
    connection: &Connection,
    context: &Context<'_>,
    user: &User,
    challenge: &Created,
    methods: Vec<Method>,
    policy: &SecurityPolicy,
) -> Result<Challenge, DbError> {
    let status = if challenge.purpose == PURPOSE_MFA {
        "mfa_required"
    } else {
        "mfa_enrollment_required"
    };
    // A remembered device skips the step but proves nothing now.
    let trusted = trusted_device(connection, user.id, context.trusted_device_cookie)?;
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
    let Some(token) = cookie.filter(|token| !token.trim().is_empty() && token.len() <= 512) else {
        return Ok(false);
    };
    Ok(connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM \"TrustedDevices\" WHERE \"UserId\" = ?1 AND \"TokenHash\" = ?2 AND \"RevokedAt\" IS NULL AND \"ExpiresAt\" > ?3)",
        params![user, tokens::hash(token), Timestamp::now()],
        |row| row.get(0),
    )?)
}

/// Whether the instance can offer passkeys: its public address is HTTPS (or
/// HTTP on localhost) and names a host, not an IP address, since browsers
/// refuse them otherwise. It is read from the settings, never from a request,
/// so a forged `Host` header cannot move it.
pub fn passkeys_available(public_url: &str) -> bool {
    let public_url = public_url.trim();
    let Some((scheme, rest)) = public_url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_and_port = authority.rsplit('@').next().unwrap_or("");
    if host_and_port.starts_with('[') {
        return false;
    }
    let host = host_and_port.split(':').next().unwrap_or("").to_lowercase();
    let is_dns = !host.is_empty()
        && host.parse::<std::net::IpAddr>().is_err()
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '.');
    is_dns
        && (scheme.eq_ignore_ascii_case("https")
            || (scheme.eq_ignore_ascii_case("http") && host == "localhost"))
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
}
