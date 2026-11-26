//! Streaming DEFLATE decoding (RFC 1951).
//!
//! The decoder pulls from any `Read` and is itself a `Read`. It keeps one
//! window of recent output and never holds more than that, so the size of the
//! result does not matter to memory. Every invalid stream is an [`Error`]:
//! nothing in here indexes by an unchecked input value, and every loop either
//! consumes input or ends.

use std::io::{self, Read};
use std::sync::LazyLock;

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

/// A code uses at most four width bits. Keeping the next two bits zero
/// lets a wrapping u64 shift consume the code using the packed entry directly.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Code(u16);

impl Code {
    const MISSING: Self = Self(u16::MAX);

    fn new(symbol: usize, width: usize) -> Self {
        Self(((symbol as u16) << 6) | width as u16)
    }

    fn width(self) -> u32 {
        u32::from(self.0 & 15)
    }

    fn symbol(self) -> usize {
        usize::from(self.0 >> 6)
    }

    fn is_literal(self) -> bool {
        self.0 < (256 << 6)
    }

    fn skip(self, bits: u64) -> u64 {
        bits.wrapping_shr(u32::from(self.0))
    }
}

/// A canonical Huffman decoding table: one lookup for codes up to
/// `FAST_BITS` long, and a bit-by-bit walk for the rare longer ones.
#[derive(Clone)]
struct Huffman {
    fast: [Code; 1 << FAST_BITS],
    mask: usize,
    count: [u16; 16],
    symbols: [u16; 288],
}

impl Huffman {
    fn new() -> Box<Self> {
        Box::new(Self {
            fast: [Code::MISSING; 1 << FAST_BITS],
            mask: 0,
            count: [0; 16],
            symbols: [0; 288],
        })
    }

    fn lookup(&self, bits: u64) -> Code {
        self.fast[bits as usize & self.mask]
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
        let longest = self.count.iter().rposition(|&n| n != 0).unwrap_or(0);
        self.mask = (1 << longest.min(FAST_BITS as usize)) - 1;
        self.fast[..=self.mask].fill(Code::MISSING);
        if codes == 0 {
            return if kind == Kind::Distance {
                Ok(())
            } else {
                Err(Error::BadCodeLengths)
            };
        }

        let mut left: i32 = 1;
        for &count in &self.count[1..16] {
            left <<= 1;
            left -= i32::from(count);
            if left < 0 {
                return Err(Error::OversubscribedCode);
            }
        }
        if left > 0 && (kind == Kind::CodeLength || codes != 1 || longest != 1) {
            return Err(Error::IncompleteCode);
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
                let entry = Code::new(symbol, length);
                let mut slot = usize::from(reversed);
                while slot <= self.mask {
                    self.fast[slot] = entry;
                    slot += 1 << length;
                }
            }
        }
        Ok(())
    }
}

// RFC 1951 assigns the same trees to every fixed block.
static FIXED: LazyLock<(Box<Huffman>, Box<Huffman>)> = LazyLock::new(|| {
    let mut lengths = [8; 288];
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    let mut literals = Huffman::new();
    literals
        .build(&lengths, Kind::Literal)
        .expect("fixed literal codes");
    let mut distances = Huffman::new();
    distances
        .build(&[5; 32], Kind::Distance)
        .expect("fixed distance codes");
    (literals, distances)
});

/// Appends four bytes when the accumulator has room for a full word.
/// The buffered loop reserves input for both refills before starting a token.
#[inline(always)]
fn refill_word(input: &[u8], pos: &mut usize, bits: &mut u64, count: &mut u32) {
    if *count < 32 {
        let word = u32::from_le_bytes(input[*pos..*pos + 4].try_into().unwrap());
        *bits |= u64::from(word) << *count;
        *count += 32;
        *pos += 4;
    }
}

#[inline(always)]
fn copy_match(window: &mut [u8], filled: usize, distance: usize, length: usize) {
    let from = filled - distance;
    if distance >= length {
        if length <= 16 {
            // The decode loop reserves MAX_MATCH bytes here. Copy a fixed
            // width and commit only the match length to the output history.
            window.copy_within(from..from + 16, filled);
        } else {
            window.copy_within(from..from + length, filled);
        }
    } else if distance == 1 {
        let byte = window[from];
        window[filled..filled + length].fill(byte);
    } else {
        // The history is periodic. Each copy doubles the prefix that can
        // be used by the next, including for matches longer than a period.
        let mut copied = 0;
        while copied < length {
            let n = (distance + copied).min(length - copied);
            window.copy_within(from..from + n, filled + copied);
            copied += n;
        }
    }
}

enum State {
    BlockHeader,
    DynamicLengths {
        literals: usize,
        total: usize,
        at: usize,
    },
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
    input: Vec<u8>,
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
    fixed_tables: bool,
    /// Literal/length and distance code lengths under construction. Only
    /// the prefix committed in DynamicLengths is read.
    lengths: [u8; 286 + 30],
    produced: u64,
    limit: u64,
    failed: Option<Error>,
    /// The input failed once: what was half decoded cannot be resumed.
    io_failed: Option<io::ErrorKind>,
    /// No block of the current stream has been started.
    fresh: bool,
    supply: Supply,
    /// The last decoding pass stopped for want of input.
    starved: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Supply {
    Reader,
    Pushing,
    Finished,
}

impl Inflate<io::Empty> {
    pub(crate) fn pushed() -> Self {
        Self::with_input(io::empty(), Vec::new(), Supply::Pushing)
    }

    pub(crate) fn push(&mut self, data: &[u8]) {
        // Append a lone byte directly to the bit buffer when it is next in order.
        if let [byte] = data
            && self.in_pos == self.in_len
            && self.bit_count < 56
            && matches!(self.state, State::Codes | State::Stored(_))
        {
            self.bits |= u64::from(*byte) << self.bit_count;
            self.bit_count += 8;
            return;
        }
        let unread = self.in_len - self.in_pos;
        // Retain a partial symbol. Only move a suffix when we can reclaim
        // at least as much space, even if a caller pushes ahead of demand.
        if self.in_pos != 0 && self.in_pos >= unread {
            self.input.copy_within(self.in_pos..self.in_len, 0);
            self.input.truncate(unread);
            self.in_pos = 0;
        }
        self.input.extend_from_slice(data);
        self.in_len = self.input.len();
    }

    pub(crate) fn finish_input(&mut self) {
        self.supply = Supply::Finished;
    }

    #[inline]
    pub(crate) fn needs_body_input(&self) -> bool {
        self.in_pos == self.in_len
            && self.taken == self.filled
            && self.supply == Supply::Pushing
            && self.failed.is_none()
            && match self.state {
                State::Stored(n) => n != 0 && self.bit_count == 0 && self.produced < self.limit,
                State::Codes => self.bit_count < 32,
                _ => false,
            }
    }

    pub(crate) fn pushed_input(&self) -> (&[u8], bool) {
        (
            &self.input[self.in_pos..self.in_len],
            self.supply == Supply::Finished,
        )
    }
}

impl<R: Read> Inflate<R> {
    pub fn new(src: R) -> Self {
        Self::with_input(src, vec![0; INPUT_BUFFER], Supply::Reader)
    }

    fn with_input(src: R, input: Vec<u8>, supply: Supply) -> Self {
        Self {
            src,
            input,
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
            fixed_tables: false,
            lengths: [0; 286 + 30],
            produced: 0,
            limit: u64::MAX,
            failed: None,
            io_failed: None,
            fresh: true,
            supply,
            starved: false,
        }
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

    /// Whether the final block has been read and all its output taken.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done) && self.taken == self.filled
    }

    pub fn get_ref(&self) -> &R {
        &self.src
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
        match self.supply {
            Supply::Pushing => return Err(io::ErrorKind::WouldBlock.into()),
            Supply::Finished => return Ok(false),
            Supply::Reader => {}
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

    #[inline(always)]
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

    #[inline(always)]
    fn decode_symbol(&mut self, kind: Kind) -> Result<usize, Fail> {
        if self.bit_count < 15 {
            self.refill()?;
        }
        let table = match kind {
            Kind::Distance => &*self.distances,
            _ => &*self.literals,
        };
        let entry = table.lookup(self.bits);
        if entry != Code::MISSING {
            let length = entry.width();
            if length > self.bit_count {
                return Err(Error::Truncated.into());
            }
            self.bits = entry.skip(self.bits);
            self.bit_count -= length;
            return Ok(entry.symbol());
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
                if !self.fixed_tables {
                    self.literals.clone_from(&FIXED.0);
                    self.distances.clone_from(&FIXED.1);
                    self.fixed_tables = true;
                }
                self.state = State::Codes;
            }
            2 => {
                self.start_dynamic_tables()?;
            }
            _ => return Err(Error::InvalidBlockType.into()),
        }
        Ok(())
    }

    fn start_dynamic_tables(&mut self) -> Result<(), Fail> {
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
        self.fixed_tables = false;

        self.state = State::DynamicLengths {
            literals: literal_count,
            total: literal_count + distance_count,
            at: 0,
        };
        Ok(())
    }

    /// Commit a complete run before asking for another symbol. A short input
    /// can split the repeat's extra bits, but never replays earlier lengths.
    fn read_length_run(&mut self, literals: usize, total: usize, at: usize) -> Result<(), Fail> {
        let symbol = self.decode_symbol(Kind::CodeLength)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 if at == 0 => return Err(Error::BadCodeLengths.into()),
            16 => (self.lengths[at - 1], 3 + self.get_bits(2)? as usize),
            17 => (0, 3 + self.get_bits(3)? as usize),
            _ => (0, 11 + self.get_bits(7)? as usize),
        };
        let end = at + repeat;
        if end > total {
            return Err(Error::BadCodeLengths.into());
        }
        self.lengths[at..end].fill(value);
        if end < total {
            self.state = State::DynamicLengths {
                literals,
                total,
                at: end,
            };
        } else {
            if self.lengths[256] == 0 {
                return Err(Error::BadCodeLengths.into());
            }
            self.literals
                .build(&self.lengths[..literals], Kind::Literal)?;
            self.distances
                .build(&self.lengths[literals..total], Kind::Distance)?;
            self.state = State::Codes;
        }
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
            if matches!(self.state, State::Codes) && self.in_len - self.in_pos >= 8 {
                self.decode_buffered_codes();
                if self.filled > end {
                    return Ok(());
                }
            }
            // Pushed input stays in place throughout the step. An incomplete
            // symbol or header can be retried from this bit position.
            let (bits, bit_count, in_pos) = (self.bits, self.bit_count, self.in_pos);
            match self.step(start) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(Fail::Io(error))
                    if self.supply == Supply::Pushing
                        && error.kind() == io::ErrorKind::WouldBlock =>
                {
                    self.bits = bits;
                    self.bit_count = bit_count;
                    self.in_pos = in_pos;
                    self.starved = true;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// One block header, one run of stored bytes or one symbol. True when
    /// decoding should stop for now.
    #[inline(always)]
    fn step(&mut self, start: usize) -> Result<bool, Fail> {
        match self.state {
            State::Done => return Ok(true),
            State::BlockHeader => {
                if self.filled > start && self.in_pos == self.in_len && self.bit_count < 32 {
                    return Ok(true);
                }
                self.read_block_header()?;
            }
            State::DynamicLengths {
                literals,
                total,
                at,
            } => self.read_length_run(literals, total, at)?,
            State::Stored(0) => self.finish_block(),
            State::Stored(remaining) => {
                if self.filled > start && self.in_pos == self.in_len && self.bit_count < 8 {
                    return Ok(true);
                }
                if self.bit_count == 8 && self.in_pos == self.in_len && self.produced < self.limit {
                    self.window[self.filled] = self.bits as u8;
                    self.bits = 0;
                    self.bit_count = 0;
                    self.filled += 1;
                    self.produced += 1;
                    self.state = State::Stored(remaining - 1);
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
                if self.in_pos == self.in_len && self.bit_count < 48 {
                    if self.filled > start {
                        return Ok(true);
                    }
                    if self.supply == Supply::Pushing && self.bit_count < 32 {
                        self.starved = true;
                        return Ok(true);
                    }
                }
                // A four-byte load fits without masking. Longer matches can
                // still request extra bits while decoding the distance.
                if self.bit_count < 32 {
                    if let Some(word) = self.input[self.in_pos..self.in_len].first_chunk::<4>() {
                        self.bits |= u64::from(u32::from_le_bytes(*word)) << self.bit_count;
                        self.bit_count += 32;
                        self.in_pos += 4;
                    } else {
                        self.refill()?;
                    }
                }
                self.decode_symbol_run()?;
            }
        }
        Ok(false)
    }

    /// Decode complete tokens from buffered input. Leave long codes, block
    /// ends and invalid tokens at their bit position for the incremental decoder.
    fn decode_buffered_codes(&mut self) {
        let input = &self.input[..self.in_len];
        let mut pos = self.in_pos;
        let mut bits = self.bits;
        let mut count = self.bit_count;
        let mut filled = self.filled;
        let room = self
            .limit
            .saturating_sub(self.produced)
            .min((CAP - filled) as u64) as usize;
        let end = filled + room;
        while filled + MAX_MATCH < end && pos + 8 <= input.len() {
            refill_word(input, &mut pos, &mut bits, &mut count);
            // One literal can precede a complete match in this iteration.
            let mut lit = self.literals.lookup(bits);
            if lit.is_literal() {
                let width = lit.width();
                self.window[filled] = lit.symbol() as u8;
                filled += 1;
                bits = lit.skip(bits);
                count -= width;
                lit = self.literals.lookup(bits);
            }
            let width = lit.width();
            let symbol = lit.symbol();
            if symbol < 256 {
                self.window[filled] = symbol as u8;
                filled += 1;
                bits = lit.skip(bits);
                count -= width;
            } else {
                let index = symbol.wrapping_sub(257);
                if index >= LEN_BASE.len() {
                    break;
                }
                refill_word(input, &mut pos, &mut bits, &mut count);
                let word = lit.skip(bits);
                let extra = LEN_EXTRA[index];
                let length = usize::from(LEN_BASE[index]) + (word as usize & ((1 << extra) - 1));
                let word = word >> extra;
                let dist = self.distances.lookup(word);
                let index = dist.symbol();
                if index >= DIST_BASE.len() {
                    break;
                }
                let dist_width = dist.width();
                let dist_extra = DIST_EXTRA[index];
                let distance = usize::from(DIST_BASE[index])
                    + ((word >> dist_width) as usize & ((1 << dist_extra) - 1));
                let used = width + u32::from(extra) + dist_width + u32::from(dist_extra);
                if used > count || distance > filled {
                    break;
                }
                copy_match(&mut self.window, filled, distance, length);
                filled += length;
                bits >>= used;
                count -= used;
            }
        }
        self.bits = bits;
        self.bit_count = count;
        self.in_pos = pos;
        let output = filled - self.filled;
        self.produced += output as u64;
        self.filled = filled;
    }

    /// One literal, one match, or the end of the block.
    #[inline(always)]
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
        copy_match(&mut self.window, self.filled, distance, length);
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
