//! A strict CBOR (RFC 8949) reader for the few shapes WebAuthn uses: the
//! attestation object, the COSE public key and the authenticator extensions.
//!
//! Decisions, each made on purpose:
//! - Indefinite lengths are refused. The authenticator encodings of CTAP2 are
//!   definite-length only, and a streaming form would only add ways to
//!   disagree about where an item ends.
//! - Integers and lengths must use their shortest form, as CTAP2 canonical
//!   encoding demands. A second spelling of the same value is refused.
//! - A map with the same key twice is refused, because two readers could pick
//!   different values for it.
//! - Map keys are integers or text only. Keys do not have to be sorted: some
//!   authenticators do not sort them and the order carries no meaning here.
//! - Tags, floats, `undefined` and other simple values are refused; nothing
//!   in a ceremony uses them.
//! - Nesting is bounded, and so is the number of entries of one array or map,
//!   so hostile input cannot exhaust the stack or the heap.

use std::fmt;

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CborError {
    Truncated,
    TrailingBytes,
    IndefiniteLength,
    NonMinimalEncoding,
    DuplicateKey,
    TooDeep,
    TooManyEntries,
    UnsupportedType,
    InvalidUtf8,
    IntegerOutOfRange,
    UnsupportedKey,
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CborError::Truncated => "input ends inside an item",
            CborError::TrailingBytes => "bytes after the item",
            CborError::IndefiniteLength => "indefinite length",
            CborError::NonMinimalEncoding => "integer or length not in shortest form",
            CborError::DuplicateKey => "duplicate map key",
            CborError::TooDeep => "nested too deeply",
            CborError::TooManyEntries => "too many entries",
            CborError::UnsupportedType => "unsupported type",
            CborError::InvalidUtf8 => "text is not valid UTF-8",
            CborError::IntegerOutOfRange => "integer does not fit 64 bits signed",
            CborError::UnsupportedKey => "map key is not an integer or text",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
}

impl Value {
    /// The value under an integer key of a map.
    pub fn get_int(&self, key: i64) -> Option<&Value> {
        self.get(&Value::Int(key))
    }

    /// The value under a text key of a map.
    pub fn get_text(&self, key: &str) -> Option<&Value> {
        self.get(&Value::Text(key.to_owned()))
    }

    fn get(&self, key: &Value) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Decodes `input` as exactly one item; anything after it is an error.
pub fn decode_exact(input: &[u8]) -> Result<Value, CborError> {
    let (value, used) = decode_prefix(input)?;
    if used != input.len() {
        return Err(CborError::TrailingBytes);
    }
    Ok(value)
}

/// Decodes the item at the start of `input` and says how many bytes it used.
/// Authenticator data needs this: a key and then optional extensions follow
/// one another without a length in between.
pub fn decode_prefix(input: &[u8]) -> Result<(Value, usize), CborError> {
    let mut reader = Reader { input, pos: 0 };
    let value = reader.item(0)?;
    Ok((value, reader.pos))
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CborError> {
        let end = self.pos.checked_add(n).ok_or(CborError::Truncated)?;
        let slice = self.input.get(self.pos..end).ok_or(CborError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, CborError> {
        Ok(self.take(1)?[0])
    }

    /// The argument of an item head: the value, or the length for strings and
    /// containers.
    fn argument(&mut self, info: u8) -> Result<u64, CborError> {
        let (value, minimum) = match info {
            0..=23 => return Ok(u64::from(info)),
            24 => (u64::from(self.byte()?), 24),
            25 => (
                u64::from(u16::from_be_bytes(
                    self.take(2)?.try_into().unwrap_or_default(),
                )),
                1 << 8,
            ),
            26 => (
                u64::from(u32::from_be_bytes(
                    self.take(4)?.try_into().unwrap_or_default(),
                )),
                1 << 16,
            ),
            27 => (
                u64::from_be_bytes(self.take(8)?.try_into().unwrap_or_default()),
                1 << 32,
            ),
            31 => return Err(CborError::IndefiniteLength),
            _ => return Err(CborError::UnsupportedType),
        };
        if value < minimum {
            return Err(CborError::NonMinimalEncoding);
        }
        Ok(value)
    }

    fn length(&mut self, info: u8) -> Result<usize, CborError> {
        usize::try_from(self.argument(info)?).map_err(|_| CborError::Truncated)
    }

    fn item(&mut self, depth: usize) -> Result<Value, CborError> {
        let head = self.byte()?;
        let (major, info) = (head >> 5, head & 31);
        match major {
            0 => {
                let n = self.argument(info)?;
                Ok(Value::Int(
                    i64::try_from(n).map_err(|_| CborError::IntegerOutOfRange)?,
                ))
            }
            1 => {
                let n = self.argument(info)?;
                let n = i64::try_from(n).map_err(|_| CborError::IntegerOutOfRange)?;
                Ok(Value::Int(-1 - n))
            }
            2 => {
                let len = self.length(info)?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = self.length(info)?;
                let text =
                    std::str::from_utf8(self.take(len)?).map_err(|_| CborError::InvalidUtf8)?;
                Ok(Value::Text(text.to_owned()))
            }
            4 => {
                let count = self.container(info, depth)?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push(self.item(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let count = self.container(info, depth)?;
                let mut entries: Vec<(Value, Value)> = Vec::new();
                for _ in 0..count {
                    let key = self.item(depth + 1)?;
                    if !matches!(key, Value::Int(_) | Value::Text(_)) {
                        return Err(CborError::UnsupportedKey);
                    }
                    if entries.iter().any(|(k, _)| *k == key) {
                        return Err(CborError::DuplicateKey);
                    }
                    entries.push((key, self.item(depth + 1)?));
                }
                Ok(Value::Map(entries))
            }
            7 => match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                31 => Err(CborError::IndefiniteLength),
                _ => Err(CborError::UnsupportedType),
            },
            _ => Err(CborError::UnsupportedType),
        }
    }

    /// The entry count of an array or map, checked against the depth limit,
    /// the entry limit and the bytes that are left (every entry takes one).
    fn container(&mut self, info: u8, depth: usize) -> Result<usize, CborError> {
        if depth >= MAX_DEPTH {
            return Err(CborError::TooDeep);
        }
        let count = self.length(info)?;
        if count > MAX_ENTRIES {
            return Err(CborError::TooManyEntries);
        }
        if count > self.input.len() - self.pos {
            return Err(CborError::Truncated);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn rfc8949_appendix_a_values_that_apply() {
        // Vectors from RFC 8949 Appendix A for the types this reader accepts.
        for (coded, expected) in [
            ("00", Value::Int(0)),
            ("17", Value::Int(23)),
            ("1818", Value::Int(24)),
            ("1864", Value::Int(100)),
            ("1903e8", Value::Int(1000)),
            ("1a000f4240", Value::Int(1_000_000)),
            ("1b000000e8d4a51000", Value::Int(1_000_000_000_000)),
            ("20", Value::Int(-1)),
            ("29", Value::Int(-10)),
            ("3863", Value::Int(-100)),
            ("3903e7", Value::Int(-1000)),
            ("40", Value::Bytes(vec![])),
            ("4401020304", Value::Bytes(vec![1, 2, 3, 4])),
            ("60", Value::Text(String::new())),
            ("6161", Value::Text("a".into())),
            ("6449455446", Value::Text("IETF".into())),
            ("62225c", Value::Text("\"\\".into())),
            ("62c3bc", Value::Text("\u{fc}".into())),
            ("80", Value::Array(vec![])),
            (
                "83010203",
                Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            ),
            ("a0", Value::Map(vec![])),
            (
                "a201020304",
                Value::Map(vec![
                    (Value::Int(1), Value::Int(2)),
                    (Value::Int(3), Value::Int(4)),
                ]),
            ),
            ("f4", Value::Bool(false)),
            ("f5", Value::Bool(true)),
            ("f6", Value::Null),
        ] {
            assert_eq!(decode_exact(&hex(coded)).unwrap(), expected, "{coded}");
        }
    }

    #[test]
    fn rfc8949_values_that_are_refused() {
        for (coded, why) in [
            ("c11a514b67b0", CborError::UnsupportedType), // tag 1
            ("f90000", CborError::UnsupportedType),       // half float 0.0
            ("fa47c35000", CborError::UnsupportedType),   // single float
            ("fb3ff199999999999a", CborError::UnsupportedType), // double float
            ("f7", CborError::UnsupportedType),           // undefined
            ("f0", CborError::UnsupportedType),           // simple(16)
            ("c249010000000000000000", CborError::UnsupportedType), // bignum
            ("1bffffffffffffffff", CborError::IntegerOutOfRange), // 2^64-1
            ("3bffffffffffffffff", CborError::IntegerOutOfRange), // -2^64
            ("5fff", CborError::IndefiniteLength),
            ("7fff", CborError::IndefiniteLength),
            ("9fff", CborError::IndefiniteLength),
            ("bfff", CborError::IndefiniteLength),
            ("ff", CborError::IndefiniteLength),
            ("1c", CborError::UnsupportedType), // reserved additional info
        ] {
            assert_eq!(decode_exact(&hex(coded)), Err(why), "{coded}");
        }
    }

    #[test]
    fn largest_and_smallest_integers_fit() {
        assert_eq!(
            decode_exact(&hex("1b7fffffffffffffff")).unwrap(),
            Value::Int(i64::MAX)
        );
        assert_eq!(
            decode_exact(&hex("3b7fffffffffffffff")).unwrap(),
            Value::Int(i64::MIN)
        );
    }

    #[test]
    fn non_shortest_forms_are_refused() {
        for coded in [
            "1800",
            "1817",
            "190000",
            "1900ff",
            "1a0000ffff",
            "1b00000000ffffffff",
            "5800",
            "5817",
            "9800",
            "b800",
        ] {
            assert_eq!(
                decode_exact(&hex(coded)),
                Err(CborError::NonMinimalEncoding),
                "{coded}"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_refused_but_a_prefix_reports_its_length() {
        assert_eq!(decode_exact(&hex("0100")), Err(CborError::TrailingBytes));
        assert_eq!(
            decode_prefix(&hex("820102ff")).unwrap(),
            (Value::Array(vec![Value::Int(1), Value::Int(2)]), 3)
        );
    }

    #[test]
    fn duplicate_keys_are_refused() {
        assert_eq!(
            decode_exact(&hex("a201020103")),
            Err(CborError::DuplicateKey)
        );
        assert_eq!(
            decode_exact(&hex("a2616101616102")),
            Err(CborError::DuplicateKey)
        );
        // The same number as a key and as text is not a duplicate.
        assert!(decode_exact(&hex("a20102613101")).is_ok());
    }

    #[test]
    fn keys_other_than_integers_and_text_are_refused() {
        assert_eq!(
            decode_exact(&hex("a1410101")),
            Err(CborError::UnsupportedKey)
        );
        assert_eq!(decode_exact(&hex("a1f401")), Err(CborError::UnsupportedKey));
    }

    #[test]
    fn unsorted_keys_are_accepted() {
        let value = decode_exact(&hex("a203040102")).unwrap();
        assert_eq!(value.get_int(1), Some(&Value::Int(2)));
        assert_eq!(value.get_int(3), Some(&Value::Int(4)));
    }

    #[test]
    fn truncated_input_is_refused_at_every_length() {
        let whole = hex("a36161426869616282010261636378797a");
        assert!(decode_exact(&whole).is_ok());
        for len in 0..whole.len() {
            assert!(decode_exact(&whole[..len]).is_err(), "{len}");
        }
    }

    #[test]
    fn lengths_larger_than_the_input_do_not_allocate() {
        for coded in [
            "5affffffff",
            "7affffffff",
            "9affffffff",
            "baffffffff",
            "5bffffffffffffffff",
            "9bffffffffffffffff",
        ] {
            assert!(decode_exact(&hex(coded)).is_err(), "{coded}");
        }
        assert_eq!(
            decode_exact(&hex("9a00010000")),
            Err(CborError::TooManyEntries)
        );
    }

    #[test]
    fn invalid_utf8_is_refused() {
        assert_eq!(decode_exact(&hex("62c328")), Err(CborError::InvalidUtf8));
        assert_eq!(decode_exact(&hex("61ff")), Err(CborError::InvalidUtf8));
    }

    #[test]
    fn nesting_is_bounded() {
        let mut ok = vec![0x81; MAX_DEPTH];
        ok.push(0x00);
        assert!(decode_exact(&ok).is_ok());
        let mut deep = vec![0x81; MAX_DEPTH + 1];
        deep.push(0x00);
        assert_eq!(decode_exact(&deep), Err(CborError::TooDeep));
        let mut huge = vec![0x81; 100_000];
        huge.push(0x00);
        assert_eq!(decode_exact(&huge), Err(CborError::TooDeep));
    }

    #[test]
    fn empty_input_is_refused() {
        assert_eq!(decode_exact(&[]), Err(CborError::Truncated));
    }
}
