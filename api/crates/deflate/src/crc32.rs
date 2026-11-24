//! CRC-32 (the zip and gzip polynomial), eight bytes at a time.

const POLY: u32 = 0xedb8_8320;

const fn build_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        tables[0][i] = crc;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let prev = tables[k - 1][i];
            tables[k][i] = (prev >> 8) ^ tables[0][(prev & 0xff) as usize];
            i += 1;
        }
        k += 1;
    }
    tables
}

static TABLES: [[u32; 256]; 8] = build_tables();

/// A running CRC-32.
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub const fn new() -> Self {
        Self { state: u32::MAX }
    }

    pub fn update(&mut self, data: &[u8]) {
        let t = &TABLES;
        let mut crc = self.state;
        let (chunks, remainder) = data.as_chunks::<8>();
        for chunk in chunks {
            let lo = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) ^ crc;
            let hi = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            crc = t[7][(lo & 0xff) as usize]
                ^ t[6][((lo >> 8) & 0xff) as usize]
                ^ t[5][((lo >> 16) & 0xff) as usize]
                ^ t[4][(lo >> 24) as usize]
                ^ t[3][(hi & 0xff) as usize]
                ^ t[2][((hi >> 8) & 0xff) as usize]
                ^ t[1][((hi >> 16) & 0xff) as usize]
                ^ t[0][(hi >> 24) as usize];
        }
        for &byte in remainder {
            crc = t[0][((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
        }
        self.state = crc;
    }

    /// The checksum of everything fed so far; more data may follow.
    pub const fn value(&self) -> u32 {
        !self.state
    }
}

/// The CRC-32 of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.value()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn pieces_equal_the_whole() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 31 + i / 7) as u8).collect();
        let whole = crc32(&data);
        for split in [0, 1, 7, 8, 9, 100, 4999, 5000] {
            let mut crc = Crc32::new();
            crc.update(&data[..split]);
            crc.update(&data[split..]);
            assert_eq!(crc.value(), whole, "split at {split}");
        }
    }
}
