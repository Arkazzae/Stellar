//! The calls a route makes for the second step: one for each thing a person
//! does, which owns the transaction it happens in.
//!
//! What a route used to have to get right is done here, once:
//!
//! - the challenge is looked up, and the account with it, inside the
//!   transaction, so nothing stale can be answered: a challenge that was
//!   consumed, has run out, has no attempts left, belongs to an account that
//!   changed or is for another purpose is refused (`Invalid`);
//! - a wrong proof is committed with what it costs (an attempt, an event, a
//!   point against the account's budget) and handed back as `Refused`; a route
//!   cannot turn it into an error and lose it;
//! - a right proof is spent, the sign-in is finished (the session, the events,
//!   the remembered device, the cookies) and the caller's own writes run in
//!   the same transaction, which is rolled back whole, the proof included, if
//!   they fail or something forbids the sign-in after all;
//! - what follows from the proof is decided by what it proved: a remembered
//!   device never makes a session fresh and is never remembered again, a
//!   passkey sign-in is a second step passed.
//!
//! The modules underneath (`mfa`, `passkeys`, `email_codes` and the rest) keep
//! their checks but their writing calls are not reachable from a route.

use rusqlite::{OptionalExtension, Transaction, params};
use stellarr_messages::MessageKey;
use stellarr_webauthn::{AssertionResponse, RegistrationResponse, RelyingParty};

use super::events;
use super::flow::{
    self, ChallengeStatus, Context, PURPOSE_ENROLLMENT, PURPOSE_RECOVERY, PURPOSE_REVERIFY,
    StoredChallenge,
};
use super::fresh;
use super::mfa::{self, EmailEnrollment, FirstCode, Outcome, Proof, Request, TotpEnrollment};
use super::mfa_methods::{self, EMAIL, PASSKEY, TOTP};
use super::passkeys::{self, Begin, Registered, SignIn};
use super::policy::SecurityPolicy;
use super::recovery;
use super::recovery_codes;
use super::session::{self, Caller, NewSession};
use super::trusted_devices;
use super::ways_in;
use crate::database::DbError;
use crate::instance::users::User;
use crate::state::AppState;
use crate::values::{Id, Timestamp};

// ---- what every call takes ----

/// The challenge a request is about, as the request names it.
#[derive(Clone, Copy)]
pub struct Ask<'a> {
    pub context: &'a Context<'a>,
    /// The id and secret in the browser's challenge cookie.
    pub preauth: Option<&'a (Id, String)>,
    /// The id the client says the challenge has.
    pub challenge_id: &'a str,
}

impl std::fmt::Debug for Ask<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ask")
            .field("challenge_id", &self.challenge_id)
            .finish_non_exhaustive()
    }
}

/// [`Ask`] with everything owned, so that it can go into a transaction.
#[derive(Clone)]
pub(super) struct Own {
    ip_address: String,
    user_agent: String,
    https: bool,
    trusted_device_cookie: Option<String>,
    passkeys_available: bool,
    preauth: Option<(Id, String)>,
    challenge_id: String,
}

impl Own {
    fn of_context(context: &Context<'_>) -> Own {
        Own {
            ip_address: context.ip_address.to_string(),
            user_agent: context.user_agent.to_string(),
            https: context.https,
            trusted_device_cookie: context.trusted_device_cookie.map(str::to_string),
            passkeys_available: context.passkeys_available,
            preauth: None,
            challenge_id: String::new(),
        }
    }

    pub(super) fn of(ask: &Ask<'_>) -> Own {
        Own {
            preauth: ask.preauth.cloned(),
            challenge_id: ask.challenge_id.to_string(),
            ..Own::of_context(ask.context)
        }
    }

    /// For a test: makes the cookie's secret the wrong one.
    #[cfg(test)]
    pub(super) fn break_secret(&mut self) {
        if let Some((_, secret)) = &mut self.preauth {
            secret.push('x');
        }
    }

    pub(super) fn context(&self) -> Context<'_> {
        Context {
            ip_address: &self.ip_address,
            user_agent: &self.user_agent,
            https: self.https,
            trusted_device_cookie: self.trusted_device_cookie.as_deref(),
            passkeys_available: self.passkeys_available,
        }
    }
}

/// Looks the challenge up in the transaction, with the account, and checks it
/// is one of the purposes the call is for.
pub(super) fn open(
    tx: &Transaction,
    own: &Own,
    allows: fn(&StoredChallenge) -> bool,
    now: Timestamp,
) -> Result<Result<StoredChallenge, ChallengeStatus>, DbError> {
    let (found, status) = flow::validate(tx, own.preauth.as_ref(), &own.challenge_id, None, now)?;
    Ok(match found {
        Some(challenge) if allows(&challenge) => Ok(challenge),
        Some(_) => Err(ChallengeStatus::Missing),
        None => Err(status),
    })
}

/// Why a transaction is rolled back: the database failed, or the sign-in
/// turned out not to be allowed after the proof was spent.
pub(super) enum Abort {
    Db(DbError),
    Rolled(ChallengeStatus),
}

impl From<DbError> for Abort {
    fn from(error: DbError) -> Abort {
        Abort::Db(error)
    }
}

impl From<rusqlite::Error> for Abort {
    fn from(error: rusqlite::Error) -> Abort {
        Abort::Db(DbError::Sqlite(error))
    }
}

/// Runs the work in a transaction of its own: `Ok` commits, whatever the
/// work says in it; an `Abort` rolls back, and a refusal that rolled back is
/// returned as the status that says why.
async fn run<T: Send + 'static>(
    state: &AppState,
    work: impl FnOnce(&Transaction) -> Result<T, Abort> + Send + 'static,
) -> Result<Result<T, ChallengeStatus>, DbError> {
    match state.database().write(work).await {
        Ok(value) => Ok(Ok(value)),
        Err(Abort::Rolled(status)) => Ok(Err(status)),
        Err(Abort::Db(error)) => Err(error),
    }
}

fn text(state: &AppState, key: MessageKey) -> Result<String, DbError> {
    state
        .messages()
        .get(key, &[])
        .map_err(|error| DbError::Task(error.to_string()))
}

fn user_of(tx: &Transaction, id: Id) -> Result<User, DbError> {
    User::find(tx, id)?.ok_or_else(|| DbError::Task("the account is gone".to_string()))
}

// ---- what finishing a sign-in gives back ----

/// A sign-in that was completed. The session row and the events are written;
/// the route sets the cookies, in this order, and answers.
pub struct Completed {
    pub user: Id,
    pub session: Id,
    pub security_version: i64,
    pub short_lived: bool,
    /// What proved the second step: `totp`, `email`, `recovery`, `passkey`,
    /// `trusted_device`, or the type of the method that was set up.
    pub evidence: String,
    /// When the remembered device the person asked for lasts until.
    pub trusted_device_expires_at: Option<Timestamp>,
    /// The recovery codes of an account that had none, to be shown once.
    pub recovery_codes: Option<Vec<String>>,
    /// The provider identity a sign-in was asked to link, if any.
    pub pending_identity: Option<Id>,
    /// `Set-Cookie` lines: the session's, the remembered device's if one was
    /// made, and the one that takes the challenge's cookie away.
    pub cookies: Vec<String>,
}

impl std::fmt::Debug for Completed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Completed")
            .field("user", &self.user)
            .field("session", &self.session)
            .field("evidence", &self.evidence)
            .finish_non_exhaustive()
    }
}

enum How<'a> {
    Proof(&'a Proof),
    Enrolled {
        method_type: &'static str,
        alert_to: Option<String>,
    },
}

/// Finishes what a proof or an enrolment leads to, in the transaction the
/// proof was spent in.
fn finish(
    tx: &Transaction,
    state: &AppState,
    own: &Own,
    challenge: &StoredChallenge,
    how: How<'_>,
    remember: bool,
    now: Timestamp,
) -> Result<Completed, Abort> {
    let context = own.context();
    let messages = state.messages();
    let policy = SecurityPolicy::load(tx)?;
    if !flow::consume(tx, challenge.id, now)? {
        return Err(Abort::Rolled(ChallengeStatus::Consumed));
    }
    let user = user_of(tx, challenge.user)?;
    let mut version = user.session_version;
    let mut codes = None;
    let mut cookies = Vec::new();
    let (evidence, fresh, remembered_before) = match &how {
        How::Proof(proof) => {
            // The requirement may have changed since the challenge was made.
            if policy.is_required_at_login(tx, user.id)?
                && !policy.is_satisfied(proof.fresh, Some(proof.evidence))
            {
                return Err(Abort::Rolled(ChallengeStatus::SecurityChanged));
            }
            (
                proof.evidence.to_string(),
                proof.fresh,
                proof.trusted_device.is_some(),
            )
        }
        How::Enrolled { method_type, .. } => {
            version =
                recovery::finish_enrollment(tx, user.id, challenge.purpose == PURPOSE_RECOVERY)?;
            codes = recovery_codes::regenerate_if_missing(tx, user.id, now)?;
            (method_type.to_string(), true, false)
        }
    };
    let alert = match &how {
        How::Enrolled {
            alert_to: Some(address),
            ..
        } => Some(address.clone()),
        _ => events::verified_email(tx, user.id)?,
    };
    // A remembered device is not remembered again by being used.
    let mut trusted_expires = None;
    let issued = if remember && !remembered_before {
        let issued = trusted_devices::issue(tx, &context, messages, user.id, &policy, now)?;
        events::record_from(
            tx,
            &context,
            user.id,
            events::TRUSTED_DEVICE_ADDED,
            &issued.name,
            None,
        )?;
        trusted_expires = Some(issued.expires_at);
        Some(issued)
    } else {
        None
    };
    if let How::Enrolled { method_type, .. } = &how {
        events::record_from(
            tx,
            &context,
            user.id,
            events::METHOD_ADDED,
            method_type,
            alert.as_deref(),
        )?;
    }
    let mut session = NewSession::new(
        user.id,
        context.device(messages)?,
        if own.ip_address.is_empty() {
            "unknown".to_string()
        } else {
            own.ip_address.clone()
        },
        &challenge.primary_source,
        version,
        challenge.short_lived,
    );
    // Only a proof that is one now makes the session one that passed a second
    // step; a remembered device is recorded as what it was, and no more.
    session.mfa_completed_at = fresh.then_some(session.created_at);
    session.mfa_evidence = Some(evidence.clone());
    session.insert(tx)?;
    let kind = match &how {
        How::Proof(proof) => proof.event_kind(),
        How::Enrolled { .. } => events::MFA_SUCCEEDED,
    };
    events::record_from(tx, &context, user.id, kind, &evidence, alert.as_deref())?;
    cookies.push(session::sign_in_cookie(
        state.sealer(),
        &state.settings().environment,
        own.https,
        user.id,
        version,
        session.id,
        challenge.short_lived,
    ));
    if let Some(issued) = issued {
        cookies.push(issued.cookie);
    }
    cookies.push(flow::clear_preauth_cookie(own.https));
    Ok(Completed {
        user: user.id,
        session: session.id,
        security_version: version,
        short_lived: challenge.short_lived,
        evidence,
        trusted_device_expires_at: trusted_expires,
        recovery_codes: codes,
        pending_identity: challenge.pending_identity,
        cookies,
    })
}

// ---- answering a challenge that waits for a second step ----

/// What a person sent to answer the challenge of a sign-in, or of a request to
/// prove a second step again.
pub enum Answer<'a> {
    /// "This browser is remembered."
    TrustedDevice,
    /// A code; for the method named, or a recovery code without one.
    Code { method: Option<Id>, code: &'a str },
    /// A passkey's signed answer.
    Passkey {
        relying_party: &'a RelyingParty,
        response: Option<&'a AssertionResponse>,
    },
}

enum OwnedAnswer {
    TrustedDevice,
    Code {
        method: Option<Id>,
        code: String,
    },
    Passkey {
        relying_party: RelyingParty,
        response: Option<AssertionResponse>,
    },
}

/// What answering a challenge came to.
#[must_use]
pub enum Answered<R> {
    /// The answer was right and the sign-in is complete. `hook` is what the
    /// caller's own writes returned.
    SignedIn { completed: Completed, hook: R },
    /// Wrong. An attempt was spent and an event written, and that is kept.
    Refused,
    /// The account has failed too often lately (a 429).
    TooManyFailures,
    /// The method asked for is not there for this challenge (a 409).
    MethodUnavailable,
    /// No code was sent (a validation problem).
    EmptyCode,
    /// The challenge cannot be answered: see [`ChallengeStatus`]; a request
    /// that ran out of attempts is a 429 and the rest a 401.
    Invalid(ChallengeStatus),
}

/// Answers the challenge of a sign-in or of a request to prove a second step
/// again: checks the proof, and on a right one finishes the sign-in with a new
/// session (a request to prove again gets a new session too, and the old one is
/// left as it was). `remember` asks to remember the browser; it is honoured
/// only for a proof that is not itself a remembered device. `hook` runs the
/// caller's writes in the same transaction, such as linking an identity the
/// sign-in was asked to link.
pub async fn answer<R, H>(
    state: &AppState,
    ask: &Ask<'_>,
    answer: Answer<'_>,
    remember: bool,
    hook: H,
) -> Result<Answered<R>, DbError>
where
    R: Send + 'static,
    H: FnOnce(&Transaction, &Completed) -> Result<R, DbError> + Send + 'static,
{
    let own = Own::of(ask);
    let state_in = state.clone();
    let answer = match answer {
        Answer::TrustedDevice => OwnedAnswer::TrustedDevice,
        Answer::Code { method, code } => OwnedAnswer::Code {
            method,
            code: code.trim().to_string(),
        },
        Answer::Passkey {
            relying_party,
            response,
        } => OwnedAnswer::Passkey {
            relying_party: relying_party.clone(),
            response: response.cloned(),
        },
    };
    let done = run(state, move |tx| {
        let now = Timestamp::now();
        let challenge = match open(tx, &own, mfa::answers_with_a_code, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(Answered::Invalid(status)),
        };
        let context = own.context();
        let sealer = state_in.sealer();
        let outcome = match &answer {
            OwnedAnswer::TrustedDevice => mfa::verify(
                tx,
                sealer,
                &context,
                &challenge,
                Request::TrustedDevice,
                now,
            )?,
            OwnedAnswer::Code { method, code } => mfa::verify(
                tx,
                sealer,
                &context,
                &challenge,
                Request::Code {
                    method: *method,
                    code,
                },
                now,
            )?,
            OwnedAnswer::Passkey {
                relying_party,
                response,
            } => passkeys::verify_second_step(
                tx,
                relying_party,
                &context,
                &challenge,
                response.as_ref(),
                now,
            )?,
        };
        match outcome {
            Outcome::Proved(proof) => {
                let completed = finish(
                    tx,
                    &state_in,
                    &own,
                    &challenge,
                    How::Proof(&proof),
                    remember,
                    now,
                )?;
                let hook = hook(tx, &completed)?;
                Ok(Answered::SignedIn { completed, hook })
            }
            Outcome::Refused => Ok(Answered::Refused),
            Outcome::TooManyFailures => Ok(Answered::TooManyFailures),
            Outcome::MethodUnavailable => Ok(Answered::MethodUnavailable),
            Outcome::EmptyCode => Ok(Answered::EmptyCode),
            Outcome::WrongChallenge => Ok(Answered::Invalid(ChallengeStatus::Missing)),
        }
    })
    .await?;
    Ok(done.unwrap_or_else(Answered::Invalid))
}

/// What asking for a passkey's options for a challenge came to.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum PasskeyOptions {
    /// What to send to the browser; they are kept on the challenge.
    Options(String),
    /// The account has no passkey (a 409).
    Unavailable,
    Invalid(ChallengeStatus),
}

/// The options a passkey answers a challenge's second step with.
pub async fn passkey_options(
    state: &AppState,
    ask: &Ask<'_>,
    relying_party: &RelyingParty,
) -> Result<PasskeyOptions, DbError> {
    let own = Own::of(ask);
    let relying_party = relying_party.clone();
    let done = run(state, move |tx| {
        let challenge = match open(tx, &own, mfa::answers_with_a_code, Timestamp::now())? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(PasskeyOptions::Invalid(status)),
        };
        Ok(
            match passkeys::begin_second_step(tx, &relying_party, &challenge)? {
                Begin::Options(json) => PasskeyOptions::Options(json),
                Begin::Unavailable => PasskeyOptions::Unavailable,
                Begin::WrongChallenge => PasskeyOptions::Invalid(ChallengeStatus::Missing),
            },
        )
    })
    .await?;
    Ok(done.unwrap_or_else(PasskeyOptions::Invalid))
}

// ---- adding a second step ----

/// What a call that sets a second step up came to.
#[must_use]
pub enum Enrolled<R> {
    /// The step was added and the sign-in finished (the person who was asked
    /// to set one up is signed in, with new recovery codes if they had none).
    Done {
        completed: Completed,
        hook: R,
    },
    /// Wrong code or answer. An attempt was spent and an event written.
    Refused,
    TooManyFailures,
    /// There is nothing to confirm for what was named (a 409).
    Unavailable,
    /// The name is too long (a validation problem); nothing was spent.
    InvalidName,
    Invalid(ChallengeStatus),
}

fn enrolling(challenge: &StoredChallenge) -> bool {
    mfa::enrols(challenge)
}

/// What starting an authenticator app came to.
#[must_use]
pub enum TotpStarted {
    /// The method to confirm, the secret to show once, the address a QR code
    /// carries.
    Started {
        method: Id,
        secret: String,
        uri: String,
    },
    InvalidName,
    Invalid(ChallengeStatus),
}

impl std::fmt::Debug for TotpStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TotpStarted::Started { method, .. } => f
                .debug_struct("TotpStarted::Started")
                .field("method", method)
                .finish_non_exhaustive(),
            TotpStarted::InvalidName => f.write_str("TotpStarted::InvalidName"),
            TotpStarted::Invalid(status) => write!(f, "TotpStarted::Invalid({status:?})"),
        }
    }
}

/// Starts adding an authenticator app to the account of an enrolment
/// challenge.
pub async fn start_totp(
    state: &AppState,
    ask: &Ask<'_>,
    requested_name: Option<&str>,
) -> Result<TotpStarted, DbError> {
    let own = Own::of(ask);
    let state_in = state.clone();
    let requested = requested_name.map(str::to_string);
    let done = run(state, move |tx| {
        let now = Timestamp::now();
        let challenge = match open(tx, &own, enrolling, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(TotpStarted::Invalid(status)),
        };
        let fallback = text(&state_in, MessageKey::SecurityDefaultTotpName)?;
        let Some(name) = mfa::method_name(requested.as_deref(), &fallback) else {
            return Ok(TotpStarted::InvalidName);
        };
        let user = user_of(tx, challenge.user)?;
        Ok(
            match mfa::start_totp_enrollment(
                tx,
                state_in.sealer(),
                &challenge,
                &user.username,
                &name,
                now,
            )? {
                Some((method, secret, uri)) => TotpStarted::Started {
                    method,
                    secret,
                    uri,
                },
                None => TotpStarted::Invalid(ChallengeStatus::Missing),
            },
        )
    })
    .await?;
    Ok(done.unwrap_or_else(TotpStarted::Invalid))
}

/// The first code of an authenticator app.
pub struct TotpConfirmation<'a> {
    pub method: Option<Id>,
    pub name: Option<&'a str>,
    pub code: &'a str,
}

/// Confirms an authenticator app with its first code and finishes the sign-in
/// of the enrolment.
pub async fn confirm_totp<R, H>(
    state: &AppState,
    ask: &Ask<'_>,
    confirmation: TotpConfirmation<'_>,
    remember: bool,
    hook: H,
) -> Result<Enrolled<R>, DbError>
where
    R: Send + 'static,
    H: FnOnce(&Transaction, &Completed) -> Result<R, DbError> + Send + 'static,
{
    let own = Own::of(ask);
    let state_in = state.clone();
    let (method, requested, code) = (
        confirmation.method,
        confirmation.name.map(str::to_string),
        confirmation.code.to_string(),
    );
    let done = run(state, move |tx| {
        let now = Timestamp::now();
        let challenge = match open(tx, &own, enrolling, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(Enrolled::Invalid(status)),
        };
        let Some(method) = method else {
            return Ok(Enrolled::Unavailable);
        };
        let Some(pending) = mfa_methods::find_pending(tx, challenge.user, method, TOTP)? else {
            return Ok(Enrolled::Unavailable);
        };
        let Some(name) = mfa::method_name(requested.as_deref(), &pending.name) else {
            return Ok(Enrolled::InvalidName);
        };
        let first = FirstCode {
            method,
            name: Some(&name),
            code: &code,
        };
        let context = own.context();
        match mfa::confirm_totp_enrollment(tx, state_in.sealer(), &context, &challenge, first, now)?
        {
            TotpEnrollment::Confirmed => {
                let completed = finish(
                    tx,
                    &state_in,
                    &own,
                    &challenge,
                    How::Enrolled {
                        method_type: TOTP,
                        alert_to: None,
                    },
                    remember,
                    now,
                )?;
                let hook = hook(tx, &completed)?;
                Ok(Enrolled::Done { completed, hook })
            }
            TotpEnrollment::Unavailable => Ok(Enrolled::Unavailable),
            TotpEnrollment::TooManyFailures => Ok(Enrolled::TooManyFailures),
            TotpEnrollment::Refused => Ok(Enrolled::Refused),
            // The right code, spent by another request first.
            TotpEnrollment::Lost => Err(Abort::Rolled(ChallengeStatus::SecurityChanged)),
            TotpEnrollment::WrongChallenge => Ok(Enrolled::Invalid(ChallengeStatus::Missing)),
        }
    })
    .await?;
    Ok(done.unwrap_or_else(Enrolled::Invalid))
}

/// Mails the code of a sign-in's second step for one of the account's mail
/// methods (see [`super::email_send`]).
pub async fn send_login_code(
    state: &AppState,
    ask: &Ask<'_>,
    method: Option<Id>,
) -> Result<super::email_send::Sent, DbError> {
    super::email_send::login_code(state, ask, method).await
}

/// Mails the code that proves the account's address, to add it as a second
/// step.
pub async fn send_enrollment_code(
    state: &AppState,
    ask: &Ask<'_>,
) -> Result<super::email_send::Sent, DbError> {
    super::email_send::enrollment_code(state, ask).await
}

/// Adds the account's address as a second step with the code that was mailed
/// to it, and finishes the sign-in of the enrolment.
pub async fn confirm_email<R, H>(
    state: &AppState,
    ask: &Ask<'_>,
    requested_name: Option<&str>,
    code: &str,
    remember: bool,
    hook: H,
) -> Result<Enrolled<R>, DbError>
where
    R: Send + 'static,
    H: FnOnce(&Transaction, &Completed) -> Result<R, DbError> + Send + 'static,
{
    let own = Own::of(ask);
    let state_in = state.clone();
    let (requested, code) = (requested_name.map(str::to_string), code.to_string());
    let done = run(state, move |tx| {
        let now = Timestamp::now();
        let challenge = match open(tx, &own, enrolling, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(Enrolled::Invalid(status)),
        };
        let context = own.context();
        let (checked, normalized, address) = match mfa::check_email_enrollment(
            tx,
            state_in.sealer(),
            &context,
            &challenge,
            &code,
            now,
        )? {
            EmailEnrollment::Checked(checked, normalized, address) => {
                (checked, normalized, address)
            }
            EmailEnrollment::Refused => return Ok(Enrolled::Refused),
            EmailEnrollment::TooManyFailures => return Ok(Enrolled::TooManyFailures),
            EmailEnrollment::Unavailable => return Ok(Enrolled::Unavailable),
            EmailEnrollment::WrongChallenge => {
                return Ok(Enrolled::Invalid(ChallengeStatus::Missing));
            }
        };
        let fallback = text(&state_in, MessageKey::SecurityDefaultEmailName)?;
        let Some(name) = mfa::method_name(requested.as_deref(), &fallback) else {
            // Checking a right code wrote nothing: the code is still good.
            return Ok(Enrolled::InvalidName);
        };
        if !mfa::finish_email_enrollment(tx, &challenge, &checked, &normalized, &name, now)? {
            return Err(Abort::Rolled(ChallengeStatus::SecurityChanged));
        }
        let completed = finish(
            tx,
            &state_in,
            &own,
            &challenge,
            How::Enrolled {
                method_type: EMAIL,
                alert_to: Some(address),
            },
            remember,
            now,
        )?;
        let hook = hook(tx, &completed)?;
        Ok(Enrolled::Done { completed, hook })
    })
    .await?;
    Ok(done.unwrap_or_else(Enrolled::Invalid))
}

/// What asking for a passkey's creation options came to.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum PasskeyStarted {
    Options(String),
    Invalid(ChallengeStatus),
}

/// The options for adding a passkey to the account of an enrolment challenge.
pub async fn start_passkey(
    state: &AppState,
    ask: &Ask<'_>,
    relying_party: &RelyingParty,
) -> Result<PasskeyStarted, DbError> {
    let own = Own::of(ask);
    let relying_party = relying_party.clone();
    let done = run(state, move |tx| {
        let challenge = match open(tx, &own, enrolling, Timestamp::now())? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(PasskeyStarted::Invalid(status)),
        };
        let user = user_of(tx, challenge.user)?;
        Ok(
            match passkeys::begin_registration(
                tx,
                &relying_party,
                &challenge,
                &user.username,
                &user.display_name,
            )? {
                Some(json) => PasskeyStarted::Options(json),
                None => PasskeyStarted::Invalid(ChallengeStatus::Missing),
            },
        )
    })
    .await?;
    Ok(done.unwrap_or_else(PasskeyStarted::Invalid))
}

/// Adds a passkey with the answer to the options of an enrolment, and
/// finishes the sign-in of the enrolment.
pub async fn confirm_passkey<R, H>(
    state: &AppState,
    ask: &Ask<'_>,
    relying_party: &RelyingParty,
    requested_name: Option<&str>,
    response: Option<&RegistrationResponse>,
    remember: bool,
    hook: H,
) -> Result<Enrolled<R>, DbError>
where
    R: Send + 'static,
    H: FnOnce(&Transaction, &Completed) -> Result<R, DbError> + Send + 'static,
{
    let own = Own::of(ask);
    let state_in = state.clone();
    let relying_party = relying_party.clone();
    let requested = requested_name.map(str::to_string);
    let response = response.cloned();
    let done = run(state, move |tx| {
        let now = Timestamp::now();
        let challenge = match open(tx, &own, enrolling, now)? {
            Ok(challenge) => challenge,
            Err(status) => return Ok(Enrolled::Invalid(status)),
        };
        let fallback = text(&state_in, MessageKey::SecurityDefaultPasskeyName)?;
        let Some(name) = mfa::method_name(requested.as_deref(), &fallback) else {
            return Ok(Enrolled::InvalidName);
        };
        let context = own.context();
        let registration = match passkeys::check_registration(
            tx,
            &relying_party,
            &context,
            &challenge,
            response.as_ref(),
            now,
        )? {
            Registered::Ready(registration) => registration,
            Registered::Refused => return Ok(Enrolled::Refused),
            Registered::TooManyFailures => return Ok(Enrolled::TooManyFailures),
            Registered::WrongChallenge => return Ok(Enrolled::Invalid(ChallengeStatus::Missing)),
        };
        if passkeys::store(tx, &challenge, &name, &registration, now)?.is_none() {
            return Err(Abort::Rolled(ChallengeStatus::SecurityChanged));
        }
        let completed = finish(
            tx,
            &state_in,
            &own,
            &challenge,
            How::Enrolled {
                method_type: PASSKEY,
                alert_to: None,
            },
            remember,
            now,
        )?;
        let hook = hook(tx, &completed)?;
        Ok(Enrolled::Done { completed, hook })
    })
    .await?;
    Ok(done.unwrap_or_else(Enrolled::Invalid))
}

// ---- signing in with a passkey alone ----

/// The options of a sign-in with no password and the cookie that binds them to
/// the browser.
pub async fn passkey_sign_in_options(
    state: &AppState,
    relying_party: &RelyingParty,
    https: bool,
) -> Result<passkeys::SignInOptions, DbError> {
    let relying_party = relying_party.clone();
    state
        .database()
        .write(move |tx| passkeys::begin_sign_in(tx, &relying_party, https, Timestamp::now()))
        .await
}

/// What a sign-in with a passkey came to.
#[must_use]
pub enum PasskeySignedIn {
    /// Signed in with a session that counts as having passed a second step.
    SignedIn(Completed),
    /// Refused, whatever the reason; what was spent is spent and what was
    /// written is kept. `cookie` takes the options' cookie away.
    Rejected { cookie: String },
}

/// Signs in with a passkey alone. The answer spends its options, right or
/// wrong, and a refusal is committed as such.
pub async fn passkey_sign_in(
    state: &AppState,
    context: &Context<'_>,
    relying_party: &RelyingParty,
    cookie: Option<&(Id, String)>,
    response: Option<&AssertionResponse>,
    keep_signed_in: bool,
) -> Result<PasskeySignedIn, DbError> {
    let own = Own::of_context(context);
    let state_in = state.clone();
    let relying_party = relying_party.clone();
    let cookie = cookie.cloned();
    let response = response.cloned();
    state
        .database()
        .write(move |tx| -> Result<PasskeySignedIn, DbError> {
            let now = Timestamp::now();
            let context = own.context();
            let clear = super::security_cookies::clear_passkey(own.https);
            let signed = passkeys::sign_in(
                tx,
                &relying_party,
                &context,
                cookie.as_ref(),
                response.as_ref(),
                now,
            )?;
            let SignIn::Accepted { user } = signed else {
                return Ok(PasskeySignedIn::Rejected { cookie: clear });
            };
            let account = user_of(tx, user)?;
            let messages = state_in.messages();
            let mut session = NewSession::new(
                user,
                context.device(messages)?,
                if own.ip_address.is_empty() {
                    "unknown".to_string()
                } else {
                    own.ip_address.clone()
                },
                PASSKEY,
                account.session_version,
                !keep_signed_in,
            );
            session.mfa_completed_at = Some(session.created_at);
            session.mfa_evidence = Some(PASSKEY.to_string());
            session.insert(tx)?;
            let alert = events::verified_email(tx, user)?;
            events::record_from(
                tx,
                &context,
                user,
                events::LOGIN_SUCCEEDED,
                PASSKEY,
                alert.as_deref(),
            )?;
            let session_cookie = session::sign_in_cookie(
                state_in.sealer(),
                &state_in.settings().environment,
                own.https,
                user,
                account.session_version,
                session.id,
                !keep_signed_in,
            );
            Ok(PasskeySignedIn::SignedIn(Completed {
                user,
                session: session.id,
                security_version: account.session_version,
                short_lived: !keep_signed_in,
                evidence: PASSKEY.to_string(),
                trusted_device_expires_at: None,
                recovery_codes: None,
                pending_identity: None,
                cookies: vec![session_cookie, clear],
            }))
        })
        .await
}

// ---- changes the account makes to itself, from a session ----

/// What a change of the account's security settings came to when it was not
/// allowed.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum Changed<T> {
    Done(T),
    /// Not allowed now: the session has not passed a second step lately, or
    /// the sign-in is too old, or the account is disabled or waiting for
    /// recovery (a 403).
    NotAllowed,
    NotFound,
}

/// What taking a second step away came to.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum Removed {
    /// Done. `cookie` replaces the session cookie of the person who did it:
    /// every other session ended, and this one moved to the new version.
    Done {
        cookie: String,
    },
    NotAllowed,
    NotFound,
    /// The passkey is the account's last way in (a 409 with the code
    /// `last-way-in`).
    LastWayIn,
    /// The account must have a second step under the instance's policy (a
    /// 409).
    RequiredByPolicy,
}

/// Takes a second step away. Besides revoking the method it moves the account
/// to a new security version, ends every other session, forgets every
/// remembered device, spends the recovery codes if it was the last method, and
/// writes the event; the session that asked stays signed in.
pub async fn remove_method(
    state: &AppState,
    caller: Caller,
    context: &Context<'_>,
    method: Id,
    providers: &ways_in::OwnedProviders,
    password_proves: impl FnOnce() -> bool + Send + 'static,
) -> Result<Removed, DbError> {
    let own = Own::of_context(context);
    let state_in = state.clone();
    let providers = providers.clone();
    state
        .database()
        .write(move |tx| -> Result<Removed, DbError> {
            let now = Timestamp::now();
            let context = own.context();
            if !fresh::can_change_security(tx, caller.user, Some(caller.session), password_proves, now)? {
                return Ok(Removed::NotAllowed);
            }
            let found: Option<(String, bool)> = tx
                .query_row(
                    "SELECT \"Type\", 1 FROM \"MfaMethods\" WHERE \"Id\" = ?1 AND \"UserId\" = ?2 AND \"RevokedAt\" IS NULL",
                    params![method, caller.user],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((kind, _)) = found else {
                return Ok(Removed::NotFound);
            };
            if kind == PASSKEY
                && !ways_in::remain(tx, &providers.view(), caller.user, None, Some(method))?
            {
                return Ok(Removed::LastWayIn);
            }
            let remaining = mfa_methods::count_active_besides(tx, caller.user, method)?;
            let policy = SecurityPolicy::load(tx)?;
            if remaining == 0 && policy.is_required_at_login(tx, caller.user)? {
                return Ok(Removed::RequiredByPolicy);
            }
            let alert = events::verified_email(tx, caller.user)?;
            mfa_methods::revoke(tx, caller.user, method, now)?;
            if remaining == 0 {
                recovery_codes::spend_all(tx, caller.user, now)?;
            }
            // A new version ends every session and remembered device but this
            // session's.
            let version = recovery::end_other_sessions(tx, caller.user, caller.session, now)?;
            let (short_lived, absolute): (bool, Timestamp) = tx.query_row(
                "SELECT \"ShortLived\", \"AbsoluteExpiresAt\" FROM \"UserSessions\" WHERE \"Id\" = ?1",
                [caller.session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let renewed = now.plus(session::LIFETIME);
            tx.execute(
                "UPDATE \"UserSessions\" SET \"ExpiresAt\" = ?2 WHERE \"Id\" = ?1",
                params![caller.session, if absolute < renewed { absolute } else { renewed }],
            )?;
            events::record_from(tx, &context, caller.user, events::METHOD_REMOVED, &kind, alert.as_deref())?;
            Ok(Removed::Done {
                cookie: session::sign_in_cookie(
                    state_in.sealer(),
                    &state_in.settings().environment,
                    own.https,
                    caller.user,
                    version,
                    caller.session,
                    short_lived,
                ),
            })
        })
        .await
}

/// A new set of recovery codes, for an account that has a second step.
pub async fn regenerate_recovery_codes(
    state: &AppState,
    caller: Caller,
    context: &Context<'_>,
    password_proves: impl FnOnce() -> bool + Send + 'static,
) -> Result<Changed<Vec<String>>, DbError> {
    let own = Own::of_context(context);
    state
        .database()
        .write(move |tx| -> Result<Changed<Vec<String>>, DbError> {
            let now = Timestamp::now();
            if !fresh::can_change_security(
                tx,
                caller.user,
                Some(caller.session),
                password_proves,
                now,
            )? {
                return Ok(Changed::NotAllowed);
            }
            if !super::policy::has_confirmed_method(tx, caller.user)? {
                return Ok(Changed::NotFound);
            }
            let codes = recovery_codes::regenerate(tx, caller.user, now)?;
            let alert = events::verified_email(tx, caller.user)?;
            events::record_from(
                tx,
                &own.context(),
                caller.user,
                events::RECOVERY_CODES_REGENERATED,
                "",
                alert.as_deref(),
            )?;
            Ok(Changed::Done(codes))
        })
        .await
}

/// Takes one remembered device of the account back.
pub async fn forget_trusted_device(
    state: &AppState,
    caller: Caller,
    context: &Context<'_>,
    device: Id,
) -> Result<bool, DbError> {
    let own = Own::of_context(context);
    state
        .database()
        .write(move |tx| -> Result<bool, DbError> {
            let now = Timestamp::now();
            if !trusted_devices::revoke(tx, caller.user, device, now)? {
                return Ok(false);
            }
            let alert = events::verified_email(tx, caller.user)?;
            events::record_from(
                tx,
                &own.context(),
                caller.user,
                events::TRUSTED_DEVICE_REMOVED,
                "",
                alert.as_deref(),
            )?;
            Ok(true)
        })
        .await
}

/// Which challenge a session asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromSession {
    /// To set a second step up. The account needs a fresh proof to do it (see
    /// [`fresh::can_change_security`]).
    Enrollment,
    /// To pass a second step again, for a change that asks for a fresh one.
    Reverification,
}

/// What a challenge asked for from a session came to.
#[must_use]
pub enum Started {
    /// The challenge and the cookie that binds it to this browser.
    Challenge {
        body: Box<flow::Challenge>,
        cookie: String,
    },
    /// The session may not (a 403): the account is disabled or waiting for
    /// recovery, the session is not good, or the proof it needs is not fresh.
    NotAllowed,
    /// There is no second step to pass again (a 409).
    NoMethods,
}

/// Starts a challenge for the session that is signed in: its own browser holds
/// the cookie, and the session must still be good when it is completed.
pub async fn start_from_session(
    state: &AppState,
    caller: Caller,
    context: &Context<'_>,
    kind: FromSession,
    password_proves: impl FnOnce() -> bool + Send + 'static,
) -> Result<Started, DbError> {
    let own = Own::of_context(context);
    state
        .database()
        .write(move |tx| -> Result<Started, DbError> {
            let now = Timestamp::now();
            let context = own.context();
            let user = user_of(tx, caller.user)?;
            let allowed = match kind {
                FromSession::Enrollment => fresh::can_change_security(
                    tx,
                    caller.user,
                    Some(caller.session),
                    password_proves,
                    now,
                )?,
                FromSession::Reverification => {
                    !user.is_disabled
                        && !user.security_recovery_required
                        && fresh::current_session(tx, &user, caller.session, now)?.is_some()
                }
            };
            let Some(current) =
                fresh::current_session(tx, &user, caller.session, now)?.filter(|_| allowed)
            else {
                return Ok(Started::NotAllowed);
            };
            if kind == FromSession::Reverification
                && !super::policy::has_confirmed_method(tx, user.id)?
            {
                return Ok(Started::NoMethods);
            }
            let purpose = match kind {
                FromSession::Enrollment => PURPOSE_ENROLLMENT,
                FromSession::Reverification => PURPOSE_REVERIFY,
            };
            let policy = SecurityPolicy::load(tx)?;
            let (body, cookie) = flow::start_challenge(
                tx,
                &context,
                &user,
                &current.primary_source,
                purpose,
                &policy,
                flow::Options {
                    short_lived: current.short_lived,
                    parent_session: Some(caller.session),
                    ..flow::Options::default()
                },
            )?;
            Ok(Started::Challenge {
                body: Box::new(body),
                cookie,
            })
        })
        .await
}

/// What redeeming an operator's recovery credential came to.
#[must_use]
pub enum Redeemed {
    /// The challenge that lets the person set up a new second step and the
    /// cookie that binds it to this browser.
    Challenge {
        body: Box<flow::Challenge>,
        cookie: String,
    },
    /// For whatever reason, said the same way.
    Invalid,
}

/// Exchanges the credential an operator handed over for the enrolment
/// challenge of a recovered account, in one transaction.
pub async fn redeem_recovery_credential(
    state: &AppState,
    context: &Context<'_>,
    credential: &str,
    keep_signed_in: bool,
) -> Result<Redeemed, DbError> {
    let own = Own::of_context(context);
    let credential = credential.to_string();
    state
        .database()
        .write(move |tx| -> Result<Redeemed, DbError> {
            let now = Timestamp::now();
            let context = own.context();
            let Some(user) = recovery::redeem(tx, &credential, now)? else {
                return Ok(Redeemed::Invalid);
            };
            let account = user_of(tx, user)?;
            let policy = SecurityPolicy::load(tx)?;
            let (body, cookie) = flow::start_challenge(
                tx,
                &context,
                &account,
                "cli-recovery",
                PURPOSE_RECOVERY,
                &policy,
                flow::Options {
                    short_lived: !keep_signed_in,
                    ..flow::Options::default()
                },
            )?;
            let alert = events::verified_email(tx, user)?;
            events::record_from(
                tx,
                &context,
                user,
                events::CLI_RECOVERY,
                "recovery credential redeemed",
                alert.as_deref(),
            )?;
            Ok(Redeemed::Challenge {
                body: Box::new(body),
                cookie,
            })
        })
        .await
}
