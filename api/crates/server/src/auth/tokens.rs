//! Random secrets, and the hashes of them that are stored in their place.

use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::digest::{SHA256, digest};
use aws_lc_rs::rand;

use crate::encoding::{base64_url, hex_upper};

/// `bytes` random bytes as Base64 for URLs.
pub fn random(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::fill(&mut buffer).expect("the system random number generator failed");
    base64_url(&buffer)
}

/// A number below `limit`, drawn without a bias towards the low ones.
pub fn random_below(limit: u32) -> u32 {
    debug_assert!(limit > 0);
    let zone = u32::MAX - (u32::MAX % limit);
    loop {
        let mut bytes = [0u8; 4];
        rand::fill(&mut bytes).expect("the system random number generator failed");
        let value = u32::from_be_bytes(bytes);
        if value < zone {
            return value % limit;
        }
    }
}

/// The SHA-256 of the text in capital hexadecimal: how a secret is stored.
pub fn hash(value: &str) -> String {
    hex_upper(digest(&SHA256, value.as_bytes()).as_ref())
}

/// Whether `supplied` is the secret whose hash is stored.
pub fn matches(supplied: &str, stored_hash: &str) -> bool {
    stored_hash.len() == 64 && fixed_time_equals(hash(supplied).as_bytes(), stored_hash.as_bytes())
}

/// Equal in the same time whichever byte differs.
pub fn fixed_time_equals(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && verify_slices_are_equal(left, right).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_is_capital_hex_of_sha256() {
        // SHA-256("abc"), from FIPS 180-2.
        assert_eq!(
            hash("abc"),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
    }

    #[test]
    fn a_secret_matches_its_own_hash_only() {
        let secret = random(32);
        let stored = hash(&secret);

        assert!(matches(&secret, &stored));
        assert!(!matches("another", &stored));
        assert!(!matches(&secret, "short"));
        assert_eq!(secret.len(), 43);
        assert_ne!(secret, random(32));
    }

    #[test]
    fn a_random_number_stays_below_its_limit() {
        for _ in 0..200 {
            assert!(random_below(7) < 7);
        }
        assert_eq!(random_below(1), 0);
    }
}
