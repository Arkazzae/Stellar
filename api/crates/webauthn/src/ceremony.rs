//! What registration and authentication have in common: reading the fields a
//! browser sends, and the checks on authenticator data that both ceremonies
//! make. Each check is a function of its own so that the two ceremonies
//! visibly make the same one, and so that a test can aim at it.

use aws_lc_rs::digest;
use serde::Deserialize;

use crate::authdata::{AuthenticatorData, MAX_CREDENTIAL_ID_LEN};
use crate::base64url;
use crate::error::Error;
use crate::relying_party::RelyingParty;

/// Upper bounds on what the browser may send, checked on the text before any
/// decoding. They are generous for real data and small for hostile data.
pub const MAX_AUTHENTICATOR_DATA_LEN: usize = 16 * 1024;
pub const MAX_ATTESTATION_OBJECT_LEN: usize = 16 * 1024;
pub const MAX_SIGNATURE_LEN: usize = 1024;
pub const MAX_USER_HANDLE_LEN: usize = 64;

/// The `type`, `id` and `rawId` members both answers carry.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope<'a> {
    pub id: &'a str,
    pub raw_id: &'a str,
    pub kind: &'a str,
}

/// Decodes one base64url field after bounding its decoded size.
pub fn field(text: &str, name: &'static str, max_decoded: usize) -> Result<Vec<u8>, Error> {
    if text.len() > max_decoded.div_ceil(3) * 4 {
        return Err(Error::FieldTooLarge(name));
    }
    let bytes = base64url::decode(text).ok_or(Error::BadBase64(name))?;
    if bytes.len() > max_decoded {
        return Err(Error::FieldTooLarge(name));
    }
    Ok(bytes)
}

/// The credential id of an answer: the type is `public-key`, `rawId` decodes
/// and `id` is the same bytes.
pub fn credential_id(envelope: &Envelope<'_>) -> Result<Vec<u8>, Error> {
    if envelope.kind != "public-key" {
        return Err(Error::CredentialTypeNotPublicKey);
    }
    let raw = field(envelope.raw_id, "rawId", MAX_CREDENTIAL_ID_LEN)?;
    if raw.is_empty() {
        return Err(Error::CredentialIdLength);
    }
    let id = field(envelope.id, "id", MAX_CREDENTIAL_ID_LEN)?;
    if id != raw {
        return Err(Error::IdDoesNotMatchRawId);
    }
    Ok(raw)
}

/// The authenticator data is for this relying party: its id hashed with
/// SHA-256 is the first 32 bytes.
pub fn check_rp_id_hash(
    auth: &AuthenticatorData,
    relying_party: &RelyingParty,
) -> Result<(), Error> {
    let expected = digest::digest(&digest::SHA256, relying_party.id().as_bytes());
    if auth.rp_id_hash != expected.as_ref() {
        return Err(Error::RpIdHashMismatch);
    }
    Ok(())
}

pub fn check_user_present(auth: &AuthenticatorData) -> Result<(), Error> {
    if !auth.flags.user_present() {
        return Err(Error::UserNotPresent);
    }
    Ok(())
}

/// Required in every ceremony because the options always ask for it.
pub fn check_user_verified(auth: &AuthenticatorData) -> Result<(), Error> {
    if !auth.flags.user_verified() {
        return Err(Error::UserNotVerified);
    }
    Ok(())
}

/// A credential cannot be backed up unless it may be.
pub fn check_backup_flags(auth: &AuthenticatorData) -> Result<(), Error> {
    if auth.flags.backed_up() && !auth.flags.backup_eligible() {
        return Err(Error::BackupStateWithoutEligibility);
    }
    Ok(())
}
