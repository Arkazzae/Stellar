//! Unpadded base64url (RFC 4648 section 5), strict in what it accepts.
//!
//! Browsers hand in keys in this form and the VAPID token is built from it.
//! The decoder is deliberately narrow: one alphabet, padding only when it is
//! exact, and no stray bits in the last character, so one key has one spelling
//! and the stored form can be compared byte for byte.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes without padding.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

/// Decodes text of at most `max_chars` characters into bytes.
///
/// Returns `None` for empty input, over-long input, whitespace, characters
/// outside the URL-safe alphabet, padding that is not exactly what the length
/// needs, a length no byte string can have, and non-zero bits after the last
/// whole byte.
pub fn decode(text: &str, max_chars: usize) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() > max_chars {
        return None;
    }
    let bytes = text.as_bytes();
    let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    let body = &bytes[..bytes.len() - padding];
    if padding > 2 || body.is_empty() || body.len() % 4 == 1 {
        return None;
    }
    // Padding, when present, has to complete the final group of four.
    if padding != 0 && !(body.len() + padding).is_multiple_of(4) {
        return None;
    }

    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in body {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if acc != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_length() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len as u8).map(|n| n.wrapping_mul(37)).collect();
            let text = encode(&data);
            if len == 0 {
                assert_eq!(decode(&text, 100), None);
            } else {
                assert_eq!(decode(&text, 100).as_deref(), Some(&data[..]));
            }
        }
    }

    #[test]
    fn matches_the_rfc_4648_vectors() {
        assert_eq!(encode(b"f"), "Zg");
        assert_eq!(encode(b"fo"), "Zm8");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg");
        assert_eq!(encode(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn accepts_exact_padding_only() {
        assert_eq!(decode("Zg==", 8).as_deref(), Some(&b"f"[..]));
        assert_eq!(decode("Zm8=", 8).as_deref(), Some(&b"fo"[..]));
        assert_eq!(decode("Zg=", 8), None);
        assert_eq!(decode("Zg===", 8), None);
        assert_eq!(decode("Zm9v=", 8), None);
        assert_eq!(decode("=", 8), None);
    }

    #[test]
    fn refuses_what_is_not_the_alphabet() {
        assert_eq!(decode("", 8), None);
        assert_eq!(decode("Zm+v", 8), None);
        assert_eq!(decode("Zm/v", 8), None);
        assert_eq!(decode("Zm v", 8), None);
        assert_eq!(decode("Zm9v\n", 8), None);
        assert_eq!(decode("Zm9\u{e9}", 8), None);
        assert_eq!(decode("Z", 8), None);
        assert_eq!(decode("Zm9vY", 8), None);
    }

    #[test]
    fn refuses_non_canonical_trailing_bits_and_long_input() {
        // "Zh" decodes to the byte of "Zg" but with a stray low bit set.
        assert_eq!(decode("Zh", 8), None);
        assert_eq!(decode("Zm9vYmFy", 7), None);
        assert!(decode("Zm9vYmFy", 8).is_some());
    }
}
