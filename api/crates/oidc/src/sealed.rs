//! Sealed values: text only this server can read or make, safe to hand to a
//! browser or a provider and take back.
//!
//! AES-256-GCM with a random nonce per value. Every value is bound to a
//! purpose and to whatever else the caller names (the browser, say), which go
//! in as associated data: the same bytes opened for another purpose or by
//! another binding fail the tag check, never reading as something else.

use aws_lc_rs::aead::{AES_256_GCM, Aad, NONCE_LEN, Nonce, RandomizedNonceKey};

use crate::b64;

const VERSION: u8 = 1;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
/// The longest sealed text opened. The largest value made is about 1 KiB.
const MAX_SEALED_LEN: usize = 8 * 1024;

/// The key values are sealed with. It comes from the application, which keeps
/// it across restarts so a sign-in under way survives one.
pub struct SealingKey(RandomizedNonceKey);

impl std::fmt::Debug for SealingKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealingKey(..)")
    }
}

/// The key is not 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadKeyLength;

impl std::fmt::Display for BadKeyLength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the sealing key must be {KEY_LEN} bytes")
    }
}

impl std::error::Error for BadKeyLength {}

impl SealingKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BadKeyLength> {
        if bytes.len() != KEY_LEN {
            return Err(BadKeyLength);
        }
        RandomizedNonceKey::new(&AES_256_GCM, bytes)
            .map(Self)
            .map_err(|_| BadKeyLength)
    }
}

fn associated_data(purpose: &str, bound_to: &str) -> Vec<u8> {
    // The purpose is fixed text without a NUL, so the split is unambiguous.
    let mut aad = purpose.as_bytes().to_vec();
    aad.push(0);
    aad.extend_from_slice(bound_to.as_bytes());
    aad
}

pub(crate) fn seal(
    key: &SealingKey,
    purpose: &str,
    bound_to: &str,
    plaintext: &[u8],
) -> Option<String> {
    let mut buffer = plaintext.to_vec();
    let nonce = key
        .0
        .seal_in_place_append_tag(Aad::from(associated_data(purpose, bound_to)), &mut buffer)
        .ok()?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + buffer.len());
    out.push(VERSION);
    out.extend_from_slice(nonce.as_ref());
    out.extend_from_slice(&buffer);
    Some(b64::encode(&out))
}

/// `None` for anything that is not a value sealed with this key for this
/// purpose and binding: damaged, cut short, forged, or made for another use.
pub(crate) fn open(key: &SealingKey, purpose: &str, bound_to: &str, text: &str) -> Option<Vec<u8>> {
    if text.len() > MAX_SEALED_LEN {
        return None;
    }
    let bytes = b64::decode(text)?;
    if bytes.len() < 1 + NONCE_LEN + TAG_LEN || bytes[0] != VERSION {
        return None;
    }
    let nonce = Nonce::try_assume_unique_for_key(&bytes[1..=NONCE_LEN]).ok()?;
    let mut buffer = bytes[1 + NONCE_LEN..].to_vec();
    let plaintext = key
        .0
        .open_in_place(
            nonce,
            Aad::from(associated_data(purpose, bound_to)),
            &mut buffer,
        )
        .ok()?;
    Some(plaintext.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> SealingKey {
        SealingKey::from_bytes(&[byte; 32]).unwrap()
    }

    #[test]
    fn opens_what_it_sealed() {
        let sealed = seal(&key(1), "p", "b", b"hello").unwrap();
        assert_eq!(open(&key(1), "p", "b", &sealed).unwrap(), b"hello");
        assert_eq!(
            open(&key(1), "p", "b", &seal(&key(1), "p", "b", b"").unwrap()).unwrap(),
            b""
        );
    }

    #[test]
    fn two_seals_of_one_value_differ() {
        assert_ne!(seal(&key(1), "p", "b", b"x"), seal(&key(1), "p", "b", b"x"));
    }

    #[test]
    fn refuses_another_key_purpose_or_binding() {
        let sealed = seal(&key(1), "p", "b", b"hello").unwrap();
        assert_eq!(open(&key(2), "p", "b", &sealed), None);
        assert_eq!(open(&key(1), "q", "b", &sealed), None);
        assert_eq!(open(&key(1), "p", "c", &sealed), None);
        assert_eq!(open(&key(1), "p", "", &sealed), None);
    }

    #[test]
    fn refuses_every_single_byte_change_and_every_truncation() {
        let sealed = seal(
            &key(1),
            "p",
            "b",
            b"a longer value than one block, to span blocks",
        )
        .unwrap();
        let bytes = b64::decode(&sealed).unwrap();
        for i in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[i] ^= 1;
            assert_eq!(
                open(&key(1), "p", "b", &b64::encode(&changed)),
                None,
                "byte {i}"
            );
        }
        for len in 0..bytes.len() {
            assert_eq!(
                open(&key(1), "p", "b", &b64::encode(&bytes[..len])),
                None,
                "length {len}"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(open(&key(1), "p", "b", &b64::encode(&longer)), None);
    }

    #[test]
    fn refuses_text_that_is_not_a_value() {
        for text in ["", "!", "AAAA", &"A".repeat(MAX_SEALED_LEN + 1)] {
            assert_eq!(open(&key(1), "p", "b", text), None);
        }
    }

    #[test]
    fn the_key_must_be_32_bytes() {
        assert!(SealingKey::from_bytes(&[0; 31]).is_err());
        assert!(SealingKey::from_bytes(&[0; 33]).is_err());
        assert!(SealingKey::from_bytes(&[]).is_err());
    }
}
