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

/// A number of `digits` digits (at most nine), zeros first, as a person types
/// a code from a mail.
pub fn random_digits(digits: u32) -> String {
    debug_assert!((1..=9).contains(&digits));
    format!(
        "{:0width$}",
        random_below(10u32.pow(digits)),
        width = digits as usize
    )
}

/// The letters and digits a code is written in: no 0, 1, I or O, which are
/// easy to confuse when it is typed from paper.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// `length` characters of [`CODE_ALPHABET`].
pub fn random_text(length: usize) -> String {
    (0..length)
        .map(|_| char::from(CODE_ALPHABET[random_below(CODE_ALPHABET.len() as u32) as usize]))
        .collect()
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
    fn digits_come_with_their_leading_zeros_and_every_digit_turns_up() {
        let mut seen = [false; 10];
        for _ in 0..400 {
            let code = random_digits(6);
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
            for b in code.bytes() {
                seen[usize::from(b - b'0')] = true;
            }
        }
        assert!(seen.iter().all(|seen| *seen));
        assert_eq!(random_digits(1).len(), 1);
        assert_eq!(random_digits(9).len(), 9);
    }

    #[test]
    fn text_codes_use_only_the_unambiguous_alphabet() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let text = random_text(28);
            assert_eq!(text.len(), 28);
            seen.extend(text.bytes());
        }
        assert!(seen.iter().all(|b| CODE_ALPHABET.contains(b)));
        assert_eq!(seen.len(), 32, "every character is drawn in time");
        assert_eq!(random_text(0), "");
    }

    #[test]
    fn a_random_number_stays_below_its_limit() {
        for _ in 0..200 {
            assert!(random_below(7) < 7);
        }
        assert_eq!(random_below(1), 0);
    }
}
