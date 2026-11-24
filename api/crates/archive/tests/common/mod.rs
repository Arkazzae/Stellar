//! Shared helpers of the archive tests: a small deterministic generator, a
//! scratch directory, and ways to build archives in memory.
#![allow(dead_code)]

use std::io::Cursor;
use std::path::PathBuf;

use stellarr_archive::{EntryOptions, Level, ZipArchive, ZipWriter};

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

    pub fn text(&mut self, n: usize) -> Vec<u8> {
        const WORDS: [&str; 12] = [
            "the", "backup", "of", "stellarr", "a", "voice", "channel", "and", "message", "to",
            "in", "archive",
        ];
        let mut out = Vec::new();
        while out.len() < n {
            out.extend_from_slice(WORDS[self.below(12) as usize].as_bytes());
            out.push(b' ');
        }
        out.truncate(n);
        out
    }
}

pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("stellarr-archive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a test archive holds: name, bytes, and whether it is deflated.
pub fn sample_entries() -> Vec<(String, Vec<u8>, bool)> {
    let mut rng = Rng::new(11);
    vec![
        ("database/stellarr.db".into(), rng.text(200_000), true),
        (
            "objects/images/2026/11/a.png".into(),
            rng.bytes(5000),
            false,
        ),
        ("objects/audio/b.webm".into(), b"voice".to_vec(), false),
        ("keys/key-1.xml".into(), b"<key />".to_vec(), true),
        (
            "instance/secrets/turn-shared-secret".into(),
            b"secret".to_vec(),
            true,
        ),
        ("instance/tls/privkey.pem".into(), rng.text(1700), true),
        (
            "objects/\u{fc}n\u{ee}code/\u{65e5}\u{672c}\u{8a9e}.txt".into(),
            b"text".to_vec(),
            false,
        ),
        ("empty-stored".into(), Vec::new(), false),
        ("empty-deflated".into(), Vec::new(), true),
        (
            "manifest.json".into(),
            b"{\"format\":\"stellarr-backup\"}".to_vec(),
            true,
        ),
    ]
}

pub fn write_sample(force_zip64: bool) -> Vec<u8> {
    let mut zip = ZipWriter::new(Vec::new()).force_zip64(force_zip64);
    for (name, data, deflated) in sample_entries() {
        let options = if deflated {
            EntryOptions::deflated(Level::Default)
        } else {
            EntryOptions::stored()
        };
        zip.add_bytes(&name, options.modified(1_700_000_000), &data)
            .unwrap();
    }
    zip.finish().unwrap()
}

pub fn open(bytes: &[u8]) -> ZipArchive<Cursor<&[u8]>> {
    ZipArchive::new(Cursor::new(bytes)).unwrap()
}

/// The index of the central directory's record for `name` in `bytes`.
pub fn central_record(bytes: &[u8], name: &str) -> usize {
    let sig = [0x50, 0x4b, 0x01, 0x02];
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(4).position(|w| w == sig) {
        let record = at + found;
        let name_len = u16::from_le_bytes([bytes[record + 28], bytes[record + 29]]) as usize;
        if &bytes[record + 46..record + 46 + name_len] == name.as_bytes() {
            return record;
        }
        at = record + 4;
    }
    panic!("no central record for {name}");
}

/// Where the data of `name` starts, from its local header.
pub fn data_start(bytes: &[u8], name: &str) -> usize {
    let record = central_record(bytes, name);
    let offset = u32::from_le_bytes(bytes[record + 42..record + 46].try_into().unwrap()) as usize;
    let name_len = u16::from_le_bytes([bytes[offset + 26], bytes[offset + 27]]) as usize;
    let extra_len = u16::from_le_bytes([bytes[offset + 28], bytes[offset + 29]]) as usize;
    offset + 30 + name_len + extra_len
}
