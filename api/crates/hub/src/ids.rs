use aws_lc_rs::rand;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A fresh unguessable name: sixteen random bytes as 22 URL-safe characters.
/// Connection ids and the tokens that claim a connection are made this way.
pub(crate) fn random_id() -> String {
    let mut bytes = [0u8; 16];
    // The system generator failing means the process cannot keep any secret.
    rand::fill(&mut bytes).expect("the system random number generator works");
    let mut out = String::with_capacity(22);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        let characters = (chunk.len() * 8).div_ceil(6);
        for i in 0..characters {
            out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_22_url_safe_characters_and_differ() {
        let a = random_id();
        let b = random_id();
        assert_eq!(a.len(), 22);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(a, b);
    }
}
