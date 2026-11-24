//! Authenticator data (WebAuthn section 6.1): the part of an answer the
//! authenticator itself signs.

use crate::cbor::{self, Value};
use crate::cose::PublicKey;
use crate::error::Error;

const HEADER_LEN: usize = 37;
/// WebAuthn limits a credential id to 1023 bytes.
pub const MAX_CREDENTIAL_ID_LEN: usize = 1023;

const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_BE: u8 = 0x08;
const FLAG_BS: u8 = 0x10;
const FLAG_AT: u8 = 0x40;
const FLAG_ED: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags(u8);

impl Flags {
    pub fn user_present(self) -> bool {
        self.0 & FLAG_UP != 0
    }
    pub fn user_verified(self) -> bool {
        self.0 & FLAG_UV != 0
    }
    pub fn backup_eligible(self) -> bool {
        self.0 & FLAG_BE != 0
    }
    pub fn backed_up(self) -> bool {
        self.0 & FLAG_BS != 0
    }
    pub fn has_attested_data(self) -> bool {
        self.0 & FLAG_AT != 0
    }
    pub fn has_extensions(self) -> bool {
        self.0 & FLAG_ED != 0
    }
}

/// The credential a registration created.
#[derive(Debug, Clone)]
pub struct AttestedCredential {
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    /// The COSE key exactly as the authenticator wrote it; this is what the
    /// application stores.
    pub cose_key: Vec<u8>,
    pub key: PublicKey,
}

#[derive(Debug, Clone)]
pub struct AuthenticatorData {
    pub rp_id_hash: [u8; 32],
    pub flags: Flags,
    pub sign_count: u32,
    pub attested: Option<AttestedCredential>,
}

impl AuthenticatorData {
    /// Reads the whole of `bytes`. The attested data is read when the AT flag
    /// says it is there, the extension map when ED does; nothing may be left
    /// over, and an extension map without its flag counts as left over.
    pub fn parse(bytes: &[u8]) -> Result<AuthenticatorData, Error> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::MalformedAuthenticatorData);
        }
        let rp_id_hash: [u8; 32] = bytes[..32]
            .try_into()
            .map_err(|_| Error::MalformedAuthenticatorData)?;
        let flags = Flags(bytes[32]);
        let sign_count = u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]);
        let mut rest = &bytes[HEADER_LEN..];

        let attested = if flags.has_attested_data() {
            let (credential, used) = read_attested(rest)?;
            rest = &rest[used..];
            Some(credential)
        } else {
            None
        };

        if flags.has_extensions() {
            let (extensions, used) = cbor::decode_prefix(rest)?;
            // Their content is not interpreted: no extension is requested.
            if !matches!(extensions, Value::Map(_)) {
                return Err(Error::MalformedAuthenticatorData);
            }
            rest = &rest[used..];
        }

        if !rest.is_empty() {
            return Err(Error::AuthenticatorDataTrailingBytes);
        }
        Ok(AuthenticatorData {
            rp_id_hash,
            flags,
            sign_count,
            attested,
        })
    }
}

fn read_attested(bytes: &[u8]) -> Result<(AttestedCredential, usize), Error> {
    if bytes.len() < 18 {
        return Err(Error::MalformedAuthenticatorData);
    }
    let aaguid: [u8; 16] = bytes[..16]
        .try_into()
        .map_err(|_| Error::MalformedAuthenticatorData)?;
    let id_len = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    if id_len == 0 || id_len > MAX_CREDENTIAL_ID_LEN {
        return Err(Error::CredentialIdLength);
    }
    let id_end = 18 + id_len;
    let credential_id = bytes
        .get(18..id_end)
        .ok_or(Error::MalformedAuthenticatorData)?
        .to_vec();
    let (value, key_len) = cbor::decode_prefix(&bytes[id_end..])?;
    let key = PublicKey::from_value(&value)?;
    let cose_key = bytes[id_end..id_end + key_len].to_vec();
    Ok((
        AttestedCredential {
            aaguid,
            credential_id,
            cose_key,
            key,
        },
        id_end + key_len,
    ))
}
