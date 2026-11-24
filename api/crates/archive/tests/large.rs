//! Sizes past what 32 bits hold, without a disk: the archive is kept as a
//! list of pieces, where a long run of zero bytes is one piece of a few bytes.
//! Slow, so ignored; run with `--release -- --ignored`.

use std::io::{self, Read, Seek, SeekFrom, Write};

use stellarr_archive::{EntryOptions, Level, ZipArchive, ZipWriter};

enum Piece {
    Bytes(Vec<u8>),
    Zeros(u64),
}

impl Piece {
    fn len(&self) -> u64 {
        match self {
            Piece::Bytes(bytes) => bytes.len() as u64,
            Piece::Zeros(n) => *n,
        }
    }
}

#[derive(Default)]
struct Pieces {
    pieces: Vec<Piece>,
    position: u64,
}

impl Write for Pieces {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let zeros = buf.len() >= 4096 && buf.iter().all(|&b| b == 0);
        match (self.pieces.last_mut(), zeros) {
            (Some(Piece::Zeros(n)), true) => *n += buf.len() as u64,
            (Some(Piece::Bytes(bytes)), false) => bytes.extend_from_slice(buf),
            (_, true) => self.pieces.push(Piece::Zeros(buf.len() as u64)),
            (_, false) => self.pieces.push(Piece::Bytes(buf.to_vec())),
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Pieces {
    fn total(&self) -> u64 {
        self.pieces.iter().map(Piece::len).sum()
    }
}

impl Read for Pieces {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut start = 0u64;
        for piece in &self.pieces {
            let end = start + piece.len();
            if self.position < end {
                let inside = self.position - start;
                let n = (buf.len() as u64).min(end - self.position) as usize;
                match piece {
                    Piece::Bytes(bytes) => {
                        buf[..n].copy_from_slice(&bytes[inside as usize..inside as usize + n])
                    }
                    Piece::Zeros(_) => buf[..n].fill(0),
                }
                self.position += n as u64;
                return Ok(n);
            }
            start = end;
        }
        Ok(0)
    }
}

impl Seek for Pieces {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.total() as i64 + n,
            SeekFrom::Current(n) => self.position as i64 + n,
        };
        self.position =
            u64::try_from(target).map_err(|_| io::Error::other("seek before the start"))?;
        Ok(self.position)
    }
}

const GIB: u64 = 1 << 30;

fn feed_zeros<W: Write>(out: &mut W, total: u64) {
    let chunk = vec![0u8; 1 << 20];
    let mut left = total;
    while left > 0 {
        let n = left.min(chunk.len() as u64) as usize;
        out.write_all(&chunk[..n]).unwrap();
        left -= n as u64;
    }
}

#[test]
#[ignore = "streams several gigabytes through the CRC and the codecs"]
fn sizes_and_offsets_past_four_gigabytes_round_trip() {
    let big = 4 * GIB + 12_345;
    let mut zip = ZipWriter::new(Pieces::default());

    let mut first = zip
        .start_entry("objects/big-stored", EntryOptions::stored())
        .unwrap();
    feed_zeros(&mut first, big);
    let first_info = first.finish().unwrap();
    assert_eq!(first_info.uncompressed_size, big);

    // Placed after four gigabytes: its offset needs ZIP64 though it is tiny.
    let after = zip
        .add_bytes("objects/after", EntryOptions::stored(), b"after")
        .unwrap();
    assert_eq!(after.uncompressed_size, 5);

    let mut second = zip
        .start_entry("database/big-deflated", EntryOptions::deflated(Level::Fast))
        .unwrap();
    feed_zeros(&mut second, big);
    let second_info = second.finish().unwrap();
    assert_eq!(second_info.uncompressed_size, big);
    assert!(second_info.compressed_size < 100 * 1024 * 1024);

    zip.add_bytes(
        "manifest.json",
        EntryOptions::deflated(Level::Default),
        b"{}",
    )
    .unwrap();
    let mut pieces = zip.finish().unwrap();
    assert!(pieces.total() > 4 * GIB);
    pieces.position = 0;

    let mut zip = ZipArchive::new(pieces).unwrap();
    let entries = zip.entries();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].uncompressed_size(), big);
    assert_eq!(entries[0].compressed_size(), big);
    assert_eq!(entries[1].name(), Some("objects/after"));
    assert_eq!(entries[2].uncompressed_size(), big);
    assert_eq!(entries[2].compressed_size(), second_info.compressed_size);
    assert_eq!(zip.total_uncompressed_size(), 2 * big + 5 + 2);
    assert_eq!(zip.read_to_vec(1, 10).unwrap(), b"after");
    assert_eq!(zip.read_to_vec(3, 10).unwrap(), b"{}");
    zip.verify_all().unwrap();
}
