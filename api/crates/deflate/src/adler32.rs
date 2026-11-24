//! Adler-32, the checksum at the end of a zlib stream.

const MOD: u32 = 65_521;
/// The most bytes that can be summed before the 32-bit sums must be reduced.
const NMAX: usize = 5552;

#[derive(Clone, Copy, Debug)]
pub struct Adler32 {
    a: u32,
    b: u32,
}

impl Default for Adler32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Adler32 {
    pub const fn new() -> Self {
        Self { a: 1, b: 0 }
    }

    pub fn update(&mut self, data: &[u8]) {
        for block in data.chunks(NMAX) {
            for &byte in block {
                self.a += u32::from(byte);
                self.b += self.a;
            }
            self.a %= MOD;
            self.b %= MOD;
        }
    }

    pub const fn value(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_known_value() {
        let mut a = Adler32::new();
        a.update(b"Wikipedia");
        assert_eq!(a.value(), 0x11e6_0398);
        assert_eq!(Adler32::new().value(), 1);
    }

    #[test]
    fn long_input_does_not_overflow() {
        let mut a = Adler32::new();
        a.update(&vec![255u8; 1_000_000]);
        let mut b = Adler32::new();
        for _ in 0..1000 {
            b.update(&[255u8; 1000]);
        }
        assert_eq!(a.value(), b.value());
    }
}
