//! Decoding with the input pushed in as it arrives.
//!
//! The readers elsewhere in this crate pull from a `Read` and cannot do
//! without the next byte: when the input runs dry in the middle of a symbol
//! they fail. A caller that is handed a body piece by piece, and must not
//! park a thread on each answer, has the opposite shape: it holds a piece,
//! asks for what can be decoded from it, and fetches the next piece only when
//! told that this one is used up. [`PushDecoder`] is that, over the same
//! strict decoder: a step that runs out of input is undone and done again
//! once more has been pushed, so splitting the input at any byte, or
//! delivering it one byte at a time, gives the same result as delivering it
//! whole.
//!
//! Pushed bytes live in the decoder's input buffer. The caller bounds the
//! output by the size of its read buffer and by the limit it sets.

use std::io::{self, Read};

use crate::adler32::Adler32;
use crate::crc32::Crc32;
use crate::error::Error;
use crate::inflate::Inflate;

/// What the input is wrapped in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// DEFLATE with nothing around it.
    Raw,
    /// A zlib header, DEFLATE, an Adler-32.
    Zlib,
    /// One gzip member or several.
    Gzip,
    /// Zlib or raw DEFLATE, whichever the first two bytes say. A valid zlib
    /// header cannot begin a raw stream that real encoders write: its low
    /// four bits are 8, which for a DEFLATE block header means a stored
    /// block that is not the last and whose padding bit is set.
    ZlibOrRaw,
}

/// What a call to [`PushDecoder::read`] achieved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// This many bytes were written to the start of the buffer.
    Data(usize),
    /// Everything pushed so far is used. Push more, or call
    /// [`PushDecoder::finish`] when there is none.
    NeedInput,
    /// The stream is complete and nothing follows it.
    End,
}

/// Where in the wrapped stream the decoder is.
enum Stage {
    /// Zlib or raw is not known yet.
    Sniff,
    ZlibHeader {
        bytes: [u8; 2],
        have: usize,
    },
    GzipHeader(Box<GzipHeader>),
    Body,
    ZlibTrailer {
        bytes: [u8; 4],
        have: usize,
    },
    GzipTrailer {
        bytes: [u8; 8],
        have: usize,
    },
    /// After a gzip member: another one, or the end.
    NextMember,
    /// After a stream that has a trailer or none: only the end may follow.
    Tail,
    End,
}

/// A decoder that is given its input in pieces. See the module comment.
pub struct PushDecoder {
    inflate: Inflate<io::Empty>,
    format: Format,
    stage: Stage,
    /// Whether the body is wrapped in zlib (known once `ZlibOrRaw` has looked).
    zlib: bool,
    crc: Crc32,
    adler: Adler32,
    length: u32,
    /// Gzip members completed.
    members: u32,
    failed: Option<Error>,
}

impl PushDecoder {
    pub fn new(format: Format) -> Self {
        let (stage, zlib) = match format {
            Format::Raw => (Stage::Body, false),
            Format::Zlib => (
                Stage::ZlibHeader {
                    bytes: [0; 2],
                    have: 0,
                },
                true,
            ),
            Format::Gzip => (Stage::GzipHeader(Box::default()), false),
            Format::ZlibOrRaw => (Stage::Sniff, false),
        };
        Self {
            inflate: Inflate::pushed(),
            format,
            stage,
            zlib,
            crc: Crc32::new(),
            adler: Adler32::new(),
            length: 0,
            members: 0,
            failed: None,
        }
    }

    /// Refuses input that decodes to more than `limit` bytes in total, with
    /// [`Error::OutputLimit`]. No more than `limit` bytes are ever produced.
    #[must_use]
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.inflate = self.inflate.with_limit(limit);
        self
    }

    /// Adds input. Call it when [`read`](Self::read) has said `NeedInput`:
    /// what is pushed is held until it is used.
    pub fn push(&mut self, data: &[u8]) {
        self.inflate.push(data);
    }

    /// Says that no more input will come. A stream that is not complete then
    /// fails with [`Error::Truncated`].
    pub fn finish(&mut self) {
        self.inflate.finish_input();
    }

    /// The next piece of output, written to the start of `out`, which must
    /// not be empty. Call until it says `NeedInput` or `End`.
    ///
    /// # Errors
    /// Every way the stream can be invalid, and [`Error::Truncated`] when the
    /// input was finished early. The error is repeated by every later call.
    pub fn read(&mut self, out: &mut [u8]) -> Result<Step, Error> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        match self.run(out) {
            Err(error) => {
                self.failed = Some(error);
                Err(error)
            }
            ok => ok,
        }
    }

    fn raw_byte(&mut self) -> Result<Option<Option<u8>>, Error> {
        let mut byte = [0u8; 1];
        match self.inflate.read_raw(&mut byte) {
            Ok(1) => Ok(Some(Some(byte[0]))),
            Ok(_) => Ok(Some(None)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(Error::from_io(&error).unwrap_or(Error::Truncated)),
        }
    }

    fn run(&mut self, out: &mut [u8]) -> Result<Step, Error> {
        if out.is_empty() {
            return Ok(Step::Data(0));
        }
        loop {
            match &mut self.stage {
                Stage::End => return Ok(Step::End),
                Stage::Sniff => {
                    let (have, finished) = self.inflate.pushed_input();
                    if have.len() < 2 && !finished {
                        return Ok(Step::NeedInput);
                    }
                    // A header that looks like zlib's is zlib's, and is refused
                    // as such when something in it is not supported.
                    self.zlib = have.len() >= 2
                        && have[0] & 0x0f == 8
                        && (u16::from(have[0]) * 256 + u16::from(have[1])) % 31 == 0;
                    self.stage = if self.zlib {
                        Stage::ZlibHeader {
                            bytes: [0; 2],
                            have: 0,
                        }
                    } else {
                        Stage::Body
                    };
                }
                Stage::ZlibHeader { .. } => {
                    let Some(byte) = self.raw_byte()? else {
                        return Ok(Step::NeedInput);
                    };
                    let byte = byte.ok_or(Error::Truncated)?;
                    let Stage::ZlibHeader { bytes, have } = &mut self.stage else {
                        unreachable!("the stage was just matched");
                    };
                    bytes[*have] = byte;
                    *have += 1;
                    if *have == 2 {
                        zlib_header(bytes[0], bytes[1])?;
                        self.stage = Stage::Body;
                    }
                }
                Stage::GzipHeader(_) => {
                    let Some(byte) = self.raw_byte()? else {
                        return Ok(Step::NeedInput);
                    };
                    let byte = byte.ok_or(Error::Truncated)?;
                    self.gzip_header_byte(byte)?;
                }
                Stage::Body => match self.inflate.read(out) {
                    Ok(0) => {
                        self.stage = match (self.format, self.zlib) {
                            (Format::Gzip, _) => Stage::GzipTrailer {
                                bytes: [0; 8],
                                have: 0,
                            },
                            (_, true) => Stage::ZlibTrailer {
                                bytes: [0; 4],
                                have: 0,
                            },
                            _ => Stage::Tail,
                        };
                    }
                    Ok(n) => {
                        if self.format == Format::Gzip {
                            self.crc.update(&out[..n]);
                            self.length = self.length.wrapping_add(n as u32);
                        } else if self.zlib {
                            self.adler.update(&out[..n]);
                        }
                        return Ok(Step::Data(n));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(Step::NeedInput);
                    }
                    Err(error) => return Err(Error::from_io(&error).unwrap_or(Error::Truncated)),
                },
                Stage::ZlibTrailer { .. } | Stage::GzipTrailer { .. } => {
                    let Some(byte) = self.raw_byte()? else {
                        return Ok(Step::NeedInput);
                    };
                    let byte = byte.ok_or(Error::Truncated)?;
                    self.trailer_byte(byte)?;
                }
                Stage::NextMember => {
                    let Some(byte) = self.raw_byte()? else {
                        return Ok(Step::NeedInput);
                    };
                    match byte {
                        None => self.stage = Stage::End,
                        Some(byte) => {
                            self.stage = Stage::GzipHeader(Box::default());
                            self.gzip_header_byte(byte)?;
                        }
                    }
                }
                Stage::Tail => {
                    let Some(byte) = self.raw_byte()? else {
                        return Ok(Step::NeedInput);
                    };
                    match byte {
                        None => self.stage = Stage::End,
                        Some(_) => return Err(Error::TrailingData),
                    }
                }
            }
        }
    }

    fn gzip_header_byte(&mut self, byte: u8) -> Result<(), Error> {
        let Stage::GzipHeader(header) = &mut self.stage else {
            unreachable!("the stage is the gzip header");
        };
        match header.feed(byte) {
            Ok(false) => Ok(()),
            Ok(true) => {
                self.crc = Crc32::new();
                self.length = 0;
                self.stage = Stage::Body;
                Ok(())
            }
            // After the first member, bytes that do not start another one
            // are junk behind the data, not a broken header.
            Err(_) if self.members > 0 => Err(Error::TrailingData),
            Err(error) => Err(error),
        }
    }

    fn trailer_byte(&mut self, byte: u8) -> Result<(), Error> {
        match &mut self.stage {
            Stage::ZlibTrailer { bytes, have } => {
                bytes[*have] = byte;
                *have += 1;
                if *have == 4 {
                    if u32::from_be_bytes(*bytes) != self.adler.value() {
                        return Err(Error::ChecksumMismatch);
                    }
                    self.stage = Stage::Tail;
                }
            }
            Stage::GzipTrailer { bytes, have } => {
                bytes[*have] = byte;
                *have += 1;
                if *have == 8 {
                    let crc = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    let length = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    if crc != self.crc.value() {
                        return Err(Error::ChecksumMismatch);
                    }
                    if length != self.length {
                        return Err(Error::LengthMismatch);
                    }
                    self.members += 1;
                    self.inflate.restart();
                    self.stage = Stage::NextMember;
                }
            }
            _ => unreachable!("the stage is a trailer"),
        }
        Ok(())
    }
}

fn zlib_header(first: u8, second: u8) -> Result<(), Error> {
    let bad = Error::BadHeader;
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

#[derive(Default)]
enum Phase {
    #[default]
    Fixed,
    ExtraLength,
    Extra,
    Name,
    Comment,
    HeaderCheck,
}

/// A gzip header, read a byte at a time. The extra field, the name and the
/// comment are skipped as they come, so a header of any length costs nothing.
#[derive(Default)]
struct GzipHeader {
    phase: Phase,
    fixed: [u8; 10],
    have: usize,
    flags: u8,
    crc: Crc32,
    /// Bytes of the extra field still to skip, or of the length before it.
    extra_length: [u8; 2],
    extra_left: usize,
    check: [u8; 2],
}

impl GzipHeader {
    /// Takes one byte; true when the header is complete.
    fn feed(&mut self, byte: u8) -> Result<bool, Error> {
        let bad = Error::BadHeader;
        match self.phase {
            Phase::Fixed => {
                match self.have {
                    0 if byte != 0x1f => return Err(bad("not a gzip stream")),
                    1 if byte != 0x8b => return Err(bad("not a gzip stream")),
                    2 if byte != 8 => return Err(bad("unknown compression method")),
                    3 if byte & 0xe0 != 0 => return Err(bad("reserved flags are set")),
                    _ => {}
                }
                self.fixed[self.have] = byte;
                self.have += 1;
                if self.have < 10 {
                    return Ok(false);
                }
                self.flags = self.fixed[3];
                self.crc.update(&self.fixed);
                Ok(self.enter(Phase::Fixed))
            }
            Phase::ExtraLength => {
                self.crc.update(&[byte]);
                self.extra_length[self.have] = byte;
                self.have += 1;
                if self.have == 2 {
                    self.extra_left = usize::from(u16::from_le_bytes(self.extra_length));
                    self.have = 0;
                    self.phase = Phase::Extra;
                    if self.extra_left == 0 {
                        return Ok(self.enter(Phase::Extra));
                    }
                }
                Ok(false)
            }
            Phase::Extra => {
                self.crc.update(&[byte]);
                self.extra_left -= 1;
                Ok(self.extra_left == 0 && self.enter(Phase::Extra))
            }
            Phase::Name | Phase::Comment => {
                self.crc.update(&[byte]);
                if byte != 0 {
                    return Ok(false);
                }
                let after = std::mem::take(&mut self.phase);
                Ok(self.enter(after))
            }
            Phase::HeaderCheck => {
                self.check[self.have] = byte;
                self.have += 1;
                if self.have < 2 {
                    return Ok(false);
                }
                if u16::from_le_bytes(self.check) != (self.crc.value() & 0xffff) as u16 {
                    return Err(bad("header checksum mismatch"));
                }
                Ok(true)
            }
        }
    }

    /// Moves on to the field that follows `after`, in the order the format
    /// gives them. True when there is none.
    fn enter(&mut self, after: Phase) -> bool {
        self.have = 0;
        let order = [
            (4u8, Phase::ExtraLength),
            (8, Phase::Name),
            (16, Phase::Comment),
            (2, Phase::HeaderCheck),
        ];
        let done = match after {
            Phase::Fixed => 0,
            Phase::ExtraLength | Phase::Extra => 1,
            Phase::Name => 2,
            Phase::Comment => 3,
            Phase::HeaderCheck => 4,
        };
        for (flag, phase) in order.into_iter().skip(done) {
            if self.flags & flag != 0 {
                self.phase = phase;
                return false;
            }
        }
        true
    }
}
