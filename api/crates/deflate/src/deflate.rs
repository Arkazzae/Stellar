//! Streaming DEFLATE encoding (RFC 1951).
//!
//! Hash-chain matching over a sliding window, then for every block the
//! smallest of a stored, a fixed and a dynamic Huffman encoding. The encoder
//! only ever holds a window and one block, whatever the input size, and every
//! code it writes is a complete prefix code.

use std::io::{self, Write};

use crate::huffman::{assign_codes, build_lengths};
use crate::tables::{
    CODE_LENGTH_ORDER, DIST_BASE, DIST_EXTRA, LEN_BASE, LEN_EXTRA, MAX_MATCH, WINDOW,
};

/// How hard the encoder looks for matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Greedy matching, short chains: for data that must be written quickly.
    Fast,
    /// Lazy matching with moderate chains.
    Default,
    /// Lazy matching with long chains.
    Best,
}

struct Params {
    max_chain: u32,
    nice_length: usize,
    lazy: bool,
    /// A match this long is taken without looking for a better one.
    max_lazy: usize,
    /// Greedy mode inserts positions inside a match only up to this length.
    max_insert: usize,
}

impl Level {
    fn params(self) -> Params {
        match self {
            Level::Fast => Params {
                max_chain: 8,
                nice_length: 32,
                lazy: false,
                max_lazy: 0,
                max_insert: 16,
            },
            Level::Default => Params {
                max_chain: 96,
                nice_length: 128,
                lazy: true,
                max_lazy: 32,
                max_insert: 0,
            },
            Level::Best => Params {
                max_chain: 1024,
                nice_length: MAX_MATCH,
                lazy: true,
                max_lazy: MAX_MATCH,
                max_insert: 0,
            },
        }
    }
}

const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Bytes kept: the window, one block of input and the look-ahead.
const DATA_CAP: usize = 1 << 18;
/// Bytes of look-ahead a position needs before it is matched, so that both it
/// and its successor can see a full-length match.
const LOOKAHEAD: usize = MAX_MATCH + 4;
const MAX_SYMBOLS: usize = 1 << 15;
/// A block is closed once this much input is in it, so that a stored block
/// (at most 65535 bytes) is always possible for it.
const BLOCK_RAW_LIMIT: usize = 65_000;
const MIN_MATCH: usize = 3;
/// A three-byte match this far back costs more than three literals.
const TOO_FAR: usize = 4096;

const fn build_length_codes() -> [u8; MAX_MATCH + 1] {
    let mut table = [0u8; MAX_MATCH + 1];
    let mut code = 0;
    while code < 29 {
        let base = LEN_BASE[code] as usize;
        let span = 1usize << LEN_EXTRA[code];
        let mut i = 0;
        while i < span && base + i <= MAX_MATCH {
            table[base + i] = code as u8;
            i += 1;
        }
        code += 1;
    }
    // 258 has a code of its own; the one before it also reaches it.
    table[MAX_MATCH] = 28;
    table
}

static LENGTH_CODE: [u8; MAX_MATCH + 1] = build_length_codes();

fn distance_code(distance: usize) -> usize {
    let d = distance - 1;
    if d < 4 {
        d
    } else {
        let high = (usize::BITS - 1 - d.leading_zeros()) as usize;
        2 * high + ((d >> (high - 1)) & 1)
    }
}

#[derive(Clone, Copy)]
struct Symbol {
    /// The byte for a literal, the length for a match.
    value: u16,
    /// 0 for a literal.
    distance: u16,
}

struct BitWriter {
    bits: u64,
    count: u32,
    out: Vec<u8>,
}

impl BitWriter {
    fn put(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32 && (n == 32 || value < (1 << n)));
        self.bits |= u64::from(value) << self.count;
        self.count += n;
        if self.count >= 32 {
            self.out
                .extend_from_slice(&(self.bits as u32).to_le_bytes());
            self.bits >>= 32;
            self.count -= 32;
        }
    }

    /// Pads with zero bits to a byte boundary.
    fn align(&mut self) {
        while self.count > 0 {
            self.out.push(self.bits as u8);
            self.bits >>= 8;
            self.count = self.count.saturating_sub(8);
        }
        self.bits = 0;
    }
}

struct Encoder {
    params: Params,
    data: Vec<u8>,
    prev: Vec<u32>,
    head: Box<[u32; HASH_SIZE]>,
    /// The next position to match.
    pos: usize,
    /// Every position below this is in the hash chains, or deliberately not.
    inserted: usize,
    block_start: usize,
    symbols: Vec<Symbol>,
    literal_freq: [u32; 286],
    distance_freq: [u32; 30],
    bits: BitWriter,
}

#[derive(Clone, Copy)]
struct Match {
    length: usize,
    distance: usize,
}

const NO_MATCH: Match = Match {
    length: 0,
    distance: 0,
};

impl Encoder {
    fn new(level: Level) -> Self {
        Self {
            params: level.params(),
            data: Vec::new(),
            prev: Vec::new(),
            head: Box::new([0; HASH_SIZE]),
            pos: 0,
            inserted: 0,
            block_start: 0,
            symbols: Vec::new(),
            literal_freq: [0; 286],
            distance_freq: [0; 30],
            bits: BitWriter {
                bits: 0,
                count: 0,
                out: Vec::new(),
            },
        }
    }

    fn hash(&self, at: usize) -> usize {
        let d = &self.data;
        let v = u32::from(d[at]) | (u32::from(d[at + 1]) << 8) | (u32::from(d[at + 2]) << 16);
        (v.wrapping_mul(0x1e35_a7bd) >> (32 - HASH_BITS)) as usize
    }

    /// Puts every position below `upto` into the hash chains.
    fn insert_until(&mut self, upto: usize) {
        while self.inserted < upto {
            let at = self.inserted;
            if at + MIN_MATCH <= self.data.len() {
                let h = self.hash(at);
                self.prev[at] = self.head[h];
                self.head[h] = at as u32 + 1;
            }
            self.inserted += 1;
        }
    }

    fn match_length(&self, a: usize, b: usize, max: usize) -> usize {
        let d = &self.data;
        let mut n = 0;
        while n + 8 <= max {
            let x = u64::from_le_bytes(d[a + n..a + n + 8].try_into().unwrap_or([0; 8]))
                ^ u64::from_le_bytes(d[b + n..b + n + 8].try_into().unwrap_or([0; 8]));
            if x != 0 {
                return n + (x.trailing_zeros() / 8) as usize;
            }
            n += 8;
        }
        while n < max && d[a + n] == d[b + n] {
            n += 1;
        }
        n
    }

    /// The longest earlier match for the bytes at `at`; every position below
    /// `at` must already be inserted.
    fn find_match(&self, at: usize) -> Match {
        let available = self.data.len() - at;
        if available < MIN_MATCH {
            return NO_MATCH;
        }
        let max = available.min(MAX_MATCH);
        let floor = at.saturating_sub(WINDOW);
        let mut best = Match {
            length: MIN_MATCH - 1,
            distance: 0,
        };
        let mut candidate = self.head[self.hash(at)];
        let mut chain = self.params.max_chain;
        while candidate != 0 && chain > 0 {
            let p = candidate as usize - 1;
            if p < floor || p >= at {
                break;
            }
            if best.length >= max {
                break;
            }
            if self.data[p + best.length] == self.data[at + best.length]
                && self.data[p] == self.data[at]
            {
                let length = self.match_length(p, at, max);
                if length > best.length {
                    best = Match {
                        length,
                        distance: at - p,
                    };
                    if length >= self.params.nice_length || length >= max {
                        break;
                    }
                }
            }
            candidate = self.prev[p];
            chain -= 1;
        }
        if best.length < MIN_MATCH || (best.length == MIN_MATCH && best.distance > TOO_FAR) {
            NO_MATCH
        } else {
            best
        }
    }

    fn emit_literal(&mut self) {
        let byte = self.data[self.pos];
        self.symbols.push(Symbol {
            value: u16::from(byte),
            distance: 0,
        });
        self.literal_freq[usize::from(byte)] += 1;
        self.pos += 1;
    }

    fn emit_match(&mut self, m: Match) {
        self.symbols.push(Symbol {
            value: m.length as u16,
            distance: m.distance as u16,
        });
        self.literal_freq[257 + usize::from(LENGTH_CODE[m.length])] += 1;
        self.distance_freq[distance_code(m.distance)] += 1;
        self.pos += m.length;
    }

    /// Encodes what is buffered. Unless `all`, positions without a full
    /// look-ahead wait for more input.
    fn run(&mut self, all: bool) {
        loop {
            if self.symbols.len() >= MAX_SYMBOLS || self.pos - self.block_start >= BLOCK_RAW_LIMIT {
                self.flush_block(false);
            }
            let available = self.data.len() - self.pos;
            if available == 0 || (!all && available < LOOKAHEAD) {
                return;
            }
            let at = self.pos;
            let mut current = self.find_match(at);
            self.insert_until(at + 1);
            if current.length == 0 {
                self.emit_literal();
                continue;
            }
            if self.params.lazy {
                while current.length < self.params.max_lazy {
                    let next = self.find_match(self.pos + 1);
                    self.insert_until(self.pos + 2);
                    if next.length > current.length {
                        self.emit_literal();
                        current = next;
                    } else {
                        break;
                    }
                }
            }
            let end = self.pos + current.length;
            if self.params.lazy || current.length <= self.params.max_insert {
                self.insert_until(end);
            } else {
                // Skip the middle of a long match, but keep its last two
                // positions findable: a run then matches itself one byte back.
                self.inserted = self.inserted.max(end - 2);
                self.insert_until(end);
            }
            self.emit_match(current);
        }
    }

    /// Writes the buffered symbols as one block, in whichever of the three
    /// block types is smallest.
    fn flush_block(&mut self, last: bool) {
        self.literal_freq[256] = 1;
        let raw = self.pos - self.block_start;

        let mut literal_lengths = [0u8; 286];
        let mut distance_lengths = [0u8; 30];
        build_lengths(&self.literal_freq, 15, &mut literal_lengths);
        build_lengths(&self.distance_freq, 15, &mut distance_lengths);

        let extra_bits: u64 = (0..29)
            .map(|c| u64::from(self.literal_freq[257 + c]) * u64::from(LEN_EXTRA[c]))
            .chain((0..30).map(|c| u64::from(self.distance_freq[c]) * u64::from(DIST_EXTRA[c])))
            .sum();
        let coded = |lengths: &[u8], freq: &[u32]| -> u64 {
            lengths
                .iter()
                .zip(freq)
                .map(|(&l, &f)| u64::from(l) * u64::from(f))
                .sum()
        };

        // The fixed code is defined over 288 symbols; the last two are never
        // used but take part in assigning the codes.
        let mut fixed_literal = [0u8; 288];
        fixed_literal[..144].fill(8);
        fixed_literal[144..256].fill(9);
        fixed_literal[256..280].fill(7);
        fixed_literal[280..].fill(8);
        let fixed_bits = 3
            + extra_bits
            + coded(&fixed_literal[..286], &self.literal_freq)
            + self
                .distance_freq
                .iter()
                .map(|&f| u64::from(f) * 5)
                .sum::<u64>();

        let header = DynamicHeader::new(&literal_lengths, &distance_lengths);
        let dynamic_bits = 3
            + header.bits
            + extra_bits
            + coded(&literal_lengths, &self.literal_freq)
            + coded(&distance_lengths, &self.distance_freq);

        let stored_bits = if raw <= 65_535 {
            let pad = (8 - (u64::from(self.bits.count) + 3) % 8) % 8;
            3 + pad + 32 + raw as u64 * 8
        } else {
            u64::MAX
        };

        let final_bit = u32::from(last);
        if stored_bits < fixed_bits && stored_bits < dynamic_bits {
            self.bits.put(final_bit, 1);
            self.bits.put(0, 2);
            self.bits.align();
            let length = raw as u16;
            self.bits.out.extend_from_slice(&length.to_le_bytes());
            self.bits.out.extend_from_slice(&(!length).to_le_bytes());
            let block = &self.data[self.block_start..self.pos];
            self.bits.out.extend_from_slice(block);
        } else if fixed_bits <= dynamic_bits {
            self.bits.put(final_bit, 1);
            self.bits.put(1, 2);
            let mut codes = [0u16; 288];
            assign_codes(&fixed_literal, &mut codes);
            let mut distance_codes = [0u16; 30];
            for (code, slot) in distance_codes.iter_mut().enumerate() {
                *slot = (code as u16).reverse_bits() >> 11;
            }
            self.write_symbols(&fixed_literal, &codes, &[5; 30], &distance_codes);
        } else {
            self.bits.put(final_bit, 1);
            self.bits.put(2, 2);
            header.write(&mut self.bits);
            let mut codes = [0u16; 286];
            assign_codes(&literal_lengths, &mut codes);
            let mut distance_codes = [0u16; 30];
            assign_codes(&distance_lengths, &mut distance_codes);
            self.write_symbols(&literal_lengths, &codes, &distance_lengths, &distance_codes);
        }

        self.symbols.clear();
        self.literal_freq = [0; 286];
        self.distance_freq = [0; 30];
        self.block_start = self.pos;
    }

    fn write_symbols(
        &mut self,
        lit_len: &[u8],
        lit_code: &[u16],
        dist_len: &[u8],
        dist_code: &[u16],
    ) {
        let symbols = std::mem::take(&mut self.symbols);
        for symbol in &symbols {
            if symbol.distance == 0 {
                let s = usize::from(symbol.value);
                self.bits.put(u32::from(lit_code[s]), u32::from(lit_len[s]));
                continue;
            }
            let length = usize::from(symbol.value);
            let lc = usize::from(LENGTH_CODE[length]);
            self.bits
                .put(u32::from(lit_code[257 + lc]), u32::from(lit_len[257 + lc]));
            self.bits.put(
                (length - usize::from(LEN_BASE[lc])) as u32,
                u32::from(LEN_EXTRA[lc]),
            );
            let distance = usize::from(symbol.distance);
            let dc = distance_code(distance);
            self.bits
                .put(u32::from(dist_code[dc]), u32::from(dist_len[dc]));
            self.bits.put(
                (distance - usize::from(DIST_BASE[dc])) as u32,
                u32::from(DIST_EXTRA[dc]),
            );
        }
        self.symbols = symbols;
        self.bits
            .put(u32::from(lit_code[256]), u32::from(lit_len[256]));
    }

    /// Makes room in the buffer by dropping what no match or stored block can
    /// reach any more.
    fn slide(&mut self) {
        let drop = self.pos.saturating_sub(WINDOW).min(self.block_start);
        if drop == 0 {
            return;
        }
        self.data.drain(..drop);
        self.prev.drain(..drop);
        self.pos -= drop;
        self.inserted -= drop.min(self.inserted);
        self.block_start -= drop;
        let shift = drop as u32;
        for entry in self.head.iter_mut().chain(self.prev.iter_mut()) {
            *entry = entry.saturating_sub(shift);
        }
    }

    fn push(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            let room = DATA_CAP - self.data.len();
            if room == 0 {
                self.run(false);
                self.slide();
                continue;
            }
            let n = room.min(input.len());
            self.data.extend_from_slice(&input[..n]);
            self.prev.resize(self.data.len(), 0);
            input = &input[n..];
            self.run(false);
        }
    }
}

/// The header of a dynamic block: how the two code-length tables are sent.
struct DynamicHeader {
    literal_count: usize,
    distance_count: usize,
    /// (code-length symbol, extra bits value, extra bit count)
    tokens: Vec<(u8, u8, u8)>,
    code_length_lengths: [u8; 19],
    code_length_codes: [u16; 19],
    code_length_count: usize,
    bits: u64,
}

impl DynamicHeader {
    fn new(literal: &[u8; 286], distance: &[u8; 30]) -> Self {
        let literal_count = literal
            .iter()
            .rposition(|&l| l > 0)
            .map_or(257, |i| i + 1)
            .max(257);
        let distance_count = distance.iter().rposition(|&l| l > 0).map_or(1, |i| i + 1);
        let mut tokens = Vec::new();
        run_length(&literal[..literal_count], &mut tokens);
        run_length(&distance[..distance_count], &mut tokens);

        let mut freq = [0u32; 19];
        for &(symbol, _, _) in &tokens {
            freq[usize::from(symbol)] += 1;
        }
        let mut code_length_lengths = [0u8; 19];
        build_lengths(&freq, 7, &mut code_length_lengths);
        let mut code_length_codes = [0u16; 19];
        assign_codes(&code_length_lengths, &mut code_length_codes);
        let code_length_count = CODE_LENGTH_ORDER
            .iter()
            .rposition(|&s| code_length_lengths[s] > 0)
            .map_or(4, |i| i + 1)
            .max(4);

        let mut bits = 14 + 3 * code_length_count as u64;
        for &(symbol, _, extra) in &tokens {
            bits += u64::from(code_length_lengths[usize::from(symbol)]) + u64::from(extra);
        }
        Self {
            literal_count,
            distance_count,
            tokens,
            code_length_lengths,
            code_length_codes,
            code_length_count,
            bits,
        }
    }

    fn write(&self, out: &mut BitWriter) {
        out.put((self.literal_count - 257) as u32, 5);
        out.put((self.distance_count - 1) as u32, 5);
        out.put((self.code_length_count - 4) as u32, 4);
        for &slot in &CODE_LENGTH_ORDER[..self.code_length_count] {
            out.put(u32::from(self.code_length_lengths[slot]), 3);
        }
        for &(symbol, value, extra) in &self.tokens {
            let s = usize::from(symbol);
            out.put(
                u32::from(self.code_length_codes[s]),
                u32::from(self.code_length_lengths[s]),
            );
            out.put(u32::from(value), u32::from(extra));
        }
    }
}

/// Run-length codes a table of code lengths (symbols 16, 17 and 18).
fn run_length(lengths: &[u8], tokens: &mut Vec<(u8, u8, u8)>) {
    let mut i = 0;
    while i < lengths.len() {
        let value = lengths[i];
        let mut run = 1;
        while i + run < lengths.len() && lengths[i + run] == value {
            run += 1;
        }
        i += run;
        if value == 0 {
            while run >= 11 {
                let n = run.min(138);
                tokens.push((18, (n - 11) as u8, 7));
                run -= n;
            }
            if run >= 3 {
                tokens.push((17, (run - 3) as u8, 3));
                run = 0;
            }
        } else {
            tokens.push((value, 0, 0));
            run -= 1;
            while run >= 3 {
                let n = run.min(6);
                tokens.push((16, (n - 3) as u8, 2));
                run -= n;
            }
        }
        for _ in 0..run {
            tokens.push((value, 0, 0));
        }
    }
}

/// A streaming DEFLATE encoder writing to `W`.
pub struct Deflater<W: Write> {
    out: W,
    encoder: Box<Encoder>,
    finished: bool,
    total_in: u64,
}

impl<W: Write> Deflater<W> {
    pub fn new(out: W, level: Level) -> Self {
        Self {
            out,
            encoder: Box::new(Encoder::new(level)),
            finished: false,
            total_in: 0,
        }
    }

    /// Bytes fed in so far.
    pub fn total_in(&self) -> u64 {
        self.total_in
    }

    pub fn get_ref(&self) -> &W {
        &self.out
    }

    /// The writer, for a caller that moves the output along as it appears.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.out
    }

    fn drain(&mut self) -> io::Result<()> {
        if !self.encoder.bits.out.is_empty() {
            self.out.write_all(&self.encoder.bits.out)?;
            self.encoder.bits.out.clear();
        }
        Ok(())
    }

    /// Ends the stream and returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        if !self.finished {
            self.finished = true;
            self.encoder.run(true);
            self.encoder.flush_block(true);
            self.encoder.bits.align();
            self.drain()?;
        }
        self.out.flush()?;
        Ok(self.out)
    }

    /// Makes everything written so far decodable without ending the stream
    /// (a sync flush). Costs a little ratio each time.
    pub fn sync_flush(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.encoder.run(true);
        if self.encoder.pos > self.encoder.block_start {
            self.encoder.flush_block(false);
        }
        // An empty stored block marks a byte boundary.
        self.encoder.bits.put(0, 3);
        self.encoder.bits.align();
        self.encoder.bits.out.extend_from_slice(&[0, 0, 0xff, 0xff]);
        self.drain()?;
        self.out.flush()
    }
}

impl<W: Write> Write for Deflater<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(io::Error::other("the deflate stream has been finished"));
        }
        self.encoder.push(buf);
        self.total_in += buf.len() as u64;
        self.drain()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sync_flush()
    }
}

/// Compresses `data` into a raw DEFLATE stream.
pub fn deflate_to_vec(data: &[u8], level: Level) -> Vec<u8> {
    let mut deflater = Deflater::new(Vec::new(), level);
    // Writing to a Vec cannot fail.
    let _ = deflater.write_all(data);
    deflater.finish().unwrap_or_default()
}
