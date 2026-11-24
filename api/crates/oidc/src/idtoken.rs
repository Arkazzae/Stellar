//! Checking an ID token: the signature first, then every claim that decides
//! whether this token is for this sign-in.
//!
//! [`verify`] is a straight line of named steps. A step that fails returns
//! its own error, and nothing after it runs, so no claim is read from a
//! token whose signature was not proven.

use std::fmt;

use aws_lc_rs::digest;
use serde_json::{Map, Value};

use crate::b64;
use crate::discovery::Discovery;
use crate::flow::Flow;
use crate::jwks::KeySet;
use crate::jws::{self, Alg};
use crate::options::{PROVIDER_MFA_MAX_AGE, canonical_issuer};
use crate::profile::claim_values;

/// How far the token's own times may be off our clock, in seconds. This is
/// the tolerance the token library applies by default; the provider's
/// `auth_time` has a tighter one of its own.
pub const CLOCK_SKEW: i64 = 5 * 60;

const MAX_TOKEN_LEN: usize = 64 * 1024;
const MAX_HEADER_LEN: usize = 4 * 1024;
/// Longest issuer and subject the application stores, in UTF-16 units.
const MAX_ISSUER_UNITS: usize = 512;
const MAX_SUBJECT_UNITS: usize = 255;
const AUTH_TIME_SKEW: i64 = 60;

/// What the token must match: the sign-in it answers.
#[derive(Clone)]
pub struct Expect<'a> {
    /// The issuer the provider's discovery document names, compared exactly.
    pub issuer: &'a str,
    pub client_id: &'a str,
    /// The nonce this sign-in sent.
    pub nonce: &'a str,
    pub now: i64,
    /// Set when a fresh sign-in at the provider was demanded: the token must
    /// then carry an `auth_time` this recent, in seconds.
    pub fresh_within: Option<i64>,
    /// The access token that came with the ID token, for `at_hash`.
    pub access_token: Option<&'a str>,
}

impl fmt::Debug for Expect<'_> {
    // The nonce and the access token are secrets of this sign-in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Expect")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("now", &self.now)
            .field("fresh_within", &self.fresh_within)
            .finish_non_exhaustive()
    }
}

impl<'a> Expect<'a> {
    /// The expectations of the sign-in `flow` started, for a token the
    /// provider's token endpoint returned together with `access_token`.
    pub fn for_flow(
        discovery: &'a Discovery,
        client_id: &'a str,
        flow: &'a Flow,
        access_token: Option<&'a str>,
        now: i64,
    ) -> Self {
        Self {
            issuer: &discovery.issuer,
            client_id,
            nonce: &flow.nonce,
            now,
            fresh_within: flow.demand_fresh_sign_in.then_some(PROVIDER_MFA_MAX_AGE),
            access_token,
        }
    }
}

/// A token whose every check passed.
#[derive(Debug, Clone, PartialEq)]
pub struct IdToken {
    /// The issuer as the token spells it.
    pub issuer: String,
    /// The subject, trimmed.
    pub subject: String,
    pub claims: Map<String, Value>,
}

impl IdToken {
    /// The issuer as it is stored: without a trailing slash.
    pub fn stored_issuer(&self) -> &str {
        canonical_issuer(&self.issuer)
    }

    /// The `auth_time` claim as text, the way the sign-in rules read it.
    pub fn auth_time(&self) -> Option<String> {
        crate::profile::claim_text(self.claims.get("auth_time")?)
    }

    /// Every `amr` value, whether the claim is a list or one string.
    pub fn amr(&self) -> Vec<String> {
        self.claims.get("amr").map(claim_values).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdTokenError {
    TooLarge,
    /// Not three dot-separated base64url parts, a header or payload that is
    /// not a JSON object, a key id that is not text, or a time claim (`exp`,
    /// `nbf`, `iat`, `auth_time`) that is not a number.
    Malformed,
    /// The header names no algorithm.
    MissingAlgorithm,
    /// The header says the token is not signed.
    Unsigned,
    /// An algorithm this crate does not verify, including every symmetric one.
    UnsupportedAlgorithm,
    /// The header carries a critical extension that is not understood.
    CriticalHeader,
    /// No key in the set has the token's key id. The set may be stale: fetch it once more.
    UnknownKeyId,
    /// The keys that could apply cannot verify the token's algorithm.
    NoUsableKey,
    BadSignature,
    WrongIssuer,
    WrongAudience,
    WrongAuthorizedParty,
    MissingExpiry,
    MissingIssuedAt,
    Expired,
    NotYetValid,
    /// `nbf` is after `exp`.
    InvalidLifetime,
    MissingNonce,
    WrongNonce,
    /// No subject, or an issuer or subject longer than the application stores.
    InvalidIdentity,
    /// A fresh sign-in was demanded and `auth_time` is missing, unreadable or too old.
    NotFreshSignIn,
    /// `at_hash` does not match the access token.
    AccessTokenHashMismatch,
}

impl fmt::Display for IdTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the ID token is refused: {self:?}")
    }
}

impl std::error::Error for IdTokenError {}

/// Verifies `token` against the provider's `keys` and what `expect` says.
pub fn verify(token: &str, keys: &KeySet, expect: &Expect<'_>) -> Result<IdToken, IdTokenError> {
    use IdTokenError as E;

    // Step 1: shape and size.
    if token.len() > MAX_TOKEN_LEN {
        return Err(E::TooLarge);
    }
    let parts: Vec<&str> = token.split('.').collect();
    let [header_part, payload_part, signature_part] = parts[..] else {
        return Err(E::Malformed);
    };
    if header_part.len() > MAX_HEADER_LEN {
        return Err(E::TooLarge);
    }

    // Step 2: the algorithm, from an allow-list.
    let header = json_object(header_part)?;
    let alg_name = header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or(E::MissingAlgorithm)?;
    if alg_name.eq_ignore_ascii_case("none") {
        return Err(E::Unsigned);
    }
    let alg = Alg::parse(alg_name).ok_or(E::UnsupportedAlgorithm)?;
    if header.contains_key("crit") {
        return Err(E::CriticalHeader);
    }

    // Step 3: the key. A key id must be text. A named key must exist; when
    // none has the id, every key that fits the algorithm is tried, and if none
    // verifies the key set may be stale. A token without a key id is checked
    // against every key that fits.
    let kid = match header.get("kid") {
        None => None,
        Some(Value::String(kid)) => Some(kid.as_str()),
        Some(_) => return Err(E::Malformed),
    };
    let fits = |key: &&crate::jwks::Jwk| alg.fits(key) && key.alg.is_none_or(|own| own == alg);
    let named: Vec<_> = keys
        .keys()
        .iter()
        .filter(|key| key.kid.as_deref() == kid)
        .collect();
    let unknown_kid = kid.is_some() && named.is_empty();
    let candidates: Vec<_> = if kid.is_some() && !unknown_kid {
        named.into_iter().filter(fits).collect()
    } else {
        keys.keys().iter().filter(fits).collect()
    };
    if candidates.is_empty() {
        return Err(if unknown_kid {
            E::UnknownKeyId
        } else {
            E::NoUsableKey
        });
    }

    // Step 4: the signature, over the text exactly as received.
    let signature = b64::decode(signature_part).ok_or(E::Malformed)?;
    let signed = &token[..header_part.len() + 1 + payload_part.len()];
    if !candidates
        .iter()
        .any(|key| jws::verify(key, alg, signed.as_bytes(), &signature))
    {
        return Err(if unknown_kid {
            E::UnknownKeyId
        } else {
            E::BadSignature
        });
    }

    // From here the claims are the provider's word.
    let claims = json_object(payload_part)?;
    check_claims(&claims, alg, expect)?;

    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    Ok(IdToken {
        issuer,
        subject,
        claims,
    })
}

fn check_claims(
    claims: &Map<String, Value>,
    alg: Alg,
    expect: &Expect<'_>,
) -> Result<(), IdTokenError> {
    use IdTokenError as E;
    let now = expect.now;

    // Step 5: issuer, compared exactly with the discovery document's.
    if claims.get("iss").and_then(Value::as_str) != Some(expect.issuer) {
        return Err(E::WrongIssuer);
    }

    // Step 6: audience, and the authorized party that goes with it.
    let audiences = match claims.get("aud") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !audiences.contains(&expect.client_id) {
        return Err(E::WrongAudience);
    }
    match claims.get("azp") {
        None | Some(Value::Null) => {}
        Some(Value::String(party)) if party.is_empty() || party == expect.client_id => {}
        Some(_) => return Err(E::WrongAuthorizedParty),
    }

    // Step 7: the times. `exp` and `iat` are required; a time of the wrong
    // type is a malformed token, not a late or early one.
    let expires = time_claim(claims, "exp")?.ok_or(E::MissingExpiry)?;
    time_claim(claims, "iat")?.ok_or(E::MissingIssuedAt)?;
    let not_before = time_claim(claims, "nbf")?;
    let auth_time = time_claim(claims, "auth_time")?;
    if expires < now - CLOCK_SKEW {
        return Err(E::Expired);
    }
    if let Some(not_before) = not_before {
        if not_before > expires {
            return Err(E::InvalidLifetime);
        }
        if not_before > now + CLOCK_SKEW {
            return Err(E::NotYetValid);
        }
    }

    // Step 8: the nonce this sign-in sent.
    match claims.get("nonce").and_then(Value::as_str) {
        None => return Err(E::MissingNonce),
        Some(nonce) if nonce != expect.nonce => return Err(E::WrongNonce),
        Some(_) => {}
    }

    // Step 9: an identity the application can store.
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let issuer = canonical_issuer(expect.issuer);
    if subject.is_empty()
        || utf16_len(subject) > MAX_SUBJECT_UNITS
        || issuer.is_empty()
        || utf16_len(issuer) > MAX_ISSUER_UNITS
    {
        return Err(E::InvalidIdentity);
    }

    // Step 10: a fresh sign-in at the provider, when one was demanded.
    if let Some(window) = expect.fresh_within {
        let fresh = auth_time
            .is_some_and(|at| at <= now + AUTH_TIME_SKEW && at >= now - window - AUTH_TIME_SKEW);
        if !fresh {
            return Err(E::NotFreshSignIn);
        }
    }

    // Step 11: the access token, when the token vouches for it.
    if let (Some(at_hash), Some(access_token)) = (claims.get("at_hash"), expect.access_token) {
        let hash = digest::digest(alg.digest(), access_token.as_bytes());
        let half = &hash.as_ref()[..hash.as_ref().len() / 2];
        if at_hash.as_str() != Some(b64::encode(half).as_str()) {
            return Err(E::AccessTokenHashMismatch);
        }
    }
    Ok(())
}

fn json_object(part: &str) -> Result<Map<String, Value>, IdTokenError> {
    let bytes = b64::decode(part).ok_or(IdTokenError::Malformed)?;
    match serde_json::from_slice(&bytes) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(IdTokenError::Malformed),
    }
}

/// A time claim in whole seconds, fractions cut off. `Ok(None)` when absent;
/// a claim that is not a number is a malformed token.
fn time_claim(claims: &Map<String, Value>, name: &str) -> Result<Option<i64>, IdTokenError> {
    let Some(value) = claims.get(name) else {
        return Ok(None);
    };
    let number = value.as_number().ok_or(IdTokenError::Malformed)?;
    number
        .as_i64()
        .or_else(|| {
            let float = number.as_f64()?;
            (float.is_finite() && float.abs() < 1e15).then(|| float.floor() as i64)
        })
        .map(Some)
        .ok_or(IdTokenError::Malformed)
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
