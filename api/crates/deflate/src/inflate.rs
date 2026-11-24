//! Streaming DEFLATE decoding (RFC 1951).
//!
//! The decoder pulls from any `Read` and is itself a `Read`. It keeps one
//! window of recent output and never holds more than that, so the size of the
//! result does not matter to memory. Every invalid stream is an [`Error`]:
//! nothing in here indexes by an unchecked input value, and every loop either
//! consumes input or ends.

use std::io::{self, Read};

use crate::error::Error;
use crate::tables::{
    CODE_LENGTH_ORDER, DIST_BASE, DIST_EXTRA, LEN_BASE, LEN_EXTRA, MAX_MATCH, WINDOW,
};

/// Window of decoded output: the last `WINDOW` bytes a match may reach, and
/// room to decode ahead of what the caller has taken.
const CAP: usize = 1 << 17;
const INPUT_BUFFER: usize = 32 * 1024;
const FAST_BITS: u32 = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Literal,
    Distance,
    CodeLength,
}

/// A canonical Huffman decoding table: one lookup for codes up to
/// `FAST_BITS` long, and a bit-by-bit walk for the rare longer ones.
struct Huffman {
    /// `symbol << 4 | length`, or 0 when the code is longer or unassigned.
    fast: [u16; 1 << FAST_BITS],
    count: [u16; 16],
    symbols: [u16; 288],
}

impl Huffman {
    fn new() -> Box<Self> {
        Box::new(Self {
            fast: [0; 1 << FAST_BITS],
            count: [0; 16],
            symbols: [0; 288],
        })
    }

    /// Builds the table for `lengths`, refusing every set of lengths that is
    /// not a prefix code. The one incomplete code the format allows is a
    /// lone code of length one (a single distance), and no code at all is
    /// allowed for distances.
    fn build(&mut self, lengths: &[u8], kind: Kind) -> Result<(), Error> {
        debug_assert!(lengths.len() <= 288);
        self.count = [0; 16];
        for &length in lengths {
            self.count[usize::from(length)] += 1;
        }
        let codes = lengths.len() - usize::from(self.count[0]);
        self.count[0] = 0;
        self.fast.fill(0);
        if codes == 0 {
            return if kind == Kind::Distance {
                Ok(())
            } else {
                Err(Error::BadCodeLengths)
            };
        }

        let mut left: i32 = 1;
        for length in 1..16 {
            left <<= 1;
            left -= i32::from(self.count[length]);
            if left < 0 {
                return Err(Error::OversubscribedCode);
            }
        }
        if left > 0 {
            let longest = (1..16).rev().find(|&l| self.count[l] > 0).unwrap_or(0);
            if kind == Kind::CodeLength || codes != 1 || longest != 1 {
                return Err(Error::IncompleteCode);
            }
        }

        let mut offsets = [0u16; 16];
        for length in 1..15 {
            offsets[length + 1] = offsets[length] + self.count[length];
        }
        let mut next_code = [0u32; 16];
        let mut code = 0u32;
        for (length, slot) in next_code.iter_mut().enumerate().skip(1) {
            code = (code + u32::from(self.count[length - 1])) << 1;
            *slot = code;
        }
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let length = usize::from(length);
            self.symbols[usize::from(offsets[length])] = symbol as u16;
            offsets[length] += 1;
            let code = next_code[length];
            next_code[length] += 1;
            if length as u32 <= FAST_BITS {
                let reversed = (code as u16).reverse_bits() >> (16 - length);
                let entry = ((symbol as u16) << 4) | length as u16;
                let mut slot = usize::from(reversed);
                while slot < self.fast.len() {
                    self.fast[slot] = entry;
                    slot += 1 << length;
                }
            }
        }
        Ok(())
    }
}

enum State {
    BlockHeader,
    Stored(u32),
    Codes,
    Done,
}

/// What can stop a decoding step: bad data, or the input failing.
enum Fail {
    Data(Error),
    Io(io::Error),
}

impl From<Error> for Fail {
    fn from(error: Error) -> Self {
        Self::Data(error)
    }
}

impl From<io::Error> for Fail {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A raw DEFLATE stream decoder over `R`.
pub struct Inflate<R> {
    src: R,
    input: Box<[u8]>,
    in_pos: usize,
    in_len: usize,
    bits: u64,
    bit_count: u32,
    window: Vec<u8>,
    /// What the caller has taken, and what has been decoded.
    taken: usize,
    filled: usize,
    state: State,
    last_block: bool,
    literals: Box<Huffman>,
    distances: Box<Huffman>,
    produced: u64,
    limit: u64,
    failed: Option<Error>,
    /// The input failed once: what was half decoded cannot be resumed.
    io_failed: Option<io::ErrorKind>,
    /// No block of the current stream has been started.
    fresh: bool,
    /// Push mode: the input may say "not yet" (`WouldBlock`) and a step that
    /// ran out of input is undone and retried when more arrives.
    resumable: bool,
    /// Where the step in progress started, so that a retry finds its input.
    snap_pos: usize,
    /// The last decoding pass stopped for want of input.
    starved: bool,
}

impl<R: Read> Inflate<R> {
    pub fn new(src: R) -> Self {
        Self {
            src,
            input: vec![0; INPUT_BUFFER].into_boxed_slice(),
            in_pos: 0,
            in_len: 0,
            bits: 0,
            bit_count: 0,
            window: vec![0; CAP],
            taken: 0,
            filled: 0,
            state: State::BlockHeader,
            last_block: false,
            literals: Huffman::new(),
            distances: Huffman::new(),
            produced: 0,
            limit: u64::MAX,
            failed: None,
            io_failed: None,
            fresh: true,
            resumable: false,
            snap_pos: 0,
            starved: false,
        }
    }

    /// Push mode, for an input that can say it has nothing yet by failing
    /// with `WouldBlock` and nothing more by returning 0. A read that runs
    /// out of input then fails with `WouldBlock` too, and nothing is lost:
    /// the step that was cut short is undone and done again when the input
    /// has grown.
    pub(crate) fn resumable(mut self) -> Self {
        self.resumable = true;
        self
    }

    /// Refuses a stream that would decode to more than `limit` bytes, in
    /// total over this decoder's life, with [`Error::OutputLimit`]. Output up
    /// to the limit is still delivered first, and no more than `limit` bytes
    /// are ever produced.
    #[must_use]
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }

    /// Bytes decoded so far.
    pub fn total_out(&self) -> u64 {
        self.produced - (self.filled - self.taken) as u64
    }

    /// Whether the final block has been read and all its output taken.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done) && self.taken == self.filled
    }

    pub fn get_ref(&self) -> &R {
        &self.src
    }

    pub(crate) fn src_mut(&mut self) -> &mut R {
        &mut self.src
    }

    pub fn into_inner(self) -> R {
        self.src
    }

    /// Before the stream starts or after it has ended: reads the bytes around
    /// it (a header, a trailer, the next gzip member) from where the stream
    /// stopped, including those the decoder had already buffered. Returns 0 at
    /// the end of the input.
    pub fn read_raw(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !self.is_finished() && !self.fresh {
            return Err(io::Error::other("the compressed stream is in progress"));
        }
        self.snap_pos = self.in_pos;
        match self.take_raw(out) {
            Ok(n) => Ok(n),
            Err(Fail::Io(error)) => Err(error),
            Err(Fail::Data(error)) => Err(error.into()),
        }
    }

    /// After the stream has ended: starts a new, independent stream right
    /// after it. Returns false when the current one has not ended.
    pub fn restart(&mut self) -> bool {
        if !self.is_finished() {
            return false;
        }
        self.taken = 0;
        self.filled = 0;
        self.state = State::BlockHeader;
        self.last_block = false;
        self.fresh = true;
        true
    }

    fn fill_input(&mut self) -> io::Result<bool> {
        if self.resumable {
            return self.fill_input_kept();
        }
        loop {
            match self.src.read(&mut self.input) {
                Ok(n) => {
                    self.in_pos = 0;
                    self.in_len = n;
                    return Ok(n > 0);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(error) => return Err(error),
            }
        }
    }

    /// Reads more input after what the step in progress has already used,
    /// which stays where it is: a step that runs dry is undone and needs it
    /// again. Fails with `WouldBlock` when there is nothing yet.
    fn fill_input_kept(&mut self) -> io::Result<bool> {
        let keep = self.snap_pos.min(self.in_pos);
        if keep > 0 {
            self.input.copy_within(keep..self.in_len, 0);
            self.in_len -= keep;
            self.in_pos -= keep;
        }
        self.snap_pos = 0;
        loop {
            if self.in_len == self.input.len() {
                return Err(io::Error::other("one step needs more input than is held"));
            }
            match self.src.read(&mut self.input[self.in_len..]) {
                Ok(n) => {
                    self.in_len += n;
                    return Ok(n > 0);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(error) => return Err(error),
            }
        }
    }

    /// Tops the bit buffer up to at least 56 bits, or as far as the input
    /// goes.
    fn refill(&mut self) -> io::Result<()> {
        loop {
            if self.in_len - self.in_pos >= 8 {
                let mut word = [0u8; 8];
                word.copy_from_slice(&self.input[self.in_pos..self.in_pos + 8]);
                let take = (63 - self.bit_count) / 8;
                self.bits |= u64::from_le_bytes(word) << self.bit_count;
                self.in_pos += take as usize;
                self.bit_count += take * 8;
                self.bits &= (1u64 << self.bit_count) - 1;
                return Ok(());
            }
            if self.bit_count > 56 {
                return Ok(());
            }
            if self.in_pos == self.in_len {
                if self.bit_count >= 32 || !self.fill_input()? {
                    return Ok(());
                }
                continue;
            }
            self.bits |= u64::from(self.input[self.in_pos]) << self.bit_count;
            self.in_pos += 1;
            self.bit_count += 8;
        }
    }

    fn get_bits(&mut self, n: u32) -> Result<u32, Fail> {
        if self.bit_count < n {
            self.refill()?;
            if self.bit_count < n {
                return Err(Error::Truncated.into());
            }
        }
        let value = (self.bits & ((1u64 << n) - 1)) as u32;
        self.bits >>= n;
        self.bit_count -= n;
        Ok(value)
    }

    fn decode_symbol(&mut self, kind: Kind) -> Result<usize, Fail> {
        if self.bit_count < 15 {
            self.refill()?;
        }
        let table = match kind {
            Kind::Distance => &*self.distances,
            _ => &*self.literals,
        };
        let entry = table.fast[(self.bits & ((1 << FAST_BITS) - 1)) as usize];
        if entry != 0 {
            let length = u32::from(entry & 15);
            if length > self.bit_count {
                return Err(Error::Truncated.into());
            }
            self.bits >>= length;
            self.bit_count -= length;
            return Ok(usize::from(entry >> 4));
        }
        self.decode_slow(kind)
    }

    /// The canonical walk, one bit at a time, for codes the fast table does
    /// not hold. It is also where an unassigned pattern is found out.
    fn decode_slow(&mut self, kind: Kind) -> Result<usize, Fail> {
        let table = match kind {
            Kind::Distance => &*self.distances,
            _ => &*self.literals,
        };
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        let mut bits = self.bits;
        for length in 1..16usize {
            if self.bit_count < length as u32 {
                return Err(Error::Truncated.into());
            }
            code |= (bits & 1) as i32;
            bits >>= 1;
            let count = i32::from(table.count[length]);
            if code - count < first {
                let symbol = table.symbols[(index + (code - first)) as usize];
                self.bits >>= length;
                self.bit_count -= length as u32;
                return Ok(usize::from(symbol));
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::InvalidSymbol.into())
    }

    fn drop_partial_byte(&mut self) {
        let partial = self.bit_count % 8;
        self.bits >>= partial;
        self.bit_count -= partial;
    }

    /// Reads up to `out.len()` bytes that sit on a byte boundary: whole bytes
    /// of the bit buffer first, then the input.
    fn take_raw(&mut self, out: &mut [u8]) -> Result<usize, Fail> {
        if out.is_empty() {
            return Ok(0);
        }
        debug_assert_eq!(self.bit_count % 8, 0);
        if self.bit_count >= 8 {
            let mut n = 0;
            while n < out.len() && self.bit_count >= 8 {
                out[n] = self.bits as u8;
                self.bits >>= 8;
                self.bit_count -= 8;
                n += 1;
            }
            return Ok(n);
        }
        if self.in_pos == self.in_len && !self.fill_input()? {
            return Ok(0);
        }
        let n = out.len().min(self.in_len - self.in_pos);
        out[..n].copy_from_slice(&self.input[self.in_pos..self.in_pos + n]);
        self.in_pos += n;
        Ok(n)
    }

    fn read_block_header(&mut self) -> Result<(), Fail> {
        self.fresh = false;
        self.last_block = self.get_bits(1)? == 1;
        match self.get_bits(2)? {
            0 => {
                self.drop_partial_byte();
                let length = self.get_bits(16)?;
                let complement = self.get_bits(16)?;
                if length != !complement & 0xffff {
                    return Err(Error::StoredLengthMismatch.into());
                }
                self.state = State::Stored(length);
            }
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                self.literals.build(&lengths, Kind::Literal)?;
                // Thirty-two five-bit codes, two of which no block may use.
                self.distances.build(&[5u8; 32], Kind::Distance)?;
                self.state = State::Codes;
            }
            2 => {
                self.read_dynamic_tables()?;
                self.state = State::Codes;
            }
            _ => return Err(Error::InvalidBlockType.into()),
        }
        Ok(())
    }

    fn read_dynamic_tables(&mut self) -> Result<(), Fail> {
        let literal_count = self.get_bits(5)? as usize + 257;
        let distance_count = self.get_bits(5)? as usize + 1;
        let code_length_count = self.get_bits(4)? as usize + 4;
        if literal_count > 286 || distance_count > 30 {
            return Err(Error::TooManySymbols.into());
        }

        let mut code_lengths = [0u8; 19];
        for &slot in &CODE_LENGTH_ORDER[..code_length_count] {
            code_lengths[slot] = self.get_bits(3)? as u8;
        }
        // The code-length code is decoded through the literal table's slot,
        // which is rebuilt right after.
        self.literals.build(&code_lengths, Kind::CodeLength)?;

        let total = literal_count + distance_count;
        let mut lengths = [0u8; 286 + 30];
        let mut at = 0;
        while at < total {
            let symbol = self.decode_symbol(Kind::Literal)?;
            let (value, repeat) = match symbol {
                0..=15 => (symbol as u8, 1),
                16 => {
                    if at == 0 {
                        return Err(Error::BadCodeLengths.into());
                    }
                    (lengths[at - 1], 3 + self.get_bits(2)? as usize)
                }
                17 => (0, 3 + self.get_bits(3)? as usize),
                _ => (0, 11 + self.get_bits(7)? as usize),
            };
            if at + repeat > total {
                return Err(Error::BadCodeLengths.into());
            }
            lengths[at..at + repeat].fill(value);
            at += repeat;
        }
        if lengths[256] == 0 {
            return Err(Error::BadCodeLengths.into());
        }
        self.literals
            .build(&lengths[..literal_count], Kind::Literal)?;
        self.distances
            .build(&lengths[literal_count..total], Kind::Distance)?;
        Ok(())
    }

    fn finish_block(&mut self) {
        if self.last_block {
            self.drop_partial_byte();
            self.state = State::Done;
        } else {
            self.state = State::BlockHeader;
        }
    }

    /// Decodes until the window is full, the stream ends, or the input has
    /// run dry after some output (so a slow input never holds decoded bytes
    /// back).
    fn decode(&mut self) -> Result<(), Fail> {
        if self.filled + MAX_MATCH > CAP {
            let keep = self.filled.min(WINDOW);
            self.window.copy_within(self.filled - keep..self.filled, 0);
            self.filled = keep;
            self.taken = keep;
        }
        self.starved = false;
        let start = self.filled;
        let end = CAP - MAX_MATCH;
        loop {
            if self.filled > end {
                return Ok(());
            }
            // A step reads all it needs before it changes anything but the
            // bit buffer, so these three are all that a retry needs back.
            let (bits, bit_count, in_pos) = (self.bits, self.bit_count, self.in_pos);
            if self.resumable {
                self.snap_pos = in_pos;
            }
            match self.step(start) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(Fail::Io(error))
                    if self.resumable && error.kind() == io::ErrorKind::WouldBlock =>
                {
                    self.bits = bits;
                    self.bit_count = bit_count;
                    // The retained input starts at the step's first byte.
                    self.in_pos = self.snap_pos;
                    self.starved = true;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// One block header, one run of stored bytes or one symbol. True when
    /// decoding should stop for now.
    fn step(&mut self, start: usize) -> Result<bool, Fail> {
        match self.state {
            State::Done => return Ok(true),
            State::BlockHeader => {
                if self.filled > start && self.in_pos == self.in_len && self.bit_count < 32 {
                    return Ok(true);
                }
                self.read_block_header()?;
            }
            State::Stored(0) => self.finish_block(),
            State::Stored(remaining) => {
                if self.filled > start && self.in_pos == self.in_len && self.bit_count < 8 {
                    return Ok(true);
                }
                let allowed = self.limit - self.produced;
                if allowed == 0 {
                    return Err(Error::OutputLimit.into());
                }
                let n = (remaining as usize)
                    .min(CAP - self.filled)
                    .min(allowed.min(CAP as u64) as usize);
                let mut chunk = std::mem::take(&mut self.window);
                let got = self.take_raw(&mut chunk[self.filled..self.filled + n]);
                self.window = chunk;
                let got = got?;
                if got == 0 {
                    return Err(Error::Truncated.into());
                }
                self.filled += got;
                self.produced += got as u64;
                self.state = State::Stored(remaining - got as u32);
            }
            State::Codes => {
                if self.filled > start && self.in_pos == self.in_len && self.bit_count < 48 {
                    return Ok(true);
                }
                if self.bit_count < 48 {
                    self.refill()?;
                }
                self.decode_symbol_run()?;
            }
        }
        Ok(false)
    }

    /// One literal, one match, or the end of the block.
    fn decode_symbol_run(&mut self) -> Result<(), Fail> {
        let symbol = self.decode_symbol(Kind::Literal)?;
        if symbol < 256 {
            if self.produced >= self.limit {
                return Err(Error::OutputLimit.into());
            }
            self.window[self.filled] = symbol as u8;
            self.filled += 1;
            self.produced += 1;
            return Ok(());
        }
        if symbol == 256 {
            self.finish_block();
            return Ok(());
        }
        let index = symbol - 257;
        if index >= LEN_BASE.len() {
            return Err(Error::InvalidSymbol.into());
        }
        let length =
            usize::from(LEN_BASE[index]) + self.get_bits(u32::from(LEN_EXTRA[index]))? as usize;
        let distance_symbol = self.decode_symbol(Kind::Distance)?;
        if distance_symbol >= DIST_BASE.len() {
            return Err(Error::InvalidSymbol.into());
        }
        let distance = usize::from(DIST_BASE[distance_symbol])
            + self.get_bits(u32::from(DIST_EXTRA[distance_symbol]))? as usize;
        if distance > self.filled {
            return Err(Error::DistanceTooFar.into());
        }
        // A match that crosses the cap still delivers the bytes up to it.
        let allowed = self.limit - self.produced;
        let over = length as u64 > allowed;
        let length = if over { allowed as usize } else { length };
        let from = self.filled - distance;
        if distance >= length {
            self.window.copy_within(from..from + length, self.filled);
        } else {
            for i in 0..length {
                self.window[self.filled + i] = self.window[from + i];
            }
        }
        self.filled += length;
        self.produced += length as u64;
        if over {
            return Err(Error::OutputLimit.into());
        }
        Ok(())
    }

    fn fill_window(&mut self) -> io::Result<()> {
        if let Some(error) = self.failed {
            return Err(error.into());
        }
        if let Some(kind) = self.io_failed {
            return Err(io::Error::new(
                kind,
                "an earlier read of the compressed input failed",
            ));
        }
        match self.decode() {
            Ok(()) => Ok(()),
            Err(Fail::Io(error)) => {
                // A symbol may be half taken: there is no going on from here.
                self.io_failed = Some(error.kind());
                Err(error)
            }
            Err(Fail::Data(error)) => {
                self.failed = Some(error);
                Err(error.into())
            }
        }
    }
}

impl<R: Read> Read for Inflate<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.taken < self.filled {
                let n = buf.len().min(self.filled - self.taken);
                buf[..n].copy_from_slice(&self.window[self.taken..self.taken + n]);
                self.taken += n;
                return Ok(n);
            }
            if matches!(self.state, State::Done) {
                return Ok(0);
            }
            if let Err(error) = self.fill_window() {
                // What was decoded before the failure is still good; the
                // failure is reported once it has been taken.
                if self.taken == self.filled {
                    return Err(error);
                }
            } else if self.starved && self.taken == self.filled {
                return Err(io::ErrorKind::WouldBlock.into());
            }
        }
    }
}

/// Decodes a whole raw DEFLATE stream held in memory, refusing one that
/// decodes to more than `limit` bytes.
pub fn inflate_to_vec(data: &[u8], limit: u64) -> Result<Vec<u8>, Error> {
    let mut decoder = Inflate::new(data).with_limit(limit);
    let out = read_all(&mut decoder)?;
    // Anything after the final block is not part of the stream.
    let mut extra = [0u8; 1];
    match decoder.read_raw(&mut extra) {
        Ok(0) => Ok(out),
        Ok(_) => Err(Error::TrailingData),
        Err(error) => Err(Error::from_io(&error).unwrap_or(Error::Truncated)),
    }
}

pub(crate) fn read_all<R: Read>(reader: &mut R) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|error| Error::from_io(&error).unwrap_or(Error::Truncated))?;
    Ok(out)
}
