//! The numbers and flags of a form, which are text.
//!
//! A browser writes a number the way JavaScript does: digits, and a point and
//! more digits for a fraction. Nothing else is a number here: no sign, no
//! spaces, no thousands separators, no exponent, no `Infinity` or `NaN`.

/// A decimal number: digits, and a point with digits after it.
pub fn decimal(text: &str) -> Option<f64> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|fraction| !digits(fraction)) {
        return None;
    }
    // A number too large to hold is infinite, and no length is.
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// A whole number of 32 bits: digits.
pub fn integer(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// `true` or `false`.
pub fn flag(text: &str) -> Option<bool> {
    match text {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_are_digits_with_a_point() {
        for (text, expected) in [("1", 1.0), ("42.1254", 42.1254), ("0.5", 0.5), ("007", 7.0), ("12.50", 12.5)] {
            assert_eq!(decimal(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn what_is_not_plain_digits_is_not_a_decimal() {
        for text in [
            "", " ", ".", "-", "+", "e5", "1e", "1e5", "abc", "1 2", "1..2", "1.2.3", ",1", "1,5", "1,000", "1_000", "--1", "-1", "+7",
            " 1", "1 ", "1\0", ".5", "5.", "0x10", "１２", "\u{a0}1", "Infinity", "-Infinity", "NaN", "nan",
        ] {
            assert_eq!(decimal(text), None, "{text:?}");
        }
        assert_eq!(decimal(&"9".repeat(400)), None, "no length is infinite");
    }

    #[test]
    fn integers_are_digits_that_fit_in_32_bits() {
        assert_eq!(integer("1920"), Some(1920));
        assert_eq!(integer("2147483647"), Some(i32::MAX));
        assert_eq!(integer("0000000000000000000012"), Some(12));
        for text in ["", "-", "-1", "+7", " 7", "7 ", "1.0", "1,000", "2147483648", "99999999999999999999", "1e3", "x"] {
            assert_eq!(integer(text), None, "{text:?}");
        }
    }

    #[test]
    fn flags_are_true_or_false() {
        assert_eq!(flag("true"), Some(true));
        assert_eq!(flag("false"), Some(false));
        for text in [" FALSE\0", "True", "FALSE", "True,True", "yes", "1", "", " true"] {
            assert_eq!(flag(text), None, "{text:?}");
        }
    }
}
