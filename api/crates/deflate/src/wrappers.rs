//! Readers for the two wrappers around a DEFLATE stream: gzip (RFC 1952) and
//! zlib (RFC 1950), as they come in HTTP bodies.

use std::io::{self, Read};

use crate::adler32::Adler32;
use crate::crc32::Crc32;
use crate::error::Error;
use crate::inflate::{Inflate, read_all};

fn raw_byte<R: Read>(inflate: &mut Inflate<R>) -> io::Result<Option<u8>> {
    let mut byte = [0u8; 1];
    Ok(if inflate.read_raw(&mut byte)? == 0 {
        None
    } else {
        Some(byte[0])
    })
}

fn need_byte<R: Read>(inflate: &mut Inflate<R>) -> io::Result<u8> {
    raw_byte(inflate)?.ok_or_else(|| Error::Truncated.into())
}

fn need_exact<R: Read>(inflate: &mut Inflate<R>, out: &mut [u8]) -> io::Result<()> {
    for slot in out {
        *slot = need_byte(inflate)?;
    }
    Ok(())
}

enum GzipState {
    /// Waiting for the first byte of a member.
    Header,
    Body,
    Done,
}

/// Reads gzip data: one member or several in a row, each with its header
/// read and its CRC-32 and length checked. Bytes after the last member that
/// are not another member are an error.
pub struct GzipReader<R> {
    inflate: Inflate<R>,
    state: GzipState,
    crc: Crc32,
    length: u32,
    first_member: bool,
}

impl<R: Read> GzipReader<R> {
    pub fn new(src: R) -> Self {
        Self::from_inflate(Inflate::new(src))
    }

    /// Refuses data that decompresses to more than `limit` bytes in total.
    #[must_use]
    pub fn with_limit(self, limit: u64) -> Self {
        Self::from_inflate(self.inflate.with_limit(limit))
    }

    fn from_inflate(inflate: Inflate<R>) -> Self {
        Self {
            inflate,
            state: GzipState::Header,
            crc: Crc32::new(),
            length: 0,
            first_member: true,
        }
    }

    fn read_header(&mut self, first: u8) -> io::Result<()> {
        let bad = |why| io::Error::from(Error::BadHeader(why));
        let inflate = &mut self.inflate;
        let mut fixed = [0u8; 10];
        fixed[0] = first;
        need_exact(inflate, &mut fixed[1..])?;
        if fixed[0] != 0x1f || fixed[1] != 0x8b {
            return Err(bad("not a gzip stream"));
        }
        if fixed[2] != 8 {
            return Err(bad("unknown compression method"));
        }
        let flags = fixed[3];
        if flags & 0xe0 != 0 {
            return Err(bad("reserved flags are set"));
        }
        let mut header = Crc32::new();
        header.update(&fixed);
        if flags & 4 != 0 {
            let mut len = [0u8; 2];
            need_exact(inflate, &mut len)?;
            header.update(&len);
            for _ in 0..u16::from_le_bytes(len) {
                header.update(&[need_byte(inflate)?]);
            }
        }
        for flag in [8, 16] {
            if flags & flag != 0 {
                loop {
                    let byte = need_byte(inflate)?;
                    header.update(&[byte]);
                    if byte == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 2 != 0 {
            let mut check = [0u8; 2];
            need_exact(inflate, &mut check)?;
            if u16::from_le_bytes(check) != (header.value() & 0xffff) as u16 {
                return Err(bad("header checksum mismatch"));
            }
        }
        Ok(())
    }

    fn read_trailer(&mut self) -> io::Result<()> {
        let mut trailer = [0u8; 8];
        need_exact(&mut self.inflate, &mut trailer)?;
        let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let length = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        if crc != self.crc.value() {
            return Err(Error::ChecksumMismatch.into());
        }
        if length != self.length {
            return Err(Error::LengthMismatch.into());
        }
        Ok(())
    }
}

impl<R: Read> Read for GzipReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            match self.state {
                GzipState::Done => return Ok(0),
                GzipState::Header => {
                    let first = if self.first_member {
                        need_byte(&mut self.inflate)?
                    } else {
                        // After a member, the end of the input is a clean end.
                        match raw_byte(&mut self.inflate)? {
                            Some(byte) => byte,
                            None => {
                                self.state = GzipState::Done;
                                return Ok(0);
                            }
                        }
                    };
                    self.read_header(first).map_err(|error| {
                        if self.first_member || Error::from_io(&error) == Some(Error::Truncated) {
                            error
                        } else {
                            Error::TrailingData.into()
                        }
                    })?;
                    self.first_member = false;
                    self.crc = Crc32::new();
                    self.length = 0;
                    self.state = GzipState::Body;
                }
                GzipState::Body => {
                    let n = self.inflate.read(buf)?;
                    if n > 0 {
                        self.crc.update(&buf[..n]);
                        self.length = self.length.wrapping_add(n as u32);
                        return Ok(n);
                    }
                    self.read_trailer()?;
                    self.inflate.restart();
                    self.state = GzipState::Header;
                }
            }
        }
    }
}

/// Reads zlib data: the two-byte header, the DEFLATE stream and the Adler-32
/// that ends it. A preset dictionary is refused, as is anything after the
/// checksum.
pub struct ZlibReader<R> {
    inflate: Inflate<R>,
    adler: Adler32,
    started: bool,
    done: bool,
}

impl<R: Read> ZlibReader<R> {
    pub fn new(src: R) -> Self {
        Self {
            inflate: Inflate::new(src),
            adler: Adler32::new(),
            started: false,
            done: false,
        }
    }

    /// Refuses data that decompresses to more than `limit` bytes.
    #[must_use]
    pub fn with_limit(self, limit: u64) -> Self {
        Self {
            inflate: self.inflate.with_limit(limit),
            ..self
        }
    }

    fn read_header(&mut self) -> io::Result<()> {
        let bad = |why| io::Error::from(Error::BadHeader(why));
        let first = need_byte(&mut self.inflate)?;
        let second = need_byte(&mut self.inflate)?;
        if first & 0x0f != 8 {
            return Err(bad("unknown compression method"));
        }
        if first >> 4 > 7 {
            return Err(bad("window larger than 32 KiB"));
        }
        if (u16::from(first) * 256 + u16::from(second)) % 31 != 0 {
            return Err(bad("header check bits are wrong"));
        }
        if second & 0x20 != 0 {
            return Err(bad("a preset dictionary is not supported"));
        }
        Ok(())
    }
}

impl<R: Read> Read for ZlibReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.done {
            return Ok(0);
        }
        if !self.started {
            self.read_header()?;
            self.started = true;
        }
        let n = self.inflate.read(buf)?;
        if n > 0 {
            self.adler.update(&buf[..n]);
            return Ok(n);
        }
        let mut trailer = [0u8; 4];
        need_exact(&mut self.inflate, &mut trailer)?;
        if u32::from_be_bytes(trailer) != self.adler.value() {
            return Err(Error::ChecksumMismatch.into());
        }
        if raw_byte(&mut self.inflate)?.is_some() {
            return Err(Error::TrailingData.into());
        }
        self.done = true;
        Ok(0)
    }
}

fn collect<R: Read>(mut reader: R) -> Result<Vec<u8>, Error> {
    read_all(&mut reader)
}

/// Decodes gzip data held in memory, refusing more than `limit` output bytes.
pub fn gunzip_to_vec(data: &[u8], limit: u64) -> Result<Vec<u8>, Error> {
    collect(GzipReader::new(data).with_limit(limit))
}

/// Decodes zlib data held in memory, refusing more than `limit` output bytes.
pub fn zlib_to_vec(data: &[u8], limit: u64) -> Result<Vec<u8>, Error> {
    collect(ZlibReader::new(data).with_limit(limit))
}
