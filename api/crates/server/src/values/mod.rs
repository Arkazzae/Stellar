//! How a value is written into the database and into JSON, once.
//!
//! Every row has an [`Id`] and most have a [`Timestamp`]; the rest are plain
//! columns with the following storage formats:
//!
//! - ids are text in capitals, and lower case in JSON;
//! - moments are UTC text with a trimmed fraction, and the same with `T` and
//!   `Z` in JSON;
//! - booleans are the integers 0 and 1, which `rusqlite` already reads and
//!   writes for `bool`;
//! - an enum is an integer (see [`int_enum!`]), and a few are text;
//! - a missing value is NULL, and `null` in JSON.
//!
//! Nothing outside this module and the database module spells one of these.

mod id;
mod int_enum;
mod timestamp;

pub use id::Id;
pub(crate) use id::json_ids;
pub use timestamp::Timestamp;
pub(crate) use timestamp::{civil_from_days, days_from_civil, is_leap};
