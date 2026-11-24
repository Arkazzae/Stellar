//! The options a browser is given to create or use a credential, as the JSON
//! the web client reads. Binary members are base64url; members that would be
//! `null` are left out, which the client treats the same way.

use aws_lc_rs::rand;
use serde::Serialize;

use crate::base64url;
use crate::cose::Algorithm;
use crate::error::Error;
use crate::relying_party::RelyingParty;

/// A hint for the browser; the application keeps its own limit.
pub const TIMEOUT_MILLISECONDS: u64 = 120_000;
pub const CHALLENGE_LEN: usize = 32;
const MAX_USER_HANDLE_LEN: usize = 64;

/// What a ceremony is answered to: random, single use, kept by the
/// application until the answer arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge([u8; CHALLENGE_LEN]);

impl Challenge {
    pub fn random() -> Result<Challenge, Error> {
        let mut bytes = [0u8; CHALLENGE_LEN];
        rand::fill(&mut bytes).map_err(|_| Error::Random)?;
        Ok(Challenge(bytes))
    }

    pub fn from_bytes(bytes: [u8; CHALLENGE_LEN]) -> Challenge {
        Challenge(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Nfc,
    Ble,
    SmartCard,
    Hybrid,
    Internal,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Usb => "usb",
            Transport::Nfc => "nfc",
            Transport::Ble => "ble",
            Transport::SmartCard => "smart-card",
            Transport::Hybrid => "hybrid",
            Transport::Internal => "internal",
        }
    }

    pub fn parse(text: &str) -> Option<Transport> {
        [
            Transport::Usb,
            Transport::Nfc,
            Transport::Ble,
            Transport::SmartCard,
            Transport::Hybrid,
            Transport::Internal,
        ]
        .into_iter()
        .find(|transport| transport.as_str() == text)
    }
}

/// A credential the browser should (or should not) use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialDescriptor {
    pub id: Vec<u8>,
    pub transports: Vec<Transport>,
}

/// The account a credential is created for.
#[derive(Debug, Clone, Copy)]
pub struct User<'a> {
    /// Opaque, 1 to 64 bytes, the same for every credential of an account.
    pub handle: &'a [u8],
    pub name: &'a str,
    pub display_name: &'a str,
}

/// Options ready to send, with the challenge the application must keep to
/// check the answer against.
#[derive(Debug, Clone)]
pub struct Options {
    pub json: String,
    pub challenge: Challenge,
}

#[derive(Serialize)]
struct Descriptor {
    #[serde(rename = "type")]
    kind: &'static str,
    id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    transports: Vec<&'static str>,
}

fn descriptors(list: &[CredentialDescriptor]) -> Vec<Descriptor> {
    list.iter()
        .map(|credential| Descriptor {
            kind: "public-key",
            id: base64url::encode(&credential.id),
            transports: credential.transports.iter().map(|t| t.as_str()).collect(),
        })
        .collect()
}

#[derive(Serialize)]
struct CreationOptions<'a> {
    rp: RpJson<'a>,
    user: UserJson<'a>,
    challenge: String,
    #[serde(rename = "pubKeyCredParams")]
    pub_key_cred_params: Vec<ParamJson>,
    timeout: u64,
    attestation: &'static str,
    #[serde(rename = "authenticatorSelection")]
    authenticator_selection: SelectionJson,
    #[serde(rename = "excludeCredentials")]
    exclude_credentials: Vec<Descriptor>,
    // Always empty: no hint is given and every attestation format is allowed
    // to come back, as the options the web client already receives say.
    #[serde(rename = "attestationFormats")]
    attestation_formats: [&'static str; 0],
    hints: [&'static str; 0],
}

#[derive(Serialize)]
struct RpJson<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct UserJson<'a> {
    name: &'a str,
    id: String,
    #[serde(rename = "displayName")]
    display_name: &'a str,
}

#[derive(Serialize)]
struct ParamJson {
    #[serde(rename = "type")]
    kind: &'static str,
    alg: i64,
}

#[derive(Serialize)]
struct SelectionJson {
    // A passkey stores the account and can sign in without a password; a
    // security key that cannot still works as a second step.
    #[serde(rename = "residentKey")]
    resident_key: &'static str,
    #[serde(rename = "requireResidentKey")]
    require_resident_key: bool,
    #[serde(rename = "userVerification")]
    user_verification: &'static str,
}

#[derive(Serialize)]
struct RequestOptions<'a> {
    challenge: String,
    timeout: u64,
    #[serde(rename = "rpId")]
    rp_id: &'a str,
    #[serde(rename = "allowCredentials")]
    allow_credentials: Vec<Descriptor>,
    #[serde(rename = "userVerification")]
    user_verification: &'static str,
    hints: [&'static str; 0],
}

/// Options for creating a credential: attestation not requested, user
/// verification required, a discoverable credential preferred, and every
/// credential in `exclude` refused so one key is not registered twice.
pub fn creation_options(
    relying_party: &RelyingParty,
    user: &User<'_>,
    exclude: &[CredentialDescriptor],
    challenge: Challenge,
) -> Result<Options, Error> {
    if user.handle.is_empty() || user.handle.len() > MAX_USER_HANDLE_LEN {
        return Err(Error::InvalidRequest("user handle must be 1 to 64 bytes"));
    }
    let options = CreationOptions {
        rp: RpJson {
            id: relying_party.id(),
            name: relying_party.name(),
        },
        user: UserJson {
            id: base64url::encode(user.handle),
            name: user.name,
            display_name: user.display_name,
        },
        challenge: base64url::encode(challenge.as_bytes()),
        pub_key_cred_params: Algorithm::OFFERED
            .iter()
            .map(|alg| ParamJson {
                kind: "public-key",
                alg: alg.cose_id(),
            })
            .collect(),
        timeout: TIMEOUT_MILLISECONDS,
        attestation: "none",
        authenticator_selection: SelectionJson {
            resident_key: "preferred",
            require_resident_key: false,
            user_verification: "required",
        },
        exclude_credentials: descriptors(exclude),
        attestation_formats: [],
        hints: [],
    };
    let json = serde_json::to_string(&options)
        .map_err(|_| Error::InvalidRequest("options do not serialize"))?;
    Ok(Options { json, challenge })
}

/// Options for signing in: user verification required. An empty `allow` lets
/// the browser offer whichever passkeys it holds for the site.
pub fn assertion_options(
    relying_party: &RelyingParty,
    allow: &[CredentialDescriptor],
    challenge: Challenge,
) -> Result<Options, Error> {
    let options = RequestOptions {
        challenge: base64url::encode(challenge.as_bytes()),
        timeout: TIMEOUT_MILLISECONDS,
        rp_id: relying_party.id(),
        allow_credentials: descriptors(allow),
        user_verification: "required",
        hints: [],
    };
    let json = serde_json::to_string(&options)
        .map_err(|_| Error::InvalidRequest("options do not serialize"))?;
    Ok(Options { json, challenge })
}
