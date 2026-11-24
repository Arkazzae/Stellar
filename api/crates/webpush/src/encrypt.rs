//! Encryption of a push message for one receiver: RFC 8291 over the
//! `aes128gcm` content coding of RFC 8188.
//!
//! The whole body, header and record, stays within 4096 bytes: RFC 8291
//! section 4 says a push service is not required to accept more, which leaves
//! 3993 bytes of plaintext. The message is one record that carries the padding
//! delimiter and no more padding than that.

use aws_lc_rs::aead::{AES_128_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, ECDH_P256, UnparsedPublicKey};
use aws_lc_rs::hkdf::{self, HKDF_SHA256};
use aws_lc_rs::rand;

use crate::p256::{POINT_LEN, is_valid_point};

/// The record size written into the header.
pub const RECORD_SIZE: u32 = 4096;
/// The most body a push service is required to accept (RFC 8291 section 4).
pub const MAX_BODY: usize = 4096;
/// Header: salt (16), record size (4), key id length (1), key id (65).
pub const HEADER_LEN: usize = 16 + 4 + 1 + POINT_LEN;
/// Authentication tag of AES-GCM.
const TAG_LEN: usize = 16;
/// The most plaintext that keeps the body within [`MAX_BODY`]: 3993 bytes.
pub const MAX_PLAINTEXT: usize = MAX_BODY - HEADER_LEN - TAG_LEN - 1;
/// Appended to the last (here: only) record to mark its end.
const LAST_RECORD_DELIMITER: u8 = 0x02;

/// Why a message could not be encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptError {
    /// The receiver's key is not a point on the curve.
    InvalidReceiverKey,
    /// The message does not fit one record.
    TooLarge,
    /// The cryptographic library refused to do its part.
    Crypto,
}

impl std::fmt::Display for EncryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidReceiverKey => "the receiver's public key is not a point on P-256",
            Self::TooLarge => "the message does not fit in one record",
            Self::Crypto => "the cryptographic library failed",
        })
    }
}

impl std::error::Error for EncryptError {}

/// Encrypts `plaintext` for a receiver with a fresh salt and a fresh
/// one-time key. The result is the whole request body: header and record.
pub fn encrypt(
    plaintext: &[u8],
    receiver_public: &[u8],
    auth_secret: &[u8; 16],
) -> Result<Vec<u8>, EncryptError> {
    let mut salt = [0u8; 16];
    rand::fill(&mut salt).map_err(|_| EncryptError::Crypto)?;
    let sender = agreement::PrivateKey::generate(&ECDH_P256).map_err(|_| EncryptError::Crypto)?;
    encrypt_with(plaintext, receiver_public, auth_secret, &salt, &sender)
}

/// The deterministic core, so the RFC's worked example can be reproduced.
pub(crate) fn encrypt_with(
    plaintext: &[u8],
    receiver_public: &[u8],
    auth_secret: &[u8; 16],
    salt: &[u8; 16],
    sender: &agreement::PrivateKey,
) -> Result<Vec<u8>, EncryptError> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(EncryptError::TooLarge);
    }
    if !is_valid_point(receiver_public) {
        return Err(EncryptError::InvalidReceiverKey);
    }
    let sender_public = sender
        .compute_public_key()
        .map_err(|_| EncryptError::Crypto)?;
    let sender_public = sender_public.as_ref();

    let shared = agreement::agree(
        sender,
        UnparsedPublicKey::new(&ECDH_P256, receiver_public),
        EncryptError::InvalidReceiverKey,
        |secret| Ok(secret.to_vec()),
    )?;

    // RFC 8291 section 3.4: the shared secret is first mixed with the
    // receiver's authentication secret and both public keys ...
    let mut key_info = Vec::with_capacity(14 + 2 * POINT_LEN);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(receiver_public);
    key_info.extend_from_slice(sender_public);
    let mut ikm = [0u8; 32];
    expand(auth_secret, &shared, &key_info, &mut ikm)?;

    // ... and then, with the salt, yields the key and nonce of RFC 8188.
    let mut key = [0u8; 16];
    expand(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut key)?;
    let mut nonce = [0u8; NONCE_LEN];
    expand(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce)?;

    let mut record = Vec::with_capacity(plaintext.len() + 1 + TAG_LEN);
    record.extend_from_slice(plaintext);
    record.push(LAST_RECORD_DELIMITER);
    let sealing =
        LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &key).map_err(|_| EncryptError::Crypto)?);
    // The only record is number zero, whose nonce is the derived one as is.
    sealing
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut record,
        )
        .map_err(|_| EncryptError::Crypto)?;

    let mut body = Vec::with_capacity(HEADER_LEN + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(POINT_LEN as u8);
    body.extend_from_slice(sender_public);
    body.extend_from_slice(&record);
    Ok(body)
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// HKDF-SHA-256 extract-then-expand into `out`.
fn expand(salt: &[u8], secret: &[u8], info: &[u8], out: &mut [u8]) -> Result<(), EncryptError> {
    let prk = hkdf::Salt::new(HKDF_SHA256, salt).extract(secret);
    prk.expand(&[info], Len(out.len()))
        .and_then(|okm| okm.fill(out))
        .map_err(|_| EncryptError::Crypto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64url;
    use crate::testing::subscription_keys;

    // Appendix A of RFC 8291, copied from the RFC text.
    const PLAINTEXT: &str = "V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24";
    const AS_PUBLIC: &str =
        "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    // The request body of section 5, with the line breaks removed.
    const SECTION_5_BODY: &str = concat!(
        "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml",
        "mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT",
        "pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
    );

    fn decode(text: &str) -> Vec<u8> {
        base64url::decode(text, 1000).unwrap()
    }

    fn sender() -> agreement::PrivateKey {
        agreement::PrivateKey::from_private_key(&ECDH_P256, &decode(AS_PRIVATE)).unwrap()
    }

    #[test]
    fn reproduces_the_worked_example_of_rfc_8291() {
        let salt = <[u8; 16]>::try_from(decode(SALT)).unwrap();
        let auth = <[u8; 16]>::try_from(decode(AUTH_SECRET)).unwrap();
        let sender = sender();
        assert_eq!(
            sender.compute_public_key().unwrap().as_ref(),
            &decode(AS_PUBLIC)[..],
            "the sender key of the example derives to its public key"
        );

        let body = encrypt_with(
            &decode(PLAINTEXT),
            &decode(UA_PUBLIC),
            &auth,
            &salt,
            &sender,
        )
        .unwrap();

        assert_eq!(base64url::encode(&body), SECTION_5_BODY);
        // 86 header + 41 plaintext + 1 delimiter + 16 tag. The RFC's sample
        // request says Content-Length 145, which does not match its own body.
        assert_eq!(body.len(), 144);
        assert_eq!(
            base64url::encode(&body[..HEADER_LEN]),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"
        );
        // The ciphertext printed in Appendix A.
        assert_eq!(
            base64url::encode(&body[HEADER_LEN..]),
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ"
        );
    }

    #[test]
    fn header_fields_are_laid_out_as_rfc_8188_says() {
        let keys = subscription_keys();
        let body = encrypt(b"hello", &keys.p256dh, &keys.auth).unwrap();
        assert_eq!(body.len(), HEADER_LEN + 5 + 1 + 16);
        assert_eq!(u32::from_be_bytes(body[16..20].try_into().unwrap()), 4096);
        assert_eq!(body[20], 65);
        assert_eq!(body[21], 0x04);
        assert!(is_valid_point(&body[21..86]));
    }

    #[test]
    fn two_messages_never_share_salt_or_key() {
        let keys = subscription_keys();
        let a = encrypt(b"same", &keys.p256dh, &keys.auth).unwrap();
        let b = encrypt(b"same", &keys.p256dh, &keys.auth).unwrap();
        assert_ne!(a[..16], b[..16]);
        assert_ne!(a[21..86], b[21..86]);
        assert_ne!(a[86..], b[86..]);
    }

    #[test]
    fn the_largest_message_fits_and_one_more_byte_does_not() {
        let keys = subscription_keys();
        let biggest = vec![b'x'; MAX_PLAINTEXT];
        let body = encrypt(&biggest, &keys.p256dh, &keys.auth).unwrap();
        assert_eq!(MAX_PLAINTEXT, 3993);
        assert_eq!(body.len(), MAX_BODY);
        let too_big = vec![b'x'; MAX_PLAINTEXT + 1];
        assert_eq!(
            encrypt(&too_big, &keys.p256dh, &keys.auth),
            Err(EncryptError::TooLarge)
        );
    }

    #[test]
    fn an_empty_message_still_makes_a_record() {
        let keys = subscription_keys();
        let body = encrypt(b"", &keys.p256dh, &keys.auth).unwrap();
        assert_eq!(body.len(), HEADER_LEN + 1 + 16);
    }

    #[test]
    fn a_bad_receiver_key_is_refused_before_any_work() {
        let keys = subscription_keys();
        assert_eq!(
            encrypt(b"x", &[0x04; 65], &keys.auth),
            Err(EncryptError::InvalidReceiverKey)
        );
        assert_eq!(
            encrypt(b"x", &keys.p256dh[..64], &keys.auth),
            Err(EncryptError::InvalidReceiverKey)
        );
        assert_eq!(
            encrypt(b"x", &[], &keys.auth),
            Err(EncryptError::InvalidReceiverKey)
        );
    }
}
