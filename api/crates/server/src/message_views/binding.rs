//! Reading ids the way the first server did, so that the same address means
//! the same thing.

use serde::de::{self, Deserializer};
use serde::Deserialize;

use crate::permissions::membership::parse_guid;
use crate::values::Id;

/// An id in a path, in any spelling the first server's route constraint read:
/// with or without hyphens, in braces, in capitals. Anything else is a path
/// that does not exist.
#[derive(Debug, Clone, Copy)]
pub struct Guid(pub Id);

impl<'de> Deserialize<'de> for Guid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Guid, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_guid(&text)
            .map(Guid)
            .ok_or_else(|| de::Error::custom("not an id"))
    }
}

