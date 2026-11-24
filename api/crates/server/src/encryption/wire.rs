//! What the routes read and say: the shapes of requests, read the way the
//! first server read them, and of answers, written in the order it wrote them.
//!
//! Every key, nonce and ciphertext travels as standard Base64 with padding. The
//! first server's reader ignored spaces, tabs and line breaks anywhere in the
//! text and was strict about everything else, so this one does the same. An
//! integer was read from a number or from a string of digits, a member named
//! twice counted as the last one, and a value of the wrong type made the whole
//! body unreadable (a 400 with no body) before anything was checked.

use std::fmt;

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

use crate::encoding::{base64, from_base64};
use crate::http::ApiError;
use crate::values::{Id, Timestamp};

/// A body that cannot be read: a 400 with no body.
#[derive(Debug)]
pub struct Malformed;

impl From<Malformed> for ApiError {
    fn from(_: Malformed) -> ApiError {
        ApiError::bad_request()
    }
}

// ---- reading ----

/// The members of the JSON object a request is, each still as written.
pub struct Members(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Members, D::Error> {
        struct Reader;
        impl<'de> Visitor<'de> for Reader {
            type Value = Members;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some(name) = map.next_key::<String>()? {
                    members.push((name, map.next_value::<Box<RawValue>>()?));
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(Reader)
    }
}

impl Members {
    /// What the member `name` holds: `None` when it is missing or null. The
    /// name is read without regard to letter case, as the first server read
    /// it. A member that appears more than once counts as its last
    /// occurrence, and every occurrence has to be of the right type.
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, Malformed> {
        let mut found = None;
        for (member, raw) in &self.0 {
            if member.eq_ignore_ascii_case(name) {
                found = serde_json::from_str::<Option<T>>(raw.get()).map_err(|_| Malformed)?;
            }
        }
        Ok(found)
    }
}

/// Bytes sent as Base64: ' ', '\t', '\r' and '\n' are left out wherever they
/// stand, and the rest must be the one spelling of some bytes.
pub struct Octets(pub Vec<u8>);

impl<'de> Deserialize<'de> for Octets {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Octets, D::Error> {
        let text = String::deserialize(deserializer)?;
        let squeezed: String = text
            .chars()
            .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
            .collect();
        from_base64(&squeezed)
            .map(Octets)
            .ok_or_else(|| de::Error::custom("not Base64"))
    }
}

impl Serialize for Octets {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64(&self.0))
    }
}

/// An integer sent as a JSON integer, or as a string of digits with an
/// optional sign and no spaces. A number with a fraction or an exponent is
/// not one, even when it is a whole number.
pub struct Whole(pub i32);

impl<'de> Deserialize<'de> for Whole {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Whole, D::Error> {
        struct Reader;
        impl Visitor<'_> for Reader {
            type Value = Whole;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an integer")
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Whole, E> {
                i32::try_from(value)
                    .map(Whole)
                    .map_err(|_| E::custom("out of range"))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Whole, E> {
                i32::try_from(value)
                    .map(Whole)
                    .map_err(|_| E::custom("out of range"))
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Whole, E> {
                let (negative, digits) = match text.as_bytes().first() {
                    Some(b'-') => (true, &text[1..]),
                    Some(b'+') => (false, &text[1..]),
                    _ => (false, text),
                };
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(E::custom("not digits"));
                }
                let mut value: i64 = 0;
                for digit in digits.bytes() {
                    value = value * 10 + i64::from(digit - b'0');
                    if value > i64::from(i32::MAX) + 1 {
                        return Err(E::custom("out of range"));
                    }
                }
                self.visit_i64(if negative { -value } else { value })
            }
        }
        deserializer.deserialize_any(Reader)
    }
}

/// An id that is only valid in one spelling: lower case with hyphens, the form
/// browsers bind keys to. Any other spelling would have the two sides derive
/// different keys.
pub fn is_wire_id(text: &str) -> Option<Id> {
    Id::parse(text).filter(|id| id.to_string() == text)
}

/// The members of a request that are bytes, text or numbers, read by name.
pub fn octets(body: &Members, name: &str) -> Result<Option<Vec<u8>>, Malformed> {
    Ok(body.get::<Octets>(name)?.map(|octets| octets.0))
}

pub fn number(body: &Members, name: &str) -> Result<Option<i32>, Malformed> {
    Ok(body.get::<Whole>(name)?.map(|whole| whole.0))
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
    fn bytes_ignore_spaces_tabs_and_line_breaks_and_nothing_else() {
        let key = base64(&[7u8; 32]);
        let split = format!("{} \t\r\n{}\n", &key[..10], &key[10..]);
        let body = members(&serde_json::json!({ "publicKey": split }).to_string());
        assert_eq!(octets(&body, "publicKey").unwrap().unwrap(), vec![7u8; 32]);

        for bad in [
            format!("{}\u{b}{}", &key[..10], &key[10..]),
            format!("{}\u{a0}{}", &key[..10], &key[10..]),
            key.trim_end_matches('=').to_string(),
            format!("{key}="),
            "AAAA=AAA".to_string(),
            "-".repeat(43) + "=",
            // The last character carries bits that are not zero.
            format!("{}B=", &base64(&[0u8; 32])[..42]),
        ] {
            let body = members(&serde_json::json!({ "publicKey": bad }).to_string());
            assert!(octets(&body, "publicKey").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn bytes_that_are_not_a_string_are_unreadable_and_null_or_empty_are_something() {
        for bad in ["5", "true", "[1,2]", "{}"] {
            let body = members(&format!("{{\"k\":{bad}}}"));
            assert!(octets(&body, "k").is_err(), "{bad}");
        }
        let body = members("{\"a\":null,\"b\":\"\",\"c\":\"   \"}");
        assert_eq!(octets(&body, "a").unwrap(), None);
        assert_eq!(octets(&body, "b").unwrap(), Some(vec![]));
        assert_eq!(octets(&body, "c").unwrap(), Some(vec![]));
        assert_eq!(octets(&body, "missing").unwrap(), None);
    }

    #[test]
    fn a_number_is_an_integer_or_a_string_of_digits() {
        for (text, wanted) in [
            ("600000", Some(600_000)),
            ("\"600000\"", Some(600_000)),
            ("\"+600000\"", Some(600_000)),
            ("\"0600000\"", Some(600_000)),
            ("\"-5\"", Some(-5)),
            ("-1", Some(-1)),
            ("\"2147483647\"", Some(i32::MAX)),
            ("\"-2147483648\"", Some(i32::MIN)),
            ("600000.0", None),
            ("6e5", None),
            ("600000.5", None),
            ("\" 600000\"", None),
            ("\"600000 \"", None),
            ("\"6e5\"", None),
            ("\"\"", None),
            ("\"+\"", None),
            ("\"2147483648\"", None),
            ("2147483648", None),
            ("true", None),
            ("{}", None),
            ("[600000]", None),
            ("\"\u{660}\"", None),
        ] {
            let body = members(&format!("{{\"n\":{text}}}"));
            match (number(&body, "n"), wanted) {
                (Ok(Some(found)), Some(wanted)) => assert_eq!(found, wanted, "{text}"),
                (Err(_), None) => {}
                (other, _) => panic!("{text}: {:?}", other.map_err(|_| "unreadable")),
            }
        }
        assert_eq!(number(&members("{\"n\":null}"), "n").unwrap(), None);
    }

    #[test]
    fn a_member_named_twice_counts_as_its_last_and_each_has_to_be_readable() {
        let body = members("{\"algorithm\":\"rsa\",\"algorithm\":\"x25519\"}");
        assert_eq!(
            body.get::<String>("algorithm").unwrap().as_deref(),
            Some("x25519")
        );
        let body = members("{\"algorithm\":5,\"algorithm\":\"x25519\"}");
        assert!(body.get::<String>("algorithm").is_err());
        let body = members("{\"algorithm\":\"x25519\",\"algorithm\":null}");
        assert_eq!(body.get::<String>("algorithm").unwrap(), None);
    }

    #[test]
    fn names_are_read_without_regard_to_letter_case_and_the_last_one_wins() {
        let body = members("{\"PublicKey\":\"AAAA\",\"publickey\":\"AAAB\"}");
        assert_eq!(octets(&body, "publicKey").unwrap(), Some(vec![0, 0, 1]));
    }

    #[test]
    fn a_body_that_is_not_an_object_is_unreadable() {
        for text in ["null", "[]", "5", "\"x\"", "true"] {
            assert!(serde_json::from_str::<Members>(text).is_err(), "{text}");
        }
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
