//! COSE public keys (RFC 9052/9053) for the algorithms the ceremonies offer,
//! and the signature check made with them.
//!
//! A key is accepted only when its algorithm, key type and curve (or size)
//! agree, with coordinates of exactly the right length. Anything else, such
//! as secp256k1, Ed448 or a SHA-1 RSA key, is refused here, once, and every
//! later step can rely on it.

use aws_lc_rs::signature::{
    self, ParsedPublicKey, RsaParameters, RsaPublicKeyComponents, VerificationAlgorithm,
};

use crate::cbor::{self, Value};
use crate::error::Error;

/// The signature algorithms the registration options offer, by COSE number,
/// and the only ones a credential key may use. The order is the order of
/// preference sent to the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    EdDsa,
    Es256,
    Rs256,
    Ps256,
    Es384,
    Rs384,
    Ps384,
    Es512,
    Rs512,
    Ps512,
}

impl Algorithm {
    pub const OFFERED: [Algorithm; 10] = [
        Algorithm::EdDsa,
        Algorithm::Es256,
        Algorithm::Rs256,
        Algorithm::Ps256,
        Algorithm::Es384,
        Algorithm::Rs384,
        Algorithm::Ps384,
        Algorithm::Es512,
        Algorithm::Rs512,
        Algorithm::Ps512,
    ];

    pub fn cose_id(self) -> i64 {
        match self {
            Algorithm::EdDsa => -8,
            Algorithm::Es256 => -7,
            Algorithm::Es384 => -35,
            Algorithm::Es512 => -36,
            Algorithm::Rs256 => -257,
            Algorithm::Rs384 => -258,
            Algorithm::Rs512 => -259,
            Algorithm::Ps256 => -37,
            Algorithm::Ps384 => -38,
            Algorithm::Ps512 => -39,
        }
    }

    pub fn from_cose_id(id: i64) -> Option<Algorithm> {
        Algorithm::OFFERED
            .into_iter()
            .find(|alg| alg.cose_id() == id)
    }
}

const KTY_OKP: i64 = 1;
const KTY_EC2: i64 = 2;
const KTY_RSA: i64 = 3;
const CRV_P256: i64 = 1;
const CRV_P384: i64 = 2;
const CRV_P521: i64 = 3;
const CRV_ED25519: i64 = 6;

/// 2048 to 8192 bits, the range the verifier itself accepts.
const RSA_MODULUS_BYTES: std::ops::RangeInclusive<usize> = 256..=1024;
const RSA_EXPONENT_MAX_BYTES: usize = 8;

/// A credential public key that passed every check of this module.
#[derive(Clone)]
pub struct PublicKey {
    algorithm: Algorithm,
    parsed: ParsedPublicKey,
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicKey")
            .field("algorithm", &self.algorithm)
            .finish_non_exhaustive()
    }
}

impl PublicKey {
    /// Parses a COSE_Key held as exactly one CBOR item.
    pub fn from_cose(bytes: &[u8]) -> Result<PublicKey, Error> {
        PublicKey::from_value(&cbor::decode_exact(bytes)?)
    }

    pub(crate) fn from_value(value: &Value) -> Result<PublicKey, Error> {
        let Value::Map(entries) = value else {
            return Err(Error::MalformedCoseKey("not a map"));
        };
        // Only the labels this module reads are allowed, so that the stored
        // bytes mean exactly what was checked.
        for (label, _) in entries {
            if !matches!(label, Value::Int(1 | 3 | -1 | -2 | -3)) {
                return Err(Error::MalformedCoseKey("unexpected label"));
            }
        }

        let kty = int(value, 1, "kty")?;
        let alg_id = int(value, 3, "alg")?;
        let algorithm = Algorithm::from_cose_id(alg_id).ok_or(Error::AlgorithmNotOffered)?;
        let parsed = match algorithm {
            Algorithm::EdDsa => okp(value, kty, algorithm)?,
            Algorithm::Es256 | Algorithm::Es384 | Algorithm::Es512 => ec2(value, kty, algorithm)?,
            Algorithm::Rs256
            | Algorithm::Rs384
            | Algorithm::Rs512
            | Algorithm::Ps256
            | Algorithm::Ps384
            | Algorithm::Ps512 => rsa(value, kty, algorithm)?,
        };
        Ok(PublicKey { algorithm, parsed })
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// Checks a signature in the form WebAuthn carries it: DER for ECDSA, the
    /// raw 64 bytes for Ed25519, the plain integer for RSA.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        self.parsed.verify_sig(message, signature).is_ok()
    }
}

fn int(map: &Value, label: i64, name: &'static str) -> Result<i64, Error> {
    match map.get_int(label) {
        Some(Value::Int(n)) => Ok(*n),
        _ => Err(Error::MalformedCoseKey(name)),
    }
}

fn bytes<'a>(map: &'a Value, label: i64, name: &'static str) -> Result<&'a [u8], Error> {
    match map.get_int(label) {
        Some(Value::Bytes(b)) => Ok(b),
        _ => Err(Error::MalformedCoseKey(name)),
    }
}

fn has(map: &Value, label: i64) -> bool {
    map.get_int(label).is_some()
}

fn okp(map: &Value, kty: i64, algorithm: Algorithm) -> Result<ParsedPublicKey, Error> {
    debug_assert_eq!(algorithm, Algorithm::EdDsa);
    if kty != KTY_OKP {
        return Err(Error::MalformedCoseKey("kty does not fit alg"));
    }
    if int(map, -1, "crv")? != CRV_ED25519 {
        return Err(Error::MalformedCoseKey("crv does not fit alg"));
    }
    if has(map, -3) {
        return Err(Error::MalformedCoseKey("y on an OKP key"));
    }
    let x = bytes(map, -2, "x")?;
    if x.len() != 32 {
        return Err(Error::MalformedCoseKey("x has the wrong length"));
    }
    ParsedPublicKey::new(&signature::ED25519, x)
        .map_err(|_| Error::MalformedCoseKey("point rejected"))
}

fn ec2(map: &Value, kty: i64, algorithm: Algorithm) -> Result<ParsedPublicKey, Error> {
    if kty != KTY_EC2 {
        return Err(Error::MalformedCoseKey("kty does not fit alg"));
    }
    let (crv, size, verifier): (i64, usize, &'static dyn VerificationAlgorithm) = match algorithm {
        Algorithm::Es256 => (CRV_P256, 32, &signature::ECDSA_P256_SHA256_ASN1),
        Algorithm::Es384 => (CRV_P384, 48, &signature::ECDSA_P384_SHA384_ASN1),
        _ => (CRV_P521, 66, &signature::ECDSA_P521_SHA512_ASN1),
    };
    if int(map, -1, "crv")? != crv {
        return Err(Error::MalformedCoseKey("crv does not fit alg"));
    }
    let x = bytes(map, -2, "x")?;
    let y = bytes(map, -3, "y")?;
    if x.len() != size || y.len() != size {
        return Err(Error::MalformedCoseKey("coordinate has the wrong length"));
    }
    let mut point = Vec::with_capacity(1 + 2 * size);
    point.push(0x04);
    point.extend_from_slice(x);
    point.extend_from_slice(y);
    // Rejects a point that is not on the curve.
    ParsedPublicKey::new(verifier, point).map_err(|_| Error::MalformedCoseKey("point rejected"))
}

fn rsa(map: &Value, kty: i64, algorithm: Algorithm) -> Result<ParsedPublicKey, Error> {
    if kty != KTY_RSA {
        return Err(Error::MalformedCoseKey("kty does not fit alg"));
    }
    if has(map, -3) {
        return Err(Error::MalformedCoseKey("y on an RSA key"));
    }
    let n = bytes(map, -1, "n")?;
    let e = bytes(map, -2, "e")?;
    if !RSA_MODULUS_BYTES.contains(&n.len()) || n[0] == 0 {
        return Err(Error::MalformedCoseKey("modulus size"));
    }
    if e.is_empty() || e.len() > RSA_EXPONENT_MAX_BYTES || e[0] == 0 {
        return Err(Error::MalformedCoseKey("exponent size"));
    }
    let params: &'static RsaParameters = match algorithm {
        Algorithm::Rs256 => &signature::RSA_PKCS1_2048_8192_SHA256,
        Algorithm::Rs384 => &signature::RSA_PKCS1_2048_8192_SHA384,
        Algorithm::Rs512 => &signature::RSA_PKCS1_2048_8192_SHA512,
        Algorithm::Ps256 => &signature::RSA_PSS_2048_8192_SHA256,
        Algorithm::Ps384 => &signature::RSA_PSS_2048_8192_SHA384,
        _ => &signature::RSA_PSS_2048_8192_SHA512,
    };
    RsaPublicKeyComponents { n, e }
        .to_parsed_public_key(params)
        .map_err(|_| Error::MalformedCoseKey("key rejected"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offered_algorithms_are_the_ten_of_the_registration_options() {
        let ids: Vec<i64> = Algorithm::OFFERED.iter().map(|a| a.cose_id()).collect();
        assert_eq!(ids, [-8, -7, -257, -37, -35, -258, -38, -36, -259, -39]);
        for alg in Algorithm::OFFERED {
            assert_eq!(Algorithm::from_cose_id(alg.cose_id()), Some(alg));
        }
        // SHA-1 RSA (-65535) and secp256k1 ECDSA (-47) are not offered.
        assert_eq!(Algorithm::from_cose_id(-65535), None);
        assert_eq!(Algorithm::from_cose_id(-47), None);
    }
}
