//! Time-based one-time codes, as an authenticator app computes them: RFC 6238
//! over HMAC-SHA1 with six digits and a 30-second step, which every app
//! supports and which is what the otpauth address below tells them to use.
//!
//! This module only does the arithmetic and the text formats. Whether a step
//! was already used, and where the secret is kept, is the business of
//! `mfa_methods`.

use aws_lc_rs::hmac::{self, HMAC_SHA1_FOR_LEGACY_USE_ONLY};
use aws_lc_rs::rand;

use super::tokens::fixed_time_equals;
use crate::values::Timestamp;

/// A secret of 160 bits, the size RFC 4226 recommends for HMAC-SHA1.
const SECRET_BYTES: usize = 20;
const DIGITS: u32 = 6;
const STEP_SECONDS: i64 = 30;
/// One step either side of the present is accepted, for a phone whose clock is
/// a little off and for a code typed as the step turns over.
const STEPS_BEFORE: i64 = 1;
const STEPS_AFTER: i64 = 1;

/// A new secret as an authenticator app takes it: unpadded Base32 of 20 random
/// bytes, which is 32 letters and digits.
pub fn new_secret() -> String {
    let mut bytes = [0u8; SECRET_BYTES];
    rand::fill(&mut bytes).expect("the system random number generator failed");
    base32_encode(&bytes)
}

/// The address a QR code carries for an app to add the account:
/// `otpauth://totp/{issuer}:{account}?secret=..&issuer=..&algorithm=SHA1&digits=6&period=30`,
/// every text part escaped as a URL component.
pub fn otpauth_uri(issuer: &str, account_name: &str, secret: &str) -> String {
    let label = escape(&format!("{issuer}:{account_name}"));
    format!(
        "otpauth://totp/{label}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECONDS}",
        escape(secret),
        escape(issuer),
    )
}

/// The step a code typed at `now` was made in, if it is a code of this secret
/// for the present step or the one before or after. The step is what a caller
/// records so that the same code is never taken twice.
///
/// A code is six digits and nothing else, apart from white space round it.
/// Three codes are always worked out and compared, so that the time taken does
/// not say which step was close. When two steps happen to give the same code,
/// the present one is reported, then the one before, then the one after.
pub fn verify(secret: &str, code: &str, now: Timestamp) -> Option<i64> {
    let code = code.trim();
    if code.len() != DIGITS as usize || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let key = key_of(secret)?;
    let present = now.unix_seconds().div_euclid(STEP_SECONDS);
    let mut found = None;
    for step in candidates(present) {
        let matches = fixed_time_equals(code.as_bytes(), hotp(&key, step, DIGITS).as_bytes());
        if matches && found.is_none() {
            found = Some(step);
        }
    }
    found
}

/// The code an app shows at `now`, or none when the secret is not Base32.
pub fn code_at(secret: &str, now: Timestamp) -> Option<String> {
    let key = key_of(secret)?;
    Some(hotp(
        &key,
        now.unix_seconds().div_euclid(STEP_SECONDS),
        DIGITS,
    ))
}

/// The key a secret spells, when it spells one.
fn key_of(secret: &str) -> Option<Vec<u8>> {
    base32_decode(secret).filter(|key| !key.is_empty())
}

/// The steps looked at, in the order they are preferred.
fn candidates(present: i64) -> Vec<i64> {
    let mut steps = vec![present];
    for offset in 1..=STEPS_BEFORE.max(STEPS_AFTER) {
        if offset <= STEPS_BEFORE {
            steps.push(present - offset);
        }
        if offset <= STEPS_AFTER {
            steps.push(present + offset);
        }
    }
    steps
}

/// RFC 4226: HMAC-SHA1 of the counter as eight big-endian bytes, cut down by
/// dynamic truncation to a number of `digits` digits.
fn hotp(key: &[u8], counter: i64, digits: u32) -> String {
    let key = hmac::Key::new(HMAC_SHA1_FOR_LEGACY_USE_ONLY, key);
    let tag = hmac::sign(&key, &counter.to_be_bytes());
    let tag = tag.as_ref();
    let offset = usize::from(tag[tag.len() - 1] & 0x0f);
    let number = u32::from_be_bytes([
        tag[offset] & 0x7f,
        tag[offset + 1],
        tag[offset + 2],
        tag[offset + 3],
    ]);
    format!(
        "{:0width$}",
        number % 10u32.pow(digits),
        width = digits as usize
    )
}

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 Base32 without padding.
pub(crate) fn base32_encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in bytes {
        buffer = buffer << 8 | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(char::from(BASE32[(buffer >> bits & 0x1f) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        text.push(char::from(BASE32[(buffer << (5 - bits) & 0x1f) as usize]));
    }
    text
}

/// RFC 4648 Base32, in either letter case and with or without the `=` padding;
/// bits left over at the end are dropped. Anything else, and nothing at all,
/// is refused.
pub(crate) fn base32_decode(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() {
        return None;
    }
    let text = text.trim_end_matches('=');
    let mut bytes = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer = 0u32;
    let mut bits = 0;
    for letter in text.bytes() {
        let value = match letter {
            b'A'..=b'Z' => letter - b'A',
            b'a'..=b'z' => letter - b'a',
            b'2'..=b'7' => letter - b'2' + 26,
            _ => return None,
        };
        buffer = buffer << 5 | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits & 0xff) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(bytes)
}

/// Percent-encodes everything but the unreserved characters of RFC 3986, as a
/// URL component is written.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, Appendix B: the SHA-1 seed and what it gives, in eight digits.
    const RFC_6238_SEED: &[u8] = b"12345678901234567890";
    const RFC_6238: [(i64, &str); 6] = [
        (59, "94287082"),
        (1111111109, "07081804"),
        (1111111111, "14050471"),
        (1234567890, "89005924"),
        (2000000000, "69279037"),
        (20000000000, "65353130"),
    ];

    /// RFC 4226, Appendix D: the same seed, counters 0 to 9, six digits.
    const RFC_4226: [&str; 10] = [
        "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871",
        "520489",
    ];

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds).unwrap()
    }

    #[test]
    fn the_codes_of_rfc_6238_come_out() {
        for (seconds, code) in RFC_6238 {
            assert_eq!(
                hotp(RFC_6238_SEED, seconds / STEP_SECONDS, 8),
                code,
                "at {seconds}"
            );
        }
    }

    #[test]
    fn the_codes_of_rfc_4226_come_out() {
        for (counter, code) in RFC_4226.iter().enumerate() {
            assert_eq!(&hotp(RFC_6238_SEED, counter as i64, DIGITS), code);
        }
    }

    #[test]
    fn base32_is_the_one_of_rfc_4648() {
        // RFC 4648, section 10, without the padding.
        for (plain, coded) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32_encode(plain.as_bytes()), coded);
        }
        for (plain, coded) in [
            ("f", "MY======"),
            ("fo", "MZXQ===="),
            ("foobar", "MZXW6YTBOI======"),
            ("foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(base32_decode(coded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn a_text_that_is_not_base32_is_no_secret() {
        for bad in [
            "",
            "====",
            "MZXW6YTB!",
            "MZXW 6YTB",
            "MZXW1YTB",
            "MZXW0YTB",
            "MZXW8YTB",
        ] {
            assert_eq!(
                base32_decode(bad).filter(|key| !key.is_empty()),
                None,
                "{bad:?}"
            );
            assert_eq!(verify(bad, "123456", at(0)), None);
            assert_eq!(code_at(bad, at(0)), None);
        }
    }

    #[test]
    fn a_new_secret_is_thirty_two_base32_letters_and_never_the_same_twice() {
        let secret = new_secret();

        assert_eq!(secret.len(), 32);
        assert!(secret.bytes().all(|b| BASE32.contains(&b)));
        assert_eq!(base32_decode(&secret).unwrap().len(), SECRET_BYTES);
        assert_ne!(secret, new_secret());
    }

    #[test]
    fn the_present_step_and_its_neighbours_are_accepted_and_no_others() {
        let secret = base32_encode(RFC_6238_SEED);
        let now = at(1_111_111_111);
        let step = 1_111_111_111 / STEP_SECONDS;
        let code = |offset: i64| hotp(RFC_6238_SEED, step + offset, DIGITS);

        assert_eq!(verify(&secret, &code(0), now), Some(step));
        assert_eq!(verify(&secret, &code(-1), now), Some(step - 1));
        assert_eq!(verify(&secret, &code(1), now), Some(step + 1));
        // Two steps away is refused, which is the check on the window.
        assert_eq!(verify(&secret, &code(-2), now), None);
        assert_eq!(verify(&secret, &code(2), now), None);
    }

    #[test]
    fn the_window_opens_and_closes_on_the_step_boundaries() {
        let secret = base32_encode(RFC_6238_SEED);
        let code = hotp(RFC_6238_SEED, 100, DIGITS);

        // Step 100 runs from second 3000 to 3029. It is the future of step 99
        // and the past of step 101, and of no step beyond them.
        assert_eq!(verify(&secret, &code, at(2969)), None);
        assert_eq!(verify(&secret, &code, at(2970)), Some(100));
        assert_eq!(verify(&secret, &code, at(3029)), Some(100));
        assert_eq!(verify(&secret, &code, at(3059)), Some(100));
        assert_eq!(verify(&secret, &code, at(3060)), None);
    }

    #[test]
    fn a_code_is_six_digits_with_white_space_round_it_and_nothing_else() {
        let secret = base32_encode(RFC_6238_SEED);
        let now = at(59);
        let good = hotp(RFC_6238_SEED, 1, DIGITS);
        assert_eq!(good, "287082");

        assert_eq!(verify(&secret, &format!(" {good}\t\n"), now), Some(1));
        assert_eq!(
            verify(&secret, &format!("\u{a0}{good}\u{2003}"), now),
            Some(1)
        );
        for bad in [
            "",
            "28708",
            "2870820",
            "287 082",
            "28708a",
            "+87082",
            "-87082",
            "٢٨٧٠٨٢",
            "２８７０８２",
        ] {
            assert_eq!(verify(&secret, bad, now), None, "{bad:?}");
        }
        // The same six digits with a letter case or a prefix of a longer code.
        assert_eq!(verify(&secret, &format!("{good}0"), now), None);
    }

    #[test]
    fn a_code_that_is_none_of_the_three_is_refused_and_a_step_is_only_reported_for_its_own_code() {
        let secret = base32_encode(RFC_6238_SEED);
        let key = RFC_6238_SEED;
        let now = at(1_800_000_000);
        let step = 1_800_000_000 / STEP_SECONDS;
        let given: Vec<String> = (-1..=1).map(|o| hotp(key, step + o, DIGITS)).collect();
        let wrong = (0..1_000_000)
            .map(|n| format!("{n:06}"))
            .find(|code| !given.contains(code))
            .unwrap();

        assert_eq!(verify(&secret, &wrong, now), None);
        for (offset, code) in (-1..=1).zip(&given) {
            assert_eq!(verify(&secret, code, now), Some(step + offset));
        }
    }

    /// Cases worked out by `tests/totp-independent.py`, which uses Python's
    /// `hmac` and `struct` and none of this code.
    #[test]
    fn an_independent_implementation_agrees() {
        let cases = include_str!("../../tests/fixtures/totp-cases.tsv");
        let mut accepted = 0;
        for line in cases.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [secret, now, code, expected] = fields[..] else {
                panic!("a case has four fields: {line}");
            };
            let code = code.replace("<TAB>", "\t");
            let expected = (expected != "-").then(|| expected.parse::<i64>().unwrap());

            assert_eq!(
                verify(secret, &code, at(now.parse().unwrap())),
                expected,
                "{line}"
            );
            accepted += usize::from(expected.is_some());
        }
        assert_eq!(cases.lines().count(), 400);
        assert!(accepted > 150, "the file holds accepted and refused cases");
    }

    #[test]
    fn the_address_for_an_app_escapes_every_part() {
        assert_eq!(
            otpauth_uri("Stellarr", "maya", "JBSWY3DPEHPK3PXP"),
            "otpauth://totp/Stellarr%3Amaya?secret=JBSWY3DPEHPK3PXP&issuer=Stellarr&algorithm=SHA1&digits=6&period=30"
        );
        assert_eq!(
            otpauth_uri("St ellarr", "ma&ya@é/ü", "A=B"),
            "otpauth://totp/St%20ellarr%3Ama%26ya%40%C3%A9%2F%C3%BC?secret=A%3DB&issuer=St%20ellarr&algorithm=SHA1&digits=6&period=30"
        );
        assert_eq!(escape("-._~ azAZ09"), "-._~%20azAZ09");
    }
}
