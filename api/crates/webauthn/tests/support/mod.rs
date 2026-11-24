//! A software authenticator for the tests. It signs with aws-lc-rs and writes
//! its own CBOR, and every answer it builds is complete and correctly signed
//! unless a test asks for one thing to be different.

#![allow(dead_code)]

use aws_lc_rs::digest;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{
    self, EcdsaKeyPair, Ed25519KeyPair, KeyPair, RsaEncoding, RsaKeyPair, RsaPublicKeyComponents,
};
use std::sync::OnceLock;

use stellarr_webauthn::{
    Algorithm, AssertionBody, AssertionResponse, AttestationBody, Registration,
    RegistrationResponse, RelyingParty, b64, verify_registration,
};

pub const RP_ID: &str = "chat.test";
pub const ORIGIN: &str = "https://chat.test";
pub const UP: u8 = 0x01;
pub const UV: u8 = 0x04;
pub const BE: u8 = 0x08;
pub const BS: u8 = 0x10;
pub const AT: u8 = 0x40;
pub const ED: u8 = 0x80;

pub fn relying_party() -> RelyingParty {
    RelyingParty::new(RP_ID, "Stellarr", &[ORIGIN]).unwrap()
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    digest::digest(&digest::SHA256, data).as_ref().to_vec()
}

// ---- a minimal CBOR writer, separate from the crate's reader ----

pub fn head(major: u8, n: u64) -> Vec<u8> {
    let m = major << 5;
    match n {
        0..=23 => vec![m | n as u8],
        24..=0xff => vec![m | 24, n as u8],
        0x100..=0xffff => [vec![m | 25], (n as u16).to_be_bytes().to_vec()].concat(),
        0x1_0000..=0xffff_ffff => [vec![m | 26], (n as u32).to_be_bytes().to_vec()].concat(),
        _ => [vec![m | 27], n.to_be_bytes().to_vec()].concat(),
    }
}

pub fn int(n: i64) -> Vec<u8> {
    if n >= 0 {
        head(0, n as u64)
    } else {
        head(1, (-1 - n) as u64)
    }
}

pub fn bytes(b: &[u8]) -> Vec<u8> {
    [head(2, b.len() as u64), b.to_vec()].concat()
}

pub fn text(s: &str) -> Vec<u8> {
    [head(3, s.len() as u64), s.as_bytes().to_vec()].concat()
}

pub fn map(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = head(5, entries.len() as u64);
    for (k, v) in entries {
        out.extend_from_slice(k);
        out.extend_from_slice(v);
    }
    out
}

// ---- keys ----

enum Signer {
    Ed(Ed25519KeyPair),
    Ec(EcdsaKeyPair, usize),
    Rsa(RsaKeyPair, &'static dyn RsaEncoding),
}

pub struct Key {
    pub algorithm: Algorithm,
    signer: Signer,
    /// The COSE_Key an authenticator would put in its attested data.
    pub cose: Vec<u8>,
}

impl Key {
    pub fn generate(algorithm: Algorithm) -> Key {
        let rng = SystemRandom::new();
        let label = |n: i64| int(n);
        match algorithm {
            Algorithm::EdDsa => {
                let pair = Ed25519KeyPair::generate().unwrap();
                let cose = map(&[
                    (label(1), int(1)),
                    (label(3), int(-8)),
                    (label(-1), int(6)),
                    (label(-2), bytes(pair.public_key().as_ref())),
                ]);
                Key {
                    algorithm,
                    signer: Signer::Ed(pair),
                    cose,
                }
            }
            Algorithm::Es256 | Algorithm::Es384 | Algorithm::Es512 => {
                let (signing, crv, size) = match algorithm {
                    Algorithm::Es256 => (&signature::ECDSA_P256_SHA256_ASN1_SIGNING, 1, 32),
                    Algorithm::Es384 => (&signature::ECDSA_P384_SHA384_ASN1_SIGNING, 2, 48),
                    _ => (&signature::ECDSA_P521_SHA512_ASN1_SIGNING, 3, 66),
                };
                let pkcs8 = EcdsaKeyPair::generate_pkcs8(signing, &rng).unwrap();
                let pair = EcdsaKeyPair::from_pkcs8(signing, pkcs8.as_ref()).unwrap();
                let point = pair.public_key().as_ref().to_vec();
                assert_eq!(point.len(), 1 + 2 * size);
                let cose = map(&[
                    (label(1), int(2)),
                    (label(3), int(algorithm.cose_id())),
                    (label(-1), int(crv)),
                    (label(-2), bytes(&point[1..1 + size])),
                    (label(-3), bytes(&point[1 + size..])),
                ]);
                Key {
                    algorithm,
                    signer: Signer::Ec(pair, size),
                    cose,
                }
            }
            _ => {
                let pair = RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
                let parts = RsaPublicKeyComponents::<Vec<u8>>::from(pair.public_key());
                let encoding: &'static dyn RsaEncoding = match algorithm {
                    Algorithm::Rs256 => &signature::RSA_PKCS1_SHA256,
                    Algorithm::Rs384 => &signature::RSA_PKCS1_SHA384,
                    Algorithm::Rs512 => &signature::RSA_PKCS1_SHA512,
                    Algorithm::Ps256 => &signature::RSA_PSS_SHA256,
                    Algorithm::Ps384 => &signature::RSA_PSS_SHA384,
                    _ => &signature::RSA_PSS_SHA512,
                };
                let cose = map(&[
                    (label(1), int(3)),
                    (label(3), int(algorithm.cose_id())),
                    (label(-1), bytes(&parts.n)),
                    (label(-2), bytes(&parts.e)),
                ]);
                Key {
                    algorithm,
                    signer: Signer::Rsa(pair, encoding),
                    cose,
                }
            }
        }
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let rng = SystemRandom::new();
        match &self.signer {
            Signer::Ed(pair) => pair.sign(message).as_ref().to_vec(),
            Signer::Ec(pair, _) => pair.sign(&rng, message).unwrap().as_ref().to_vec(),
            Signer::Rsa(pair, encoding) => {
                let mut out = vec![0u8; pair.public_modulus_len()];
                pair.sign(*encoding, &rng, message, &mut out).unwrap();
                out
            }
        }
    }
}

// ---- client data ----

#[derive(Clone)]
pub struct ClientDataSpec {
    pub kind: String,
    pub challenge: Vec<u8>,
    pub origin: String,
    pub cross_origin: Option<bool>,
    pub top_origin: Option<String>,
    pub raw: Option<Vec<u8>>,
}

impl ClientDataSpec {
    pub fn new(kind: &str, challenge: &[u8]) -> ClientDataSpec {
        ClientDataSpec {
            kind: kind.into(),
            challenge: challenge.to_vec(),
            origin: ORIGIN.into(),
            cross_origin: Some(false),
            top_origin: None,
            raw: None,
        }
    }

    pub fn json(&self) -> Vec<u8> {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let mut members = vec![
            format!("\"type\":\"{}\"", self.kind),
            format!("\"challenge\":\"{}\"", b64::encode(&self.challenge)),
            format!("\"origin\":\"{}\"", self.origin),
        ];
        if let Some(flag) = self.cross_origin {
            members.push(format!("\"crossOrigin\":{flag}"));
        }
        if let Some(top) = &self.top_origin {
            members.push(format!("\"topOrigin\":\"{top}\""));
        }
        format!("{{{}}}", members.join(",")).into_bytes()
    }
}

// ---- registration ----

#[derive(Clone)]
pub enum Statement {
    None,
    /// Packed self attestation, signed with the credential key.
    PackedSelf,
    /// An explicit format and statement, written by the test.
    Custom(String, Vec<u8>),
}

#[derive(Clone)]
pub struct RegistrationSpec {
    pub rp_id: String,
    pub flags: u8,
    pub sign_count: u32,
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    /// What `rawId` and `id` say, when it should differ from the attested id.
    pub raw_id: Option<Vec<u8>>,
    pub id: Option<String>,
    pub kind: String,
    pub client: ClientDataSpec,
    pub statement: Statement,
    /// Replaces the COSE key in the attested data.
    pub cose: Option<Vec<u8>>,
    /// Bytes appended after the authenticator data (and its extensions).
    pub trailing: Vec<u8>,
    pub extensions: Option<Vec<u8>>,
    pub transports: Option<Vec<String>>,
    pub attestation_object: Option<Vec<u8>>,
    pub attestation_alg: Option<i64>,
}

impl RegistrationSpec {
    pub fn new(challenge: &[u8]) -> RegistrationSpec {
        RegistrationSpec {
            rp_id: RP_ID.into(),
            flags: UP | UV | AT,
            sign_count: 0,
            aaguid: [7; 16],
            credential_id: (0..32u8)
                .map(|n| n.wrapping_mul(7).wrapping_add(3))
                .collect(),
            raw_id: None,
            id: None,
            kind: "public-key".into(),
            client: ClientDataSpec::new("webauthn.create", challenge),
            statement: Statement::PackedSelf,
            cose: None,
            trailing: vec![],
            extensions: None,
            transports: Some(vec!["internal".into()]),
            attestation_object: None,
            attestation_alg: None,
        }
    }

    pub fn auth_data(&self, key: &Key) -> Vec<u8> {
        let mut out = sha256(self.rp_id.as_bytes());
        out.push(self.flags);
        out.extend_from_slice(&self.sign_count.to_be_bytes());
        out.extend_from_slice(&self.aaguid);
        out.extend_from_slice(&(self.credential_id.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.credential_id);
        out.extend_from_slice(self.cose.as_ref().unwrap_or(&key.cose));
        if let Some(extensions) = &self.extensions {
            out.extend_from_slice(extensions);
        }
        out.extend_from_slice(&self.trailing);
        out
    }

    pub fn build(&self, key: &Key) -> RegistrationResponse {
        let client = self.client.json();
        let auth = self.auth_data(key);
        let mut signed = auth.clone();
        signed.extend_from_slice(&sha256(&client));
        let (format, statement) = match &self.statement {
            Statement::None => ("none".to_string(), map(&[])),
            Statement::PackedSelf => (
                "packed".to_string(),
                map(&[
                    (
                        text("alg"),
                        int(self.attestation_alg.unwrap_or(key.algorithm.cose_id())),
                    ),
                    (text("sig"), bytes(&key.sign(&signed))),
                ]),
            ),
            Statement::Custom(format, statement) => (format.clone(), statement.clone()),
        };
        let object = self.attestation_object.clone().unwrap_or_else(|| {
            // Members are not in sorted order, as some authenticators write them.
            map(&[
                (text("fmt"), text(&format)),
                (text("authData"), bytes(&auth)),
                (text("attStmt"), statement),
            ])
        });
        let raw_id = self
            .raw_id
            .clone()
            .unwrap_or_else(|| self.credential_id.clone());
        RegistrationResponse {
            id: self.id.clone().unwrap_or_else(|| b64::encode(&raw_id)),
            raw_id: b64::encode(&raw_id),
            kind: self.kind.clone(),
            response: AttestationBody {
                attestation_object: b64::encode(&object),
                client_data_json: b64::encode(&client),
                transports: self.transports.clone(),
            },
        }
    }
}

// ---- assertion ----

#[derive(Clone)]
pub struct AssertionSpec {
    pub rp_id: String,
    pub flags: u8,
    pub sign_count: u32,
    pub credential_id: Vec<u8>,
    pub raw_id: Option<Vec<u8>>,
    pub id: Option<String>,
    pub kind: String,
    pub client: ClientDataSpec,
    pub user_handle: Option<String>,
    pub trailing: Vec<u8>,
    pub extensions: Option<Vec<u8>>,
    /// Attested credential data to put in the assertion, when a test wants it.
    pub attested: Option<Vec<u8>>,
    pub signature: Option<Vec<u8>>,
}

impl AssertionSpec {
    pub fn new(credential_id: &[u8], challenge: &[u8]) -> AssertionSpec {
        AssertionSpec {
            rp_id: RP_ID.into(),
            flags: UP | UV,
            sign_count: 1,
            credential_id: credential_id.to_vec(),
            raw_id: None,
            id: None,
            kind: "public-key".into(),
            client: ClientDataSpec::new("webauthn.get", challenge),
            user_handle: None,
            trailing: vec![],
            extensions: None,
            attested: None,
            signature: None,
        }
    }

    pub fn auth_data(&self) -> Vec<u8> {
        let mut out = sha256(self.rp_id.as_bytes());
        out.push(self.flags);
        out.extend_from_slice(&self.sign_count.to_be_bytes());
        if let Some(attested) = &self.attested {
            out.extend_from_slice(attested);
        }
        if let Some(extensions) = &self.extensions {
            out.extend_from_slice(extensions);
        }
        out.extend_from_slice(&self.trailing);
        out
    }

    pub fn build(&self, key: &Key) -> AssertionResponse {
        let client = self.client.json();
        let auth = self.auth_data();
        let mut signed = auth.clone();
        signed.extend_from_slice(&sha256(&client));
        let signature = self.signature.clone().unwrap_or_else(|| key.sign(&signed));
        let raw_id = self
            .raw_id
            .clone()
            .unwrap_or_else(|| self.credential_id.clone());
        AssertionResponse {
            id: self.id.clone().unwrap_or_else(|| b64::encode(&raw_id)),
            raw_id: b64::encode(&raw_id),
            kind: self.kind.clone(),
            response: AssertionBody {
                authenticator_data: b64::encode(&auth),
                client_data_json: b64::encode(&client),
                signature: b64::encode(&signature),
                user_handle: self.user_handle.clone(),
            },
        }
    }
}

// ---- shared keys and enrolment ----

/// One key per algorithm for the whole test run: generating RSA keys is slow.
pub fn key(algorithm: Algorithm) -> &'static Key {
    static KEYS: [OnceLock<Key>; 10] = [const { OnceLock::new() }; 10];
    let index = Algorithm::OFFERED
        .iter()
        .position(|a| *a == algorithm)
        .unwrap();
    KEYS[index].get_or_init(|| Key::generate(algorithm))
}

/// A second key of the same algorithm, for "someone else signed this".
pub fn other_key(algorithm: Algorithm) -> &'static Key {
    static KEYS: [OnceLock<Key>; 10] = [const { OnceLock::new() }; 10];
    let index = Algorithm::OFFERED
        .iter()
        .position(|a| *a == algorithm)
        .unwrap();
    KEYS[index].get_or_init(|| Key::generate(algorithm))
}

pub const CHALLENGE: [u8; 32] = [0xA5; 32];
pub const USER_HANDLE: [u8; 16] = [0x42; 16];

/// A registration of `key` through the real verifier.
pub fn enroll(algorithm: Algorithm) -> Registration {
    let spec = RegistrationSpec::new(&CHALLENGE);
    verify_registration(&relying_party(), &CHALLENGE, &spec.build(key(algorithm))).unwrap()
}
