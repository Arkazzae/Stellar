//! Where uploaded files are kept.
//!
//! Everything people upload is an object under a key, in a folder of the data
//! volume ([`FolderStore`]) or in an S3-compatible bucket ([`BucketStore`]).
//! Both keep the same promises, written down once in [`ObjectStorage`]; the
//! same test suite runs against each.
//!
//! The folder store's layout on disk is part of the interface: a key
//! `audio/2026/11/x.webm` is the file `files/audio/2026/11/x.webm`, and files
//! being written are staged in `files/.incoming`. An installation's files are
//! readable by anything that follows this layout.
//!
//! Nothing here holds an object in memory whole. The folder store moves
//! 128 KiB at a time. The bucket store holds one part of a large upload
//! (8 MiB unless set otherwise) and passes downloads through as they come.
//!
//! [`validation`] decides what may be uploaded at all: the allow-list of
//! types and the signature each file has to start with.
//!
//! # How the folder store runs file calls
//!
//! File calls block, so none of them may run on an async worker. Each read,
//! write, sync and open goes through `tokio::fs`, which hands the call to
//! tokio's blocking pool and buffers in between; a hand-written
//! `spawn_blocking` around every read and write would do the same with more
//! code and the same cost. Work that touches many files (listing, and the
//! final link and clean-up of a write) is one blocking job, not one job per
//! file, so a thousand-file backup walk costs one hand-off.

mod body;
mod bucket;
mod error;
mod folder;
pub mod key;
mod settings;
pub mod validation;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::io::AsyncRead;

pub use body::ObjectBody;
pub use bucket::{BucketStore, BucketTuning, REGION};
pub use error::{Error, Result, StoreFault};
pub use folder::{FOLDER_NAME, FolderStore};
pub use settings::{StorageOptions, StorageProvider, UploadOptions};

/// A boxed future, which is what keeps [`ObjectStorage`] usable as
/// `Arc<dyn ObjectStorage>`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A byte range as an HTTP `Range` header describes it: both ends are
/// inclusive, and no end runs to the end of the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub from: u64,
    pub to: Option<u64>,
}

/// One object's bytes, plus what a response needs to describe them. The store
/// keeps bytes only: what they are, their content type included, is in the
/// row that names the key.
#[derive(Debug)]
pub struct StoredObject {
    pub body: ObjectBody,
    /// Bytes in this response, so the slice rather than the object.
    pub length: u64,
    /// Bytes in the whole object.
    pub total_length: u64,
    /// The slice that was served, or `None` when the whole object was.
    pub range: Option<ByteRange>,
}

/// An object as a listing names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObjectInfo {
    pub key: String,
    pub size: u64,
}

/// Every object of a store, in key order, read page by page.
pub struct Listing {
    pages: Box<dyn ListPages>,
    page: std::vec::IntoIter<StoredObjectInfo>,
    finished: bool,
}

/// The pages of a listing, for a store to provide.
pub trait ListPages: Send {
    /// The next page, or `None` when there is no more.
    fn next_page(&mut self) -> BoxFuture<'_, Result<Option<Vec<StoredObjectInfo>>>>;
}

impl Listing {
    pub fn new(pages: impl ListPages + 'static) -> Self {
        Self {
            pages: Box::new(pages),
            page: Vec::new().into_iter(),
            finished: false,
        }
    }

    /// A listing that is already complete.
    pub fn of(objects: Vec<StoredObjectInfo>) -> Self {
        struct Whole(Option<Vec<StoredObjectInfo>>);
        impl ListPages for Whole {
            fn next_page(&mut self) -> BoxFuture<'_, Result<Option<Vec<StoredObjectInfo>>>> {
                let page = self.0.take();
                Box::pin(async move { Ok(page) })
            }
        }
        Self::new(Whole(Some(objects)))
    }

    /// The next object, or `None` at the end.
    pub async fn next(&mut self) -> Result<Option<StoredObjectInfo>> {
        loop {
            if let Some(object) = self.page.next() {
                return Ok(Some(object));
            }
            if self.finished {
                return Ok(None);
            }
            match self.pages.next_page().await? {
                Some(page) => self.page = page.into_iter(),
                None => self.finished = true,
            }
        }
    }

    /// Every object. Holds the whole listing; for tests and small stores.
    pub async fn collect(mut self) -> Result<Vec<StoredObjectInfo>> {
        let mut all = Vec::new();
        while let Some(object) = self.next().await? {
            all.push(object);
        }
        Ok(all)
    }
}

/// What every object store promises. Dropping a returned future cancels the
/// call. A write cancelled on its way leaves no object and nothing staged; one
/// cancelled in its last step (the link into place, or the request that
/// completes a multipart upload) may still land, so a caller that gives up
/// on a write deletes its key.
pub trait ObjectStorage: Send + Sync {
    /// Gets the store ready at start-up, creating what it needs; fails while
    /// it cannot be used.
    fn prepare(&self) -> BoxFuture<'_, Result<()>>;

    /// Stores the rest of `data` under a key that names no object yet, and
    /// fails with [`Error::AlreadyExists`] for one that does. The object
    /// appears only once it is whole.
    ///
    /// Of two writers racing for one key, exactly one wins on the folder
    /// store, and on a bucket whose server honours the conditional create
    /// the bucket store sends (`If-None-Match: *`). A server that ignores
    /// that header replaces the object instead, and nothing here can tell; the
    /// operator's choice of server decides.
    fn put<'a>(
        &'a self,
        key: &'a str,
        data: &'a mut (dyn AsyncRead + Send + Unpin),
    ) -> BoxFuture<'a, Result<()>>;

    /// Reads an object, or only the requested part of it. The range reaches
    /// the store instead of being applied to the returned bytes, so that a
    /// browser's seek in a media file costs the slice and not the file.
    ///
    /// The end of a range is clamped to the object, as S3 does. A range that
    /// starts at or past the end, an empty object's included, is
    /// [`Error::RangeNotSatisfiable`].
    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<ByteRange>,
    ) -> BoxFuture<'a, Result<StoredObject>>;

    /// Removes an object. A key that names none is already removed, so that
    /// is no error.
    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>>;

    /// Every object in the store, in key order (byte order, as an S3 listing
    /// has it), so a backup misses none a table forgot. Keys no store takes
    /// are left out.
    fn list(&self) -> Listing;

    /// Asks the store whether it answers, without changing anything; fails
    /// when it does not.
    fn probe(&self) -> BoxFuture<'_, Result<()>>;
}

/// The store the settings name: the folder `files` under `data_root`, or the
/// bucket. `http` carries the bucket store's requests and is not used for the
/// folder.
pub fn open(
    options: &StorageOptions,
    data_root: &std::path::Path,
    http: stellarr_outbound::Client,
) -> Result<Arc<dyn ObjectStorage>> {
    Ok(match options.provider {
        StorageProvider::FileSystem => Arc::new(FolderStore::new(data_root.join(FOLDER_NAME))),
        StorageProvider::S3 => Arc::new(BucketStore::new(options, http)?),
    })
}
