//! The bodies of the routes, read as the first server read them: a member by
//! its name in any letter case, the last of a name given twice, null where
//! nothing is needed, and a value of the wrong type making the whole body
//! unreadable (a 400 with no body) before anything is checked.
//!
//! Bytes travel as standard Base64 with padding; spaces, tabs and line breaks
//! anywhere in the text are ignored, as the first server's reader ignored
//! them.

use std::fmt;

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::encoding::from_base64;
use crate::http::ApiError;

/// A body that cannot be read: a 400 with no body.
#[derive(Debug)]
pub struct Malformed;

impl From<Malformed> for ApiError {
    fn from(_: Malformed) -> ApiError {
        ApiError::bad_request()
    }
}

/// The members of the JSON object a request is, each still as written.
pub struct Body(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Body {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Body, D::Error> {
        struct Reader;
        impl<'de> Visitor<'de> for Reader {
            type Value = Body;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Body, A::Error> {
                let mut members = Vec::new();
                while let Some(name) = map.next_key::<String>()? {
                    members.push((name, map.next_value::<Box<RawValue>>()?));
                }
                Ok(Body(members))
            }
        }
        deserializer.deserialize_map(Reader)
    }
}

impl Body {
    /// What the member `name` holds: `None` when it is missing or null. A
    /// member that appears more than once counts as its last occurrence, and
    /// every occurrence has to be of the right type.
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

/// Bytes sent as Base64.
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
