//! Verifying the answer to `navigator.credentials.create`
//! (WebAuthn section 7.1).
//!
//! Attestation is not asked for, so the formats accepted are the two a
//! browser produces then: `none`, and `packed` self attestation, whose
//! signature is checked with the new credential's own key. A statement that
//! brings a certificate chain is refused rather than accepted unchecked.

use serde::Deserialize;

use crate::authdata::{AttestedCredential, AuthenticatorData};
use crate::cbor::{self, Value};
use crate::ceremony::{self, Envelope, MAX_ATTESTATION_OBJECT_LEN};
use crate::client_data::{self, Ceremony, MAX_CLIENT_DATA_LEN};
use crate::cose::Algorithm;
use crate::error::Error;
use crate::options::Transport;
use crate::relying_party::RelyingParty;

/// What `navigator.credentials.create` returned, as the web client sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationResponse {
    pub id: String,
    pub raw_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub response: AttestationBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttestationBody {
    #[serde(rename = "attestationObject")]
    pub attestation_object: String,
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    #[serde(default)]
    pub transports: Option<Vec<String>>,
}

/// What the application stores about the new credential.
#[derive(Debug, Clone)]
pub struct Registration {
    pub credential_id: Vec<u8>,
    /// The COSE key, to be handed back unchanged when an assertion is checked.
    pub public_key: Vec<u8>,
    pub algorithm: Algorithm,
    pub sign_count: u32,
    pub backup_eligible: bool,
    pub backed_up: bool,
    pub aaguid: [u8; 16],
    pub transports: Vec<Transport>,
}

/// The attestation object, once its three members are read.
struct AttestationObject {
    format: String,
    statement: Value,
    auth_data: Vec<u8>,
}

/// Checks the answer to `navigator.credentials.create` against the challenge
/// that was issued.
///
/// The caller must still: refuse a challenge that was used or has expired
/// (and spend it on the first attempt); make sure the returned credential id
/// is unique across all accounts, with a unique index and an atomic check at
/// insert, since sign-in finds a credential by its id alone; and store the
/// returned public key bytes unchanged.
pub fn verify_registration(
    relying_party: &RelyingParty,
    expected_challenge: &[u8],
    response: &RegistrationResponse,
) -> Result<Registration, Error> {
    let envelope = Envelope {
        id: &response.id,
        raw_id: &response.raw_id,
        kind: &response.kind,
    };
    let raw_id = ceremony::credential_id(&envelope)?;
    let client_data = ceremony::field(
        &response.response.client_data_json,
        "clientDataJSON",
        MAX_CLIENT_DATA_LEN,
    )?;
    let object = ceremony::field(
        &response.response.attestation_object,
        "attestationObject",
        MAX_ATTESTATION_OBJECT_LEN,
    )?;
    let transports = read_transports(response.response.transports.as_deref())?;

    // Steps 7 to 11: type, challenge, origin, cross-origin; then the hash.
    let client_data_hash = client_data::verify(
        &client_data,
        Ceremony::Create,
        expected_challenge,
        relying_party,
    )?;

    // Step 13: the attestation object, and the authenticator data inside it.
    let object = read_attestation_object(&object)?;
    let auth = AuthenticatorData::parse(&object.auth_data)?;

    // Steps 14 to 17.
    ceremony::check_rp_id_hash(&auth, relying_party)?;
    ceremony::check_user_present(&auth)?;
    ceremony::check_user_verified(&auth)?;
    ceremony::check_backup_flags(&auth)?;

    // Steps 18 and 19: the credential, its id and its algorithm.
    let credential = auth.attested.as_ref().ok_or(Error::AttestedDataMissing)?;
    check_credential_id(credential, &raw_id)?;
    // The key was accepted only for an algorithm that was offered (see cose).

    // Steps 20 to 24: the attestation statement.
    verify_statement(&object, &client_data_hash, credential)?;

    Ok(Registration {
        credential_id: credential.credential_id.clone(),
        public_key: credential.cose_key.clone(),
        algorithm: credential.key.algorithm(),
        sign_count: auth.sign_count,
        backup_eligible: auth.flags.backup_eligible(),
        backed_up: auth.flags.backed_up(),
        aaguid: credential.aaguid,
        transports,
    })
}

/// The id the browser reports and the id the authenticator attested are one.
fn check_credential_id(credential: &AttestedCredential, raw_id: &[u8]) -> Result<(), Error> {
    if credential.credential_id != raw_id {
        return Err(Error::CredentialIdMismatch);
    }
    Ok(())
}

/// Transports are a hint kept for later. A name this library does not know is
/// refused, as the stored form could not represent it.
fn read_transports(names: Option<&[String]>) -> Result<Vec<Transport>, Error> {
    let mut out = Vec::new();
    for name in names.unwrap_or_default() {
        let transport = Transport::parse(name).ok_or(Error::UnknownTransport)?;
        if !out.contains(&transport) {
            out.push(transport);
        }
    }
    Ok(out)
}

/// Exactly `fmt`, `attStmt` and `authData`, with the right types.
fn read_attestation_object(bytes: &[u8]) -> Result<AttestationObject, Error> {
    let value = cbor::decode_exact(bytes)?;
    let Value::Map(entries) = &value else {
        return Err(Error::MalformedAttestationObject);
    };
    if entries.len() != 3 {
        return Err(Error::MalformedAttestationObject);
    }
    let (Some(Value::Text(format)), Some(statement @ Value::Map(_)), Some(Value::Bytes(auth_data))) = (
        value.get_text("fmt"),
        value.get_text("attStmt"),
        value.get_text("authData"),
    ) else {
        return Err(Error::MalformedAttestationObject);
    };
    Ok(AttestationObject {
        format: format.clone(),
        statement: statement.clone(),
        auth_data: auth_data.clone(),
    })
}

fn verify_statement(
    object: &AttestationObject,
    client_data_hash: &[u8; 32],
    credential: &AttestedCredential,
) -> Result<(), Error> {
    match object.format.as_str() {
        "none" => verify_none(&object.statement),
        "packed" => verify_packed_self(object, client_data_hash, credential),
        _ => Err(Error::UnsupportedAttestationFormat),
    }
}

/// `none` carries no statement at all.
fn verify_none(statement: &Value) -> Result<(), Error> {
    match statement {
        Value::Map(entries) if entries.is_empty() => Ok(()),
        _ => Err(Error::AttestationStatementInvalid),
    }
}

/// Packed self attestation (WebAuthn section 8.2, "self attestation"): the
/// statement is `alg` and `sig` only, `alg` is the credential key's own, and
/// `sig` covers the authenticator data and the client data hash.
fn verify_packed_self(
    object: &AttestationObject,
    client_data_hash: &[u8; 32],
    credential: &AttestedCredential,
) -> Result<(), Error> {
    let statement = &object.statement;
    if statement.get_text("x5c").is_some() || statement.get_text("ecdaaKeyId").is_some() {
        return Err(Error::UnsupportedAttestationFormat);
    }
    let Value::Map(entries) = statement else {
        return Err(Error::AttestationStatementInvalid);
    };
    let (Some(Value::Int(alg)), Some(Value::Bytes(sig)), 2) = (
        statement.get_text("alg"),
        statement.get_text("sig"),
        entries.len(),
    ) else {
        return Err(Error::AttestationStatementInvalid);
    };

    if *alg != credential.key.algorithm().cose_id() {
        return Err(Error::AttestationAlgorithmMismatch);
    }
    let mut signed = object.auth_data.clone();
    signed.extend_from_slice(client_data_hash);
    if !credential.key.verify(&signed, sig) {
        return Err(Error::AttestationSignatureInvalid);
    }
    Ok(())
}
