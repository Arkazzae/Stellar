//! Writing a zip file to anything that can be written to, one entry at a
//! time, without knowing any entry's size in advance and without seeking.
//!
//! Each entry's sizes and checksum follow its data in a data descriptor, and
//! every local header carries the ZIP64 field that makes the descriptor's
//! sizes eight bytes wide, so an entry may be of any size. The central
//! directory at the end repeats what a reader needs and is the only part a
//! reader goes by; it uses ZIP64 fields exactly where a size, an offset or
//! the number of entries calls for them.

use std::io::{self, Write};

use stellarr_deflate::{Crc32, Deflater, Level};

use crate::error::Error;
use crate::time::dos_from_unix;

pub(crate) const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
pub(crate) const DESCRIPTOR_SIGNATURE: u32 = 0x0807_4b50;
pub(crate) const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
pub(crate) const END_SIGNATURE: u32 = 0x0605_4b50;
pub(crate) const ZIP64_END_SIGNATURE: u32 = 0x0606_4b50;
pub(crate) const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;

pub(crate) const ZIP64_FIELD: u16 = 0x0001;
pub(crate) const TIMESTAMP_FIELD: u16 = 0x5455;

pub(crate) const FLAG_DESCRIPTOR: u16 = 0x0008;
pub(crate) const FLAG_UTF8: u16 = 0x0800;

/// A 32-bit field holding this value says the real one is in the ZIP64 field.
const MAX32: u64 = 0xffff_ffff;
const MAX16: u64 = 0xffff;

const VERSION_ZIP64: u16 = 45;
const VERSION_PLAIN: u16 = 20;
/// Made by Unix, specification 6.3: what lets the external attributes carry a
/// file mode.
const MADE_BY_UNIX: u16 = (3 << 8) | 63;

/// How an entry's bytes are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Stored,
    Deflated(Level),
}

/// What is written about an entry besides its bytes.
#[derive(Clone, Copy, Debug)]
pub struct EntryOptions {
    compression: Compression,
    modified: Option<i64>,
    unix_mode: Option<u32>,
}

impl EntryOptions {
    pub fn stored() -> Self {
        Self {
            compression: Compression::Stored,
            modified: None,
            unix_mode: None,
        }
    }

    pub fn deflated(level: Level) -> Self {
        Self {
            compression: Compression::Deflated(level),
            modified: None,
            unix_mode: None,
        }
    }

    /// The modification time, in seconds since 1970-01-01 UTC. Without one,
    /// the entry is dated 1980-01-01, the earliest date a zip can hold.
    #[must_use]
    pub fn modified(mut self, unix_seconds: i64) -> Self {
        self.modified = Some(unix_seconds);
        self
    }

    /// The Unix file mode, permissions and file type bits, kept for tools
    /// that restore it. Without one an entry is given 0644, or 0755 for a
    /// folder; secrets should say 0600 here.
    #[must_use]
    pub fn unix_mode(mut self, mode: u32) -> Self {
        self.unix_mode = Some(mode);
        self
    }
}

/// The most an entry adds to the archive beyond its data, for a name of
/// `name_len` bytes: local header, data descriptor and central record. For a
/// caller that checks the disk has room before it starts.
pub const fn entry_overhead(name_len: usize) -> u64 {
    166 + 2 * name_len as u64
}

/// The most the end of the archive takes: the ZIP64 records and the end
/// record.
pub const ARCHIVE_OVERHEAD: u64 = 98;

/// What an entry came to, as written in its descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryInfo {
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

struct CountingWriter<W> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// An entry that has its header written and its data coming.
struct Open {
    name: Vec<u8>,
    compression: Compression,
    flags: u16,
    time: u16,
    date: u16,
    modified: Option<i64>,
    unix_mode: Option<u32>,
    header_offset: u64,
    data_offset: u64,
}

/// Writes a zip file to `W`.
pub struct ZipWriter<W: Write> {
    out: CountingWriter<W>,
    /// The central directory, built as entries are finished.
    central: Vec<u8>,
    entries: u64,
    force_zip64: bool,
    open: Option<Open>,
}

impl<W: Write> ZipWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out: CountingWriter {
                inner: out,
                written: 0,
            },
            central: Vec::new(),
            entries: 0,
            force_zip64: false,
            open: None,
        }
    }

    /// Writes every record in its ZIP64 form even where the sizes would fit.
    /// The result is as valid as one without; it exists so that readers of
    /// the ZIP64 form can be tried on small files.
    #[must_use]
    pub fn force_zip64(mut self, force: bool) -> Self {
        self.force_zip64 = force;
        self
    }

    /// Bytes written so far.
    pub fn bytes_written(&self) -> u64 {
        self.out.written
    }

    /// Starts an entry. Its bytes are written through the returned writer,
    /// which must be finished before the next entry starts.
    pub fn start_entry(
        &mut self,
        name: &str,
        options: EntryOptions,
    ) -> Result<EntryWriter<'_, W>, Error> {
        let name = name.as_bytes();
        if self.open.is_some() {
            return Err(Error::Invalid("the previous entry was not finished".into()));
        }
        if name.is_empty() || name.len() > usize::from(u16::MAX) || name.contains(&0) {
            return Err(Error::Invalid(format!(
                "'{}' cannot be an entry name",
                String::from_utf8_lossy(name)
            )));
        }
        let (time, date) = dos_from_unix(options.modified.unwrap_or(0));
        let mut flags = FLAG_DESCRIPTOR;
        if !name.is_ascii() {
            flags |= FLAG_UTF8;
        }
        let header_offset = self.out.written;

        let mut extra = Vec::with_capacity(29);
        // Sizes are not known yet: the field is there, empty, to say that the
        // descriptor after the data has them in eight bytes.
        extra.extend_from_slice(&ZIP64_FIELD.to_le_bytes());
        extra.extend_from_slice(&16u16.to_le_bytes());
        extra.extend_from_slice(&[0; 16]);
        if let Some(modified) = options.modified.and_then(|m| i32::try_from(m).ok()) {
            extra.extend_from_slice(&TIMESTAMP_FIELD.to_le_bytes());
            extra.extend_from_slice(&5u16.to_le_bytes());
            extra.push(1);
            extra.extend_from_slice(&modified.to_le_bytes());
        }

        let method: u16 = match options.compression {
            Compression::Stored => 0,
            Compression::Deflated(_) => 8,
        };
        let mut header = Vec::with_capacity(30 + name.len() + extra.len());
        header.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
        header.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
        header.extend_from_slice(&flags.to_le_bytes());
        header.extend_from_slice(&method.to_le_bytes());
        header.extend_from_slice(&time.to_le_bytes());
        header.extend_from_slice(&date.to_le_bytes());
        header.extend_from_slice(&[0; 12]); // checksum and sizes follow the data
        header.extend_from_slice(&(name.len() as u16).to_le_bytes());
        header.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        header.extend_from_slice(name);
        header.extend_from_slice(&extra);
        self.out.write_all(&header)?;

        self.open = Some(Open {
            name: name.to_vec(),
            compression: options.compression,
            flags,
            time,
            date,
            modified: options.modified,
            unix_mode: options.unix_mode,
            header_offset,
            data_offset: self.out.written,
        });
        let sink = match options.compression {
            Compression::Stored => Sink::Stored,
            Compression::Deflated(level) => {
                Sink::Deflated(Box::new(Deflater::new(Vec::new(), level)))
            }
        };
        Ok(EntryWriter {
            zip: self,
            sink,
            crc: Crc32::new(),
            uncompressed: 0,
        })
    }

    /// Writes a whole entry from memory.
    pub fn add_bytes(
        &mut self,
        name: &str,
        options: EntryOptions,
        data: &[u8],
    ) -> Result<EntryInfo, Error> {
        let mut entry = self.start_entry(name, options)?;
        entry.write_all(data)?;
        entry.finish()
    }

    /// Writes a whole entry from a reader, in pieces.
    pub fn add_reader<R: io::Read>(
        &mut self,
        name: &str,
        options: EntryOptions,
        reader: &mut R,
    ) -> Result<EntryInfo, Error> {
        let mut entry = self.start_entry(name, options)?;
        io::copy(reader, &mut entry)?;
        entry.finish()
    }

    /// Writes the central directory and ends the archive.
    pub fn finish(mut self) -> Result<W, Error> {
        if self.open.is_some() {
            return Err(Error::Invalid("the last entry was not finished".into()));
        }
        let directory_offset = self.out.written;
        let directory_size = self.central.len() as u64;
        let central = std::mem::take(&mut self.central);
        self.out.write_all(&central)?;

        let needs_zip64 = self.force_zip64
            || self.entries >= MAX16
            || directory_offset >= MAX32
            || directory_size >= MAX32;
        if needs_zip64 {
            let record_offset = self.out.written;
            let mut record = Vec::with_capacity(56 + 20);
            record.extend_from_slice(&ZIP64_END_SIGNATURE.to_le_bytes());
            record.extend_from_slice(&44u64.to_le_bytes());
            record.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
            record.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
            record.extend_from_slice(&0u32.to_le_bytes());
            record.extend_from_slice(&0u32.to_le_bytes());
            record.extend_from_slice(&self.entries.to_le_bytes());
            record.extend_from_slice(&self.entries.to_le_bytes());
            record.extend_from_slice(&directory_size.to_le_bytes());
            record.extend_from_slice(&directory_offset.to_le_bytes());
            record.extend_from_slice(&ZIP64_LOCATOR_SIGNATURE.to_le_bytes());
            record.extend_from_slice(&0u32.to_le_bytes());
            record.extend_from_slice(&record_offset.to_le_bytes());
            record.extend_from_slice(&1u32.to_le_bytes());
            self.out.write_all(&record)?;
        }

        let mut end = Vec::with_capacity(22);
        end.extend_from_slice(&END_SIGNATURE.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        let count16 = self.entries.min(MAX16) as u16;
        end.extend_from_slice(&count16.to_le_bytes());
        end.extend_from_slice(&count16.to_le_bytes());
        end.extend_from_slice(&(directory_size.min(MAX32) as u32).to_le_bytes());
        end.extend_from_slice(&(directory_offset.min(MAX32) as u32).to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        self.out.write_all(&end)?;
        self.out.flush()?;
        Ok(self.out.inner)
    }

    fn finish_entry(&mut self, info: EntryInfo) -> Result<(), Error> {
        let open = self
            .open
            .take()
            .ok_or_else(|| Error::Invalid("no entry is open".into()))?;
        let mut descriptor = Vec::with_capacity(24);
        descriptor.extend_from_slice(&DESCRIPTOR_SIGNATURE.to_le_bytes());
        descriptor.extend_from_slice(&info.crc32.to_le_bytes());
        descriptor.extend_from_slice(&info.compressed_size.to_le_bytes());
        descriptor.extend_from_slice(&info.uncompressed_size.to_le_bytes());
        self.out.write_all(&descriptor)?;

        let sizes_need =
            self.force_zip64 || info.compressed_size >= MAX32 || info.uncompressed_size >= MAX32;
        let offset_needs = self.force_zip64 || open.header_offset >= MAX32;

        let mut extra = Vec::with_capacity(48);
        if sizes_need || offset_needs {
            let mut data = Vec::with_capacity(24);
            if sizes_need {
                data.extend_from_slice(&info.uncompressed_size.to_le_bytes());
                data.extend_from_slice(&info.compressed_size.to_le_bytes());
            }
            if offset_needs {
                data.extend_from_slice(&open.header_offset.to_le_bytes());
            }
            extra.extend_from_slice(&ZIP64_FIELD.to_le_bytes());
            extra.extend_from_slice(&(data.len() as u16).to_le_bytes());
            extra.extend_from_slice(&data);
        }
        if let Some(modified) = open.modified.and_then(|m| i32::try_from(m).ok()) {
            extra.extend_from_slice(&TIMESTAMP_FIELD.to_le_bytes());
            extra.extend_from_slice(&5u16.to_le_bytes());
            extra.push(1);
            extra.extend_from_slice(&modified.to_le_bytes());
        }

        let method: u16 = match open.compression {
            Compression::Stored => 0,
            Compression::Deflated(_) => 8,
        };
        let is_directory = open.name.ends_with(b"/");
        // Every entry says it was made on Unix, with a mode: tools that read
        // names in the DOS code page do so for entries made elsewhere. An
        // entry given no mode gets the usual one for a file or a folder.
        let mode = open
            .unix_mode
            .unwrap_or(if is_directory { 0o040_755 } else { 0o100_644 });
        let external = (mode << 16) | (u32::from(is_directory) * 0x10);
        let record = &mut self.central;
        record.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
        record.extend_from_slice(&MADE_BY_UNIX.to_le_bytes());
        let version = if extra.starts_with(&ZIP64_FIELD.to_le_bytes()) {
            VERSION_ZIP64
        } else {
            VERSION_PLAIN
        };
        record.extend_from_slice(&version.to_le_bytes());
        record.extend_from_slice(&open.flags.to_le_bytes());
        record.extend_from_slice(&method.to_le_bytes());
        record.extend_from_slice(&open.time.to_le_bytes());
        record.extend_from_slice(&open.date.to_le_bytes());
        record.extend_from_slice(&info.crc32.to_le_bytes());
        let size32 = |size: u64, needs: bool| if needs { MAX32 as u32 } else { size as u32 };
        record.extend_from_slice(&size32(info.compressed_size, sizes_need).to_le_bytes());
        record.extend_from_slice(&size32(info.uncompressed_size, sizes_need).to_le_bytes());
        record.extend_from_slice(&(open.name.len() as u16).to_le_bytes());
        record.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes()); // comment
        record.extend_from_slice(&0u16.to_le_bytes()); // first disk
        record.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        record.extend_from_slice(&external.to_le_bytes());
        record.extend_from_slice(&size32(open.header_offset, offset_needs).to_le_bytes());
        record.extend_from_slice(&open.name);
        record.extend_from_slice(&extra);
        self.entries += 1;
        Ok(())
    }
}

enum Sink {
    Stored,
    /// The encoder writes into a buffer that is moved on to the archive
    /// after every write, so the archive is never borrowed twice.
    Deflated(Box<Deflater<Vec<u8>>>),
}

/// The bytes of one entry, on their way into the archive.
pub struct EntryWriter<'a, W: Write> {
    zip: &'a mut ZipWriter<W>,
    sink: Sink,
    crc: Crc32,
    uncompressed: u64,
}

impl<W: Write> EntryWriter<'_, W> {
    /// Ends the entry: writes what follows its data, and records it in the
    /// central directory.
    pub fn finish(mut self) -> Result<EntryInfo, Error> {
        let data_offset = self.zip.open.as_ref().map_or(0, |open| open.data_offset);
        if let Sink::Deflated(deflater) = std::mem::replace(&mut self.sink, Sink::Stored) {
            let rest = deflater.finish()?;
            self.zip.out.write_all(&rest)?;
        }
        let info = EntryInfo {
            crc32: self.crc.value(),
            compressed_size: self.zip.out.written - data_offset,
            uncompressed_size: self.uncompressed,
        };
        self.zip.finish_entry(info)?;
        Ok(info)
    }
}

impl<W: Write> Write for EntryWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.sink {
            Sink::Stored => self.zip.out.write_all(buf)?,
            Sink::Deflated(deflater) => {
                deflater.write_all(buf)?;
                let pending = deflater.get_mut();
                self.zip.out.write_all(pending)?;
                pending.clear();
            }
        }
        self.crc.update(buf);
        self.uncompressed += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
