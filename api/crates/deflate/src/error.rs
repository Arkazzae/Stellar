use std::fmt;
use std::io;

/// Why a compressed stream was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended before the stream did.
    Truncated,
    /// A block type of 3, which the format reserves.
    InvalidBlockType,
    /// A stored block whose length and its complement disagree.
    StoredLengthMismatch,
    /// A dynamic block that announces more codes than the format has.
    TooManySymbols,
    /// A set of code lengths that cannot be a prefix code: more codes than
    /// the bit lengths leave room for.
    OversubscribedCode,
    /// A set of code lengths that leaves bit patterns unassigned, where the
    /// format only allows that for a lone code of length one.
    IncompleteCode,
    /// A repeat code with nothing to repeat, or one that runs past the end of
    /// the table, or a literal/length table without an end-of-block code.
    BadCodeLengths,
    /// A bit pattern no code was assigned to, or a symbol the format reserves.
    InvalidSymbol,
    /// A match that reaches back before the start of the output.
    DistanceTooFar,
    /// The output would exceed the cap the caller set.
    OutputLimit,
    /// A gzip or zlib header that is not one, or asks for something this
    /// reader does not do.
    BadHeader(&'static str),
    /// The checksum at the end does not match the data.
    ChecksumMismatch,
    /// The length at the end of a gzip member does not match the data.
    LengthMismatch,
    /// Bytes after the end of the stream that are not another gzip member.
    TrailingData,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("the compressed stream ends early"),
            Self::InvalidBlockType => f.write_str("invalid deflate block type"),
            Self::StoredLengthMismatch => {
                f.write_str("stored block length does not match its complement")
            }
            Self::TooManySymbols => f.write_str("a block declares more codes than the format has"),
            Self::OversubscribedCode => {
                f.write_str("a Huffman table has more codes than its lengths allow")
            }
            Self::IncompleteCode => f.write_str("a Huffman table leaves bit patterns unassigned"),
            Self::BadCodeLengths => f.write_str("invalid code lengths in a dynamic block"),
            Self::InvalidSymbol => f.write_str("invalid Huffman code in the compressed stream"),
            Self::DistanceTooFar => {
                f.write_str("a match reaches back before the start of the output")
            }
            Self::OutputLimit => f.write_str("the decompressed data exceeds the allowed size"),
            Self::BadHeader(why) => write!(f, "invalid stream header: {why}"),
            Self::ChecksumMismatch => f.write_str("the stream's checksum does not match its data"),
            Self::LengthMismatch => f.write_str("the stream's length does not match its data"),
            Self::TrailingData => f.write_str("unexpected data after the end of the stream"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        let kind = if error == Error::Truncated {
            io::ErrorKind::UnexpectedEof
        } else {
            io::ErrorKind::InvalidData
        };
        io::Error::new(kind, error)
    }
}

impl Error {
    /// The decoding error an `io::Error` from one of this crate's readers
    /// carries, if it carries one; `None` for a failure of the input itself.
    pub fn from_io(error: &io::Error) -> Option<Error> {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Error>())
            .copied()
    }
}
