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

/// The wait before a wrong password is answered, between 80 and 179
/// milliseconds, so that how long an answer takes only blurs which refusal it
/// was.
pub async fn wrong_password_wait() {
    let jitter = u64::from(super::tokens::random_below(100));
    tokio::time::sleep(std::time::Duration::from_millis(80 + jitter)).await;
}

/// How many derivations this process has made, which a test reads to see that
/// a refusal did the work of any other.
static DERIVATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Observe password-derivation work in integration tests.
#[doc(hidden)]
pub fn derivations() -> u64 {
    DERIVATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

fn derive(password: &str, salt: &[u8], iterations: u32) -> [u8; HASH_SIZE] {
    DERIVATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

/// A fresh salt from the system random number generator.
fn random_salt() -> [u8; SALT_SIZE] {
    rand::generate::<[u8; SALT_SIZE]>(&rand::SystemRandom::new())
        .expect("the system random number generator failed")
        .expose()
}

/// The stored form of a new password.
pub fn hash(password: &str) -> String {
    let salt = random_salt();
    let hash = derive(password, &salt, ITERATIONS);
    format!("{SCHEME}${ITERATIONS}${}${}", base64(&salt), base64(&hash))
}

/// Whether the password matches the stored hash. An account without a
/// password, and a hash that cannot be used, match nothing, after the same
/// work a real check does (one derivation with the usual parameters), so how
/// long the answer takes does not tell them from a wrong password.
pub fn verify(password: &str, stored: Option<&str>) -> bool {
    let decoy = || {
        let _ = derive(password, &random_salt(), ITERATIONS);
        false
    };
    let Some(stored) = stored else {
        return decoy();
    };
    let parts: Vec<&str> = stored.split('$').collect();
    let [scheme, iterations, salt, expected] = parts.as_slice() else {
        return decoy();
    };
    let Some(iterations) = iterations
        .parse::<u32>()
        .ok()
        .filter(|n| (MINIMUM_ACCEPTED_ITERATIONS..=MAXIMUM_ACCEPTED_ITERATIONS).contains(n))
    else {
        return decoy();
    };
    let (Some(salt), Some(expected)) = (decoded(salt, SALT_SIZE), decoded(expected, HASH_SIZE))
    else {
        return decoy();
    };
    if *scheme != SCHEME {
        return decoy();
    }
    DERIVATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    pbkdf2::verify(
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
    password.is_none_or(|p| stellarr_unicode::utf16::length(p) < MIN_LENGTH)
}

pub fn is_too_long(password: Option<&str>) -> bool {
    password.is_some_and(|p| stellarr_unicode::utf16::length(p) > MAX_LENGTH)
}

#[cfg(test)]
mod tests;
