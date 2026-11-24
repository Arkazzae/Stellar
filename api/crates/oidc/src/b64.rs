//! Base64url without padding, as JOSE and PKCE use it.
//!
//! Decoding is strict: one alphabet, no padding, no whitespace, and no
//! leftover bits set, so a value has exactly one spelling.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let mut group = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            group |= u32::from(*byte) << (16 - 8 * i);
        }
        // A chunk of n bytes makes n + 1 characters.
        for i in 0..=chunk.len() {
            out.push(char::from(
                ALPHABET[((group >> (18 - 6 * i)) & 63) as usize],
            ));
        }
    }
    out
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in bytes {
        let value = ALPHABET.iter().position(|&a| a == c)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // Bits that carry no data must be zero, or two texts would mean one value.
    (acc == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_length() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode(&encode(&data)).as_deref(), Some(&data[..]));
        }
    }

    #[test]
    fn matches_rfc_4648_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg");
        assert_eq!(encode(b"fo"), "Zm8");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg");
        assert_eq!(encode(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn refuses_what_is_not_canonical() {
        for bad in ["Zg==", "Zh", "Z", "Zm9v!", "Zm 9v", "+/8", "Zm9v\n", "é"] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }
}
