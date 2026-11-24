//! Numbers as clients send them in the bodies of these routes.
//!
//! The first server read a number from a JSON string as well as from a number:
//! `"1.5"`, `"+1"`, `".5"`, `"1."`, `"1e1"` and `"05"` were numbers, and so
//! were the words `"NaN"`, `"Infinity"` and `"-Infinity"` for a real number. A
//! whole number could not be written with a fraction or an exponent, in either
//! form, and had to fit 32 bits. Space, a hex number, a comma and digits of
//! other scripts were refused. `null` was no number where one was required.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};

struct Real {
    /// Whether `null` is nothing, rather than an error.
    optional: bool,
}

impl Visitor<'_> for Real {
    type Value = Option<f64>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a number")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Option<f64>, E> {
        Ok(Some(value as f64))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Option<f64>, E> {
        Ok(Some(value as f64))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Option<f64>, E> {
        Ok(Some(value))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Option<f64>, E> {
        real(text).map(Some).ok_or_else(|| E::custom("not a number"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Option<f64>, E> {
        if self.optional { Ok(None) } else { Err(E::custom("null is no number")) }
    }

    fn visit_none<E: de::Error>(self) -> Result<Option<f64>, E> {
        self.visit_unit()
    }
}

/// A real number from a string: an optional sign, digits with an optional
/// point and fraction (either side of the point may be empty, not both), an
/// optional exponent; or one of the three words.
fn real(text: &str) -> Option<f64> {
    match text {
        "NaN" => return Some(f64::NAN),
        "Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let bytes = text.as_bytes();
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let digits = |at: &mut usize| {
        let start = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        *at - start
    };
    let whole = digits(&mut at);
    let mut fraction = 0;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        fraction = digits(&mut at);
    }
    if whole == 0 && fraction == 0 {
        return None;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if digits(&mut at) == 0 {
            return None;
        }
    }
    if at != bytes.len() {
        return None;
    }
    // Rust reads the same grammar, and says infinity for what is too large, as
    // the first server did.
    text.parse::<f64>().ok()
}

/// A whole number from a string: an optional sign and digits.
fn whole(text: &str) -> Option<i32> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.strip_prefix('+').unwrap_or(text).parse().ok()
}

struct Whole;

impl Visitor<'_> for Whole {
    type Value = Option<i32>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a whole number")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Option<i32>, E> {
        i32::try_from(value).map(Some).map_err(|_| E::custom("out of range"))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Option<i32>, E> {
        i32::try_from(value).map(Some).map_err(|_| E::custom("out of range"))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Option<i32>, E> {
        whole(text).map(Some).ok_or_else(|| E::custom("not a whole number"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Option<i32>, E> {
        Ok(None)
    }

    fn visit_none<E: de::Error>(self) -> Result<Option<i32>, E> {
        Ok(None)
    }
}

/// A real number that may be left out or `null`.
pub fn optional_real<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    deserializer.deserialize_any(Real { optional: true })
}

/// A real number that may be left out, but not `null`.
pub fn required_real<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    deserializer
        .deserialize_any(Real { optional: false })
        .map(|value| value.unwrap_or(0.0))
}

/// A whole number that may be left out or `null`.
pub fn optional_whole<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i32>, D::Error> {
    deserializer.deserialize_any(Whole)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct Body {
        #[serde(default, deserialize_with = "optional_real")]
        real: Option<f64>,
        #[serde(default, deserialize_with = "required_real")]
        needed: f64,
        #[serde(default, deserialize_with = "optional_whole")]
        whole: Option<i32>,
    }

    fn real_of(json: &str) -> Option<Option<f64>> {
        serde_json::from_str::<serde_json::Value>(&format!("{{\"real\":{json}}}"))
            .ok()
            .and_then(|value| Body::deserialize(value).ok())
            .map(|body| body.real)
    }

    fn whole_of(json: &str) -> Option<Option<i32>> {
        serde_json::from_str::<serde_json::Value>(&format!("{{\"whole\":{json}}}"))
            .ok()
            .and_then(|value| Body::deserialize(value).ok())
            .map(|body| body.whole)
    }

    #[test]
    fn a_real_number_is_a_number_or_a_string_of_one() {
        for (json, value) in [
            ("1", 1.0), ("1.5", 1.5), ("1e1", 10.0), ("\"1\"", 1.0), ("\"1.5\"", 1.5), ("\"+1\"", 1.0), ("\"1e1\"", 10.0),
            ("\"1E1\"", 10.0), ("\".5\"", 0.5), ("\"1.\"", 1.0), ("\"01\"", 1.0), ("\"\\u0031\"", 1.0), ("\"-1.5e-1\"", -0.15),
            ("\"5.0000000001\"", 5.0000000001),
        ] {
            assert_eq!(real_of(json), Some(Some(value)), "{json}");
        }
        assert_eq!(real_of("null"), Some(None));
        assert!(real_of("\"NaN\"").unwrap().unwrap().is_nan());
        assert_eq!(real_of("\"Infinity\""), Some(Some(f64::INFINITY)));
        assert_eq!(real_of("\"-Infinity\""), Some(Some(f64::NEG_INFINITY)));
        assert!(real_of("\"-0\"").unwrap().unwrap().is_sign_negative());
        assert_eq!(real_of("\"1e999\""), Some(Some(f64::INFINITY)));
        for bad in ["\" 1\"", "\"1 \"", "\"0x1\"", "\"1,5\"", "\"\"", "\"1_0\"", "\"\\u0661\"", "true", "\"true\"", "[]", "{}", "\"infinity\"", "\"nan\"", "\".\"", "\"e1\"", "\"1e\"", "\"--1\""] {
            assert_eq!(real_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_real_number_that_is_needed_may_not_be_null() {
        let read = |json: &str| {
            serde_json::from_str::<serde_json::Value>(json)
                .ok()
                .and_then(|value| Body::deserialize(value).ok())
                .map(|body| body.needed)
        };
        assert_eq!(read("{}"), Some(0.0));
        assert_eq!(read("{\"needed\":\"2\"}"), Some(2.0));
        assert_eq!(read("{\"needed\":null}"), None);
    }

    #[test]
    fn a_whole_number_has_no_fraction_or_exponent_and_fits_32_bits() {
        for (json, value) in [("5", 5), ("\"5\"", 5), ("\"+5\"", 5), ("\"05\"", 5), ("\"-5\"", -5), ("2147483647", i32::MAX), ("-2147483648", i32::MIN)] {
            assert_eq!(whole_of(json), Some(Some(value)), "{json}");
        }
        assert_eq!(whole_of("null"), Some(None));
        for bad in ["5.0", "50.5", "1e1", "\"5.0\"", "\"50.5\"", "\"1e1\"", "\" 5\"", "\"\"", "\"+\"", "2147483648", "-2147483649", "\"3000000000\"", "true", "\"\\u0665\""] {
            assert_eq!(whole_of(bad), None, "{bad}");
        }
    }
}
