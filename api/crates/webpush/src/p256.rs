//! Small P-256 helpers shared by the encryption and the VAPID code.

use aws_lc_rs::agreement::{self, ECDH_P256, UnparsedPublicKey};

/// Length of an uncompressed P-256 point: `0x04 || X || Y`.
pub const POINT_LEN: usize = 65;
/// Length of a P-256 private scalar.
pub const SCALAR_LEN: usize = 32;

/// The order of the P-256 group, big endian. A private scalar must be below it.
const ORDER: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63, 0x25, 0x51,
];

/// Whether `bytes` is an uncompressed point that lies on the curve.
///
/// The check is made by the library: an agreement with a point that is not on
/// the curve is refused, so a throw-away key is enough to ask the question.
pub fn is_valid_point(bytes: &[u8]) -> bool {
    if bytes.len() != POINT_LEN || bytes[0] != 0x04 {
        return false;
    }
    let Ok(probe) = agreement::PrivateKey::generate(&ECDH_P256) else {
        return false;
    };
    agreement::agree(
        &probe,
        UnparsedPublicKey::new(&ECDH_P256, bytes),
        (),
        |_| Ok(()),
    )
    .is_ok()
}

/// Whether a scalar is a usable private key: 32 bytes, not zero, below the order.
pub fn is_valid_scalar(bytes: &[u8]) -> bool {
    bytes.len() == SCALAR_LEN && bytes.iter().any(|&b| b != 0) && bytes < &ORDER[..]
}

/// The public point belonging to a private scalar, or `None` if it is unusable.
pub fn public_from_scalar(scalar: &[u8]) -> Option<[u8; POINT_LEN]> {
    if !is_valid_scalar(scalar) {
        return None;
    }
    let key = agreement::PrivateKey::from_private_key(&ECDH_P256, scalar).ok()?;
    let public = key.compute_public_key().ok()?;
    <[u8; POINT_LEN]>::try_from(public.as_ref()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_bounds() {
        assert!(!is_valid_scalar(&[0; 32]));
        assert!(!is_valid_scalar(&ORDER));
        assert!(!is_valid_scalar(&[0xff; 32]));
        assert!(!is_valid_scalar(&[1; 31]));
        assert!(!is_valid_scalar(&[1; 33]));
        let mut below = ORDER;
        below[31] -= 1;
        assert!(is_valid_scalar(&below));
        let mut one = [0u8; 32];
        one[31] = 1;
        assert!(is_valid_scalar(&one));
    }

    #[test]
    fn generator_point_is_valid_and_its_neighbour_is_not() {
        let mut one = [0u8; 32];
        one[31] = 1;
        let g = public_from_scalar(&one).expect("scalar one is valid");
        assert!(is_valid_point(&g));
        let mut off = g;
        off[64] ^= 1;
        assert!(!is_valid_point(&off));
        let mut compressed = g;
        compressed[0] = 0x02;
        assert!(!is_valid_point(&compressed));
        assert!(!is_valid_point(&g[..64]));
        assert!(!is_valid_point(&[0x04; 65]));
    }
}
