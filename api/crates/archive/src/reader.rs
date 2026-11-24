//! Reading a zip file by its central directory.
//!
//! Nothing is believed before it is checked: the directory must fit the file
//! and the caller's limits, every entry's data must lie where it can and
//! never overlap another's, the local header must agree with the directory,
//! and what comes out of an entry must have the length and the CRC-32 the
//! directory promised. A check that fails is an [`Error`], never a panic.

use std::io::{self, BufReader, Read, Seek, SeekFrom, Take, Write};

use stellarr_deflate::{Crc32, Inflate};

use crate::error::Error;
use crate::time::unix_from_dos;
use crate::writer::{
    CENTRAL_SIGNATURE, END_SIGNATURE, LOCAL_SIGNATURE, TIMESTAMP_FIELD, ZIP64_END_SIGNATURE,
    ZIP64_FIELD, ZIP64_LOCATOR_SIGNATURE,
};

const MAX32: u64 = 0xffff_ffff;
const MAX16: u64 = 0xffff;
const END_LEN: u64 = 22;
const LOCATOR_LEN: u64 = 20;
const CENTRAL_FIXED: usize = 46;
const LOCAL_FIXED: u64 = 30;
/// The most a deflate stream can expand: 258 bytes for two bits.
const MAX_EXPANSION: u64 = 1032;

const FLAG_ENCRYPTED: u16 = 0x0001;
const FLAG_STRONG_ENCRYPTION: u16 = 0x0040;
const FLAG_UTF8: u16 = 0x0800;

/// What the reader refuses to hold or to promise, so that a hostile archive
/// cannot make it use more than the caller meant to give.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most entries in the directory. The directory is read into memory,
    /// about a hundred bytes and the name per entry.
    pub max_entries: u64,
    /// Largest central directory, in bytes.
    pub max_directory_bytes: u64,
    /// Largest uncompressed size of any one entry.
    pub max_entry_bytes: u64,
    /// Largest sum of the entries' uncompressed sizes.
    pub max_total_bytes: u64,
}

impl Default for Limits {
    /// Four million entries and a gigabyte of directory, and no cap on the
    /// sizes of the entries: a caller that unpacks them sets its own.
    fn default() -> Self {
        Self {
            max_entries: 4_000_000,
            max_directory_bytes: 1 << 30,
            max_entry_bytes: u64::MAX,
            max_total_bytes: u64::MAX,
        }
    }
}

/// How an entry's bytes are kept in the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Stored,
    Deflated,
}

/// One entry of the central directory.
#[derive(Clone, Debug)]
pub struct Entry {
    name: Box<[u8]>,
    flags: u16,
    method: Method,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_offset: u64,
    time: u16,
    date: u16,
    unix_modified: Option<i64>,
    made_by_unix: bool,
    external_attributes: u32,
}

impl Entry {
    /// The name exactly as stored: no decoding, no normalising, no checking.
    /// It may hold `..`, a backslash, a NUL, a leading slash or bytes that
    /// are not text; whether it is safe to use is for the caller to judge.
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// The name as text, when it is valid UTF-8.
    pub fn name(&self) -> Option<&str> {
        std::str::from_utf8(&self.name).ok()
    }

    /// The name for a message, with anything unreadable replaced.
    pub fn name_lossy(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    /// Whether the archive says the name is UTF-8. A name without that flag
    /// is, by the format, in an old DOS code page; its bytes are still given
    /// as stored.
    pub fn name_is_flagged_utf8(&self) -> bool {
        self.flags & FLAG_UTF8 != 0
    }

    pub fn method(&self) -> Method {
        self.method
    }

    /// The CRC-32 of the uncompressed bytes, as the directory states it.
    pub fn crc32(&self) -> u32 {
        self.crc32
    }

    pub fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    pub fn uncompressed_size(&self) -> u64 {
        self.uncompressed_size
    }

    /// A folder: its name ends in a slash, or its attributes say so.
    pub fn is_directory(&self) -> bool {
        self.name.ends_with(b"/")
            || self.external_attributes & 0x10 != 0
            || self.unix_type() == Some(0o040_000)
    }

    /// The Unix file mode, when the archive was written on a Unix system.
    pub fn unix_mode(&self) -> Option<u32> {
        self.made_by_unix
            .then_some(self.external_attributes >> 16)
            .filter(|mode| *mode != 0)
    }

    fn unix_type(&self) -> Option<u32> {
        self.unix_mode().map(|mode| mode & 0o170_000)
    }

    /// Whether the entry is a symbolic link, which an unpacker must not
    /// follow or recreate blindly.
    pub fn is_symlink(&self) -> bool {
        self.unix_type() == Some(0o120_000)
    }

    /// When the entry was last changed, in Unix seconds, read as UTC: the
    /// extended timestamp when there is one, else the DOS date and time.
    pub fn modified_unix(&self) -> i64 {
        self.unix_modified
            .unwrap_or_else(|| unix_from_dos(self.time, self.date))
    }
}

/// An open zip file.
pub struct ZipArchive<R> {
    reader: R,
    entries: Vec<Entry>,
    directory_offset: u64,
    total_uncompressed: u64,
}

fn corrupt(why: impl Into<String>) -> Error {
    Error::Corrupt(why.into())
}

fn short(error: io::Error, what: &str) -> Error {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        corrupt(format!("{what} is cut short"))
    } else {
        Error::Io(error)
    }
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

impl<R: Read + Seek> ZipArchive<R> {
    pub fn new(reader: R) -> Result<Self, Error> {
        Self::with_limits(reader, Limits::default())
    }

    pub fn with_limits(mut reader: R, limits: Limits) -> Result<Self, Error> {
        let (directory_offset, directory_size, count) = locate_directory(&mut reader, &limits)?;
        let entries = read_directory(&mut reader, directory_offset, directory_size, count)?;

        let mut total: u64 = 0;
        for entry in &entries {
            if entry.uncompressed_size > limits.max_entry_bytes {
                return Err(Error::TooLarge(format!(
                    "'{}' is {} bytes, more than the {} allowed",
                    entry.name_lossy(),
                    entry.uncompressed_size,
                    limits.max_entry_bytes
                )));
            }
            total = total.saturating_add(entry.uncompressed_size);
        }
        if total > limits.max_total_bytes {
            return Err(Error::TooLarge(format!(
                "the entries add up to {total} bytes, more than the {} allowed",
                limits.max_total_bytes
            )));
        }
        check_layout(&entries, directory_offset)?;
        Ok(Self {
            reader,
            entries,
            directory_offset,
            total_uncompressed: total,
        })
    }

    /// The entries, in the order the archive lists them.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The sum of the entries' uncompressed sizes, as the directory states
    /// them; reading an entry proves its own.
    pub fn total_uncompressed_size(&self) -> u64 {
        self.total_uncompressed
    }

    /// The index of the first entry with exactly this name.
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.entries.iter().position(|entry| *entry.name == *name)
    }

    /// A name that more than one entry carries, if any does. Which of them an
    /// extractor takes is a matter of the tool, so a caller that does not
    /// want the question should refuse such an archive.
    pub fn duplicate_name(&self) -> Option<&[u8]> {
        let mut seen = std::collections::HashSet::with_capacity(self.entries.len());
        self.entries
            .iter()
            .map(|entry| &*entry.name)
            .find(|name| !seen.insert(*name))
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Opens an entry for reading. The bytes come out unpacked, and once the
    /// last of them has been read the entry's length and CRC-32 have been
    /// checked: a reader that stops early has not verified anything.
    pub fn open(&mut self, index: usize) -> Result<EntryReader<'_, R>, Error> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| Error::Invalid(format!("there is no entry {index}")))?;
        let name = entry.name_lossy();

        self.reader.seek(SeekFrom::Start(entry.local_offset))?;
        let mut header = [0u8; LOCAL_FIXED as usize];
        self.reader
            .read_exact(&mut header)
            .map_err(|e| short(e, "a local header"))?;
        if le32(&header, 0) != LOCAL_SIGNATURE {
            return Err(corrupt(format!(
                "the local header of '{name}' is not where the directory says"
            )));
        }
        let flags = le16(&header, 6);
        if flags & (FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION) != 0 {
            return Err(Error::Unsupported(format!("'{name}' is encrypted")));
        }
        let name_length = u64::from(le16(&header, 26));
        let extra_length = u64::from(le16(&header, 28));
        if name_length != entry.name.len() as u64 {
            return Err(corrupt(format!(
                "the local header of '{name}' names another entry"
            )));
        }
        let mut local_name = vec![0u8; entry.name.len()];
        self.reader
            .read_exact(&mut local_name)
            .map_err(|e| short(e, "a local header"))?;
        if local_name != *entry.name {
            return Err(corrupt(format!(
                "the local header of '{name}' names another entry"
            )));
        }

        let data_start = entry.local_offset + LOCAL_FIXED + name_length + extra_length;
        let data_end = data_start.saturating_add(entry.compressed_size);
        if data_end > self.directory_offset {
            return Err(corrupt(format!(
                "the data of '{name}' runs past where the archive's data ends"
            )));
        }
        self.reader.seek(SeekFrom::Start(data_start))?;

        let limited = (&mut self.reader).take(entry.compressed_size);
        let source = match entry.method {
            Method::Stored => Source::Stored(limited),
            // Some writers leave an empty entry with no stream at all.
            Method::Deflated if entry.compressed_size == 0 && entry.uncompressed_size == 0 => {
                Source::Stored(limited)
            }
            Method::Deflated => Source::Deflated(Box::new(
                Inflate::new(limited).with_limit(entry.uncompressed_size),
            )),
        };
        Ok(EntryReader {
            source,
            name,
            crc: Crc32::new(),
            read: 0,
            expected_size: entry.uncompressed_size,
            expected_crc: entry.crc32,
            verified: false,
        })
    }

    /// Reads an entry whole into memory, refusing one over `max` bytes.
    pub fn read_to_vec(&mut self, index: usize, max: u64) -> Result<Vec<u8>, Error> {
        let size = self
            .entries
            .get(index)
            .map_or(0, |entry| entry.uncompressed_size);
        if size > max {
            return Err(Error::TooLarge(format!(
                "an entry of {size} bytes is more than the {max} allowed"
            )));
        }
        let mut out = Vec::with_capacity(size.min(1 << 20) as usize);
        self.open(index)?
            .read_to_end(&mut out)
            .map_err(Error::from_io)?;
        Ok(out)
    }

    /// Reads an entry through to its end into `sink`, which checks every
    /// byte and the totals. Returns the bytes written.
    pub fn copy_to<W: Write>(&mut self, index: usize, sink: &mut W) -> Result<u64, Error> {
        let mut reader = self.open(index)?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = reader.read(&mut buffer).map_err(Error::from_io)?;
            if n == 0 {
                return Ok(total);
            }
            sink.write_all(&buffer[..n])?;
            total += n as u64;
        }
    }

    /// Reads every entry through, checking every length and CRC-32.
    pub fn verify_all(&mut self) -> Result<(), Error> {
        for index in 0..self.entries.len() {
            self.copy_to(index, &mut io::sink())?;
        }
        Ok(())
    }
}

enum Source<'a, R> {
    Stored(Take<&'a mut R>),
    Deflated(Box<Inflate<Take<&'a mut R>>>),
}

/// The unpacked bytes of one entry. Reading to the end checks them.
pub struct EntryReader<'a, R: Read> {
    source: Source<'a, R>,
    name: String,
    crc: Crc32,
    read: u64,
    expected_size: u64,
    expected_crc: u32,
    verified: bool,
}

impl<R: Read> EntryReader<'_, R> {
    fn check_end(&mut self) -> Result<(), Error> {
        if self.read != self.expected_size {
            return Err(if self.read < self.expected_size {
                corrupt(format!(
                    "the data of '{}' ends before its recorded size",
                    self.name
                ))
            } else {
                corrupt(format!(
                    "the data of '{}' goes past its recorded size",
                    self.name
                ))
            });
        }
        if self.crc.value() != self.expected_crc {
            return Err(Error::Checksum(self.name.clone()));
        }
        if let Source::Deflated(inflate) = &mut self.source {
            // The recorded compressed size is the stream and nothing more.
            let mut extra = [0u8; 1];
            match inflate.read_raw(&mut extra) {
                Ok(0) => {}
                Ok(_) => {
                    return Err(corrupt(format!(
                        "'{}' has bytes after its compressed data",
                        self.name
                    )));
                }
                Err(error) => return Err(self.map_error(error)),
            }
        }
        self.verified = true;
        Ok(())
    }

    fn map_error(&self, error: io::Error) -> Error {
        if let Some(inner) = stellarr_deflate::Error::from_io(&error) {
            return match inner {
                stellarr_deflate::Error::Truncated => corrupt(format!(
                    "the compressed data of '{}' is cut short",
                    self.name
                )),
                stellarr_deflate::Error::OutputLimit => corrupt(format!(
                    "'{}' unpacks to more than its recorded size",
                    self.name
                )),
                other => corrupt(format!(
                    "the compressed data of '{}' is invalid: {other}",
                    self.name
                )),
            };
        }
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return corrupt(format!("the data of '{}' is cut short", self.name));
        }
        Error::Io(error)
    }
}

impl<R: Read> Read for EntryReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.verified {
            return Ok(0);
        }
        let result = match &mut self.source {
            Source::Stored(limited) => limited.read(buf),
            Source::Deflated(inflate) => inflate.read(buf),
        };
        let n = result.map_err(|error| io::Error::from(self.map_error(error)))?;
        if n == 0 {
            return self.check_end().map(|()| 0).map_err(io::Error::from);
        }
        self.crc.update(&buf[..n]);
        self.read += n as u64;
        if self.read > self.expected_size {
            return Err(corrupt(format!(
                "the data of '{}' goes past its recorded size",
                self.name
            ))
            .into());
        }
        Ok(n)
    }
}

/// Finds the central directory: `(offset, size, entry count)`.
fn locate_directory<R: Read + Seek>(
    reader: &mut R,
    limits: &Limits,
) -> Result<(u64, u64, u64), Error> {
    let length = reader.seek(SeekFrom::End(0))?;
    if length < END_LEN {
        return Err(corrupt("the file is too short to be a zip file"));
    }
    let window = length.min(END_LEN + MAX16);
    let window_start = length - window;
    reader.seek(SeekFrom::Start(window_start))?;
    let mut tail = vec![0u8; window as usize];
    reader
        .read_exact(&mut tail)
        .map_err(|e| short(e, "the end of the archive"))?;

    // The last record that starts with the signature and whose comment ends
    // exactly at the end of the file.
    let end_at = (0..=tail.len() - END_LEN as usize)
        .rev()
        .find(|&at| {
            le32(&tail, at) == END_SIGNATURE
                && at + END_LEN as usize + usize::from(le16(&tail, at + 20)) == tail.len()
        })
        .ok_or_else(|| {
            corrupt("there is no end-of-archive record: not a zip file, or cut short")
        })?;
    let end_offset = window_start + end_at as u64;

    let disk = le16(&tail, end_at + 4);
    let directory_disk = le16(&tail, end_at + 6);
    let entries_here = u64::from(le16(&tail, end_at + 8));
    let entries_total = u64::from(le16(&tail, end_at + 10));
    let directory_size = u64::from(le32(&tail, end_at + 12));
    let directory_offset = u64::from(le32(&tail, end_at + 16));

    // The record is the one in use when its locator is there, whatever the
    // plain record's fields say: a count of 65,535 is a count when nothing
    // points to a ZIP64 record, and only then.
    let (offset, size, count, records_end) = if has_zip64_locator(reader, end_offset)? {
        read_zip64_end(reader, end_offset)?
    } else {
        if disk != 0 || directory_disk != 0 || entries_here != entries_total {
            return Err(Error::Unsupported("an archive in several parts".into()));
        }
        (directory_offset, directory_size, entries_total, end_offset)
    };

    if count > limits.max_entries {
        return Err(Error::TooLarge(format!(
            "{count} entries, more than the {} allowed",
            limits.max_entries
        )));
    }
    if size > limits.max_directory_bytes {
        return Err(Error::TooLarge(format!(
            "a central directory of {size} bytes, more than the {} allowed",
            limits.max_directory_bytes
        )));
    }
    if offset.checked_add(size).is_none_or(|end| end > records_end) {
        return Err(corrupt("the central directory does not fit in the file"));
    }
    if count
        .checked_mul(CENTRAL_FIXED as u64)
        .is_none_or(|least| least > size)
    {
        return Err(corrupt(
            "the central directory is too small for its entry count",
        ));
    }
    Ok((offset, size, count))
}

fn has_zip64_locator<R: Read + Seek>(reader: &mut R, end_offset: u64) -> Result<bool, Error> {
    if end_offset < LOCATOR_LEN {
        return Ok(false);
    }
    reader.seek(SeekFrom::Start(end_offset - LOCATOR_LEN))?;
    let mut signature = [0u8; 4];
    reader
        .read_exact(&mut signature)
        .map_err(|e| short(e, "the ZIP64 locator"))?;
    Ok(u32::from_le_bytes(signature) == ZIP64_LOCATOR_SIGNATURE)
}

/// Reads the ZIP64 end record, the locator of which sits just before the
/// ordinary end record. Returns where the directory is and where the records
/// after the directory begin.
fn read_zip64_end<R: Read + Seek>(
    reader: &mut R,
    end_offset: u64,
) -> Result<(u64, u64, u64, u64), Error> {
    if end_offset < LOCATOR_LEN {
        return Err(corrupt("the ZIP64 end record is missing"));
    }
    let locator_offset = end_offset - LOCATOR_LEN;
    reader.seek(SeekFrom::Start(locator_offset))?;
    let mut locator = [0u8; LOCATOR_LEN as usize];
    reader
        .read_exact(&mut locator)
        .map_err(|e| short(e, "the ZIP64 locator"))?;
    if le32(&locator, 0) != ZIP64_LOCATOR_SIGNATURE {
        return Err(corrupt(
            "the archive points to a ZIP64 end record it does not have",
        ));
    }
    if le32(&locator, 4) != 0 || le32(&locator, 16) > 1 {
        return Err(Error::Unsupported("an archive in several parts".into()));
    }
    let record_offset = le64(&locator, 8);
    if record_offset
        .checked_add(56)
        .is_none_or(|end| end > locator_offset)
    {
        return Err(corrupt("the ZIP64 end record is out of place"));
    }
    reader.seek(SeekFrom::Start(record_offset))?;
    let mut record = [0u8; 56];
    reader
        .read_exact(&mut record)
        .map_err(|e| short(e, "the ZIP64 end record"))?;
    if le32(&record, 0) != ZIP64_END_SIGNATURE {
        return Err(corrupt(
            "the ZIP64 end record is not where the locator says",
        ));
    }
    if le32(&record, 16) != 0 || le32(&record, 20) != 0 || le64(&record, 24) != le64(&record, 32) {
        return Err(Error::Unsupported("an archive in several parts".into()));
    }
    Ok((
        le64(&record, 48),
        le64(&record, 40),
        le64(&record, 32),
        record_offset,
    ))
}

fn read_directory<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    size: u64,
    count: u64,
) -> Result<Vec<Entry>, Error> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut source = BufReader::with_capacity(64 * 1024, reader.take(size));
    let mut entries = Vec::with_capacity(count.min(1 << 16) as usize);
    let mut used: u64 = 0;
    for _ in 0..count {
        let mut fixed = [0u8; CENTRAL_FIXED];
        source
            .read_exact(&mut fixed)
            .map_err(|e| short(e, "the central directory"))?;
        if le32(&fixed, 0) != CENTRAL_SIGNATURE {
            return Err(corrupt("an entry of the central directory is malformed"));
        }
        let name_length = usize::from(le16(&fixed, 28));
        let extra_length = usize::from(le16(&fixed, 30));
        let comment_length = usize::from(le16(&fixed, 32));
        let mut name = vec![0u8; name_length];
        let mut extra = vec![0u8; extra_length];
        source
            .read_exact(&mut name)
            .map_err(|e| short(e, "the central directory"))?;
        source
            .read_exact(&mut extra)
            .map_err(|e| short(e, "the central directory"))?;
        io::copy(
            &mut (&mut source).take(comment_length as u64),
            &mut io::sink(),
        )?;
        used += (CENTRAL_FIXED + name_length + extra_length + comment_length) as u64;
        entries.push(parse_entry(&fixed, name, &extra)?);
    }
    if used != size {
        return Err(corrupt(
            "the central directory has bytes no entry accounts for",
        ));
    }
    Ok(entries)
}

fn parse_entry(fixed: &[u8; CENTRAL_FIXED], name: Vec<u8>, extra: &[u8]) -> Result<Entry, Error> {
    let shown = String::from_utf8_lossy(&name).into_owned();
    let made_by = le16(fixed, 4);
    let flags = le16(fixed, 8);
    let method = le16(fixed, 10);
    let (time, date) = (le16(fixed, 12), le16(fixed, 14));
    let crc32 = le32(fixed, 16);
    let mut compressed = u64::from(le32(fixed, 20));
    let mut uncompressed = u64::from(le32(fixed, 24));
    let disk = le16(fixed, 34);
    let external_attributes = le32(fixed, 38);
    let mut local_offset = u64::from(le32(fixed, 42));

    if flags & (FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION) != 0 {
        return Err(Error::Unsupported(format!("'{shown}' is encrypted")));
    }
    let method = match method {
        0 => Method::Stored,
        8 => Method::Deflated,
        other => {
            return Err(Error::Unsupported(format!(
                "'{shown}' uses compression method {other}"
            )));
        }
    };

    let mut unix_modified = None;
    let mut rest = extra;
    while !rest.is_empty() {
        if rest.len() < 4 {
            return Err(corrupt(format!("an extra field of '{shown}' is cut short")));
        }
        let (id, length) = (le16(rest, 0), usize::from(le16(rest, 2)));
        if rest.len() < 4 + length {
            return Err(corrupt(format!("an extra field of '{shown}' is cut short")));
        }
        let data = &rest[4..4 + length];
        rest = &rest[4 + length..];
        match id {
            ZIP64_FIELD => {
                let mut at = 0;
                let mut take = |wanted: usize| -> Result<u64, Error> {
                    let field = data.get(at..at + wanted).ok_or_else(|| {
                        corrupt(format!("the ZIP64 field of '{shown}' is too short"))
                    })?;
                    at += wanted;
                    Ok(if wanted == 8 {
                        le64(field, 0)
                    } else {
                        u64::from(le32(field, 0))
                    })
                };
                if uncompressed == MAX32 {
                    uncompressed = take(8)?;
                }
                if compressed == MAX32 {
                    compressed = take(8)?;
                }
                if local_offset == MAX32 {
                    local_offset = take(8)?;
                }
                if disk == 0xffff && take(4)? != 0 {
                    return Err(Error::Unsupported("an archive in several parts".into()));
                }
            }
            TIMESTAMP_FIELD if length >= 5 && data[0] & 1 != 0 => {
                unix_modified = Some(i64::from(i32::from_le_bytes([
                    data[1], data[2], data[3], data[4],
                ])));
            }
            _ => {}
        }
    }
    if disk != 0 && disk != 0xffff {
        return Err(Error::Unsupported("an archive in several parts".into()));
    }
    if compressed == MAX32 || uncompressed == MAX32 || local_offset == MAX32 {
        // The marker value without the field that holds the real one.
        return Err(corrupt(format!(
            "'{shown}' has a size or offset that its ZIP64 field does not give"
        )));
    }

    match method {
        Method::Stored if compressed != uncompressed => {
            return Err(corrupt(format!(
                "'{shown}' is stored, but its two sizes differ"
            )));
        }
        Method::Deflated
            if uncompressed > compressed.saturating_mul(MAX_EXPANSION).saturating_add(64) =>
        {
            return Err(corrupt(format!(
                "'{shown}' claims a size no compressed stream of its length can have"
            )));
        }
        _ => {}
    }

    Ok(Entry {
        name: name.into_boxed_slice(),
        flags,
        method,
        crc32,
        compressed_size: compressed,
        uncompressed_size: uncompressed,
        local_offset,
        time,
        date,
        unix_modified,
        made_by_unix: made_by >> 8 == 3,
        external_attributes,
    })
}

/// No entry's stored bytes may overlap another's, nor reach the directory.
/// Without this, a few kilobytes of archive can point many entries at the
/// same compressed data.
fn check_layout(entries: &[Entry], directory_offset: u64) -> Result<(), Error> {
    let mut order: Vec<u32> = (0..entries.len() as u32).collect();
    order.sort_unstable_by_key(|&i| entries[i as usize].local_offset);
    let floor_of = |entry: &Entry| {
        entry
            .local_offset
            .saturating_add(LOCAL_FIXED + entry.name.len() as u64)
            .saturating_add(entry.compressed_size)
    };
    for pair in order.windows(2) {
        let (before, after) = (&entries[pair[0] as usize], &entries[pair[1] as usize]);
        if floor_of(before) > after.local_offset {
            return Err(corrupt(format!(
                "'{}' and '{}' overlap in the file",
                before.name_lossy(),
                after.name_lossy()
            )));
        }
    }
    if let Some(&last) = order.last()
        && floor_of(&entries[last as usize]) > directory_offset
    {
        return Err(corrupt(format!(
            "the data of '{}' runs into the central directory",
            entries[last as usize].name_lossy()
        )));
    }
    Ok(())
}
