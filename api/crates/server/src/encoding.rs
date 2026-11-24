//! The two spellings of bytes as text that the server writes: Base64 with
//! padding, as stored password hashes have it, and Base64 for URLs without
//! padding, as tokens and sealed values have it; and capital hexadecimal, as
//! stored hashes of tokens have it.

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
        if data.len() % 4 != 0 {
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
}
