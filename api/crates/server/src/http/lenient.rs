//! The bodies that need the first server's leniency, read as it read them: a
//! member by its name in any letter case, the last of a name given twice, null
//! where nothing is needed, white space in Base64 ignored, an integer from a
//! number or from a string of digits, and a value of the wrong type making the
//! whole body unreadable (a 400 with no body) before anything is checked.
//!
//! Take the body as `Json<Members>` and read each member with
//! [`Members::get`], [`octets`] or [`number`].

use std::fmt;

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

use super::ApiError;
use crate::encoding::{base64, from_base64};

/// A body that cannot be read: a 400 with no body.
#[derive(Debug)]
pub struct Malformed;

impl From<Malformed> for ApiError {
    fn from(_: Malformed) -> ApiError {
        ApiError::bad_request()
    }
}

/// The members of the JSON object a request is, each still as written.
pub struct Members(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Members, D::Error> {
        struct Reader;
        impl<'de> Visitor<'de> for Reader {
            type Value = Members;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some(name) = map.next_key::<String>()? {
                    members.push((name, map.next_value::<Box<RawValue>>()?));
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(Reader)
    }
}

impl Members {
    /// What the member `name` holds: `None` when it is missing or null. The
    /// name is read without regard to letter case, as the first server read
    /// it. A member that appears more than once counts as its last
    /// occurrence, and every occurrence has to be of the right type.
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, Malformed> {
        let mut found = None;
        for (member, raw) in &self.0 {
            if member.eq_ignore_ascii_case(name) {
                found = serde_json::from_str::<Option<T>>(raw.get()).map_err(|_| Malformed)?;
            }
        }
        Ok(found)
    }

    /// Whether the last occurrence of `name` is `null`.
    pub fn is_null(&self, name: &str) -> bool {
        self.0
            .iter()
            .rfind(|(member, _)| member.eq_ignore_ascii_case(name))
            .is_some_and(|(_, raw)| raw.get() == "null")
    }
}

/// Bytes sent as Base64: ' ', '\t', '\r' and '\n' are left out wherever they
/// stand, and the rest must be the one spelling of some bytes.
pub struct Octets(pub Vec<u8>);

impl<'de> Deserialize<'de> for Octets {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Octets, D::Error> {
        let text = String::deserialize(deserializer)?;
        let squeezed: String = text
            .chars()
            .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
            .collect();
        from_base64(&squeezed)
            .map(Octets)
            .ok_or_else(|| de::Error::custom("not Base64"))
    }
}

impl Serialize for Octets {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64(&self.0))
    }
}

/// An integer sent as a JSON integer, or as a string of digits with an
/// optional sign and no spaces. A number with a fraction or an exponent is
/// not one, even when it is a whole number.
pub struct Whole(pub i32);

impl<'de> Deserialize<'de> for Whole {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Whole, D::Error> {
        struct Reader;
        impl Visitor<'_> for Reader {
            type Value = Whole;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an integer")
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Whole, E> {
                i32::try_from(value)
                    .map(Whole)
                    .map_err(|_| E::custom("out of range"))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Whole, E> {
                i32::try_from(value)
                    .map(Whole)
                    .map_err(|_| E::custom("out of range"))
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Whole, E> {
                let (negative, digits) = match text.as_bytes().first() {
                    Some(b'-') => (true, &text[1..]),
                    Some(b'+') => (false, &text[1..]),
                    _ => (false, text),
                };
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(E::custom("not digits"));
                }
                let mut value: i64 = 0;
                for digit in digits.bytes() {
                    value = value * 10 + i64::from(digit - b'0');
                    if value > i64::from(i32::MAX) + 1 {
                        return Err(E::custom("out of range"));
                    }
                }
                self.visit_i64(if negative { -value } else { value })
            }
        }
        deserializer.deserialize_any(Reader)
    }
}

/// The members of a request that are bytes, text or numbers, read by name.
pub fn octets(body: &Members, name: &str) -> Result<Option<Vec<u8>>, Malformed> {
    Ok(body.get::<Octets>(name)?.map(|octets| octets.0))
}

pub fn number(body: &Members, name: &str) -> Result<Option<i32>, Malformed> {
    Ok(body.get::<Whole>(name)?.map(|whole| whole.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(text: &str) -> Members {
        serde_json::from_str(text).expect("an object")
    }

    #[test]
    fn bytes_ignore_spaces_tabs_and_line_breaks_and_nothing_else() {
        let key = base64(&[7u8; 32]);
        let split = format!("{} \t\r\n{}\n", &key[..10], &key[10..]);
        let body = members(&serde_json::json!({ "publicKey": split }).to_string());
        assert_eq!(octets(&body, "publicKey").unwrap().unwrap(), vec![7u8; 32]);

        for bad in [
            format!("{}\u{b}{}", &key[..10], &key[10..]),
            format!("{}\u{a0}{}", &key[..10], &key[10..]),
            key.trim_end_matches('=').to_string(),
            format!("{key}="),
            "AAAA=AAA".to_string(),
            "-".repeat(43) + "=",
            // The last character carries bits that are not zero.
            format!("{}B=", &base64(&[0u8; 32])[..42]),
        ] {
            let body = members(&serde_json::json!({ "publicKey": bad }).to_string());
            assert!(octets(&body, "publicKey").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn bytes_that_are_not_a_string_are_unreadable_and_null_or_empty_are_something() {
        for bad in ["5", "true", "[1,2]", "{}"] {
            let body = members(&format!("{{\"k\":{bad}}}"));
            assert!(octets(&body, "k").is_err(), "{bad}");
        }
        let body = members("{\"a\":null,\"b\":\"\",\"c\":\"   \"}");
        assert_eq!(octets(&body, "a").unwrap(), None);
        assert_eq!(octets(&body, "b").unwrap(), Some(vec![]));
        assert_eq!(octets(&body, "c").unwrap(), Some(vec![]));
        assert_eq!(octets(&body, "missing").unwrap(), None);
    }

    #[test]
    fn a_number_is_an_integer_or_a_string_of_digits() {
        for (text, wanted) in [
            ("600000", Some(600_000)),
            ("\"600000\"", Some(600_000)),
            ("\"+600000\"", Some(600_000)),
            ("\"0600000\"", Some(600_000)),
            ("\"-5\"", Some(-5)),
            ("-1", Some(-1)),
            ("\"2147483647\"", Some(i32::MAX)),
            ("\"-2147483648\"", Some(i32::MIN)),
            ("600000.0", None),
            ("6e5", None),
            ("600000.5", None),
            ("\" 600000\"", None),
            ("\"600000 \"", None),
            ("\"6e5\"", None),
            ("\"\"", None),
            ("\"+\"", None),
            ("\"2147483648\"", None),
            ("2147483648", None),
            ("true", None),
            ("{}", None),
            ("[600000]", None),
            ("\"\u{660}\"", None),
        ] {
            let body = members(&format!("{{\"n\":{text}}}"));
            match (number(&body, "n"), wanted) {
                (Ok(Some(found)), Some(wanted)) => assert_eq!(found, wanted, "{text}"),
                (Err(_), None) => {}
                (other, _) => panic!("{text}: {:?}", other.map_err(|_| "unreadable")),
            }
        }
        assert_eq!(number(&members("{\"n\":null}"), "n").unwrap(), None);
    }

    #[test]
    fn a_member_named_twice_counts_as_its_last_and_each_has_to_be_readable() {
        let body = members("{\"algorithm\":\"rsa\",\"algorithm\":\"x25519\"}");
        assert_eq!(
            body.get::<String>("algorithm").unwrap().as_deref(),
            Some("x25519")
        );
        let body = members("{\"algorithm\":5,\"algorithm\":\"x25519\"}");
        assert!(body.get::<String>("algorithm").is_err());
        let body = members("{\"algorithm\":\"x25519\",\"algorithm\":null}");
        assert_eq!(body.get::<String>("algorithm").unwrap(), None);
    }

    #[test]
    fn names_are_read_without_regard_to_letter_case_and_the_last_one_wins() {
        let body = members("{\"PublicKey\":\"AAAA\",\"publickey\":\"AAAB\"}");
        assert_eq!(octets(&body, "publicKey").unwrap(), Some(vec![0, 0, 1]));
    }

    #[test]
    fn a_body_that_is_not_an_object_is_unreadable() {
        for text in ["null", "[]", "5", "\"x\"", "true"] {
            assert!(serde_json::from_str::<Members>(text).is_err(), "{text}");
        }
    }
}
