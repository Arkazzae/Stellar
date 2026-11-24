//! Passwords, stored as `pbkdf2-sha256$iterations$salt$hash` with both binary
//! parts in Base64. A hash written with another iteration count still
//! verifies, as long as the count stays inside the accepted window, and a hash
//! written by this server verifies on the first one and the other way round.

use std::num::NonZeroU32;

use aws_lc_rs::pbkdf2::{self, PBKDF2_HMAC_SHA256};
use aws_lc_rs::rand;

use crate::encoding::{base64, from_base64};

const SCHEME: &str = "pbkdf2-sha256";
const ITERATIONS: u32 = 210_000;
const MINIMUM_ACCEPTED_ITERATIONS: u32 = 100_000;
const MAXIMUM_ACCEPTED_ITERATIONS: u32 = 1_000_000;
const SALT_SIZE: usize = 16;
const HASH_SIZE: usize = 32;

pub const MIN_LENGTH: usize = 8;
pub const MAX_LENGTH: usize = 256;

fn derive(password: &str, salt: &[u8], iterations: u32) -> [u8; HASH_SIZE] {
    let mut hash = [0u8; HASH_SIZE];
    let iterations = NonZeroU32::new(iterations).expect("the iteration count is not zero");
    pbkdf2::derive(
        PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut hash,
    );
    hash
}

/// The stored form of a new password.
pub fn hash(password: &str) -> String {
    let mut salt = [0u8; SALT_SIZE];
    rand::fill(&mut salt).expect("the system random number generator failed");
    let hash = derive(password, &salt, ITERATIONS);
    format!("{SCHEME}${ITERATIONS}${}${}", base64(&salt), base64(&hash))
}

/// Whether the password matches the stored hash. An account without a
/// password matches nothing, after the same work a real check does, so how
/// long the answer takes does not tell it from a wrong password.
pub fn verify(password: &str, stored: Option<&str>) -> bool {
    let Some(stored) = stored else {
        let _ = derive(password, &[0u8; SALT_SIZE], ITERATIONS);
        return false;
    };
    let parts: Vec<&str> = stored.split('$').collect();
    let [scheme, iterations, salt, expected] = parts.as_slice() else {
        return false;
    };
    let Some(iterations) = iterations
        .parse::<u32>()
        .ok()
        .filter(|n| (MINIMUM_ACCEPTED_ITERATIONS..=MAXIMUM_ACCEPTED_ITERATIONS).contains(n))
    else {
        return false;
    };
    let (Some(salt), Some(expected)) = (decoded(salt, SALT_SIZE), decoded(expected, HASH_SIZE))
    else {
        return false;
    };
    *scheme == SCHEME
        && pbkdf2::verify(
            PBKDF2_HMAC_SHA256,
            NonZeroU32::new(iterations).expect("the count is at least 100000"),
            &salt,
            password.as_bytes(),
            &expected,
        )
        .is_ok()
}

fn decoded(text: &str, length: usize) -> Option<Vec<u8>> {
    from_base64(text).filter(|bytes| bytes.len() == length)
}

/// Whether a new password is shorter than the policy allows. Length is
/// counted in UTF-16 code units, as the first server did.
pub fn is_too_short(password: Option<&str>) -> bool {
    password.is_none_or(|p| p.encode_utf16().count() < MIN_LENGTH)
}

pub fn is_too_long(password: Option<&str>) -> bool {
    password.is_some_and(|p| p.encode_utf16().count() > MAX_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Written by Python's hashlib.pbkdf2_hmac('sha256', password, salt, iterations, 32).
    const FROM_PYTHON: [(&str, &str); 3] = [
        (
            "Passw0rd!2026",
            "pbkdf2-sha256$210000$AAECAwQFBgcICQoLDA0ODw==$F7XbbcEiPZ9xiSyI75OfVvMnEQBrxxhoPRwTxTL13LM=",
        ),
        (
            "zażółć gęślą jaźń",
            "pbkdf2-sha256$100000$BwcHBwcHBwcHBwcHBwcHBw==$viF1WPZGU5GDXyNVfSjE++2NKjoI4YRnCNxCGQpWnoY=",
        ),
        (
            "",
            "pbkdf2-sha256$210000$AAAAAAAAAAAAAAAAAAAAAA==$PTtFNTvzwocxY8dnIaViLKZoW77k1SXKIetMHXRZe9I=",
        ),
    ];

    #[test]
    fn hashes_written_elsewhere_verify_here() {
        for (password, stored) in FROM_PYTHON {
            assert!(verify(password, Some(stored)), "{password:?}");
            assert!(!verify("another password", Some(stored)), "{password:?}");
        }
    }

    #[test]
    fn a_new_hash_has_the_stored_shape_and_verifies() {
        let stored = hash("Passw0rd!2026");

        let parts: Vec<&str> = stored.split('$').collect();
        assert_eq!(
            (parts[0], parts[1], parts.len()),
            ("pbkdf2-sha256", "210000", 4)
        );
        assert_eq!(from_base64(parts[2]).unwrap().len(), 16);
        assert_eq!(from_base64(parts[3]).unwrap().len(), 32);
        assert!(verify("Passw0rd!2026", Some(&stored)));
        assert!(!verify("Passw0rd!2027", Some(&stored)));
        assert_ne!(stored, hash("Passw0rd!2026"));
    }

    #[test]
    fn a_hash_the_policy_does_not_accept_verifies_nothing() {
        let (password, stored) = FROM_PYTHON[0];
        for broken in [
            stored.replace("210000", "99999"),
            stored.replace("210000", "1000001"),
            stored.replace("pbkdf2-sha256", "pbkdf2-sha512"),
            stored.replace("$F7Xb", "$F7X"),
            stored.replace("AAECAwQFBgcICQoLDA0ODw==", "AAEC"),
            format!("{stored}$extra"),
            "pbkdf2-sha256$210000$AAECAwQFBgcICQoLDA0ODw==".to_string(),
            String::new(),
        ] {
            assert!(!verify(password, Some(&broken)), "{broken}");
        }
    }

    #[test]
    fn an_account_without_a_password_matches_nothing() {
        assert!(!verify("anything", None));
        assert!(!verify("", None));
    }

    #[test]
    fn length_is_counted_in_utf16_units() {
        assert!(is_too_short(None));
        assert!(is_too_short(Some("short12")));
        assert!(!is_too_short(Some("12345678")));
        // Four characters outside the basic plane are eight units.
        assert!(!is_too_short(Some("𝔘𝔘𝔘𝔘")));
        assert!(!is_too_long(Some(&"p".repeat(256))));
        assert!(is_too_long(Some(&"p".repeat(257))));
        assert!(is_too_long(Some(&"𝔘".repeat(129))));
    }
}
