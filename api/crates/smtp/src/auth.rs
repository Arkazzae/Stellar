//! SASL mechanisms: PLAIN, LOGIN and SCRAM (SHA-256 and SHA-1).
//!
//! CRAM-MD5, DIGEST-MD5 and NTLM are not implemented: the crypto library the
//! project uses has no MD5 or MD4 (nor does it offer them as a MAC), and
//! none may be written by hand. SCRAM-SHA-512 and the channel-binding
//! variants (-PLUS) are left out too. A server that offers only those is
//! reported as one without a usable mechanism.

use aws_lc_rs::{constant_time, digest, hmac, pbkdf2, rand};
use std::num::NonZeroU32;

use crate::encode::{base64, base64_decode};

/// The most iterations of the password hash a server may ask for; more would
/// let a hostile server hold the thread for long.
const MAX_ITERATIONS: u32 = 600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mechanism {
    ScramSha256,
    ScramSha1,
    Plain,
    Login,
}

impl Mechanism {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mechanism::ScramSha256 => "SCRAM-SHA-256",
            Mechanism::ScramSha1 => "SCRAM-SHA-1",
            Mechanism::Plain => "PLAIN",
            Mechanism::Login => "LOGIN",
        }
    }

    /// The mechanisms to try with these credentials, in order: the strongest
    /// SCRAM the server offers, then PLAIN, or LOGIN when PLAIN is not
    /// offered. SCRAM is advertised by servers that have no SCRAM secret for
    /// the user, so a refusal of it is followed by the simple one; trying
    /// PLAIN and then LOGIN would repeat the same refusal and only count as
    /// one more failed login. SCRAM needs the user name and password
    /// prepared with SASLprep, which is not done here, so it is only used
    /// when both are plain ASCII.
    pub(crate) fn candidates(offered: &[String], username: &str, password: &str) -> Vec<Mechanism> {
        let has = |m: Mechanism| offered.iter().any(|o| o.eq_ignore_ascii_case(m.name()));
        let mut order = Vec::new();
        if username.is_ascii() && password.is_ascii() {
            order.extend(
                [Mechanism::ScramSha256, Mechanism::ScramSha1]
                    .into_iter()
                    .find(|&m| has(m)),
            );
        }
        order.extend(
            [Mechanism::Plain, Mechanism::Login]
                .into_iter()
                .find(|&m| has(m)),
        );
        order
    }
}

pub(crate) fn plain(username: &str, password: &str) -> Vec<u8> {
    let mut out = vec![0];
    out.extend_from_slice(username.as_bytes());
    out.push(0);
    out.extend_from_slice(password.as_bytes());
    out
}

#[derive(Debug, Clone, Copy)]
enum Hash {
    Sha256,
    Sha1,
}

impl Hash {
    fn hmac(self) -> hmac::Algorithm {
        match self {
            Hash::Sha256 => hmac::HMAC_SHA256,
            Hash::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        }
    }

    fn digest(self) -> &'static digest::Algorithm {
        match self {
            Hash::Sha256 => &digest::SHA256,
            Hash::Sha1 => &digest::SHA1_FOR_LEGACY_USE_ONLY,
        }
    }

    fn pbkdf2(self) -> pbkdf2::Algorithm {
        match self {
            Hash::Sha256 => pbkdf2::PBKDF2_HMAC_SHA256,
            Hash::Sha1 => pbkdf2::PBKDF2_HMAC_SHA1,
        }
    }

    fn sign(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        hmac::sign(&hmac::Key::new(self.hmac(), key), data)
            .as_ref()
            .to_vec()
    }
}

/// One SCRAM exchange (RFC 5802, RFC 7677), without channel binding.
pub(crate) struct Scram {
    hash: Hash,
    password: String,
    nonce: String,
    first_bare: String,
    /// The signature the server must answer with, known once the server's
    /// first message has been taken.
    expected_signature: Option<Vec<u8>>,
}

impl Scram {
    pub(crate) fn new(
        mechanism: Mechanism,
        username: &str,
        password: &str,
    ) -> Option<(Scram, Vec<u8>)> {
        let mut raw = [0u8; 18];
        rand::fill(&mut raw).ok()?;
        Scram::with_nonce(mechanism, username, password, base64(&raw))
    }

    pub(crate) fn with_nonce(
        mechanism: Mechanism,
        username: &str,
        password: &str,
        nonce: String,
    ) -> Option<(Scram, Vec<u8>)> {
        let hash = match mechanism {
            Mechanism::ScramSha256 => Hash::Sha256,
            Mechanism::ScramSha1 => Hash::Sha1,
            _ => return None,
        };
        let user = username.replace('=', "=3D").replace(',', "=2C");
        let first_bare = format!("n={user},r={nonce}");
        let first = format!("n,,{first_bare}").into_bytes();
        let scram = Scram {
            hash,
            password: password.to_owned(),
            nonce,
            first_bare,
            expected_signature: None,
        };
        Some((scram, first))
    }

    /// Takes the server's first message and returns the client's final one.
    pub(crate) fn respond(&mut self, server_first: &[u8]) -> Result<Vec<u8>, String> {
        let server_first =
            std::str::from_utf8(server_first).map_err(|_| "the server's challenge is not text")?;
        let mut nonce = None;
        let mut salt = None;
        let mut iterations = None;
        for field in server_first.split(',') {
            match field.split_once('=') {
                Some(("r", v)) => nonce = Some(v),
                Some(("s", v)) => salt = base64_decode(v),
                Some(("i", v)) => iterations = v.parse::<u32>().ok(),
                Some(("m", _)) => {
                    return Err("the server requires an extension that is not supported".into());
                }
                _ => {}
            }
        }
        let nonce = nonce.ok_or("the server's challenge has no nonce")?;
        if !nonce.starts_with(&self.nonce) || nonce.len() == self.nonce.len() {
            return Err("the server's nonce does not extend ours".into());
        }
        let salt = salt.ok_or("the server's challenge has no usable salt")?;
        let iterations = iterations
            .filter(|&i| i <= MAX_ITERATIONS)
            .and_then(NonZeroU32::new)
            .ok_or("the server's iteration count is missing or out of range")?;

        let mut salted = vec![0u8; self.hash.digest().output_len()];
        pbkdf2::derive(
            self.hash.pbkdf2(),
            iterations,
            &salt,
            self.password.as_bytes(),
            &mut salted,
        );
        let client_key = self.hash.sign(&salted, b"Client Key");
        let stored_key = digest::digest(self.hash.digest(), &client_key);
        let without_proof = format!("c=biws,r={nonce}");
        let auth_message = format!("{},{server_first},{without_proof}", self.first_bare);
        let signature = self.hash.sign(stored_key.as_ref(), auth_message.as_bytes());
        let proof: Vec<u8> = client_key
            .iter()
            .zip(&signature)
            .map(|(a, b)| a ^ b)
            .collect();
        let server_key = self.hash.sign(&salted, b"Server Key");
        self.expected_signature = Some(self.hash.sign(&server_key, auth_message.as_bytes()));
        Ok(format!("{without_proof},p={}", base64(&proof)).into_bytes())
    }

    /// Checks the server's final message: the server proves that it knows the
    /// password too.
    pub(crate) fn verify(&self, server_final: &[u8]) -> Result<(), String> {
        let text = std::str::from_utf8(server_final)
            .map_err(|_| "the server's final message is not text")?;
        if let Some(error) = text.strip_prefix("e=") {
            return Err(format!("the server refused the proof: {error}"));
        }
        let signature = text
            .strip_prefix("v=")
            .and_then(base64_decode)
            .ok_or("the server's final message has no signature")?;
        let expected = self
            .expected_signature
            .as_deref()
            .ok_or("the exchange is out of order")?;
        constant_time::verify_slices_are_equal(&signature, expected)
            .map_err(|_| "the server's signature is wrong; it may not know the password".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The vectors of RFC 7677 section 3 and RFC 5802 section 5, and the same
    // values recomputed with Python's hashlib and hmac.
    #[test]
    fn scram_sha_256_matches_the_rfc_7677_exchange() {
        let (mut scram, first) = Scram::with_nonce(
            Mechanism::ScramSha256,
            "user",
            "pencil",
            "rOprNGfwEbeRWgbNEkqO".into(),
        )
        .unwrap();
        assert_eq!(first, b"n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
        let last = scram
            .respond(b"r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096")
            .unwrap();
        assert_eq!(
            String::from_utf8(last).unwrap(),
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        scram
            .verify(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .unwrap();
        assert!(
            scram
                .verify(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G5=")
                .is_err()
        );
        assert!(scram.verify(b"e=invalid-proof").is_err());
        assert!(scram.verify(b"").is_err());
    }

    #[test]
    fn scram_sha_1_matches_the_rfc_5802_exchange() {
        let (mut scram, first) = Scram::with_nonce(
            Mechanism::ScramSha1,
            "user",
            "pencil",
            "fyko+d2lbbFgONRv9qkxdawL".into(),
        )
        .unwrap();
        assert_eq!(first, b"n,,n=user,r=fyko+d2lbbFgONRv9qkxdawL");
        let last = scram
            .respond(b"r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096")
            .unwrap();
        assert_eq!(
            String::from_utf8(last).unwrap(),
            "c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,p=v0X8v3Bz2T0CJGbJQyF0X+HI4Ts="
        );
        scram.verify(b"v=rmF9pqV8S7suAoZWja4dJRkFsKQ=").unwrap();
    }

    #[test]
    fn a_hostile_challenge_is_refused() {
        let fresh = || {
            Scram::with_nonce(Mechanism::ScramSha256, "u", "p", "abc".into())
                .unwrap()
                .0
        };
        for bad in [
            "r=abc,s=c2FsdA==,i=4096",          // nonce not extended
            "r=xyzabc,s=c2FsdA==,i=4096",       // nonce does not start with ours
            "r=abcdef,s=c2FsdA==",              // no iterations
            "r=abcdef,s=c2FsdA==,i=0",          // zero
            "r=abcdef,s=c2FsdA==,i=4000000000", // far too many
            "r=abcdef,i=4096",                  // no salt
            "r=abcdef,s=!!!,i=4096",            // salt is not base64
            "m=x,r=abcdef,s=c2FsdA==,i=4096",   // mandatory extension
            "",
        ] {
            assert!(fresh().respond(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(fresh().respond(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn the_user_name_is_escaped() {
        let (_, first) = Scram::with_nonce(Mechanism::ScramSha1, "a=b,c", "p", "n".into()).unwrap();
        assert_eq!(first, b"n,,n=a=3Db=2Cc,r=n");
    }

    #[test]
    fn the_strongest_scram_and_one_simple_mechanism_are_tried() {
        let offered = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let pick = |names: &[&str], u: &str, p: &str| Mechanism::candidates(&offered(names), u, p);
        use Mechanism::*;
        assert_eq!(
            pick(
                &["PLAIN", "LOGIN", "SCRAM-SHA-1", "SCRAM-SHA-256"],
                "u",
                "p"
            ),
            [ScramSha256, Plain]
        );
        assert_eq!(
            pick(&["SCRAM-SHA-1", "LOGIN"], "u", "p"),
            [ScramSha1, Login]
        );
        assert_eq!(pick(&["login", "plain"], "u", "p"), [Plain]);
        assert_eq!(pick(&["LOGIN"], "u", "p"), [Login]);
        assert!(
            pick(
                &[
                    "CRAM-MD5",
                    "DIGEST-MD5",
                    "NTLM",
                    "XOAUTH2",
                    "SCRAM-SHA-512",
                    "SCRAM-SHA-256-PLUS"
                ],
                "u",
                "p"
            )
            .is_empty()
        );
        // Without SASLprep a non-ASCII secret does not go through SCRAM.
        assert_eq!(pick(&["SCRAM-SHA-256", "PLAIN"], "u", "zażółć"), [Plain]);
        assert!(pick(&["SCRAM-SHA-256"], "u", "zażółć").is_empty());
    }

    #[test]
    fn plain_joins_the_parts_with_nul() {
        assert_eq!(plain("jo", "pw"), b"\0jo\0pw");
    }
}
