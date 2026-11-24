//! DEFLATE (RFC 1951) in both directions, with the gzip and zlib wrappers for
//! reading and CRC-32, using nothing but the standard library.
//!
//! * [`Inflate`] decodes a stream, strictly: every invalid stream is an
//!   [`Error`], and the caller can cap how much it may produce.
//! * [`Deflater`] encodes a stream, writing as it goes.
//! * [`GzipReader`] and [`ZlibReader`] read the wrapped forms, checking the
//!   trailer's checksum and length.
//! * [`Crc32`] and [`Adler32`] are the checksums.

mod adler32;
mod crc32;
mod deflate;
mod error;
mod huffman;
mod inflate;
mod tables;
mod wrappers;

pub use adler32::Adler32;
pub use crc32::{Crc32, crc32};
pub use deflate::{Deflater, Level, deflate_to_vec};
pub use error::Error;
pub use inflate::{Inflate, inflate_to_vec};
pub use wrappers::{GzipReader, ZlibReader, gunzip_to_vec, zlib_to_vec};
