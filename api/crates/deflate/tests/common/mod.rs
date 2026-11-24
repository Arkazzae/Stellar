//! A tiny deterministic generator of test inputs, shared by the test files.
#![allow(dead_code)]

/// xorshift64*: enough randomness for test data, and the same everywhere.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn words(rng: &mut Rng, n: usize) -> Vec<u8> {
    const VOCAB: [&str; 24] = [
        "the", "of", "backup", "archive", "stellarr", "voice", "channel", "message", "server",
        "and", "a", "to", "in", "is", "that", "it", "was", "for", "on", "are", "with", "as", "his",
        "they",
    ];
    let mut out = Vec::new();
    while out.len() < n {
        out.extend_from_slice(VOCAB[rng.below(VOCAB.len() as u64) as usize].as_bytes());
        out.push(if rng.below(12) == 0 { b'\n' } else { b' ' });
    }
    out.truncate(n);
    out
}

/// A skewed alphabet of `k` symbols: dynamic Huffman territory.
fn skewed(rng: &mut Rng, n: usize, k: u64) -> Vec<u8> {
    (0..n)
        .map(|_| {
            let a = rng.below(k);
            let b = rng.below(k);
            a.min(b) as u8
        })
        .collect()
}

/// Byte values whose frequencies are Fibonacci numbers: the shape that makes
/// a Huffman code as deep as possible, so the limit on code length has to act.
fn fibonacci(rng: &mut Rng, symbols: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut a, mut b) = (1usize, 1usize);
    for symbol in 0..symbols {
        out.extend(std::iter::repeat_n((symbol * 9 + 3) as u8, a));
        (a, b) = (b, a + b);
    }
    // Shuffled, so that no run or repeat helps and only the literals count.
    for i in (1..out.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        out.swap(i, j);
    }
    out
}

/// Pages of a table-like binary file: headers, counters, a little text.
fn records(rng: &mut Rng, n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut counter = 0u32;
    while out.len() < n {
        out.extend_from_slice(&counter.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 1, 0]);
        out.extend_from_slice(&rng.below(1000).to_le_bytes()[..3]);
        let n = 8 + rng.below(24) as usize;
        out.extend_from_slice(&words(rng, n));
        counter += 1;
    }
    out.truncate(n);
    out
}

/// A block of random bytes repeated at a chosen distance.
fn repeated_at(rng: &mut Rng, distance: usize, total: usize) -> Vec<u8> {
    let mut out = rng.bytes(distance.min(total));
    while out.len() < total {
        let from = out.len() - distance;
        let take = distance.min(total - out.len());
        let chunk: Vec<u8> = out[from..from + take].to_vec();
        out.extend_from_slice(&chunk);
    }
    out
}

/// Compressible stretches between incompressible ones.
fn mixed(rng: &mut Rng, n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while out.len() < n {
        let part = 200 + rng.below(20_000) as usize;
        match rng.below(4) {
            0 => out.extend_from_slice(&rng.bytes(part)),
            1 => out.extend(std::iter::repeat_n(rng.next() as u8, part)),
            2 => out.extend_from_slice(&words(rng, part)),
            _ => out.extend_from_slice(&skewed(rng, part, 6)),
        }
    }
    out.truncate(n);
    out
}

/// `(name, bytes)` for a spread of shapes and sizes, the same on every run.
pub fn corpus(scale: usize) -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng::new(0x5eed);
    let mut cases: Vec<(String, Vec<u8>)> = vec![
        ("empty".into(), Vec::new()),
        ("one-byte".into(), vec![b'a']),
        ("two-bytes".into(), b"ab".to_vec()),
        ("three-same".into(), b"aaa".to_vec()),
        ("hello".into(), b"hello hello hello hello".to_vec()),
    ];
    for n in [
        1usize, 2, 3, 257, 258, 259, 1000, 32_767, 32_768, 32_769, 65_000, 65_535, 65_536, 70_000,
        300_000,
    ] {
        cases.push((format!("zeros-{n}"), vec![0; n]));
        cases.push((format!("ones-{n}"), vec![0xff; n]));
    }
    for n in [1usize, 10, 100, 1000, 4096, 65_535, 65_536, 65_537, 200_000] {
        cases.push((format!("random-{n}"), rng.bytes(n)));
    }
    for n in [100usize, 5000, 70_000, 400_000] {
        cases.push((format!("words-{n}"), words(&mut rng, n)));
        cases.push((format!("records-{n}"), records(&mut rng, n)));
        cases.push((format!("mixed-{n}"), mixed(&mut rng, n)));
    }
    for symbols in [18usize, 22, 26] {
        cases.push((format!("fibonacci-{symbols}"), fibonacci(&mut rng, symbols)));
    }
    for k in [2u64, 3, 5, 17, 64, 200] {
        cases.push((format!("skewed-{k}"), skewed(&mut rng, 50_000, k)));
    }
    for distance in [
        1usize, 2, 3, 4, 100, 258, 259, 4096, 4097, 32_767, 32_768, 32_769, 40_000,
    ] {
        cases.push((
            format!("repeat-{distance}"),
            repeated_at(&mut rng, distance, 3 * distance + 1000),
        ));
    }
    // Big enough to slide the window and close many blocks.
    for n in [700_000usize, 1_500_000] {
        let n = n * scale.max(1);
        cases.push((format!("big-mixed-{n}"), mixed(&mut rng, n)));
        cases.push((format!("big-words-{n}"), words(&mut rng, n)));
    }
    for i in 0..60 {
        let n = rng.below(3000) as usize;
        let kind = rng.below(3);
        let data = match kind {
            0 => rng.bytes(n),
            1 => {
                let k = 1 + rng.below(40);
                skewed(&mut rng, n, k)
            }
            _ => words(&mut rng, n),
        };
        cases.push((format!("small-{i}"), data));
    }
    cases
}

/// Writes bits the way a deflate stream is written, for hand-built streams.
#[derive(Default)]
pub struct BitWriter {
    pub bytes: Vec<u8>,
    bits: u64,
    count: u32,
}

impl BitWriter {
    pub fn bits(&mut self, value: u32, n: u32) -> &mut Self {
        self.bits |= u64::from(value) << self.count;
        self.count += n;
        while self.count >= 8 {
            self.bytes.push(self.bits as u8);
            self.bits >>= 8;
            self.count -= 8;
        }
        self
    }

    /// A Huffman code: sent most significant bit first.
    pub fn code(&mut self, code: u32, len: u32) -> &mut Self {
        for i in (0..len).rev() {
            self.bits((code >> i) & 1, 1);
        }
        self
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if self.count > 0 {
            self.bytes.push(self.bits as u8);
            self.bits = 0;
            self.count = 0;
        }
        std::mem::take(&mut self.bytes)
    }
}
