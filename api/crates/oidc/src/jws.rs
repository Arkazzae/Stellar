//! The signature algorithms an ID token may use, and checking one against a key.

use aws_lc_rs::signature::{self, RsaParameters, RsaPublicKeyComponents, UnparsedPublicKey};

use crate::jwks::{Curve, Jwk, Material};

/// The asymmetric algorithms a provider's key set can serve. No symmetric
/// algorithm is here on purpose: the client secret must never verify a token,
/// and a public key must never be taken as a MAC key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alg {
    Rs256,
    Rs384,
    Rs512,
    Ps256,
    Ps384,
    Ps512,
    Es256,
    Es384,
    Es512,
}

impl Alg {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "RS256" => Self::Rs256,
            "RS384" => Self::Rs384,
            "RS512" => Self::Rs512,
            "PS256" => Self::Ps256,
            "PS384" => Self::Ps384,
            "PS512" => Self::Ps512,
            "ES256" => Self::Es256,
            "ES384" => Self::Es384,
            "ES512" => Self::Es512,
            _ => return None,
        })
    }

    /// The curve an EC key must be on for this algorithm; `None` for RSA ones.
    fn curve(self) -> Option<Curve> {
        match self {
            Self::Es256 => Some(Curve::P256),
            Self::Es384 => Some(Curve::P384),
            Self::Es512 => Some(Curve::P521),
            _ => None,
        }
    }

    fn rsa_parameters(self) -> Option<&'static RsaParameters> {
        Some(match self {
            Self::Rs256 => &signature::RSA_PKCS1_2048_8192_SHA256,
            Self::Rs384 => &signature::RSA_PKCS1_2048_8192_SHA384,
            Self::Rs512 => &signature::RSA_PKCS1_2048_8192_SHA512,
            Self::Ps256 => &signature::RSA_PSS_2048_8192_SHA256,
            Self::Ps384 => &signature::RSA_PSS_2048_8192_SHA384,
            Self::Ps512 => &signature::RSA_PSS_2048_8192_SHA512,
            _ => return None,
        })
    }

    /// The digest the algorithm hashes with, which also sizes `at_hash`.
    pub(crate) fn digest(self) -> &'static aws_lc_rs::digest::Algorithm {
        use aws_lc_rs::digest;
        match self {
            Self::Rs256 | Self::Ps256 | Self::Es256 => &digest::SHA256,
            Self::Rs384 | Self::Ps384 | Self::Es384 => &digest::SHA384,
            Self::Rs512 | Self::Ps512 | Self::Es512 => &digest::SHA512,
        }
    }

    /// Whether a key of this material can verify with this algorithm.
    pub(crate) fn fits(self, key: &Jwk) -> bool {
        match (&key.material, self.curve()) {
            (Material::Rsa { .. }, None) => true,
            (Material::Ec { curve, .. }, Some(wanted)) => *curve == wanted,
            _ => false,
        }
    }
}

/// Whether `signature` is the algorithm's signature of `message` by `key`.
pub(crate) fn verify(key: &Jwk, alg: Alg, message: &[u8], signature: &[u8]) -> bool {
    match &key.material {
        Material::Rsa { n, e } => alg.rsa_parameters().is_some_and(|parameters| {
            RsaPublicKeyComponents { n, e }
                .verify(parameters, message, signature)
                .is_ok()
        }),
        Material::Ec { curve, point } => {
            let algorithm = match curve {
                Curve::P256 => &signature::ECDSA_P256_SHA256_FIXED,
                Curve::P384 => &signature::ECDSA_P384_SHA384_FIXED,
                Curve::P521 => &signature::ECDSA_P521_SHA512_FIXED,
            };
            alg.curve() == Some(*curve)
                && UnparsedPublicKey::new(algorithm, point)
                    .verify(message, signature)
                    .is_ok()
        }
    }
}
