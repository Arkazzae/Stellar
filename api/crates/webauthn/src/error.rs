//! One error type for both ceremonies. Every check the specification lists is
//! its own variant, so a test can prove that exactly that check refused an
//! answer and not some other one that happened to fire first.

use std::fmt;

use crate::cbor::CborError;

/// Why an options request, a registration or an assertion was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    // The relying party or the request given by the application is unusable.
    InvalidRelyingParty(&'static str),
    InvalidRequest(&'static str),
    Random,

    // The shape of what the browser sent.
    BadBase64(&'static str),
    FieldTooLarge(&'static str),
    CredentialTypeNotPublicKey,
    IdDoesNotMatchRawId,
    UnknownTransport,
    MalformedClientData,
    MalformedAttestationObject,
    MalformedAuthenticatorData,
    MalformedCoseKey(&'static str),
    Cbor(CborError),

    // The steps of the registration and authentication ceremonies.
    ClientDataTypeMismatch,
    ChallengeMismatch,
    OriginNotAllowed,
    CrossOriginNotAllowed,
    RpIdHashMismatch,
    UserNotPresent,
    UserNotVerified,
    BackupStateWithoutEligibility,
    BackupEligibilityChanged,
    AttestedDataMissing,
    AttestedDataUnexpected,
    CredentialIdLength,
    CredentialIdMismatch,
    CredentialNotAllowed,
    UserHandleEmpty,
    UserHandleMissing,
    UserHandleMismatch,
    AlgorithmNotOffered,
    AuthenticatorDataTrailingBytes,
    UnsupportedAttestationFormat,
    AttestationStatementInvalid,
    AttestationAlgorithmMismatch,
    AttestationSignatureInvalid,
    StoredKeyInvalid,
    SignatureInvalid,
    CounterNotIncreased,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidRelyingParty(why) => write!(f, "invalid relying party: {why}"),
            Error::InvalidRequest(why) => write!(f, "invalid request: {why}"),
            Error::Random => f.write_str("the system random source failed"),
            Error::BadBase64(field) => write!(f, "{field} is not canonical base64url"),
            Error::FieldTooLarge(field) => write!(f, "{field} is larger than allowed"),
            Error::CredentialTypeNotPublicKey => f.write_str("credential type is not public-key"),
            Error::IdDoesNotMatchRawId => f.write_str("id does not match rawId"),
            Error::UnknownTransport => f.write_str("unknown authenticator transport"),
            Error::MalformedClientData => f.write_str("malformed clientDataJSON"),
            Error::MalformedAttestationObject => f.write_str("malformed attestation object"),
            Error::MalformedAuthenticatorData => f.write_str("malformed authenticator data"),
            Error::MalformedCoseKey(why) => write!(f, "malformed COSE key: {why}"),
            Error::Cbor(cause) => write!(f, "malformed CBOR: {cause}"),
            Error::ClientDataTypeMismatch => {
                f.write_str("clientDataJSON type is wrong for this ceremony")
            }
            Error::ChallengeMismatch => f.write_str("challenge does not match"),
            Error::OriginNotAllowed => f.write_str("origin is not allowed"),
            Error::CrossOriginNotAllowed => f.write_str("cross-origin ceremonies are not allowed"),
            Error::RpIdHashMismatch => f.write_str("relying party id hash does not match"),
            Error::UserNotPresent => f.write_str("user present flag is not set"),
            Error::UserNotVerified => f.write_str("user verified flag is not set"),
            Error::BackupStateWithoutEligibility => {
                f.write_str("backed up without being backup eligible")
            }
            Error::BackupEligibilityChanged => {
                f.write_str("backup eligibility differs from the stored credential")
            }
            Error::AttestedDataMissing => f.write_str("attested credential data is missing"),
            Error::AttestedDataUnexpected => {
                f.write_str("attested credential data in an assertion")
            }
            Error::CredentialIdLength => f.write_str("credential id length is out of range"),
            Error::CredentialIdMismatch => f.write_str("credential id does not match"),
            Error::CredentialNotAllowed => f.write_str("credential is not in the allowed list"),
            Error::UserHandleEmpty => f.write_str("user handle is empty"),
            Error::UserHandleMissing => f.write_str("user handle is missing"),
            Error::UserHandleMismatch => f.write_str("user handle does not match"),
            Error::AlgorithmNotOffered => f.write_str("algorithm was not offered"),
            Error::AuthenticatorDataTrailingBytes => f.write_str("bytes after authenticator data"),
            Error::UnsupportedAttestationFormat => {
                f.write_str("attestation format is not supported")
            }
            Error::AttestationStatementInvalid => f.write_str("attestation statement is invalid"),
            Error::AttestationAlgorithmMismatch => {
                f.write_str("attestation algorithm differs from the credential key")
            }
            Error::AttestationSignatureInvalid => f.write_str("attestation signature is invalid"),
            Error::StoredKeyInvalid => f.write_str("stored public key is unusable"),
            Error::SignatureInvalid => f.write_str("signature is invalid"),
            Error::CounterNotIncreased => f.write_str("signature counter did not increase"),
        }
    }
}

impl std::error::Error for Error {}

impl From<CborError> for Error {
    fn from(cause: CborError) -> Self {
        Error::Cbor(cause)
    }
}
