//! The numbers and flags of a form, read the way the first server read them.
//!
//! A value is text; whether it is a number is decided by rules that are not
//! the standard library's: white space around it, a sign, commas inside it
//! that mean nothing, `Infinity` and `NaN` for a decimal number. Clients send
//! what a browser's form sends, but a script may send anything, and the two
//! servers have to agree on what counts.

/// The characters a number may have around it.
fn is_number_space(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ')
}

/// A decimal number: spaces, a sign, digits with commas between them, a point,
/// an exponent, and spaces; a name such as `Infinity` or `NaN` in any case. A
/// number too large to hold is infinite.
pub fn decimal(text: &str) -> Option<f64> {
    let trimmed = text.trim_start_matches(is_number_space);
    let (negative, rest) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if let Some((digits, tail)) = number_part(rest) {
        if !ends_well(tail) {
            return None;
        }
        let value: f64 = digits.parse().ok()?;
        return Some(if negative { -value } else { value });
    }
    named(text)
}

/// Whether what follows a number is only spaces, and then only NUL characters.
fn ends_well(tail: &str) -> bool {
    tail.trim_start_matches(is_number_space)
        .chars()
        .all(|c| c == '\0')
}

/// The digits, point and exponent at the start of `text`, written for the
/// standard library to read, and what follows them.
fn number_part(text: &str) -> Option<(String, &str)> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut out = String::new();
    let mut digits = 0;
    // Commas count between digits and before the point, and nowhere else.
    while at < bytes.len() {
        match bytes[at] {
            b'0'..=b'9' => {
                out.push(char::from(bytes[at]));
                digits += 1;
            }
            b',' if digits > 0 => {}
            _ => break,
        }
        at += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        out.push('.');
        at += 1;
        while let Some(digit @ b'0'..=b'9') = bytes.get(at) {
            out.push(char::from(*digit));
            digits += 1;
            at += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let mut probe = at + 1;
        let mut exponent = String::from("e");
        if let Some(sign @ (b'+' | b'-')) = bytes.get(probe) {
            exponent.push(char::from(*sign));
            probe += 1;
        }
        let first_digit = probe;
        while let Some(digit @ b'0'..=b'9') = bytes.get(probe) {
            exponent.push(char::from(*digit));
            probe += 1;
        }
        if probe > first_digit {
            out.push_str(&exponent);
            at = probe;
        }
    }
    Some((out, &text[at..]))
}

/// `Infinity`, `-Infinity` and `NaN`, with a sign where one is given, in any
/// letter case, with white space around.
fn named(text: &str) -> Option<f64> {
    let trimmed = text.trim_matches(char::is_whitespace);
    let (negative, rest) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if rest.eq_ignore_ascii_case("infinity") {
        Some(if negative { f64::NEG_INFINITY } else { f64::INFINITY })
    } else if rest.eq_ignore_ascii_case("nan") {
        Some(f64::NAN)
    } else {
        None
    }
}

/// A whole number of 32 bits: spaces, a sign and digits.
pub fn integer(text: &str) -> Option<i32> {
    let trimmed = text.trim_start_matches(is_number_space);
    let (negative, rest) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 || !ends_well(&rest[end..]) {
        return None;
    }
    // Leading zeros are no part of the size of the number.
    let digits = rest[..end].trim_start_matches('0');
    if digits.len() > 10 {
        return None;
    }
    let magnitude: i64 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
    i32::try_from(if negative { -magnitude } else { magnitude }).ok()
}

/// `true` or `false` in any letter case, with white space and NUL characters
/// around.
pub fn flag(text: &str) -> Option<bool> {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if trimmed.eq_ignore_ascii_case("true") {
        Some(true)
    } else if trimmed.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_are_read_with_commas_and_exponents() {
        for (text, expected) in [
            ("1", 1.0),
            ("42.1254", 42.1254),
            (" \t-1.5e2 \n", -150.0),
            ("1,2", 12.0),
            ("1,000.5", 1000.5),
            ("1,", 1.0),
            ("1,,2", 12.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("+7", 7.0),
            ("1E+3", 1000.0),
            ("2\0\0", 2.0),
            ("2 \0", 2.0),
            ("0x10", f64::NAN),
        ] {
            let read = decimal(text);
            if expected.is_nan() {
                assert!(read.is_none(), "{text:?}");
            } else {
                assert_eq!(read, Some(expected), "{text:?}");
            }
        }
    }

    #[test]
    fn what_is_not_a_decimal_is_refused() {
        for text in [
            "", " ", ".", "-", "+", "e5", "1e", "1e+", "abc", "1 2", "1..2", "1.2.3", ",1", "1_000", "--1", "+-1", "1,5e", "1 e5",
            "0x10", "１２", "\u{a0}1",
        ] {
            assert_eq!(decimal(text), None, "{text:?}");
        }
    }

    #[test]
    fn names_are_infinite_or_not_a_number() {
        assert_eq!(decimal("Infinity"), Some(f64::INFINITY));
        assert_eq!(decimal(" -infinity "), Some(f64::NEG_INFINITY));
        assert_eq!(decimal("+INFINITY"), Some(f64::INFINITY));
        assert!(decimal("NaN").is_some_and(f64::is_nan));
        assert!(decimal("-nan").is_some_and(f64::is_nan));
        assert_eq!(decimal("inf"), None);
        assert_eq!(decimal("1e999"), Some(f64::INFINITY));
    }

    #[test]
    fn integers_have_a_sign_and_fit_in_32_bits() {
        assert_eq!(integer("1920"), Some(1920));
        assert_eq!(integer(" +7 "), Some(7));
        assert_eq!(integer("-2147483648"), Some(i32::MIN));
        assert_eq!(integer("2147483647"), Some(i32::MAX));
        assert_eq!(integer("0000000000000000000012"), Some(12));
        for text in ["", "-", "1.0", "1,000", "2147483648", "-2147483649", "99999999999999999999", "1e3", "x"] {
            assert_eq!(integer(text), None, "{text:?}");
        }
    }

    #[test]
    fn flags_are_true_or_false() {
        assert_eq!(flag("true"), Some(true));
        assert_eq!(flag(" FALSE\0"), Some(false));
        assert_eq!(flag("True,True"), None);
        assert_eq!(flag("yes"), None);
        assert_eq!(flag("1"), None);
        assert_eq!(flag(""), None);
    }
}
