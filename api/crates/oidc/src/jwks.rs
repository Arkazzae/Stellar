//! The provider's key set: which keys it publishes and which of them can
//! verify a signature.

use std::fmt;

use serde_json::{Map, Value};

use crate::MAX_DOCUMENT;
use crate::b64;
use crate::jws::Alg;

const MAX_KEYS: usize = 64;
const MAX_KID_LEN: usize = 256;
/// The key sizes the signature verifier accepts for RSA, in bytes of modulus.
const RSA_MODULUS_BYTES: std::ops::RangeInclusive<usize> = 256..=1024;
const MAX_EXPONENT_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Curve {
    P256,
    P384,
    P521,
}

impl Curve {
    fn parse(name: &str) -> Option<(Self, usize)> {
        match name {
            "P-256" => Some((Self::P256, 32)),
            "P-384" => Some((Self::P384, 48)),
            "P-521" => Some((Self::P521, 66)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Material {
    Rsa {
        n: Vec<u8>,
        e: Vec<u8>,
    },
    /// An uncompressed point, `0x04 || x || y`.
    Ec {
        curve: Curve,
        point: Vec<u8>,
    },
}

/// A public key that can verify signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jwk {
    pub kid: Option<String>,
    /// The one algorithm the provider says the key is for, if it says.
    pub alg: Option<Alg>,
    pub(crate) material: Material,
}

/// The usable keys of a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySet {
    keys: Vec<Jwk>,
    skipped: usize,
}

impl KeySet {
    pub fn keys(&self) -> &[Jwk] {
        &self.keys
    }

    /// How many entries were left out as unusable: a key for encryption, of
    /// a kind or size no token here may use, or malformed.
    pub fn skipped(&self) -> usize {
        self.skipped
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwksError {
    TooLarge,
    NotJson,
    TooManyKeys,
    /// The set holds no key that can verify a signature.
    NoUsableKeys,
}

impl fmt::Display for JwksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "the key set is too large",
            Self::NotJson => "the key set is not a JSON object with a keys array",
            Self::TooManyKeys => "the key set holds too many keys",
            Self::NoUsableKeys => "the key set holds no usable key",
        })
    }
}

impl std::error::Error for JwksError {}

/// Reads the document at the provider's `jwks_uri`. A key that cannot be used
/// is skipped, not fatal: providers publish keys for other purposes beside
/// the signing ones.
pub fn parse(body: &[u8]) -> Result<KeySet, JwksError> {
    if body.len() > MAX_DOCUMENT {
        return Err(JwksError::TooLarge);
    }
    let Ok(Value::Object(doc)) = serde_json::from_slice::<Value>(body) else {
        return Err(JwksError::NotJson);
    };
    let Some(Value::Array(entries)) = doc.get("keys") else {
        return Err(JwksError::NotJson);
    };
    if entries.len() > MAX_KEYS {
        return Err(JwksError::TooManyKeys);
    }
    let keys: Vec<Jwk> = entries.iter().filter_map(read_key).collect();
    if keys.is_empty() {
        return Err(JwksError::NoUsableKeys);
    }
    Ok(KeySet {
        skipped: entries.len() - keys.len(),
        keys,
    })
}

fn read_key(entry: &Value) -> Option<Jwk> {
    let key = entry.as_object()?;
    if let Some(usage) = key.get("use")
        && usage.as_str() != Some("sig")
    {
        return None;
    }
    if let Some(operations) = key.get("key_ops")
        && !operations
            .as_array()?
            .iter()
            .any(|op| op.as_str() == Some("verify"))
    {
        return None;
    }
    let alg = match key.get("alg") {
        None => None,
        Some(name) => Some(Alg::parse(name.as_str()?)?),
    };
    let kid = match key.get("kid") {
        None => None,
        Some(Value::String(kid)) if kid.len() <= MAX_KID_LEN => Some(kid.clone()),
        Some(_) => return None,
    };
    let material = match key.get("kty")?.as_str()? {
        "RSA" => rsa(key)?,
        "EC" => ec(key)?,
        _ => return None,
    };
    let jwk = Jwk { kid, alg, material };
    // A key that names an algorithm its own kind cannot do is a broken entry.
    if jwk.alg.is_some_and(|alg| !alg.fits(&jwk)) {
        return None;
    }
    Some(jwk)
}

fn field(key: &Map<String, Value>, name: &str) -> Option<Vec<u8>> {
    b64::decode(key.get(name)?.as_str()?)
}

fn rsa(key: &Map<String, Value>) -> Option<Material> {
    let n = strip_zeros(field(key, "n")?);
    let e = strip_zeros(field(key, "e")?);
    (RSA_MODULUS_BYTES.contains(&n.len()) && !e.is_empty() && e.len() <= MAX_EXPONENT_BYTES)
        .then_some(Material::Rsa { n, e })
}

fn strip_zeros(bytes: Vec<u8>) -> Vec<u8> {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    bytes[start..].to_vec()
}

fn ec(key: &Map<String, Value>) -> Option<Material> {
    let (curve, size) = Curve::parse(key.get("crv")?.as_str()?)?;
    let (x, y) = (field(key, "x")?, field(key, "y")?);
    // Both coordinates are exactly as long as the curve's field.
    if x.len() != size || y.len() != size {
        return None;
    }
    let mut point = vec![4];
    point.extend(x);
    point.extend(y);
    Some(Material::Ec { curve, point })
}
