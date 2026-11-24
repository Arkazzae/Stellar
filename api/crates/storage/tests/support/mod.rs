//! What the test files share: scratch folders, readers that misbehave, and the
//! suite every store has to pass.
#![allow(dead_code)]

pub mod contract;
pub mod fake_s3;
pub mod points;
pub mod server;

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use stellarr_storage::{ByteRange, Error, ObjectStorage, StoredObject};
use tokio::io::{AsyncRead, ReadBuf};

/// A folder under the system's temporary folder that goes away with it.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "stellarr-storage-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub async fn put(store: &dyn ObjectStorage, key: &str, bytes: &[u8]) -> Result<(), Error> {
    let mut data = bytes;
    store.put(key, &mut data).await
}

pub async fn read(object: StoredObject) -> Vec<u8> {
    object.body.collect(u64::MAX).await.unwrap().to_vec()
}

pub async fn read_range(
    store: &dyn ObjectStorage,
    key: &str,
    range: ByteRange,
) -> Result<(StoredObject, Vec<u8>), Error> {
    let mut object = store.get(key, Some(range)).await?;
    let body = std::mem::replace(
        &mut object.body,
        stellarr_storage::ObjectBody::from_bytes(Vec::new()),
    );
    let bytes = body.collect(u64::MAX).await?.to_vec();
    Ok((object, bytes))
}

/// Bytes that look like nothing in particular and are the same every time.
pub fn pattern(length: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// Gives a few bytes and then fails, as a client that drops mid-upload does.
pub struct FailingReader {
    pub before_failure: usize,
}

impl AsyncRead for FailingReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.before_failure == 0 {
            return Poll::Ready(Err(std::io::Error::other("the client went away")));
        }
        let n = self.before_failure.min(buf.remaining());
        buf.put_slice(&vec![7u8; n]);
        self.before_failure -= n;
        Poll::Ready(Ok(()))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    stellarr_s3::internals::hex(&stellarr_s3::sha256(bytes))
}
