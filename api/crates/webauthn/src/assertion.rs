//! Verifying the answer to `navigator.credentials.get`
//! (WebAuthn section 7.2).

use aws_lc_rs::constant_time;
use serde::Deserialize;

use crate::authdata::AuthenticatorData;
use crate::ceremony::{
    self, Envelope, MAX_AUTHENTICATOR_DATA_LEN, MAX_SIGNATURE_LEN, MAX_USER_HANDLE_LEN,
};
use crate::client_data::{self, Ceremony, MAX_CLIENT_DATA_LEN};
use crate::cose::PublicKey;
use crate::error::Error;
use crate::relying_party::RelyingParty;

/// What `navigator.credentials.get` returned, as the web client sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssertionResponse {
    pub id: String,
    pub raw_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub response: AssertionBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssertionBody {
    #[serde(rename = "authenticatorData")]
    pub authenticator_data: String,
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    pub signature: String,
    #[serde(rename = "userHandle", default)]
    pub user_handle: Option<String>,
}

/// The credential as the application stored it at registration and after the
/// last sign-in. It must already be the one that belongs to the account.
#[derive(Debug, Clone, Copy)]
pub struct StoredCredential<'a> {
    pub credential_id: &'a [u8],
    pub public_key: &'a [u8],
    pub user_handle: &'a [u8],
    pub sign_count: u32,
}

/// What the ceremony was started with.
#[derive(Debug, Clone, Copy)]
pub struct AssertionRequest<'a> {
    pub relying_party: &'a RelyingParty,
    pub challenge: &'a [u8],
    /// The credential ids the options named. Empty when they named none, as
    /// when signing in without saying who.
    pub allowed_credentials: &'a [&'a [u8]],
    /// Sign-in without a password has no other way to know the account, so
    /// the authenticator must send the handle.
    pub require_user_handle: bool,
}

/// What the application records after a good answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assertion {
    pub sign_count: u32,
    pub backed_up: bool,
    /// What the authenticator reported this time. A change from registration
    /// is not refused (the flags are signed, and an authenticator may report
    /// them differently after a migration); the application may store or log it.
    pub backup_eligible: bool,
}

/// Checks the answer to `navigator.credentials.get` against the stored
/// credential and the options the ceremony started with.
///
/// The caller must still: refuse a challenge that was used or has expired
/// (and spend it on the first attempt); replace the stored signature count
/// with a compare-and-set on the value it read, so two simultaneous answers
/// cannot both pass; set `require_user_handle` for sign-in without a user
/// name and pass the allowed list from the stored options; and look the
/// credential up by id only among credentials it has already tied to the
/// account (ids are unique across accounts).
pub fn verify_assertion(
    request: &AssertionRequest<'_>,
    stored: &StoredCredential<'_>,
    response: &AssertionResponse,
) -> Result<Assertion, Error> {
    let envelope = Envelope {
        id: &response.id,
        raw_id: &response.raw_id,
        kind: &response.kind,
    };
    let raw_id = ceremony::credential_id(&envelope)?;

    // Steps 5 and 6: the credential is one that was asked for and is the
    // stored one; the handle, when given, is the stored account's.
    check_credential_allowed(&raw_id, request.allowed_credentials)?;
    check_credential_is_stored(&raw_id, stored)?;
    check_user_handle(
        response.response.user_handle.as_deref(),
        stored,
        request.require_user_handle,
    )?;

    let authenticator_data = ceremony::field(
        &response.response.authenticator_data,
        "authenticatorData",
        MAX_AUTHENTICATOR_DATA_LEN,
    )?;
    let client_data = ceremony::field(
        &response.response.client_data_json,
        "clientDataJSON",
        MAX_CLIENT_DATA_LEN,
    )?;
    let signature = ceremony::field(&response.response.signature, "signature", MAX_SIGNATURE_LEN)?;

    // Steps 11 to 15: type, challenge, origin, cross-origin; then the hash.
    let client_data_hash = client_data::verify(
        &client_data,
        Ceremony::Get,
        request.challenge,
        request.relying_party,
    )?;

    // Steps 16 to 18.
    let auth = AuthenticatorData::parse(&authenticator_data)?;
    check_no_attested_data(&auth)?;
    ceremony::check_rp_id_hash(&auth, request.relying_party)?;
    ceremony::check_user_present(&auth)?;
    ceremony::check_user_verified(&auth)?;
    ceremony::check_backup_flags(&auth)?;

    // Steps 19 to 21: the signature over the data and the hash.
    let key = PublicKey::from_cose(stored.public_key).map_err(|_| Error::StoredKeyInvalid)?;
    check_signature(&key, &authenticator_data, &client_data_hash, &signature)?;

    // Step 22: the counter, only meaningful once the signature is trusted.
    check_counter(auth.sign_count, stored.sign_count)?;

    Ok(Assertion {
        sign_count: auth.sign_count,
        backed_up: auth.flags.backed_up(),
        backup_eligible: auth.flags.backup_eligible(),
    })
}

/// When the options named credentials, the answer is for one of them.
fn check_credential_allowed(raw_id: &[u8], allowed: &[&[u8]]) -> Result<(), Error> {
    if !allowed.is_empty() && !allowed.contains(&raw_id) {
        return Err(Error::CredentialNotAllowed);
    }
    Ok(())
}

fn check_credential_is_stored(raw_id: &[u8], stored: &StoredCredential<'_>) -> Result<(), Error> {
    if raw_id != stored.credential_id {
        return Err(Error::CredentialIdMismatch);
    }
    Ok(())
}

/// A handle that is sent must be the stored one, and a present but empty one
/// is a malformed answer. Whether it must be sent depends on the ceremony.
fn check_user_handle(
    sent: Option<&str>,
    stored: &StoredCredential<'_>,
    required: bool,
) -> Result<(), Error> {
    let Some(text) = sent else {
        return if required {
            Err(Error::UserHandleMissing)
        } else {
            Ok(())
        };
    };
    let handle = ceremony::field(text, "userHandle", MAX_USER_HANDLE_LEN)?;
    if handle.is_empty() {
        return Err(Error::UserHandleEmpty);
    }
    constant_time::verify_slices_are_equal(&handle, stored.user_handle)
        .map_err(|_| Error::UserHandleMismatch)
}

/// An assertion has no attested credential data.
fn check_no_attested_data(auth: &AuthenticatorData) -> Result<(), Error> {
    if auth.attested.is_some() {
        return Err(Error::AttestedDataUnexpected);
    }
    Ok(())
}

fn check_signature(
    key: &PublicKey,
    authenticator_data: &[u8],
    client_data_hash: &[u8; 32],
    signature: &[u8],
) -> Result<(), Error> {
    let mut signed = authenticator_data.to_vec();
    signed.extend_from_slice(client_data_hash);
    if !key.verify(&signed, signature) {
        return Err(Error::SignatureInvalid);
    }
    Ok(())
}

/// A counter that never moves (both zero) is what a synced passkey reports
/// and is allowed. Once either side counts, the new value must be larger: a
/// smaller or equal one is a sign of a cloned authenticator.
fn check_counter(received: u32, stored: u32) -> Result<(), Error> {
    if (received != 0 || stored != 0) && received <= stored {
        return Err(Error::CounterNotIncreased);
    }
    Ok(())
}
