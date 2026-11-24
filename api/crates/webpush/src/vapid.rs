//! Voluntary Application Server Identification (RFC 8292): the key pair a
//! server identifies itself with, and the `Authorization` header it signs.
//!
//! An installation that is configured wrongly gets an error from
//! [`VapidKeys::from_config`], not a panic; the caller turns that into "push is
//! not available" and keeps serving everything else.

use aws_lc_rs::encoding::AsBigEndian;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};

use crate::base64url;
use crate::p256::{self, POINT_LEN, SCALAR_LEN};

/// How long a token is valid after it is signed: twelve hours, which is under
/// the day RFC 8292 allows at most.
pub const TOKEN_LIFETIME_SECONDS: u64 = 12 * 60 * 60;

const SUBJECT_MAX_LENGTH: usize = 512;
const PUBLIC_KEY_MAX_CHARS: usize = 128;
const PRIVATE_KEY_MAX_CHARS: usize = 64;

/// Why a configured key pair or subject was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VapidError {
    /// The contact is not a `mailto:` address or an `https` URL.
    Subject,
    /// The public key is not a valid uncompressed P-256 point in base64url.
    PublicKey,
    /// The private key is not a valid 32-byte P-256 scalar in base64url.
    PrivateKey,
    /// Both keys are valid, but they are not a pair.
    Mismatch,
    /// The cryptographic library failed.
    Crypto,
}

impl std::fmt::Display for VapidError {
    // The messages never repeat a key: the private one is a secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Subject => "the VAPID subject must be a mailto: address or an https URL",
            Self::PublicKey => "the VAPID public key is not a valid base64url P-256 point",
            Self::PrivateKey => "the VAPID private key is not a valid base64url P-256 scalar",
            Self::Mismatch => "the VAPID public key does not belong to the private key",
            Self::Crypto => "the cryptographic library failed",
        })
    }
}

impl std::error::Error for VapidError {}

/// A validated VAPID identity: contact, key pair and the signer made from it.
pub struct VapidKeys {
    subject: String,
    public: [u8; POINT_LEN],
    private: [u8; SCALAR_LEN],
    signer: EcdsaKeyPair,
}

impl std::fmt::Debug for VapidKeys {
    // The private key must never reach a log through a stray `{:?}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VapidKeys")
            .field("subject", &self.subject)
            .field("public_key", &self.public_key())
            .finish_non_exhaustive()
    }
}

/// A freshly made pair, in the form it is configured in.
pub struct GeneratedKeys {
    pub public_key: String,
    pub private_key: String,
}

impl std::fmt::Debug for GeneratedKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedKeys")
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

/// Makes a new key pair for an installation to configure.
pub fn generate_keys() -> Result<GeneratedKeys, VapidError> {
    let pair =
        EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).map_err(|_| VapidError::Crypto)?;
    let private = pair
        .private_key()
        .as_be_bytes()
        .map_err(|_| VapidError::Crypto)?;
    Ok(GeneratedKeys {
        public_key: base64url::encode(pair.public_key().as_ref()),
        private_key: base64url::encode(private.as_ref()),
    })
}

impl VapidKeys {
    /// Validates configured values.
    ///
    /// The subject is trimmed; the keys are base64url, unpadded or padded. The
    /// public key must be on the curve, the private key must be a usable
    /// scalar, and the two must belong together.
    pub fn from_config(
        subject: &str,
        public_key: &str,
        private_key: &str,
    ) -> Result<Self, VapidError> {
        let subject = normalize_subject(subject).ok_or(VapidError::Subject)?;

        let public = base64url::decode(public_key, PUBLIC_KEY_MAX_CHARS)
            .and_then(|bytes| <[u8; POINT_LEN]>::try_from(bytes).ok())
            .filter(|point| p256::is_valid_point(point))
            .ok_or(VapidError::PublicKey)?;

        let private = base64url::decode(private_key, PRIVATE_KEY_MAX_CHARS)
            .and_then(|bytes| <[u8; SCALAR_LEN]>::try_from(bytes).ok())
            .filter(|scalar| p256::is_valid_scalar(scalar))
            .ok_or(VapidError::PrivateKey)?;
        let derived = p256::public_from_scalar(&private).ok_or(VapidError::PrivateKey)?;
        if derived != public {
            return Err(VapidError::Mismatch);
        }

        let signer = EcdsaKeyPair::from_private_key_and_public_key(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &private,
            &public,
        )
        .map_err(|_| VapidError::Mismatch)?;
        Ok(Self {
            subject,
            public,
            private,
            signer,
        })
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The key browsers subscribe with, as unpadded base64url.
    pub fn public_key(&self) -> String {
        base64url::encode(&self.public)
    }

    /// The private key as it is configured. Only for tools that write the
    /// configuration out; never for logs.
    pub fn private_key(&self) -> String {
        base64url::encode(&self.private)
    }

    /// The signed token (a JWT, RFC 7519) for a push service origin.
    ///
    /// `now` is the time in seconds since the Unix epoch; the token expires
    /// [`TOKEN_LIFETIME_SECONDS`] later. `audience` is the origin of the push
    /// endpoint, such as `https://push.example.net`.
    pub fn token(&self, audience: &str, now: u64) -> Result<String, VapidError> {
        let claims = Claims {
            aud: audience,
            exp: now.saturating_add(TOKEN_LIFETIME_SECONDS),
            sub: &self.subject,
        };
        let payload = serde_json::to_vec(&claims).map_err(|_| VapidError::Crypto)?;
        let signing_input = format!(
            "{}.{}",
            base64url::encode(br#"{"typ":"JWT","alg":"ES256"}"#),
            base64url::encode(&payload)
        );
        let signature = self
            .signer
            .sign(&SystemRandom::new(), signing_input.as_bytes())
            .map_err(|_| VapidError::Crypto)?;
        Ok(format!(
            "{signing_input}.{}",
            base64url::encode(signature.as_ref())
        ))
    }

    /// The value of the `Authorization` header: `vapid t=<token>, k=<key>`.
    pub fn authorization(&self, audience: &str, now: u64) -> Result<String, VapidError> {
        Ok(format!(
            "vapid t={}, k={}",
            self.token(audience, now)?,
            self.public_key()
        ))
    }
}

/// Claims in the order RFC 8292 prints them.
#[derive(serde::Serialize)]
struct Claims<'a> {
    aud: &'a str,
    exp: u64,
    sub: &'a str,
}

/// Accepts a `mailto:` address or an `https` URL with a host, without
/// whitespace, in 1 to 512 characters.
fn normalize_subject(value: &str) -> Option<String> {
    let candidate = value.trim();
    let length = candidate.chars().count();
    if length == 0
        || length > SUBJECT_MAX_LENGTH
        || candidate
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    if let Some(address) = strip_prefix_ignore_case(candidate, "mailto:") {
        return (!address.is_empty()).then(|| candidate.to_owned());
    }
    let rest = strip_prefix_ignore_case(candidate, "https://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host_and_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_and_port.starts_with('[') {
        host_and_port
    } else {
        host_and_port.split(':').next().unwrap_or("")
    };
    (!host.is_empty()).then(|| candidate.to_owned())
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    // RFC 8292 section 2.4, copied from the RFC text.
    const RFC_TOKEN: &str = concat!(
        "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJhdWQiOiJodHRwczovL3",
        "B1c2guZXhhbXBsZS5uZXQiLCJleHAiOjE0NTM1MjM3NjgsInN1YiI6Im1ha",
        "Wx0bzpwdXNoQGV4YW1wbGUuY29tIn0.i3CYb7t4xfxCDquptFOepC9GAu_H",
        "LGkMlMuCGSK2rpiUfnK9ojFwDXb1JrErtmysazNjjvW2L9OkSSHzvoD1oA"
    );
    const RFC_KEY: &str = concat!(
        "BA1Hxzyi1RUM1b5wjxsn7nGxAszw2u61m164i3MrAIxHF6YK5h4SDYic-dR",
        "uU_RCPCfA5aq9ojSwk5Y2EmClBPs"
    );

    fn configured() -> (VapidKeys, GeneratedKeys) {
        let generated = generate_keys().unwrap();
        let keys = VapidKeys::from_config(
            "mailto:push@example.test",
            &generated.public_key,
            &generated.private_key,
        )
        .unwrap();
        (keys, generated)
    }

    fn verify(token: &str, public: &[u8]) -> bool {
        let (signed, signature) = token.rsplit_once('.').unwrap();
        let signature = base64url::decode(signature, 200).unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public)
            .verify(signed.as_bytes(), &signature)
            .is_ok()
    }

    #[test]
    fn the_example_token_of_rfc_8292_has_the_form_and_signature_we_produce() {
        let public = base64url::decode(RFC_KEY, 200).unwrap();
        assert!(p256::is_valid_point(&public), "the example key is a point");
        // The signature in the RFC verifies against its key with the verifier
        // we rely on, so the format (raw r||s, ES256, this signing input) is
        // the one the RFC shows.
        assert!(verify(RFC_TOKEN, &public));

        let mut parts = RFC_TOKEN.split('.');
        let header = base64url::decode(parts.next().unwrap(), 200).unwrap();
        let claims = base64url::decode(parts.next().unwrap(), 400).unwrap();
        assert_eq!(header, br#"{"typ":"JWT","alg":"ES256"}"#);
        assert_eq!(
            claims,
            br#"{"aud":"https://push.example.net","exp":1453523768,"sub":"mailto:push@example.com"}"#
        );
    }

    #[test]
    fn our_token_has_the_same_header_and_claims_layout_as_the_rfc() {
        let (keys, _) = configured();
        let now = 1_453_523_768 - TOKEN_LIFETIME_SECONDS;
        let token = keys.token("https://push.example.net", now).unwrap();
        let mut parts = token.split('.');
        assert_eq!(
            parts.next().unwrap(),
            RFC_TOKEN.split('.').next().unwrap(),
            "header is byte for byte the RFC's"
        );
        let claims = base64url::decode(parts.next().unwrap(), 400).unwrap();
        assert_eq!(
            claims,
            br#"{"aud":"https://push.example.net","exp":1453523768,"sub":"mailto:push@example.test"}"#
        );
        let signature = base64url::decode(parts.next().unwrap(), 200).unwrap();
        assert_eq!(signature.len(), 64);
        assert!(parts.next().is_none());
    }

    #[test]
    fn the_signature_verifies_with_the_public_key_and_not_with_another() {
        let (keys, generated) = configured();
        let token = keys.token("https://push.example.test", 1_000).unwrap();
        let public = base64url::decode(&generated.public_key, 200).unwrap();
        assert!(verify(&token, &public));
        let other = generate_keys().unwrap();
        assert!(!verify(
            &token,
            &base64url::decode(&other.public_key, 200).unwrap()
        ));
        let tampered = token.replacen("eyJhdWQ", "eyJhdWR", 1);
        assert!(!verify(&tampered, &public));
    }

    #[test]
    fn the_header_carries_token_and_key_in_the_rfc_shape() {
        let (keys, generated) = configured();
        let header = keys.authorization("https://push.example.test", 5).unwrap();
        let rest = header.strip_prefix("vapid t=").expect("scheme and t");
        let (token, key) = rest.split_once(", k=").expect("k follows");
        assert_eq!(key, generated.public_key);
        assert_eq!(token.split('.').count(), 3);
        assert!(!header.contains(&generated.private_key));
        // The RFC's header is `vapid t=..., k=...`; its separator is a comma.
        assert!(header.starts_with("vapid t=eyJ"));
    }

    #[test]
    fn token_expires_twelve_hours_after_it_is_signed() {
        let (keys, _) = configured();
        let token = keys.token("https://a.test", 100).unwrap();
        let claims = base64url::decode(token.split('.').nth(1).unwrap(), 400).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&claims).unwrap();
        assert_eq!(claims["exp"], 100 + 43_200);
        assert_eq!(claims["aud"], "https://a.test");
        assert_eq!(claims["sub"], "mailto:push@example.test");
    }

    #[test]
    fn subject_is_escaped_in_the_claims() {
        let generated = generate_keys().unwrap();
        let keys = VapidKeys::from_config(
            "https://ops.example.test/a\"b\\c",
            &generated.public_key,
            &generated.private_key,
        )
        .unwrap();
        let token = keys.token("https://a.test", 1).unwrap();
        let claims = base64url::decode(token.split('.').nth(1).unwrap(), 400).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&claims).unwrap();
        assert_eq!(claims["sub"], "https://ops.example.test/a\"b\\c");
    }

    #[test]
    fn generated_keys_are_in_the_configured_shape() {
        let generated = generate_keys().unwrap();
        assert_eq!(generated.public_key.len(), 87);
        assert_eq!(generated.private_key.len(), 43);
        assert!(
            VapidKeys::from_config(
                "mailto:a@b.test",
                &generated.public_key,
                &generated.private_key
            )
            .is_ok()
        );
        assert_ne!(generate_keys().unwrap().private_key, generated.private_key);
    }

    #[test]
    fn keys_with_padding_are_accepted_and_normalized() {
        let generated = generate_keys().unwrap();
        // 87 characters need one '=' to complete a group of four; 43 need one.
        let keys = VapidKeys::from_config(
            "mailto:a@b.test",
            &format!("{}=", generated.public_key),
            &format!("{}=", generated.private_key),
        )
        .unwrap();
        assert_eq!(keys.public_key(), generated.public_key);
        assert_eq!(keys.private_key(), generated.private_key);
    }

    #[test]
    fn debug_output_never_shows_the_private_key() {
        let (keys, generated) = configured();
        assert!(!format!("{keys:?}").contains(&generated.private_key));
        assert!(!format!("{generated:?}").contains(&generated.private_key));
    }

    #[test]
    fn invalid_subjects_are_refused() {
        let g = generate_keys().unwrap();
        let attempt =
            |subject: &str| VapidKeys::from_config(subject, &g.public_key, &g.private_key).err();
        for good in [
            "mailto:push@example.test",
            "MAILTO:push@example.test",
            "  https://ops.example.test  ",
            "https://ops.example.test:8443/contact",
            "https://user@ops.example.test",
        ] {
            assert_eq!(attempt(good), None, "{good}");
        }
        let long = format!("mailto:{}@example.test", "a".repeat(512));
        for bad in [
            "",
            "   ",
            "mailto:",
            "mailto:  ",
            "mailto:a b@example.test",
            "mailto:a\u{0}b@example.test",
            "mailto:a\u{7f}b@example.test",
            "mailto:a\u{1}@example.test",
            "https://ops.example.test/\u{1b}[0m",
            "http://ops.example.test",
            "https://",
            "https:///path",
            "https://:443/",
            "https://ops.example.test/a b",
            "ftp://ops.example.test",
            "push@example.test",
            "https:ops.example.test",
            long.as_str(),
        ] {
            assert_eq!(attempt(bad), Some(VapidError::Subject), "{bad:?}");
        }
    }

    #[test]
    fn invalid_public_keys_are_refused() {
        let g = generate_keys().unwrap();
        let attempt =
            |public: &str| VapidKeys::from_config("mailto:a@b.test", public, &g.private_key).err();
        let point = base64url::decode(&g.public_key, 200).unwrap();
        let mut off = point.clone();
        off[64] ^= 1;
        let mut compressed = point[..33].to_vec();
        compressed[0] = 0x02;
        let mut wrong_prefix = point.clone();
        wrong_prefix[0] = 0x05;
        for bad in [
            "".to_owned(),
            "not-a-key".to_owned(),
            "public".to_owned(),
            base64url::encode(&off),
            base64url::encode(&compressed),
            base64url::encode(&wrong_prefix),
            base64url::encode(&point[..64]),
            base64url::encode(&[0u8; 65]),
            "A".repeat(129),
            format!("{} ", g.public_key),
            format!("{}+", g.public_key),
        ] {
            assert_eq!(attempt(&bad), Some(VapidError::PublicKey), "{bad:?}");
        }
    }

    #[test]
    fn invalid_private_keys_are_refused() {
        let g = generate_keys().unwrap();
        let attempt =
            |private: &str| VapidKeys::from_config("mailto:a@b.test", &g.public_key, private).err();
        let order = [
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2,
            0xfc, 0x63, 0x25, 0x51,
        ];
        for bad in [
            "".to_owned(),
            "private".to_owned(),
            "not-a-key".to_owned(),
            base64url::encode(&[0u8; 32]),
            base64url::encode(&order),
            base64url::encode(&[0xff; 32]),
            base64url::encode(&[1u8; 31]),
            base64url::encode(&[1u8; 33]),
            "A".repeat(65),
        ] {
            assert_eq!(attempt(&bad), Some(VapidError::PrivateKey), "{bad:?}");
        }
    }

    #[test]
    fn a_public_key_of_another_pair_is_a_mismatch() {
        let a = generate_keys().unwrap();
        let b = generate_keys().unwrap();
        assert_eq!(
            VapidKeys::from_config("mailto:a@b.test", &a.public_key, &b.private_key).err(),
            Some(VapidError::Mismatch)
        );
    }

    #[test]
    fn the_scalar_one_is_a_pair_with_the_generator() {
        // Smallest legal private key, as a check that leading zeros are kept.
        let mut one = [0u8; 32];
        one[31] = 1;
        let public = p256::public_from_scalar(&one).unwrap();
        let keys = VapidKeys::from_config(
            "mailto:a@b.test",
            &base64url::encode(&public),
            &base64url::encode(&one),
        )
        .unwrap();
        assert_eq!(keys.private_key().len(), 43);
        let token = keys.token("https://a.test", 1).unwrap();
        assert!(verify(&token, &public));
    }
}
