//! What the routes read and say: the shapes of requests, read the way the
//! first server read them, and of answers, written in the order it wrote them.
//!
//! Every key, nonce and ciphertext travels as standard Base64 with padding. The
//! first server's reader ignored spaces, tabs and line breaks anywhere in the
//! text and was strict about everything else, so this one does the same. An
//! integer was read from a number or from a string of digits, a member named
//! twice counted as the last one, and a value of the wrong type made the whole
//! body unreadable (a 400 with no body) before anything was checked.

use serde::Serialize;

pub use crate::http::lenient::{Malformed, Members, Octets};
use crate::http::lenient::{number, octets};
use crate::values::{Id, Timestamp};

/// An id that is only valid in one spelling: lower case with hyphens, the form
/// browsers bind keys to. Any other spelling would have the two sides derive
/// different keys.
pub fn is_wire_id(text: &str) -> Option<Id> {
    Id::parse(text).filter(|id| id.to_string() == text)
}

// ---- requests ----

/// The PIN-protected copy of the account's private key, as the browser made it.
pub struct BackupRequest {
    pub salt: Option<Vec<u8>>,
    pub iterations: Option<i32>,
    pub verifier: Option<Vec<u8>>,
    pub iv: Option<Vec<u8>>,
    pub ciphertext: Option<Vec<u8>>,
}

impl BackupRequest {
    fn read(body: &Members) -> Result<BackupRequest, Malformed> {
        Ok(BackupRequest {
            salt: octets(body, "salt")?,
            iterations: number(body, "iterations")?,
            verifier: octets(body, "verifier")?,
            iv: octets(body, "iv")?,
            ciphertext: octets(body, "ciphertext")?,
        })
    }
}

/// A backup that has every part, each of a size the protocol allows. Leaving
/// room around what a browser sends means a wrong client is refused instead
/// of stored.
pub struct NewBackup {
    pub salt: Vec<u8>,
    pub iterations: i32,
    pub verifier: Vec<u8>,
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl BackupRequest {
    /// The backup, when it is complete and its parts fit.
    pub fn checked(self) -> Option<NewBackup> {
        let backup = NewBackup {
            salt: self.salt?,
            iterations: self.iterations?,
            verifier: self.verifier?,
            iv: self.iv?,
            ciphertext: self.ciphertext?,
        };
        let fits = (16..=64).contains(&backup.salt.len())
            && (600_000..=10_000_000).contains(&backup.iterations)
            && backup.verifier.len() == 32
            && backup.iv.len() == 12
            && (17..=1024).contains(&backup.ciphertext.len());
        fits.then_some(backup)
    }
}

fn backup(body: &Members) -> Result<Option<BackupRequest>, Malformed> {
    body.get::<Members>("backup")?
        .map(|members| BackupRequest::read(&members))
        .transpose()
}

/// First set-up: the account's public key and the backup of its private key.
pub struct SetUp {
    pub algorithm: Option<String>,
    pub public_key: Option<Vec<u8>>,
    pub backup: Option<BackupRequest>,
}

impl SetUp {
    pub fn read(body: &Members) -> Result<SetUp, Malformed> {
        Ok(SetUp {
            algorithm: body.get("algorithm")?,
            public_key: octets(body, "publicKey")?,
            backup: backup(body)?,
        })
    }
}

/// A new account key and its backup, for when nobody holds the old one.
pub struct Reset {
    pub current_password: Option<String>,
    pub algorithm: Option<String>,
    pub public_key: Option<Vec<u8>>,
    pub backup: Option<BackupRequest>,
}

impl Reset {
    pub fn read(body: &Members) -> Result<Reset, Malformed> {
        Ok(Reset {
            current_password: body.get("currentPassword")?,
            algorithm: body.get("algorithm")?,
            public_key: octets(body, "publicKey")?,
            backup: backup(body)?,
        })
    }
}

/// A new PIN: the same account key, encrypted again.
pub struct Replace {
    pub current_password: Option<String>,
    pub account_key_id: Option<String>,
    pub backup: Option<BackupRequest>,
}

impl Replace {
    pub fn read(body: &Members) -> Result<Replace, Malformed> {
        Ok(Replace {
            current_password: body.get("currentPassword")?,
            account_key_id: body.get("accountKeyId")?,
            backup: backup(body)?,
        })
    }
}

pub struct Unlock {
    pub auth_key: Option<Vec<u8>>,
}

impl Unlock {
    pub fn read(body: &Members) -> Result<Unlock, Malformed> {
        Ok(Unlock {
            auth_key: octets(body, "authKey")?,
        })
    }
}

pub struct Register {
    pub algorithm: Option<String>,
    pub public_key: Option<Vec<u8>>,
    pub label: Option<String>,
}

impl Register {
    pub fn read(body: &Members) -> Result<Register, Malformed> {
        Ok(Register {
            algorithm: body.get("algorithm")?,
            public_key: octets(body, "publicKey")?,
            label: body.get("label")?,
        })
    }
}

pub struct EnvelopeRequest {
    pub recipient_key_id: Option<String>,
    pub ephemeral_public_key: Option<Vec<u8>>,
    pub iv: Option<Vec<u8>>,
    pub ciphertext: Option<Vec<u8>>,
}

pub struct EpochRequest {
    pub epoch_id: Option<String>,
    pub previous_epoch_id: Option<Id>,
    /// `None` where the request had `null` for an envelope.
    pub envelopes: Option<Vec<Option<EnvelopeRequest>>>,
}

impl EpochRequest {
    pub fn read(body: &Members) -> Result<EpochRequest, Malformed> {
        let envelopes = body
            .get::<Vec<Option<Members>>>("envelopes")?
            .map(|list| {
                list.iter()
                    .map(|item| {
                        item.as_ref()
                            .map(|members| {
                                Ok(EnvelopeRequest {
                                    recipient_key_id: members.get("recipientKeyId")?,
                                    ephemeral_public_key: octets(members, "ephemeralPublicKey")?,
                                    iv: octets(members, "iv")?,
                                    ciphertext: octets(members, "ciphertext")?,
                                })
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<_>, Malformed>>()
            })
            .transpose()?;
        Ok(EpochRequest {
            epoch_id: body.get("epochId")?,
            previous_epoch_id: body.get("previousEpochId")?,
            envelopes,
        })
    }
}

// ---- answers ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyDto {
    pub key_id: Id,
    pub algorithm: String,
    pub public_key: Octets,
    pub created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatusDto {
    pub iterations: i32,
    pub failed_attempts: i32,
    pub remaining_attempts: i32,
    pub updated_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupParametersDto {
    pub salt: Octets,
    pub iterations: i32,
    pub remaining_attempts: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockedDto {
    pub account_key_id: Id,
    pub iv: Octets,
    pub ciphertext: Octets,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnDeviceDto {
    pub id: Id,
    pub algorithm: String,
    pub public_key: Octets,
    pub label: String,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub current: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub account_key: Option<PublicKeyDto>,
    pub backup: Option<BackupStatusDto>,
    pub device: Option<OwnDeviceDto>,
}

#[derive(Serialize)]
pub struct DevicesDto {
    pub devices: Vec<OwnDeviceDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetDto {
    pub account_key: PublicKeyDto,
    pub backup: BackupStatusDto,
    pub revoked_devices: i64,
}

/// Where a conversation stands with encryption.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionDto {
    pub state: &'static str,
    pub requested_by_id: Option<Id>,
    pub requested_at: Option<Timestamp>,
    pub enabled_at: Option<Timestamp>,
    pub current_epoch_id: Option<Id>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceKeyDto {
    pub key_id: Id,
    pub algorithm: String,
    pub public_key: Octets,
    pub created_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonKeysDto {
    pub user_id: Id,
    pub account_key: Option<PublicKeyDto>,
    pub devices: Vec<DeviceKeyDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientKeyDto {
    pub key_id: Id,
    pub kind: &'static str,
    pub user_id: Id,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeDto {
    pub recipient_key_id: Id,
    pub ephemeral_public_key: Octets,
    pub iv: Octets,
    pub ciphertext: Octets,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochDto {
    pub id: Id,
    pub sequence: i32,
    pub previous_epoch_id: Option<Id>,
    pub created_by_id: Id,
    pub created_at: Timestamp,
    pub recipients: Vec<RecipientKeyDto>,
    pub envelopes: Vec<EnvelopeDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysDto {
    pub conversation_id: Id,
    pub encryption: EncryptionDto,
    #[serde(rename = "self")]
    pub own: PersonKeysDto,
    pub peer: PersonKeysDto,
    pub current_device_key_id: Option<Id>,
    pub recipients: Vec<RecipientKeyDto>,
    pub current_epoch: Option<EpochDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochsDto {
    pub current_epoch_id: Option<Id>,
    pub epochs: Vec<EpochDto>,
    pub has_more: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionChangedDto {
    pub conversation_id: Id,
    pub encryption: EncryptionDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochCreatedDto {
    pub conversation_id: Id,
    pub epoch: EpochDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysChangedDto {
    pub conversation_id: Id,
    pub user_id: Id,
}

/// The payload of an event that only says "look again".
#[derive(Serialize)]
pub struct Nothing {}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(text: &str) -> Members {
        serde_json::from_str(text).expect("an object")
    }

    #[test]
    fn an_id_is_only_good_in_the_one_spelling_browsers_use() {
        let id = Id::random();
        assert_eq!(is_wire_id(&id.to_string()), Some(id));
        assert_eq!(is_wire_id(&id.to_string().to_uppercase()), None);
        assert_eq!(is_wire_id(&id.to_string().replace('-', "")), None);
        assert_eq!(is_wire_id(&format!("{{{id}}}")), None);
        assert_eq!(is_wire_id(""), None);
    }

    #[test]
    fn an_epoch_request_with_a_null_envelope_keeps_the_hole() {
        let body = members(
            "{\"epochId\":\"x\",\"envelopes\":[null,{\"recipientKeyId\":\"y\",\"iv\":\"AAAA\"}]}",
        );
        let read = EpochRequest::read(&body).unwrap();
        let envelopes = read.envelopes.unwrap();
        assert!(envelopes[0].is_none());
        assert_eq!(envelopes[1].as_ref().unwrap().iv.as_deref(), Some(&[0, 0, 0][..]));
    }
}
