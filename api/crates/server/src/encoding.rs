//! The two spellings of bytes as text that the server writes: Base64 with
//! padding, as stored password hashes have it, and Base64 for URLs without
//! padding, as tokens and sealed values have it; and capital hexadecimal, as
//! stored hashes of tokens have it, and lower-case hexadecimal as ids in logs
//! and invite codes have it.

use serde::Deserialize;

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn encode(bytes: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let block = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let value = u32::from(block[0]) << 16 | u32::from(block[1]) << 8 | u32::from(block[2]);
        let characters = chunk.len() + 1;
        for index in 0..4 {
            if index < characters {
                text.push(char::from(
                    alphabet[(value >> (18 - 6 * index) & 63) as usize],
                ));
            } else if pad {
                text.push('=');
            }
        }
    }
    text
}

/// Decodes text that must be exactly as `encode` writes it: the right
/// alphabet, and padding where `pad` says and nowhere else. A trailing group
/// with bits left over is refused, so every byte string has one spelling.
fn decode(text: &str, alphabet: &[u8; 64], pad: bool) -> Option<Vec<u8>> {
    let mut data = text.as_bytes();
    if pad {
        if !data.len().is_multiple_of(4) {
            return None;
        }
        let padding = data.iter().rev().take_while(|&&b| b == b'=').count();
        if padding > 2 {
            return None;
        }
        data = &data[..data.len() - padding];
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(data.len() * 3 / 4);
    for chunk in data.chunks(4) {
        let mut value = 0u32;
        for &character in chunk {
            let digit = alphabet.iter().position(|&a| a == character)? as u32;
            value = value << 6 | digit;
        }
        value <<= 6 * (4 - chunk.len()) as u32;
        let produced = chunk.len() * 6 / 8;
        let decoded = [(value >> 16) as u8, (value >> 8) as u8, value as u8];
        // The bits that do not make a whole byte are zero in a canonical text.
        if decoded[produced..].iter().any(|&b| b != 0) {
            return None;
        }
        bytes.extend_from_slice(&decoded[..produced]);
    }
    Some(bytes)
}

pub fn base64(bytes: &[u8]) -> String {
    encode(bytes, STANDARD, true)
}

pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    decode(text, STANDARD, true)
}

pub fn base64_url(bytes: &[u8]) -> String {
    encode(bytes, URL_SAFE, false)
}

pub fn from_base64_url(text: &str) -> Option<Vec<u8>> {
    decode(text, URL_SAFE, false)
}

pub fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

pub fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The value of one hexadecimal digit in either case.
pub fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Writes text as one URL component: every byte of its UTF-8 but the
/// unreserved characters of RFC 3986 (`-._~`, letters, digits) as `%` and two
/// capital hexadecimal digits.
pub fn percent_encode(text: &str) -> String {
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

/// Bytes in a JSON body: standard Base64 with padding, exactly as
/// [`base64`] writes it. White space is not part of it.
pub struct Octets(pub Vec<u8>);

impl<'de> serde::Deserialize<'de> for Octets {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Octets, D::Error> {
        let text = String::deserialize(deserializer)?;
        from_base64(&text)
            .map(Octets)
            .ok_or_else(|| serde::de::Error::custom("not Base64"))
    }
}

impl serde::Serialize for Octets {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64(&self.0))
    }
}

/// For `#[serde(default, deserialize_with = "optional_octets")]`: bytes that
/// may be left out or `null`.
pub fn optional_octets<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<u8>>, D::Error> {
    Ok(Option::<Octets>::deserialize(deserializer)?.map(|octets| octets.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_base64_matches_the_rfc_vectors() {
        for (plain, text) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), text);
            assert_eq!(from_base64(text).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn the_url_spelling_has_no_padding_and_its_own_last_two_characters() {
        let bytes = [0xfb, 0xff, 0xfe, 0x01];
        assert_eq!(base64(&bytes), "+//+AQ==");
        assert_eq!(base64_url(&bytes), "-__-AQ");
        assert_eq!(from_base64_url("-__-AQ").unwrap(), bytes);
    }

    #[test]
    fn text_that_is_not_the_one_spelling_of_some_bytes_is_refused() {
        for text in [
            "Zg", "Zg=", "Z", "Zm9v=", "Zh==", "Zm8_", "Zm 8=", "====", "Zg===", "Zm9vY",
        ] {
            assert_eq!(from_base64(text), None, "{text:?}");
        }
        for text in ["Z", "Zh", "Zg==", "Zm+/", "Zm9vY", " Zg"] {
            assert_eq!(from_base64_url(text), None, "{text:?}");
        }
    }

    #[test]
    fn hex_is_in_capitals() {
        assert_eq!(hex_upper(&[0x00, 0xab, 0xff]), "00ABFF");
    }

    #[test]
    fn bytes_in_a_body_are_base64_and_nothing_around_it() {
        let key = base64(&[7u8; 32]);
        let read = |text: &str| serde_json::from_value::<Octets>(serde_json::json!(text));
        assert_eq!(read(&key).unwrap().0, vec![7u8; 32]);
        for bad in [
            format!("{} {}", &key[..10], &key[10..]),
            format!("{}\n", key),
            format!(" {key}"),
            format!("{}\t{}", &key[..10], &key[10..]),
            key.trim_end_matches('=').to_string(),
        ] {
            assert!(read(&bad).is_err(), "{bad:?}");
        }
        assert!(serde_json::from_value::<Octets>(serde_json::json!(5)).is_err());
    }
}
