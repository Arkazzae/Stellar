//! One sign-in round trip: the request that sends the person to the provider,
//! the sealed state that comes back with the answer, and the checks on that
//! answer before anything is trusted.
//!
//! What a browser must carry between the two halves is a *binding*: a random
//! value the application puts in a cookie when the request is made and reads
//! back when the answer arrives. The state is sealed to it, so a state taken
//! to another browser does not open. The application clears the cookie once
//! the answer is handled.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use aws_lc_rs::{digest, rand};
use serde::{Deserialize, Serialize};

use crate::b64;
use crate::discovery::Discovery;
use crate::form;
use crate::options::{PROVIDER_MFA_MAX_AGE, Provider, safe_return_url};
use crate::sealed::{SealingKey, open, seal};

/// How long a sign-in may take from the redirect to the answer, in seconds.
pub const STATE_LIFETIME: i64 = 15 * 60;

const PURPOSE: &str = "stellarr.oidc.state.v1";
const SCOPE: &str = "openid profile email";
const MAX_REDIRECT_URI: usize = 2048;
const MAX_BINDING: usize = 128;
const MAX_STATE: usize = 4 * 1024;
const MAX_CODE: usize = 8 * 1024;
/// Most flows remembered as used at once. A full guard refuses new answers
/// rather than forget one, so the limit cannot be used to replay a state.
const MAX_REMEMBERED: usize = 100_000;

/// The signed-in session a round trip is made for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub user_id: String,
    pub session_id: String,
    pub security_version: u32,
}

/// What the answer is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Purpose {
    /// Sign the person in, or keep the identity for the sign-in page.
    SignIn,
    /// Link the identity to the account of a session that asked.
    Link(Actor),
    /// Count a fresh sign-in as the proof the session asked for.
    Reauthenticate(Actor),
}

/// Everything a sign-in remembers between the redirect and the answer. It
/// travels sealed inside the `state` parameter.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub provider_id: String,
    pub purpose: Purpose,
    /// Where to send the person afterwards; always a path on this origin.
    pub return_url: String,
    /// The person chose not to stay signed in.
    pub short_lived: bool,
    /// Sent to the provider and sent again to the token endpoint, which
    /// compares them.
    pub redirect_uri: String,
    pub nonce: String,
    pub code_verifier: String,
    /// A fresh sign-in at the provider was demanded, so a token that shows
    /// none is refused.
    pub demand_fresh_sign_in: bool,
    pub state_id: String,
    /// Unix seconds.
    pub expires_at: i64,
}

impl fmt::Debug for Flow {
    // The nonce and the verifier are secrets of this sign-in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flow")
            .field("provider_id", &self.provider_id)
            .field("purpose", &self.purpose)
            .field("return_url", &self.return_url)
            .finish_non_exhaustive()
    }
}

/// What to do to begin a sign-in.
#[derive(Debug, Clone)]
pub struct Start<'a> {
    pub provider: &'a Provider,
    pub discovery: &'a Discovery,
    /// The absolute address of the provider's callback on this server.
    pub redirect_uri: &'a str,
    pub purpose: Purpose,
    /// Cleaned here: anything that leaves this origin becomes `/`.
    pub return_url: Option<&'a str>,
    pub short_lived: bool,
    pub now: i64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Started {
    /// Where to redirect the browser.
    pub redirect_to: String,
    /// The value for the cookie that ties the answer to this browser. The
    /// provider answers with a cross-site POST, so the cookie must be
    /// `SameSite=None; Secure` (and `HttpOnly`) to come along with it.
    pub browser_binding: String,
}

impl fmt::Debug for Started {
    // The binding and the address (which holds the nonce) are secrets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Started").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// The redirect address is not an absolute http(s) address without a fragment.
    BadRedirectUri,
    /// Random bytes or sealing failed.
    Unavailable,
    /// The sealed state would be longer than a callback may carry, even with
    /// the return address left out.
    TooLarge,
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadRedirectUri => "the redirect address is not usable",
            Self::Unavailable => "the sign-in state could not be made",
            Self::TooLarge => "the sign-in state is too large",
        })
    }
}

impl std::error::Error for StartError {}

fn random_text(bytes: usize) -> Result<String, StartError> {
    let mut buffer = vec![0u8; bytes];
    rand::fill(&mut buffer).map_err(|_| StartError::Unavailable)?;
    Ok(b64::encode(&buffer))
}

/// The PKCE challenge of a verifier: SHA-256, base64url.
pub fn code_challenge(verifier: &str) -> String {
    b64::encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref())
}

/// Builds the authorization request and the sealed state that goes with it.
pub fn start(key: &SealingKey, request: &Start<'_>) -> Result<Started, StartError> {
    let uri = request.redirect_uri;
    let scheme_ok = ["http://", "https://"].iter().any(|s| {
        uri.get(..s.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(s))
    });
    if !scheme_ok
        || uri.len() > MAX_REDIRECT_URI
        || uri.contains('#')
        || uri.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(StartError::BadRedirectUri);
    }
    let reauthenticate = matches!(request.purpose, Purpose::Reauthenticate(_));
    let mut flow = Flow {
        provider_id: request.provider.id.clone(),
        purpose: request.purpose.clone(),
        return_url: safe_return_url(request.return_url),
        short_lived: request.short_lived,
        redirect_uri: uri.to_string(),
        nonce: random_text(32)?,
        code_verifier: random_text(32)?,
        demand_fresh_sign_in: reauthenticate,
        state_id: random_text(16)?,
        expires_at: request.now.saturating_add(STATE_LIFETIME),
    };
    let binding = random_text(32)?;
    let seal_flow = |flow: &Flow| {
        serde_json::to_vec(flow)
            .ok()
            .and_then(|json| seal(key, PURPOSE, &binding, &json))
            .ok_or(StartError::Unavailable)
    };
    // A state its own callback would refuse is no use: leave the return
    // address out before giving up.
    let mut sealed = seal_flow(&flow)?;
    if sealed.len() > MAX_STATE {
        flow.return_url = "/".to_string();
        sealed = seal_flow(&flow)?;
        if sealed.len() > MAX_STATE {
            return Err(StartError::TooLarge);
        }
    }

    let challenge = code_challenge(&flow.code_verifier);
    let max_age = PROVIDER_MFA_MAX_AGE.to_string();
    let mut query = vec![
        ("client_id", request.provider.client_id()),
        ("redirect_uri", uri),
        ("response_type", "code"),
        ("scope", SCOPE),
        // The provider answers with a POST, which keeps the code out of
        // addresses and logs.
        ("response_mode", "form_post"),
        ("nonce", flow.nonce.as_str()),
        ("state", sealed.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    // A provider whose second step counts here is asked to sign the person in
    // again when its session is older, and every provider when that sign-in
    // is the proof.
    if request.provider.settings.trust_provider_mfa || reauthenticate {
        query.push(("max_age", max_age.as_str()));
    }
    // A sign-in asked for as proof asks for the credentials again even on a
    // new session at the provider. What it proves is still read from
    // `auth_time`, since a provider may not honour the prompt.
    if reauthenticate {
        query.push(("prompt", "login"));
    }
    let endpoint = &request.discovery.authorization_endpoint;
    let glue = if !endpoint.contains('?') {
        "?"
    } else if endpoint.ends_with(['?', '&']) {
        ""
    } else {
        "&"
    };
    Ok(Started {
        redirect_to: format!("{endpoint}{glue}{}", form::encode(&query)),
        browser_binding: binding,
    })
}

/// The answer the provider sent to the callback, as parameters of a query or
/// a posted form.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl fmt::Debug for Callback {
    // The code is a credential until it is redeemed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Callback")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackError {
    /// Not a form, too large, or a parameter given twice.
    Malformed,
    MissingState,
    /// The state is not one this server sealed for this browser, or was damaged.
    /// A state taken to another browser looks the same on purpose.
    UnreadableState,
    Expired,
    /// The state belongs to another provider's callback.
    WrongProvider,
    /// The state was already used.
    Replayed,
    /// The replay guard is full of live states and cannot take another.
    /// Nothing is wrong with this callback; the route that starts sign-ins
    /// must be rate-limited so this cannot be provoked.
    Busy,
    MissingCode,
}

impl fmt::Display for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the callback is refused: {self:?}")
    }
}

impl std::error::Error for CallbackError {}

impl Callback {
    /// Reads the query string or the posted body of the callback.
    pub fn parse(input: &str) -> Result<Self, CallbackError> {
        let pairs = form::parse(input).ok_or(CallbackError::Malformed)?;
        let mut callback = Callback::default();
        for (name, value) in pairs {
            let slot = match name.as_str() {
                "code" if value.len() <= MAX_CODE => &mut callback.code,
                "state" if value.len() <= MAX_STATE => &mut callback.state,
                "error" if value.len() <= 256 => &mut callback.error,
                "error_description" if value.len() <= 1024 => &mut callback.error_description,
                "code" | "state" | "error" | "error_description" => {
                    return Err(CallbackError::Malformed);
                }
                _ => continue,
            };
            if slot.replace(value).is_some() {
                return Err(CallbackError::Malformed);
            }
        }
        Ok(callback)
    }
}

/// Remembers the sign-ins already answered, until they would have expired
/// anyway, so a state is good for one answer. It lives in memory: a restart
/// forgets it, so a state answered just before a restart could be answered
/// again until it expires.
///
/// It holds at most 100,000 live states. The route that starts a sign-in
/// needs no account, so the application must rate-limit it; otherwise anyone
/// could fill the guard and make every answer `Busy`.
#[derive(Debug, Default)]
pub struct ReplayGuard {
    used: Mutex<HashMap<String, i64>>,
}

impl ReplayGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// True the first time an id is seen before it expires.
    fn consume(&self, id: &str, expires_at: i64, now: i64) -> Result<(), CallbackError> {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        if used.contains_key(id) {
            return Err(CallbackError::Replayed);
        }
        if used.len() >= MAX_REMEMBERED {
            used.retain(|_, until| *until > now);
            if used.len() >= MAX_REMEMBERED {
                return Err(CallbackError::Busy);
            }
        }
        used.insert(id.to_string(), expires_at);
        Ok(())
    }
}

/// What a valid callback says to do next.
#[derive(Clone, PartialEq, Eq)]
pub enum Answer {
    /// Redeem this code at the token endpoint.
    Code { code: String, flow: Flow },
    /// The provider refused or the person cancelled.
    Refused {
        error: String,
        description: Option<String>,
        /// The person said no, as opposed to something failing.
        cancelled: bool,
        flow: Flow,
    },
}

impl fmt::Debug for Answer {
    // The code is a credential until it is redeemed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code { flow, .. } => f
                .debug_struct("Code")
                .field("flow", flow)
                .finish_non_exhaustive(),
            Self::Refused {
                error,
                cancelled,
                flow,
                ..
            } => f
                .debug_struct("Refused")
                .field("error", error)
                .field("cancelled", cancelled)
                .field("flow", flow)
                .finish_non_exhaustive(),
        }
    }
}

/// Opens the callback's state and decides what the answer is. Each step is a
/// refusal of its own: the state must open for this browser, be unexpired,
/// be for this provider, and not have been used. An error from the provider
/// is read only after that, so it cannot be forged by anyone who could not
/// have started the sign-in.
pub fn accept(
    key: &SealingKey,
    guard: &ReplayGuard,
    provider_id: &str,
    browser_binding: &str,
    callback: &Callback,
    now: i64,
) -> Result<Answer, CallbackError> {
    accept_detailed(key, guard, provider_id, browser_binding, callback, now)
        .map_err(|(error, _)| error)
}

/// [`accept`], and for a refusal that came after the state was opened (it
/// expired, was for another provider, was used, or the answer had no code) the
/// flow it held, so that the application can send the person to the page that
/// fits what the round trip was for. The state opened only for the browser it
/// was made for, so nobody else learns anything from this; for a state that
/// did not open there is no flow.
pub fn accept_detailed(
    key: &SealingKey,
    guard: &ReplayGuard,
    provider_id: &str,
    browser_binding: &str,
    callback: &Callback,
    now: i64,
) -> Result<Answer, (CallbackError, Option<Flow>)> {
    let opened = |error: CallbackError| (error, None);
    let state = callback
        .state
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or(opened(CallbackError::MissingState))?;
    if browser_binding.is_empty() || browser_binding.len() > MAX_BINDING {
        return Err(opened(CallbackError::UnreadableState));
    }
    let json =
        open(key, PURPOSE, browser_binding, state).ok_or(opened(CallbackError::UnreadableState))?;
    let flow: Flow =
        serde_json::from_slice(&json).map_err(|_| opened(CallbackError::UnreadableState))?;
    if flow.expires_at <= now {
        return Err((CallbackError::Expired, Some(flow)));
    }
    if flow.provider_id != provider_id {
        return Err((CallbackError::WrongProvider, Some(flow)));
    }
    if let Err(error) = guard.consume(&flow.state_id, flow.expires_at, now) {
        return Err((error, Some(flow)));
    }
    // An empty `error` is no error.
    if let Some(error) = callback.error.as_ref().filter(|error| !error.is_empty()) {
        return Ok(Answer::Refused {
            cancelled: error == "access_denied",
            error: error.clone(),
            description: callback.error_description.clone(),
            flow,
        });
    }
    match callback.code.as_deref().filter(|code| !code.is_empty()) {
        Some(code) => Ok(Answer::Code {
            code: code.to_string(),
            flow,
        }),
        None => Err((CallbackError::MissingCode, Some(flow))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_is_good_for_one_answer() {
        let guard = ReplayGuard::new();
        assert_eq!(guard.consume("a", 100, 0), Ok(()));
        assert_eq!(guard.consume("a", 100, 0), Err(CallbackError::Replayed));
        assert_eq!(guard.consume("b", 100, 0), Ok(()));
    }

    #[test]
    fn a_full_guard_refuses_rather_than_forgets() {
        let guard = ReplayGuard::new();
        for i in 0..MAX_REMEMBERED {
            assert_eq!(guard.consume(&i.to_string(), 100, 0), Ok(()));
        }
        assert_eq!(guard.consume("one more", 100, 0), Err(CallbackError::Busy));
        assert_eq!(guard.consume("5", 100, 0), Err(CallbackError::Replayed));
        // Once the remembered ones have expired there is room again, and a
        // state that expired is refused by its own expiry, never by the guard.
        assert_eq!(guard.consume("later", 300, 200), Ok(()));
    }
}
