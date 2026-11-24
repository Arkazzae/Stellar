//! Standard base64 with padding, as the opening handshake uses it. It is
//! only the two directions the handshake needs: the accept value is encoded,
//! and the client's key is decoded to check that it holds sixteen bytes.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn value(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes padded base64; anything else, a wrong length or stray bits in the
/// padding included, is `None`.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let groups = bytes.len() / 4;
    for (index, group) in bytes.chunks(4).enumerate() {
        let last = index + 1 == groups;
        let padding = group.iter().rev().take_while(|b| **b == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for byte in &group[..4 - padding] {
            n = (n << 6) | value(*byte)?;
        }
        n <<= 6 * padding as u32;
        // Bits that padding discards must be zero, or two texts would decode
        // to the same key.
        if padding == 1 && n & 0xff != 0 || padding == 2 && n & 0xffff != 0 {
            return None;
        }
        let triple = n.to_be_bytes();
        out.extend_from_slice(&triple[1..4 - padding]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_4648_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            if !plain.is_empty() {
                assert_eq!(decode(coded).unwrap(), plain.as_bytes());
            }
        }
    }

    #[test]
    fn decode_rejects_what_is_not_base64() {
        for bad in [
            "",
            "Zg=",
            "Zg",
            "Z===",
            "Zm9v=Zm9v",
            "Zm9!",
            "Zh==",
            "Zm9=",
            "====",
            "Zm 9",
        ] {
            assert!(decode(bad).is_none(), "{bad:?}");
        }
    }
}
