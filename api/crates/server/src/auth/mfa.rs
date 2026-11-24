//! Checking a second step: one call that takes what a person sent to complete
//! a sign-in (or to prove themselves again) and says what it came to, and the
//! calls that set a second step up.
//!
//! Everything here runs inside the transaction it is given, which is the one
//! the route completes the sign-in in, so there is no gap between a code being
//! checked and being spent.
//!
//! A wrong proof is an ordinary outcome and not an error, because it has to be
//! committed: it spends one of the challenge's attempts, is written to the
//! account's events and counts towards its budget of failures. The route
//! commits after [`Outcome::Refused`] as it does after a proof. The reverse
//! holds too: a proof spends a code or a step, so a route that goes on to
//! refuse the sign-in for another reason must roll the transaction back.
//!
//! Which methods a challenge may be answered with, and how often, is decided
//! here and nowhere else: a code is looked at only after the account's budget
//! of failures was checked, a trusted device never answers a fresh proof or a
//! sign-in that links a provider, and every method is looked up among the
//! account's own.

use rusqlite::{OptionalExtension, Transaction};

use super::email_codes::{self, Reservation, Scope};
use super::events;
use super::flow::{
    self, Context, PURPOSE_ENROLLMENT, PURPOSE_MFA, PURPOSE_REVERIFY, StoredChallenge,
};
use super::mfa_methods::{self, EMAIL, TOTP};
use super::policy::SecurityPolicy;
use super::recovery_codes;
use super::trusted_devices;
use crate::database::DbError;
use crate::seal::Sealer;
use crate::values::{Id, Timestamp};

/// The name an authenticator app is added under in the app itself.
pub const ISSUER: &str = "Stellarr";

/// What was sent to answer a challenge.
#[derive(Clone, Copy)]
pub enum Request<'a> {
    /// "This browser is remembered": the cookie the request carries (see
    /// `Context`) stands in for a code.
    TrustedDevice,
    /// A code, trimmed and not empty. For the method named; without one it is
    /// a recovery code.
    Code { method: Option<Id>, code: &'a str },
}

/// Which kind of answer it is, never the code in it.
impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Request::TrustedDevice => f.write_str("Request::TrustedDevice"),
            Request::Code { method, .. } => f
                .debug_struct("Request::Code")
                .field("method", method)
                .finish_non_exhaustive(),
        }
    }
}

/// A second step that was proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    /// What proved it: `totp`, `email`, `recovery`, `passkey` or
    /// `trusted_device`. The session records it.
    pub evidence: &'static str,
    /// Whether it is a proof now, which makes the session one that passed a
    /// second step. A remembered device is not.
    pub fresh: bool,
    /// The remembered device that was used, already noted as used.
    pub trusted_device: Option<Id>,
}

impl Proof {
    /// The kind of event that a completed sign-in records.
    pub fn event_kind(&self) -> &'static str {
        if self.evidence == "recovery" {
            events::RECOVERY_CODE_USED
        } else {
            events::MFA_SUCCEEDED
        }
    }
}

/// What a request to answer a challenge came to.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub enum Outcome {
    Proved(Proof),
    /// Wrong. An attempt of the challenge was spent and an event written.
    Refused,
    /// The account has failed too many times lately; the code was not looked
    /// at (a 429).
    TooManyFailures,
    /// The challenge is not one this can answer, or not in this way (a 401
    /// that says the challenge is not valid).
    WrongChallenge,
    /// The method asked for is not there for this challenge (a 409).
    MethodUnavailable,
    /// No code was sent (a validation problem; nothing was spent).
    EmptyCode,
}

/// Challenges that wait for a second step the person already has.
pub fn answers_with_a_code(challenge: &StoredChallenge) -> bool {
    challenge.purpose == PURPOSE_MFA || challenge.purpose == PURPOSE_REVERIFY
}

/// Challenges that let a person add a second step.
pub fn enrols(challenge: &StoredChallenge) -> bool {
    challenge.purpose == PURPOSE_ENROLLMENT || challenge.purpose == super::flow::PURPOSE_RECOVERY
}

/// What the account has as an address: the one it was written with, trimmed,
/// and the one it is compared by. Either may be missing, or empty.
struct Address {
    written: Option<String>,
    normalized: Option<String>,
}

impl Address {
    fn of(tx: &Transaction, user: Id) -> Result<Address, DbError> {
        let found = tx
            .query_row(
                "SELECT \"Email\", \"NormalizedEmail\" FROM \"Users\" WHERE \"Id\" = ?1",
                [user],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        let (written, normalized) = found.unwrap_or((None, None));
        Ok(Address {
            written: written
                .map(|address| address.trim().to_string())
                .filter(|address| !address.is_empty()),
            normalized: normalized.filter(|address| !address.is_empty()),
        })
    }
}

/// Spends an attempt of the challenge for a proof that was wrong, and writes
/// what was tried to the account's events.
pub(super) fn refuse(
    tx: &Transaction,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    tried: &str,
    now: Timestamp,
) -> Result<Outcome, DbError> {
    record_failure(tx, context, challenge, tried, now)?;
    Ok(Outcome::Refused)
}

/// What a wrong proof costs: an attempt of the challenge and an event.
pub(super) fn record_failure(
    tx: &Transaction,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    tried: &str,
    now: Timestamp,
) -> Result<(), DbError> {
    flow::fail_attempt(tx, challenge.id, now)?;
    events::record_from(tx, context, challenge.user, events::MFA_FAILED, tried, None)
}

/// Checks what was sent to answer a challenge of a sign-in or of a fresh
/// proof, and spends it if it was right.
pub(super) fn verify(
    tx: &Transaction,
    sealer: &Sealer,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    request: Request<'_>,
    now: Timestamp,
) -> Result<Outcome, DbError> {
    if !answers_with_a_code(challenge) {
        return Ok(Outcome::WrongChallenge);
    }
    if matches!(request, Request::TrustedDevice) {
        // A remembered device proves nothing now, so it cannot answer a
        // request for a fresh proof; and a sign-in that links a provider asks
        // for the step a link from the settings would.
        if challenge.purpose != PURPOSE_MFA {
            return Ok(Outcome::WrongChallenge);
        }
        if challenge.pending_identity.is_some() {
            return Ok(Outcome::MethodUnavailable);
        }
    }
    if events::failure_budget_spent(tx, challenge.user, now)? {
        return Ok(Outcome::TooManyFailures);
    }
    match request {
        Request::TrustedDevice => verify_trusted_device(tx, context, challenge, now),
        Request::Code { code, .. } if code.is_empty() => Ok(Outcome::EmptyCode),
        Request::Code {
            method: Some(method),
            code,
        } => verify_method(tx, sealer, context, challenge, method, code, now),
        Request::Code { method: None, code } => {
            if recovery_codes::consume(tx, challenge.user, code, now)? {
                Ok(Outcome::Proved(proof("recovery")))
            } else {
                refuse(tx, context, challenge, "recovery", now)
            }
        }
    }
}

fn proof(evidence: &'static str) -> Proof {
    Proof {
        evidence,
        fresh: true,
        trusted_device: None,
    }
}

/// A remembered device is not a code, so a wrong one costs an attempt and
/// leaves no event.
fn verify_trusted_device(
    tx: &Transaction,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    now: Timestamp,
) -> Result<Outcome, DbError> {
    let found = trusted_devices::find(tx, challenge.user, context.trusted_device_cookie, now)?;
    let Some(device) = found else {
        flow::fail_attempt(tx, challenge.id, now)?;
        return Ok(Outcome::Refused);
    };
    if !trusted_devices::use_for_sign_in(tx, challenge.user, &device, now)? {
        return Ok(Outcome::WrongChallenge);
    }
    Ok(Outcome::Proved(Proof {
        evidence: "trusted_device",
        fresh: false,
        trusted_device: Some(device.id),
    }))
}

fn verify_method(
    tx: &Transaction,
    sealer: &Sealer,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    method: Id,
    code: &str,
    now: Timestamp,
) -> Result<Outcome, DbError> {
    let user = challenge.user;
    let Some(found) = mfa_methods::find_active(tx, user, method)? else {
        return refuse(tx, context, challenge, "unknown", now);
    };
    match found.kind.as_str() {
        TOTP => {
            let spent = match mfa_methods::totp_step_of(sealer, &found, code, now) {
                Some(step) => mfa_methods::spend_totp_step(tx, user, method, step, now, false)?,
                None => false,
            };
            if spent {
                Ok(Outcome::Proved(proof(TOTP)))
            } else {
                refuse(tx, context, challenge, TOTP, now)
            }
        }
        EMAIL => {
            let account = Address::of(tx, user)?;
            let Some(address) = account.written.as_deref() else {
                mfa_methods::revoke_if_address_changed(tx, user, method, now)?;
                return refuse(tx, context, challenge, EMAIL, now);
            };
            if found.email_address != account.normalized {
                // The address the method was made for is not the account's any
                // more: the method goes, and the codes sent for it.
                mfa_methods::revoke_if_address_changed(tx, user, method, now)?;
                return refuse(tx, context, challenge, EMAIL, now);
            }
            let scope = Scope {
                user,
                challenge: challenge.id,
                purpose: email_codes::LOGIN,
                method: Some(method),
                address,
            };
            let spent = match email_codes::check(tx, sealer, &scope, code, now)? {
                Some(checked) => email_codes::consume(tx, &checked, now)?,
                None => false,
            };
            if spent {
                Ok(Outcome::Proved(proof(EMAIL)))
            } else {
                refuse(tx, context, challenge, EMAIL, now)
            }
        }
        // A passkey is answered with a signed assertion, not with a code.
        _ => refuse(tx, context, challenge, "unknown", now),
    }
}

/// Asks for a code to be mailed for a method of a sign-in. `None` when the
/// method is not an e-mail method of the account that is in use, or the
/// account's address is not the one it was made for.
pub(super) fn request_login_email_code(
    tx: &Transaction,
    sealer: &Sealer,
    challenge: &StoredChallenge,
    method: Id,
    policy: &SecurityPolicy,
    now: Timestamp,
) -> Result<Option<Reservation>, DbError> {
    if !answers_with_a_code(challenge) {
        return Ok(None);
    }
    let Some(found) = mfa_methods::find_active(tx, challenge.user, method)? else {
        return Ok(None);
    };
    let account = Address::of(tx, challenge.user)?;
    let (Some(address), Some(bound)) = (account.written.as_deref(), found.email_address.as_deref())
    else {
        return Ok(None);
    };
    if found.kind != EMAIL || account.normalized.as_deref() != Some(bound) {
        return Ok(None);
    }
    email_codes::reserve(
        tx,
        sealer,
        &Scope {
            user: challenge.user,
            challenge: challenge.id,
            purpose: email_codes::LOGIN,
            method: Some(method),
            address,
        },
        policy,
        now,
    )
    .map(Some)
}

/// Asks for a code to be mailed to prove the address, to add it as a second
/// step. `None` when the account has no address or already has the step.
pub(super) fn start_email_enrollment(
    tx: &Transaction,
    sealer: &Sealer,
    challenge: &StoredChallenge,
    policy: &SecurityPolicy,
    now: Timestamp,
) -> Result<Option<Reservation>, DbError> {
    if !enrols(challenge) {
        return Ok(None);
    }
    let account = Address::of(tx, challenge.user)?;
    let (Some(address), Some(normalized)) =
        (account.written.as_deref(), account.normalized.as_deref())
    else {
        return Ok(None);
    };
    if mfa_methods::has_email(tx, challenge.user, normalized)? {
        return Ok(None);
    }
    email_codes::reserve(
        tx,
        sealer,
        &Scope {
            user: challenge.user,
            challenge: challenge.id,
            purpose: email_codes::ENROLLMENT,
            method: None,
            address,
        },
        policy,
        now,
    )
    .map(Some)
}

/// What checking the code that proves an address came to.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub enum EmailEnrollment {
    /// The code was right; [`finish_email_enrollment`] adds the step.
    /// The proof, the address as it is compared, and as the account has it.
    Checked(email_codes::Proof, String, String),
    Refused,
    TooManyFailures,
    /// The account has no address.
    Unavailable,
    /// The challenge does not let a second step be added.
    WrongChallenge,
}

/// Checks the code that was mailed to prove an address.
pub(super) fn check_email_enrollment(
    tx: &Transaction,
    sealer: &Sealer,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    code: &str,
    now: Timestamp,
) -> Result<EmailEnrollment, DbError> {
    if !enrols(challenge) {
        return Ok(EmailEnrollment::WrongChallenge);
    }
    let account = Address::of(tx, challenge.user)?;
    let (Some(address), Some(normalized)) = (account.written, account.normalized) else {
        return Ok(EmailEnrollment::Unavailable);
    };
    if events::failure_budget_spent(tx, challenge.user, now)? {
        return Ok(EmailEnrollment::TooManyFailures);
    }
    let scope = Scope {
        user: challenge.user,
        challenge: challenge.id,
        purpose: email_codes::ENROLLMENT,
        method: None,
        address: &address,
    };
    match email_codes::check(tx, sealer, &scope, code, now)? {
        Some(checked) => Ok(EmailEnrollment::Checked(checked, normalized, address)),
        None => {
            record_failure(tx, context, challenge, "email-enrollment", now)?;
            Ok(EmailEnrollment::Refused)
        }
    }
}

/// Adds the address as a second step, with the code that proved it. False when
/// the address changed since, the account already has the step, or the code was
/// spent by another request; nothing is added then, and the route answers that
/// the challenge is no longer valid.
pub(super) fn finish_email_enrollment(
    tx: &Transaction,
    challenge: &StoredChallenge,
    checked: &email_codes::Proof,
    normalized: &str,
    name: &str,
    now: Timestamp,
) -> Result<bool, DbError> {
    if !enrols(challenge) {
        return Ok(false);
    }
    let user = challenge.user;
    let account = Address::of(tx, user)?;
    if account.normalized.as_deref() != Some(normalized) {
        return Ok(false);
    }
    if mfa_methods::has_email(tx, user, normalized)? || !email_codes::consume(tx, checked, now)? {
        return Ok(false);
    }
    mfa_methods::add_email(tx, challenge, normalized, name, now)
}

/// Starts adding an authenticator app: the method, the secret to show once and
/// the address a QR code carries.
pub(super) fn start_totp_enrollment(
    tx: &Transaction,
    sealer: &Sealer,
    challenge: &StoredChallenge,
    username: &str,
    name: &str,
    now: Timestamp,
) -> Result<Option<(Id, String, String)>, DbError> {
    if !enrols(challenge) {
        return Ok(None);
    }
    let (method, secret) = mfa_methods::start_totp(tx, sealer, challenge.user, name, now)?;
    let uri = super::totp::otpauth_uri(ISSUER, username, &secret);
    Ok(Some((method, secret, uri)))
}

/// What the first code of an authenticator app came to.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub enum TotpEnrollment {
    /// The method is confirmed and the code's step spent.
    Confirmed,
    /// The account has no such method waiting for its code.
    Unavailable,
    TooManyFailures,
    /// Wrong. An attempt was spent and an event written.
    Refused,
    /// The code was right but was spent by another request first.
    Lost,
    /// The challenge does not let a second step be added.
    WrongChallenge,
}

/// The first code of an authenticator app, and what the person called it.
#[derive(Clone, Copy)]
pub struct FirstCode<'a> {
    pub method: Id,
    /// The name asked for, already checked with [`method_name`]; none keeps
    /// the one the method was started with.
    pub name: Option<&'a str>,
    pub code: &'a str,
}

/// The method and the name, never the code.
impl std::fmt::Debug for FirstCode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirstCode")
            .field("method", &self.method)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Confirms an authenticator app with the first code it shows.
pub(super) fn confirm_totp_enrollment(
    tx: &Transaction,
    sealer: &Sealer,
    context: &Context<'_>,
    challenge: &StoredChallenge,
    first: FirstCode<'_>,
    now: Timestamp,
) -> Result<TotpEnrollment, DbError> {
    if !enrols(challenge) {
        return Ok(TotpEnrollment::WrongChallenge);
    }
    let FirstCode { method, name, code } = first;
    let Some(pending) = mfa_methods::find_pending(tx, challenge.user, method, TOTP)? else {
        return Ok(TotpEnrollment::Unavailable);
    };
    if events::failure_budget_spent(tx, challenge.user, now)? {
        return Ok(TotpEnrollment::TooManyFailures);
    }
    let Some(step) = mfa_methods::totp_step_of(sealer, &pending, code, now) else {
        record_failure(tx, context, challenge, "totp-enrollment", now)?;
        return Ok(TotpEnrollment::Refused);
    };
    let name = name.unwrap_or(&pending.name);
    if mfa_methods::confirm_totp(tx, challenge.user, method, step, name, now)? {
        Ok(TotpEnrollment::Confirmed)
    } else {
        Ok(TotpEnrollment::Lost)
    }
}

/// The method a request names, read as the first server read it: the id
/// with its dashes, without them, in braces or in parentheses, in either
/// letter case and with white space round it. Anything else names no method,
/// and the code that goes with it is taken for a recovery code.
pub fn parse_method_id(text: &str) -> Option<Id> {
    let text = text.trim();
    let inner = if let Some(inner) = text.strip_prefix('{') {
        inner.strip_suffix('}')?
    } else if let Some(inner) = text.strip_prefix('(') {
        inner.strip_suffix(')')?
    } else {
        text
    };
    if inner.len() == 32
        && !text.starts_with(['{', '('])
        && inner.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Id::parse(&format!(
            "{}-{}-{}-{}-{}",
            &inner[..8],
            &inner[8..12],
            &inner[12..16],
            &inner[16..20],
            &inner[20..]
        ));
    }
    Id::parse(inner)
}

/// The name a person gave a method, or the one to use when they gave none:
/// `None` when it is too long. The length is counted the way the web client
/// counts it, in UTF-16 units.
pub fn method_name(requested: Option<&str>, fallback: &str) -> Option<String> {
    let name = requested.unwrap_or("").trim();
    if name.is_empty() {
        return Some(fallback.to_string());
    }
    (name.encode_utf16().count() <= mfa_methods::MAX_NAME_LENGTH).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::{Connection, params};
    use stellarr_messages::{Language, Messages};

    use super::*;
    use crate::auth::fixtures::{
        add_challenge, add_method, add_pending_method, add_user, attempts_of, events_of, open,
    };
    use crate::auth::mfa_methods::PASSKEY;
    use crate::auth::totp;

    const ADDRESS: &str = "maya@example.test";

    fn sealer() -> Sealer {
        Sealer::from_key([5; 32])
    }

    fn context(cookie: Option<&str>) -> Context<'_> {
        Context {
            ip_address: "203.0.113.9",
            user_agent: "Firefox/130",
            https: true,
            trusted_device_cookie: cookie,
            passkeys_available: false,
        }
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds).unwrap()
    }

    /// An account with a confirmed authenticator app, and the secret of it.
    fn with_totp(connection: &mut Connection, user: Id) -> (Id, String) {
        let tx = connection.transaction().unwrap();
        let (method, secret) =
            mfa_methods::start_totp(&tx, &sealer(), user, "Phone", at(1_800_000_000)).unwrap();
        assert!(
            mfa_methods::confirm_totp(&tx, user, method, 1, "Phone", at(1_800_000_000)).unwrap()
        );
        tx.commit().unwrap();
        (method, secret)
    }

    fn run(
        connection: &mut Connection,
        challenge: &StoredChallenge,
        request: Request<'_>,
        cookie: Option<&str>,
        now: Timestamp,
    ) -> Outcome {
        let tx = connection.transaction().unwrap();
        let outcome = verify(&tx, &sealer(), &context(cookie), challenge, request, now).unwrap();
        // A wrong proof must be committed, so the tests always commit.
        tx.commit().unwrap();
        outcome
    }

    fn code(method: Option<Id>, code: &str) -> Request<'_> {
        Request::Code { method, code }
    }

    #[test]
    fn a_right_code_proves_the_step_and_the_same_code_proves_nothing_twice() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (method, secret) = with_totp(&mut connection, user);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = at(1_800_000_100);
        let typed = totp::code_at(&secret, now).unwrap();

        let proved = run(
            &mut connection,
            &challenge,
            code(Some(method), &typed),
            None,
            now,
        );

        assert_eq!(
            proved,
            Outcome::Proved(Proof {
                evidence: "totp",
                fresh: true,
                trusted_device: None
            })
        );
        assert_eq!(attempts_of(&connection, challenge.id), (0, false));
        // The step is spent: the same code, a moment later, is refused.
        let replay = run(
            &mut connection,
            &challenge,
            code(Some(method), &typed),
            None,
            at(1_800_000_105),
        );
        assert_eq!(replay, Outcome::Refused);
        assert_eq!(attempts_of(&connection, challenge.id), (1, false));
        assert_eq!(events_of(&connection, user, "mfa-failed"), vec!["totp"]);
    }

    #[test]
    fn a_wrong_proof_is_an_outcome_that_leaves_its_attempt_and_its_event_behind() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (method, secret) = with_totp(&mut connection, user);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = at(1_800_000_100);
        let right = totp::code_at(&secret, now).unwrap();
        let wrong = if right == "123456" {
            "123457"
        } else {
            "123456"
        };

        assert_eq!(
            run(
                &mut connection,
                &challenge,
                code(Some(method), wrong),
                None,
                now
            ),
            Outcome::Refused
        );

        assert_eq!(attempts_of(&connection, challenge.id), (1, false));
        assert_eq!(events_of(&connection, user, "mfa-failed"), vec!["totp"]);
        let (ip, device): (String, String) = connection
            .query_row(
                "SELECT \"IpAddress\", \"Device\" FROM \"SecurityEvents\" WHERE \"UserId\" = ?1",
                [user],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (ip.as_str(), device.as_str()),
            ("203.0.113.9", "Firefox/130")
        );
        // The step was not spent by the wrong code: the right one still works.
        assert!(matches!(
            run(
                &mut connection,
                &challenge,
                code(Some(method), &right),
                None,
                now
            ),
            Outcome::Proved(_)
        ));
    }

    #[test]
    fn the_last_attempt_closes_the_challenge() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (method, _) = with_totp(&mut connection, user);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);

        for _ in 0..5 {
            let _ = run(
                &mut connection,
                &challenge,
                code(Some(method), "000000"),
                None,
                at(1_800_000_100),
            );
        }

        assert_eq!(attempts_of(&connection, challenge.id), (5, true));
    }

    #[test]
    fn an_account_that_failed_too_often_is_not_asked_for_its_code_at_all() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (method, secret) = with_totp(&mut connection, user);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        for _ in 0..10 {
            events::record(&tx, user, events::MFA_FAILED, "", "", "totp", None).unwrap();
        }
        tx.commit().unwrap();
        let typed = totp::code_at(&secret, now).unwrap();

        let refused = run(
            &mut connection,
            &challenge,
            code(Some(method), &typed),
            None,
            now,
        );

        assert_eq!(refused, Outcome::TooManyFailures);
        // Nothing was spent, no attempt and no step, and nothing was written.
        assert_eq!(attempts_of(&connection, challenge.id), (0, false));
        assert_eq!(events_of(&connection, user, "mfa-failed").len(), 10);
        let step: Option<i64> = connection
            .query_row(
                "SELECT \"LastTotpStep\" FROM \"MfaMethods\" WHERE \"Id\" = ?1",
                [method],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(step, Some(1));
        // The failures of other accounts are not this account's.
        let other = add_user(&connection, None);
        let (other_method, other_secret) = with_totp(&mut connection, other);
        let theirs = add_challenge(&connection, other, PURPOSE_MFA);
        let typed = totp::code_at(&other_secret, now).unwrap();
        assert!(matches!(
            run(
                &mut connection,
                &theirs,
                code(Some(other_method), &typed),
                None,
                now
            ),
            Outcome::Proved(_)
        ));
    }

    #[test]
    fn only_a_challenge_that_waits_for_a_second_step_is_answered() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let (method, secret) = with_totp(&mut connection, user);
        let now = Timestamp::now();
        let typed = totp::code_at(&secret, now).unwrap();

        for purpose in [PURPOSE_ENROLLMENT, "recovery", "anything"] {
            let challenge = add_challenge(&connection, user, purpose);
            assert_eq!(
                run(
                    &mut connection,
                    &challenge,
                    code(Some(method), &typed),
                    None,
                    now
                ),
                Outcome::WrongChallenge,
                "{purpose}"
            );
            assert_eq!(attempts_of(&connection, challenge.id), (0, false));
        }
        let reverify = add_challenge(&connection, user, PURPOSE_REVERIFY);
        assert!(matches!(
            run(
                &mut connection,
                &reverify,
                code(Some(method), &typed),
                None,
                now
            ),
            Outcome::Proved(_)
        ));
    }

    #[test]
    fn a_method_that_is_not_the_accounts_not_confirmed_not_in_use_or_not_a_code_gets_no_code() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let (theirs, secret) = with_totp(&mut connection, other);
        let (revoked, _) = with_totp(&mut connection, user);
        let waiting = add_pending_method(&connection, user, TOTP);
        let passkey = add_method(&connection, user, PASSKEY, None);
        connection
            .execute(
                "UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
                params![revoked, Timestamp::now()],
            )
            .unwrap();
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let typed = totp::code_at(&secret, now).unwrap();

        for (method, why) in [
            (theirs, "another account's"),
            (revoked, "revoked"),
            (waiting, "pending"),
            (passkey, "a passkey"),
            (Id::random(), "unknown"),
        ] {
            assert_eq!(
                run(
                    &mut connection,
                    &challenge,
                    code(Some(method), &typed),
                    None,
                    now
                ),
                Outcome::Refused,
                "{why}"
            );
        }
        assert_eq!(
            events_of(&connection, user, "mfa-failed"),
            vec!["unknown"; 5]
        );
        assert_eq!(attempts_of(&connection, challenge.id), (5, true));
    }

    #[test]
    fn a_mailed_code_proves_the_address_and_is_spent_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let method = add_method(&connection, user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        let Some(Reservation::Reserved(mail)) = request_login_email_code(
            &tx,
            &sealer(),
            &challenge,
            method,
            &SecurityPolicy::default(),
            now,
        )
        .unwrap() else {
            panic!("a code was expected");
        };
        email_codes::mark_delivered(&tx, mail.id, now).unwrap();
        tx.commit().unwrap();

        let wrong = if mail.code == "000000" {
            "000001"
        } else {
            "000000"
        };
        assert_eq!(
            run(
                &mut connection,
                &challenge,
                code(Some(method), wrong),
                None,
                now
            ),
            Outcome::Refused
        );
        let proved = run(
            &mut connection,
            &challenge,
            code(Some(method), &mail.code),
            None,
            now,
        );
        assert_eq!(
            proved,
            Outcome::Proved(Proof {
                evidence: "email",
                fresh: true,
                trusted_device: None
            })
        );
        assert_eq!(
            run(
                &mut connection,
                &challenge,
                code(Some(method), &mail.code),
                None,
                now
            ),
            Outcome::Refused
        );
        assert_eq!(
            events_of(&connection, user, "mfa-failed"),
            vec!["email", "email"]
        );
    }

    #[test]
    fn a_mail_method_whose_address_moved_is_taken_back_and_answers_nothing() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let method = add_method(&connection, user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        let Some(Reservation::Reserved(mail)) = request_login_email_code(
            &tx,
            &sealer(),
            &challenge,
            method,
            &SecurityPolicy::default(),
            now,
        )
        .unwrap() else {
            panic!("a code was expected");
        };
        email_codes::mark_delivered(&tx, mail.id, now).unwrap();
        tx.execute("UPDATE \"Users\" SET \"Email\" = 'new@example.test', \"NormalizedEmail\" = 'NEW@EXAMPLE.TEST' WHERE \"Id\" = ?1", [user]).unwrap();
        tx.commit().unwrap();

        assert_eq!(
            run(
                &mut connection,
                &challenge,
                code(Some(method), &mail.code),
                None,
                now
            ),
            Outcome::Refused
        );

        assert!(
            mfa_methods::find_active(&connection, user, method)
                .unwrap()
                .is_none()
        );
        assert_eq!(events_of(&connection, user, "mfa-failed"), vec!["email"]);
        let unspent: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM \"EmailOtpCodes\" WHERE \"ConsumedAt\" IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unspent, 0);
    }

    #[test]
    fn a_code_is_mailed_only_for_a_mail_method_of_the_account_with_its_address() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let other = add_user(&connection, Some("other@example.test"));
        let mail = add_method(&connection, user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
        let stale = add_method(&connection, user, EMAIL, Some("OLD@EXAMPLE.TEST"));
        let app = add_method(&connection, user, TOTP, None);
        let theirs = add_method(&connection, other, EMAIL, Some("OTHER@EXAMPLE.TEST"));
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let enrolling = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let policy = SecurityPolicy::default();
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        let ask = |challenge: &StoredChallenge, method: Id| {
            request_login_email_code(&tx, &sealer(), challenge, method, &policy, now).unwrap()
        };

        assert!(ask(&challenge, stale).is_none());
        assert!(ask(&challenge, app).is_none());
        assert!(ask(&challenge, theirs).is_none());
        assert!(ask(&challenge, Id::random()).is_none());
        assert!(ask(&enrolling, mail).is_none());
        assert!(matches!(
            ask(&challenge, mail),
            Some(Reservation::Reserved(_))
        ));
    }

    #[test]
    fn a_recovery_code_answers_when_no_method_is_named_and_is_spent() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        add_method(&connection, user, TOTP, None);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let tx = connection.transaction().unwrap();
        let codes = recovery_codes::regenerate(&tx, user, Timestamp::now()).unwrap();
        tx.commit().unwrap();
        let now = Timestamp::now();

        let proved = run(
            &mut connection,
            &challenge,
            code(None, &codes[0].to_lowercase()),
            None,
            now,
        );

        let Outcome::Proved(proved) = proved else {
            panic!("a proof was expected")
        };
        assert_eq!(proved.evidence, "recovery");
        assert_eq!(proved.event_kind(), "recovery-code-used");
        assert_eq!(
            run(
                &mut connection,
                &challenge,
                code(None, &codes[0]),
                None,
                now
            ),
            Outcome::Refused
        );
        assert_eq!(events_of(&connection, user, "mfa-failed"), vec!["recovery"]);
        assert_eq!(recovery_codes::remaining(&connection, user).unwrap(), 9);
    }

    fn remembered(connection: &mut Connection, user: Id, now: Timestamp) -> (Id, String) {
        let tx = connection.transaction().unwrap();
        let issued = trusted_devices::issue(
            &tx,
            &context(None),
            &Messages::new(Language::English),
            user,
            &SecurityPolicy::default(),
            now,
        )
        .unwrap();
        tx.commit().unwrap();
        let token = issued
            .cookie
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_string();
        (issued.id, token)
    }

    #[test]
    fn a_remembered_device_answers_a_sign_in_without_being_a_fresh_proof() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        add_method(&connection, user, TOTP, None);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let (device, token) = remembered(&mut connection, user, now);
        let later = now.plus(Duration::from_secs(90));

        let proved = run(
            &mut connection,
            &challenge,
            Request::TrustedDevice,
            Some(&token),
            later,
        );

        assert_eq!(
            proved,
            Outcome::Proved(Proof {
                evidence: "trusted_device",
                fresh: false,
                trusted_device: Some(device)
            })
        );
        let used: Option<Timestamp> = connection
            .query_row(
                "SELECT \"LastUsedAt\" FROM \"TrustedDevices\" WHERE \"Id\" = ?1",
                [device],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(used, Some(later));
        assert!(events_of(&connection, user, "mfa-succeeded").is_empty());
    }

    #[test]
    fn a_remembered_device_never_answers_a_fresh_proof_or_a_sign_in_that_links_a_provider() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        add_method(&connection, user, TOTP, None);
        let now = Timestamp::now();
        let (_, token) = remembered(&mut connection, user, now);
        let reverify = add_challenge(&connection, user, PURPOSE_REVERIFY);
        let linking = StoredChallenge {
            pending_identity: Some(Id::random()),
            ..add_challenge(&connection, user, PURPOSE_MFA)
        };

        assert_eq!(
            run(
                &mut connection,
                &reverify,
                Request::TrustedDevice,
                Some(&token),
                now
            ),
            Outcome::WrongChallenge
        );
        assert_eq!(
            run(
                &mut connection,
                &linking,
                Request::TrustedDevice,
                Some(&token),
                now
            ),
            Outcome::MethodUnavailable
        );
        assert_eq!(attempts_of(&connection, reverify.id), (0, false));
        assert_eq!(attempts_of(&connection, linking.id), (0, false));
    }

    #[test]
    fn a_browser_that_is_not_remembered_costs_an_attempt_and_leaves_no_event() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let other = add_user(&connection, None);
        let now = Timestamp::now();
        let (_, theirs) = remembered(&mut connection, other, now);
        let (revoked, mine) = remembered(&mut connection, user, now);
        connection
            .execute(
                "UPDATE \"TrustedDevices\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1",
                params![revoked, now],
            )
            .unwrap();
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);

        for cookie in [
            None,
            Some("nothing"),
            Some(theirs.as_str()),
            Some(mine.as_str()),
        ] {
            assert_eq!(
                run(
                    &mut connection,
                    &challenge,
                    Request::TrustedDevice,
                    cookie,
                    now
                ),
                Outcome::Refused
            );
        }

        assert_eq!(attempts_of(&connection, challenge.id), (4, false));
        assert!(events_of(&connection, user, "mfa-failed").is_empty());
    }

    #[test]
    fn a_device_does_not_answer_when_the_account_has_failed_too_often() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let now = Timestamp::now();
        let (_, token) = remembered(&mut connection, user, now);
        let challenge = add_challenge(&connection, user, PURPOSE_MFA);
        let tx = connection.transaction().unwrap();
        for _ in 0..10 {
            events::record(&tx, user, events::MFA_FAILED, "", "", "totp", None).unwrap();
        }
        tx.commit().unwrap();

        assert_eq!(
            run(
                &mut connection,
                &challenge,
                Request::TrustedDevice,
                Some(&token),
                now
            ),
            Outcome::TooManyFailures
        );
    }

    #[test]
    fn what_is_printed_of_a_request_holds_no_code() {
        let method = Some(Id::random());
        for printed in [
            format!(
                "{:?}",
                Request::Code {
                    method,
                    code: "482913"
                }
            ),
            format!(
                "{:?}",
                Request::Code {
                    method: None,
                    code: "ABCD-EFGH"
                }
            ),
            format!(
                "{:?}",
                FirstCode {
                    method: Id::random(),
                    name: Some("Phone"),
                    code: "482913"
                }
            ),
        ] {
            assert!(
                !printed.contains("482913") && !printed.contains("ABCD"),
                "{printed}"
            );
        }
        assert_eq!(
            format!("{:?}", Request::TrustedDevice),
            "Request::TrustedDevice"
        );
    }

    #[test]
    fn a_method_is_named_as_a_guid_is_written() {
        let id = Id::parse("049083c7-a28f-4b91-b21b-2a0920dc1686").unwrap();
        for text in [
            "049083c7-a28f-4b91-b21b-2a0920dc1686",
            "049083C7-A28F-4B91-B21B-2A0920DC1686",
            "049083c7a28f4b91b21b2a0920dc1686",
            "{049083c7-a28f-4b91-b21b-2a0920dc1686}",
            "(049083c7-a28f-4b91-b21b-2a0920dc1686)",
            " 049083c7-a28f-4b91-b21b-2a0920dc1686\t",
        ] {
            assert_eq!(parse_method_id(text), Some(id), "{text:?}");
        }
        for text in [
            "",
            "a recovery code",
            "ABCD-EFGH-IJKL-MNOP",
            "{049083c7-a28f-4b91-b21b-2a0920dc1686",
            "049083c7-a28f-4b91-b21b-2a0920dc1686)",
            "{049083c7a28f4b91b21b2a0920dc1686}",
            "049083c7-a28f-4b91-b21b-2a0920dc168",
        ] {
            assert_eq!(parse_method_id(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_name_is_trimmed_defaulted_and_cut_at_sixty_four_units() {
        assert_eq!(method_name(None, "Phone").as_deref(), Some("Phone"));
        assert_eq!(method_name(Some("  "), "Phone").as_deref(), Some("Phone"));
        assert_eq!(
            method_name(Some(" Work "), "Phone").as_deref(),
            Some("Work")
        );
        assert_eq!(
            method_name(Some(&"a".repeat(64)), "x").map(|n| n.len()),
            Some(64)
        );
        assert_eq!(method_name(Some(&"a".repeat(65)), "x"), None);
        // An emoji is two units.
        assert!(method_name(Some(&"\u{1F600}".repeat(32)), "x").is_some());
        assert_eq!(method_name(Some(&"\u{1F600}".repeat(33)), "x"), None);
    }

    #[test]
    fn an_authenticator_app_is_set_up_with_its_first_code_once() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let now = at(1_800_000_000);
        let tx = connection.transaction().unwrap();
        let (method, secret, uri) =
            start_totp_enrollment(&tx, &sealer(), &challenge, "maya", "Phone", now)
                .unwrap()
                .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            uri,
            format!(
                "otpauth://totp/Stellarr%3Amaya?secret={secret}&issuer=Stellarr&algorithm=SHA1&digits=6&period=30"
            )
        );
        let right = totp::code_at(&secret, now).unwrap();
        let wrong = if right == "123456" {
            "123457"
        } else {
            "123456"
        };
        let confirm = |connection: &mut Connection, id: Id, name: Option<&str>, code: &str| {
            let tx = connection.transaction().unwrap();
            let outcome = confirm_totp_enrollment(
                &tx,
                &sealer(),
                &context(None),
                &challenge,
                FirstCode {
                    method: id,
                    name,
                    code,
                },
                now,
            )
            .unwrap();
            tx.commit().unwrap();
            outcome
        };

        assert_eq!(
            confirm(&mut connection, Id::random(), None, &right),
            TotpEnrollment::Unavailable
        );
        assert_eq!(
            confirm(&mut connection, method, None, wrong),
            TotpEnrollment::Refused
        );
        assert_eq!(attempts_of(&connection, challenge.id), (1, false));
        assert_eq!(
            events_of(&connection, user, "mfa-failed"),
            vec!["totp-enrollment"]
        );
        assert_eq!(
            confirm(&mut connection, method, Some("Mine"), &right),
            TotpEnrollment::Confirmed
        );
        let done = mfa_methods::find_active(&connection, user, method)
            .unwrap()
            .unwrap();
        assert_eq!(done.name, "Mine");
        // It is confirmed, so it waits for no code any more.
        assert_eq!(
            confirm(&mut connection, method, None, &right),
            TotpEnrollment::Unavailable
        );
    }

    #[test]
    fn a_first_code_that_was_spent_in_the_meantime_is_lost_not_wrong() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let now = at(1_800_000_000);
        let tx = connection.transaction().unwrap();
        let (method, secret, _) =
            start_totp_enrollment(&tx, &sealer(), &challenge, "maya", "Phone", now)
                .unwrap()
                .unwrap();
        // A step later than the one the code is for was spent already.
        tx.execute(
            "UPDATE \"MfaMethods\" SET \"LastTotpStep\" = ?2 WHERE \"Id\" = ?1",
            params![method, 1_800_000_000 / 30 + 1],
        )
        .unwrap();
        let right = totp::code_at(&secret, now).unwrap();

        let outcome = confirm_totp_enrollment(
            &tx,
            &sealer(),
            &context(None),
            &challenge,
            FirstCode {
                method,
                name: None,
                code: &right,
            },
            now,
        )
        .unwrap();

        assert_eq!(outcome, TotpEnrollment::Lost);
    }

    #[test]
    fn an_account_that_failed_too_often_is_not_asked_for_a_first_code() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        let (method, secret, _) =
            start_totp_enrollment(&tx, &sealer(), &challenge, "maya", "Phone", now)
                .unwrap()
                .unwrap();
        for _ in 0..10 {
            events::record(&tx, user, events::MFA_FAILED, "", "", "totp", None).unwrap();
        }
        let right = totp::code_at(&secret, now).unwrap();

        let outcome = confirm_totp_enrollment(
            &tx,
            &sealer(),
            &context(None),
            &challenge,
            FirstCode {
                method,
                name: None,
                code: &right,
            },
            now,
        )
        .unwrap();

        assert_eq!(outcome, TotpEnrollment::TooManyFailures);
    }

    #[test]
    fn an_address_is_added_as_a_step_with_the_code_that_was_mailed_to_it() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let now = Timestamp::now();
        let policy = SecurityPolicy::default();
        let tx = connection.transaction().unwrap();
        let Some(Reservation::Reserved(mail)) =
            start_email_enrollment(&tx, &sealer(), &challenge, &policy, now).unwrap()
        else {
            panic!("a code was expected");
        };
        email_codes::mark_delivered(&tx, mail.id, now).unwrap();
        tx.commit().unwrap();
        let wrong = if mail.code == "000000" {
            "000001"
        } else {
            "000000"
        };

        let tx = connection.transaction().unwrap();
        assert_eq!(
            check_email_enrollment(&tx, &sealer(), &context(None), &challenge, wrong, now).unwrap(),
            EmailEnrollment::Refused
        );
        tx.commit().unwrap();
        assert_eq!(
            events_of(&connection, user, "mfa-failed"),
            vec!["email-enrollment"]
        );
        assert_eq!(attempts_of(&connection, challenge.id), (1, false));

        let tx = connection.transaction().unwrap();
        let EmailEnrollment::Checked(checked, normalized, _) =
            check_email_enrollment(&tx, &sealer(), &context(None), &challenge, &mail.code, now)
                .unwrap()
        else {
            panic!("the code was right");
        };
        assert_eq!(normalized, "MAYA@EXAMPLE.TEST");
        assert!(
            finish_email_enrollment(&tx, &challenge, &checked, &normalized, "Mail", now).unwrap()
        );
        // The code is spent with the step, so it cannot add a second one.
        assert!(
            !finish_email_enrollment(&tx, &challenge, &checked, &normalized, "Mail", now).unwrap()
        );
        tx.commit().unwrap();
        assert!(mfa_methods::has_email(&connection, user, "MAYA@EXAMPLE.TEST").unwrap());
        // An account that has the step is not offered it again.
        let tx = connection.transaction().unwrap();
        assert!(
            start_email_enrollment(
                &tx,
                &sealer(),
                &challenge,
                &policy,
                now.plus(Duration::from_secs(600))
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn a_step_for_an_address_is_not_added_when_the_address_changed_after_the_code_was_checked() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        let Some(Reservation::Reserved(mail)) =
            start_email_enrollment(&tx, &sealer(), &challenge, &SecurityPolicy::default(), now)
                .unwrap()
        else {
            panic!("a code was expected");
        };
        email_codes::mark_delivered(&tx, mail.id, now).unwrap();
        let EmailEnrollment::Checked(checked, normalized, _) =
            check_email_enrollment(&tx, &sealer(), &context(None), &challenge, &mail.code, now)
                .unwrap()
        else {
            panic!("the code was right");
        };
        tx.execute("UPDATE \"Users\" SET \"Email\" = 'new@example.test', \"NormalizedEmail\" = 'NEW@EXAMPLE.TEST' WHERE \"Id\" = ?1", [user]).unwrap();

        assert!(
            !finish_email_enrollment(&tx, &challenge, &checked, &normalized, "Mail", now).unwrap()
        );
        assert!(!mfa_methods::has_email(&tx, user, &normalized).unwrap());
    }

    #[test]
    fn an_enrolment_code_is_sent_for_an_enrolling_challenge_only() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        let signing_in = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();

        assert!(
            start_email_enrollment(&tx, &sealer(), &signing_in, &SecurityPolicy::default(), now)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_step_the_account_already_has_is_not_added_and_the_code_that_proved_it_is_kept() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, Some(ADDRESS));
        add_method(&connection, user, EMAIL, Some("MAYA@EXAMPLE.TEST"));
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();
        let Reservation::Reserved(mail) = email_codes::reserve(
            &tx,
            &sealer(),
            &Scope {
                user,
                challenge: challenge.id,
                purpose: email_codes::ENROLLMENT,
                method: None,
                address: ADDRESS,
            },
            &SecurityPolicy::default(),
            now,
        )
        .unwrap() else {
            panic!("a code was expected");
        };
        email_codes::mark_delivered(&tx, mail.id, now).unwrap();
        let EmailEnrollment::Checked(checked, normalized, _) =
            check_email_enrollment(&tx, &sealer(), &context(None), &challenge, &mail.code, now)
                .unwrap()
        else {
            panic!("the code was right");
        };

        assert!(
            !finish_email_enrollment(&tx, &challenge, &checked, &normalized, "Again", now).unwrap()
        );

        let consumed: bool = tx
            .query_row(
                "SELECT \"ConsumedAt\" IS NOT NULL FROM \"EmailOtpCodes\" WHERE \"Id\" = ?1",
                [mail.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!consumed);
    }

    #[test]
    fn nothing_is_mailed_for_an_account_without_an_address() {
        let (_folder, mut connection) = open();
        let user = add_user(&connection, None);
        let challenge = add_challenge(&connection, user, PURPOSE_ENROLLMENT);
        let reverify = add_challenge(&connection, user, PURPOSE_MFA);
        let now = Timestamp::now();
        let tx = connection.transaction().unwrap();

        assert!(
            start_email_enrollment(&tx, &sealer(), &challenge, &SecurityPolicy::default(), now)
                .unwrap()
                .is_none()
        );
        assert!(
            start_email_enrollment(&tx, &sealer(), &reverify, &SecurityPolicy::default(), now)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            check_email_enrollment(&tx, &sealer(), &context(None), &challenge, "123456", now)
                .unwrap(),
            EmailEnrollment::Unavailable
        );
    }
}
