//! The identifier of every row: a random 128-bit value written as a hyphenated
//! UUID.
//!
//! It is written in two spellings. The database holds it in capitals, as the
//! rows the first server wrote do; the JSON the clients see has it in
//! lower case. Either spelling is read back.

use std::fmt;

use aws_lc_rs::rand;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id([u8; 16]);

impl Id {
    /// A new random identifier (version 4).
    pub fn random() -> Id {
        let mut bytes = [0u8; 16];
        // The system source of randomness failing is not something a request
        // can recover from; the process is better off stopped.
        rand::fill(&mut bytes).expect("the system random number generator failed");
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Id(bytes)
    }

    pub const fn from_bytes(bytes: [u8; 16]) -> Id {
        Id(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Reads the 36-character hyphenated form, in either letter case, and
    /// nothing else: no braces, no spaces, no missing hyphens. Every id that
    /// crosses the wire or is typed by an operator is read here.
    pub fn parse(text: &str) -> Option<Id> {
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        let mut out = [0u8; 16];
        let mut index = 0;
        let mut position = 0;
        while position < 36 {
            if matches!(position, 8 | 13 | 18 | 23) {
                if bytes[position] != b'-' {
                    return None;
                }
                position += 1;
                continue;
            }
            let high = hex_value(bytes[position])?;
            let low = hex_value(bytes[position + 1])?;
            out[index] = high << 4 | low;
            index += 1;
            position += 2;
        }
        Some(Id(out))
    }

    /// The spelling the database holds: capitals.
    pub fn to_db_text(&self) -> String {
        self.spell(true)
    }

    fn spell(&self, upper: bool) -> String {
        let digits: &[u8; 16] = if upper {
            b"0123456789ABCDEF"
        } else {
            b"0123456789abcdef"
        };
        let mut text = String::with_capacity(36);
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                text.push('-');
            }
            text.push(char::from(digits[usize::from(byte >> 4)]));
            text.push(char::from(digits[usize::from(byte & 0x0f)]));
        }
        text
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The wire spelling: lower case.
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.spell(false))
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({self})")
    }
}

impl ToSql for Id {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_db_text()))
    }
}

impl FromSql for Id {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Id> {
        let text = value.as_str()?;
        Id::parse(text)
            .ok_or_else(|| FromSqlError::Other(format!("\"{text}\" is not an id").into()))
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.spell(false))
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Id, D::Error> {
        let text = String::deserialize(deserializer)?;
        Id::parse(&text).ok_or_else(|| de::Error::custom("not an id"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ids as they stand in a database the first server wrote.
    const STORED: &str = "8F62A4D1-BDCB-4D86-BD1C-6E351900E629";
    const ON_THE_WIRE: &str = "8f62a4d1-bdcb-4d86-bd1c-6e351900e629";

    #[test]
    fn the_database_spelling_is_in_capitals_and_the_wire_one_is_not() {
        let id = Id::parse(STORED).unwrap();

        assert_eq!(id.to_db_text(), STORED);
        assert_eq!(id.to_string(), ON_THE_WIRE);
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            format!("\"{ON_THE_WIRE}\"")
        );
    }

    #[test]
    fn either_spelling_is_read() {
        assert_eq!(Id::parse(STORED), Id::parse(ON_THE_WIRE));
        let from_json: Id = serde_json::from_str(&format!("\"{STORED}\"")).unwrap();
        assert_eq!(from_json, Id::parse(ON_THE_WIRE).unwrap());
    }

    #[test]
    fn a_text_that_is_not_an_id_is_refused() {
        for text in [
            "",
            "8f62a4d1bdcb4d86bd1c6e351900e629",
            "{8f62a4d1-bdcb-4d86-bd1c-6e351900e629}",
            "(8f62a4d1-bdcb-4d86-bd1c-6e351900e629)",
            " 8f62a4d1-bdcb-4d86-bd1c-6e351900e629",
            "8f62a4d1-bdcb-4d86-bd1c-6e351900e629 ",
            "8f62a4d1-bdcb-4d86-bd1c-6e351900e629\n",
            "+f62a4d1-bdcb-4d86-bd1c-6e351900e629",
            "8f62a4d1-0xcb-4d86-bd1c-6e351900e629",
            "{0x8f62a4d1,0xbdcb,0x4d86,{0xbd,0x1c,0x6e,0x35,0x19,0x00,0xe6,0x29}}",
            "8f62a4d1-bdcb-4d86-bd1c-6e351900e62",
            "8f62a4d1-bdcb-4d86-bd1c-6e351900e62g",
            "8f62a4d1_bdcb_4d86_bd1c_6e351900e629",
            "8f62a4d1-bdcb-4d86-bd1c-6e351900e6299",
            "システム",
        ] {
            assert_eq!(Id::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_new_id_is_random_and_version_four() {
        let first = Id::random();
        let second = Id::random();

        assert_ne!(first, second);
        let text = first.to_string();
        assert_eq!(text.as_bytes()[14], b'4');
        assert!(matches!(text.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn it_goes_into_and_out_of_a_text_column_in_capitals() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t (Id TEXT NOT NULL)")
            .unwrap();
        let id = Id::parse(ON_THE_WIRE).unwrap();

        db.execute("INSERT INTO t (Id) VALUES (?1)", [&id]).unwrap();

        let raw: String = db
            .query_row("SELECT Id FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(raw, STORED);
        let back: Id = db
            .query_row("SELECT Id FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(back, id);
        db.execute("UPDATE t SET Id = lower(Id)", []).unwrap();
        let lower: Id = db
            .query_row("SELECT Id FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(lower, id);
        db.execute("UPDATE t SET Id = 'nonsense'", []).unwrap();
        assert!(
            db.query_row("SELECT Id FROM t", [], |row| row.get::<_, Id>(0))
                .is_err()
        );
    }
}
