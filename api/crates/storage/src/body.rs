//! The bytes of a stored object, read piece by piece.

use std::io::SeekFrom;

use bytes::{Bytes, BytesMut};
use stellarr_outbound::ResponseBody;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::error::{Error, Result, StoreFault};

/// The most one read of a file returns.
const FILE_CHUNK: usize = 128 * 1024;

/// The bytes of an object, or of the slice of it that was asked for, to be
/// read in order. Dropping it closes the file or the connection.
///
/// It is not an `AsyncRead` on purpose: the bucket store hands over the
/// pieces as the network delivered them, and a caller that writes them to an
/// HTTP response wants them as they are.
pub struct ObjectBody {
    inner: Inner,
}

enum Inner {
    Memory(Option<Bytes>),
    File {
        file: File,
        remaining: u64,
    },
    Http {
        body: ResponseBody,
        expected: u64,
        seen: u64,
    },
}

impl ObjectBody {
    /// A body that is already in memory, for stores that keep objects there.
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self {
            inner: Inner::Memory(Some(bytes.into())),
        }
    }

    pub(crate) async fn from_file(mut file: File, from: u64, length: u64) -> Result<Self> {
        if from > 0 {
            file.seek(SeekFrom::Start(from)).await?;
        }
        Ok(Self {
            inner: Inner::File {
                file,
                remaining: length,
            },
        })
    }

    pub(crate) fn from_http(body: ResponseBody, expected: u64) -> Self {
        Self {
            inner: Inner::Http {
                body,
                expected,
                seen: 0,
            },
        }
    }

    /// The next piece, or `None` once every byte has been given. A body that
    /// ends before its announced length, or runs past it, is an error: a
    /// download cut short must not look complete.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>> {
        match &mut self.inner {
            Inner::Memory(bytes) => Ok(bytes.take().filter(|bytes| !bytes.is_empty())),
            Inner::File { file, remaining } => {
                if *remaining == 0 {
                    return Ok(None);
                }
                let want = usize::try_from(*remaining)
                    .unwrap_or(usize::MAX)
                    .min(FILE_CHUNK);
                let mut buffer = BytesMut::with_capacity(want);
                let read = file.take(want as u64).read_buf(&mut buffer).await?;
                if read == 0 {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the file is shorter than it was a moment ago",
                    )));
                }
                *remaining -= read as u64;
                Ok(Some(buffer.freeze()))
            }
            Inner::Http {
                body,
                expected,
                seen,
            } => match body.chunk().await {
                Ok(Some(piece)) => {
                    *seen += piece.len() as u64;
                    if *seen > *expected {
                        return Err(Error::store(
                            StoreFault::Malformed,
                            "the store sent more bytes than it announced",
                        ));
                    }
                    Ok(Some(piece))
                }
                Ok(None) if *seen == *expected => Ok(None),
                Ok(None) => Err(Error::store(
                    StoreFault::Unreachable,
                    "the store ended the object before its announced length",
                )),
                Err(error) => Err(crate::bucket::from_outbound(&error)),
            },
        }
    }

    /// Reads everything, refusing more than `limit` bytes. For small objects
    /// and tests: a large object is read with [`chunk`](Self::chunk).
    pub async fn collect(mut self, limit: u64) -> Result<Bytes> {
        let mut all = BytesMut::new();
        while let Some(piece) = self.chunk().await? {
            if all.len() as u64 + piece.len() as u64 > limit {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the object is larger than the limit given",
                )));
            }
            all.extend_from_slice(&piece);
        }
        Ok(all.freeze())
    }
}

impl std::fmt::Debug for ObjectBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObjectBody")
    }
}
