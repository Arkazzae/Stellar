//! The client data the browser signs over (WebAuthn section 5.8.1) and the
//! checks every ceremony makes on it.

use aws_lc_rs::{constant_time, digest};
use serde::Deserialize;

use crate::base64url;
use crate::error::Error;
use crate::relying_party::RelyingParty;

/// Real client data is a few hundred bytes.
pub const MAX_CLIENT_DATA_LEN: usize = 8 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    cross_origin: Option<bool>,
    top_origin: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ceremony {
    Create,
    Get,
}

impl Ceremony {
    fn kind(self) -> &'static str {
        match self {
            Ceremony::Create => "webauthn.create",
            Ceremony::Get => "webauthn.get",
        }
    }
}

/// Parses `raw` and checks the type, the challenge, the origin and the
/// cross-origin flag, in the order the specification lists them. Returns the
/// SHA-256 hash of `raw`, which the signatures cover.
pub fn verify(
    raw: &[u8],
    ceremony: Ceremony,
    expected_challenge: &[u8],
    relying_party: &RelyingParty,
) -> Result<[u8; 32], Error> {
    let data: ClientData = serde_json::from_slice(raw).map_err(|_| Error::MalformedClientData)?;

    check_type(&data, ceremony)?;
    check_challenge(&data, expected_challenge)?;
    check_origin(&data, relying_party)?;
    check_cross_origin(&data)?;

    let hash = digest::digest(&digest::SHA256, raw);
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_ref());
    Ok(out)
}

fn check_type(data: &ClientData, ceremony: Ceremony) -> Result<(), Error> {
    if data.kind != ceremony.kind() {
        return Err(Error::ClientDataTypeMismatch);
    }
    Ok(())
}

/// Constant time, although a challenge is not a long-lived secret: there is
/// no reason to leak how much of a guess was right.
fn check_challenge(data: &ClientData, expected: &[u8]) -> Result<(), Error> {
    if expected.is_empty() {
        return Err(Error::ChallengeMismatch);
    }
    let sent = base64url::decode(&data.challenge).ok_or(Error::ChallengeMismatch)?;
    constant_time::verify_slices_are_equal(&sent, expected).map_err(|_| Error::ChallengeMismatch)
}

/// An exact string match against the allowed origins. A browser writes an
/// origin in one canonical form, so no normalising is done that could make
/// two different origins look alike.
fn check_origin(data: &ClientData, relying_party: &RelyingParty) -> Result<(), Error> {
    if !relying_party.origins().contains(&data.origin) {
        return Err(Error::OriginNotAllowed);
    }
    Ok(())
}

/// A ceremony run inside another site's frame is refused, and so is a
/// top-level origin, which only exists for such a frame.
fn check_cross_origin(data: &ClientData) -> Result<(), Error> {
    if data.cross_origin == Some(true) || data.top_origin.is_some() {
        return Err(Error::CrossOriginNotAllowed);
    }
    Ok(())
}
