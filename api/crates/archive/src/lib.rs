//! Zip files for backups: a writer that streams entries of any size to a
//! file, and a reader that goes by the central directory and checks every
//! length and CRC-32 it hands out.
//!
//! * Entries are stored or deflated; the sizes need not be known before the
//!   data is written. ZIP64 records appear wherever a size, an offset or the
//!   entry count needs them.
//! * Names are UTF-8 when written and are given back exactly as stored.
//!   Deciding whether a name is safe to unpack is the caller's job; the reader
//!   hands over the bytes, whether the entry is a folder or a symbolic link,
//!   and a way to find duplicate names.
//! * Encryption, other compression methods and archives in several parts are
//!   refused with [`Error::Unsupported`].
//!
//! All the I/O here blocks; run it on a blocking thread.

mod error;
mod reader;
mod time;
mod writer;

pub use error::Error;
pub use reader::{Entry, EntryReader, Limits, Method, ZipArchive};
pub use stellarr_deflate::Level;
pub use writer::{Compression, EntryInfo, EntryOptions, EntryWriter, ZipWriter};
