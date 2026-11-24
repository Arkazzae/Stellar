//! The folder store: every object is a plain file in one folder of the data
//! volume, at the path its key names.
//!
//! A file appears under its key only once it is whole. It is written in
//! `.incoming`, flushed to the disk and then linked into place, and nobody
//! sees a move half done. The link itself is not flushed, so a power cut right
//! after an upload can lose that one file; its row then answers "not found",
//! as it would for a file deleted by hand.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::body::ObjectBody;
use crate::error::{Error, Result};
use crate::key::{self, INCOMING_FOLDER};
use crate::{
    BoxFuture, ByteRange, ListPages, Listing, ObjectStorage, StoredObject, StoredObjectInfo,
};

/// The folder of the data volume that holds the files.
pub const FOLDER_NAME: &str = "files";

/// How much of an upload is read before it is written.
const COPY_CHUNK: usize = 128 * 1024;

#[derive(Clone)]
pub struct FolderStore {
    inner: Arc<Inner>,
}

struct Inner {
    root: PathBuf,
    /// Where files are written before they get their key. It sits inside the
    /// root, so that the link that publishes a file never crosses file
    /// systems.
    incoming: PathBuf,
    staged: AtomicU64,
}

impl FolderStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let absolute = std::path::absolute(root).unwrap_or_else(|_| root.to_owned());
        // Without a trailing separator, and so comparable with the paths a
        // directory walk yields.
        let root: PathBuf = absolute.components().collect();
        Self {
            inner: Arc::new(Inner {
                incoming: root.join(INCOMING_FOLDER),
                root,
                staged: AtomicU64::new(0),
            }),
        }
    }

    /// The folder every object lives in.
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    fn path_of(&self, key: &str) -> Result<PathBuf> {
        key::check(key)?;
        let mut path = self.inner.root.clone();
        path.extend(key.split('/'));
        Ok(path)
    }

    /// A name for a file being written: 32 hex digits made of the time, the
    /// process and a counter. `create_new` is what guarantees it is unused.
    fn staging_name(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let count = self.inner.staged.fetch_add(1, Ordering::Relaxed);
        format!("{nanos:016x}{:08x}{:08x}", std::process::id(), count as u32)
    }

    async fn stage(&self) -> io::Result<(tokio::fs::File, PathBuf)> {
        fs::create_dir_all(&self.inner.incoming).await?;
        loop {
            let path = self.inner.incoming.join(self.staging_name());
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(file) => return Ok((file, path)),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    async fn put(&self, key: &str, data: &mut (dyn AsyncRead + Send + Unpin)) -> Result<()> {
        let path = self.path_of(key)?;
        // Refuse before reading a byte of what may be a large upload. The
        // link below is what decides, for a write that races this one.
        if fs::symlink_metadata(&path).await.is_ok() {
            return Err(Error::AlreadyExists(key.to_owned()));
        }

        let (mut file, staged_path) = self.stage().await?;
        let mut staged = Staged::new(staged_path);
        let mut buffer = vec![0u8; COPY_CHUNK];
        loop {
            let read = data.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read]).await?;
        }
        drop(buffer);
        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        let from = staged.path.clone();
        let owned_key = key.to_owned();
        tokio::task::spawn_blocking(move || publish(&from, &path, &owned_key))
            .await
            .map_err(|_| Error::Io(io::Error::other("the publishing job was cancelled")))??;
        staged.disarm();
        Ok(())
    }

    async fn get(&self, key: &str, range: Option<ByteRange>) -> Result<StoredObject> {
        let path = self.path_of(key)?;
        let absent = || Error::NotFound(key.to_owned());
        // A key that is the folder of other keys names no object, and opening
        // a directory succeeds on this platform, so ask first.
        match fs::metadata(&path).await {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(absent()),
            Err(error) if names_nothing(&error) => return Err(absent()),
            Err(error) => return Err(error.into()),
        }
        let file = match fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) if names_nothing(&error) => return Err(absent()),
            Err(error) => return Err(error.into()),
        };
        let total = file.metadata().await?.len();
        let Some(wanted) = range else {
            return Ok(StoredObject {
                body: ObjectBody::from_file(file, 0, total).await?,
                length: total,
                total_length: total,
                range: None,
            });
        };
        if wanted.from >= total || wanted.to.is_some_and(|to| to < wanted.from) {
            return Err(Error::RangeNotSatisfiable(key.to_owned()));
        }
        // The end is clamped to the file, as an S3 store does.
        let to = wanted.to.unwrap_or(total - 1).min(total - 1);
        let length = to - wanted.from + 1;
        Ok(StoredObject {
            body: ObjectBody::from_file(file, wanted.from, length).await?,
            length,
            total_length: total,
            range: Some(ByteRange {
                from: wanted.from,
                to: Some(to),
            }),
        })
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.path_of(key)?;
        match fs::symlink_metadata(&path).await {
            // A folder of other keys is no object, and removing it as a file fails.
            Ok(meta) if meta.is_dir() => return Ok(()),
            Ok(_) => {}
            Err(error) if names_nothing(&error) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if names_nothing(&error) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn prepare(&self) -> Result<()> {
        let incoming = self.inner.incoming.clone();
        tokio::task::spawn_blocking(move || -> io::Result<()> {
            std::fs::create_dir_all(&incoming)?;
            for entry in std::fs::read_dir(&incoming)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    continue;
                }
                std::fs::remove_file(entry.path())?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Io(io::Error::other("the preparing job was cancelled")))??;
        Ok(())
    }

    async fn probe(&self) -> Result<()> {
        match fs::metadata(&self.inner.root).await {
            Ok(meta) if meta.is_dir() => Ok(()),
            Ok(_) => Err(Error::Io(io::Error::new(
                ErrorKind::NotADirectory,
                "the storage folder is not a folder",
            ))),
            Err(error) => Err(Error::Io(error)),
        }
    }
}

impl ObjectStorage for FolderStore {
    fn prepare(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(FolderStore::prepare(self))
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        data: &'a mut (dyn AsyncRead + Send + Unpin),
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(FolderStore::put(self, key, data))
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        range: Option<ByteRange>,
    ) -> BoxFuture<'a, Result<StoredObject>> {
        Box::pin(FolderStore::get(self, key, range))
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(FolderStore::delete(self, key))
    }

    fn list(&self) -> Listing {
        Listing::new(FolderPages {
            store: self.clone(),
            done: false,
        })
    }

    fn probe(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(FolderStore::probe(self))
    }
}

/// A listing of a folder is one page: the walk has to see every file to put
/// them in key order, and what it keeps is a key and a size for each.
struct FolderPages {
    store: FolderStore,
    done: bool,
}

impl ListPages for FolderPages {
    fn next_page(&mut self) -> BoxFuture<'_, Result<Option<Vec<StoredObjectInfo>>>> {
        Box::pin(async move {
            if self.done {
                return Ok(None);
            }
            self.done = true;
            let root = self.store.inner.root.clone();
            let incoming = self.store.inner.incoming.clone();
            let found = tokio::task::spawn_blocking(move || walk(&root, &incoming))
                .await
                .map_err(|_| Error::Io(io::Error::other("the listing job was cancelled")))??;
            Ok(Some(found))
        })
    }
}

/// Every file but those still being written, ordered by key.
fn walk(root: &Path, incoming: &Path) -> io::Result<Vec<StoredObjectInfo>> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_owned()];
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            // Gone since it was seen, or never there: it holds nothing.
            Err(error) if names_nothing(&error) => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                if path != incoming {
                    folders.push(path);
                }
            } else if kind.is_file() {
                let Some(key) = key_of(root, &path) else {
                    tracing::warn!(
                        "a file in the storage folder has no valid key and is not listed"
                    );
                    continue;
                };
                match entry.metadata() {
                    Ok(meta) => found.push(StoredObjectInfo {
                        key,
                        size: meta.len(),
                    }),
                    Err(error) if names_nothing(&error) => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    found.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    Ok(found)
}

fn key_of(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut key = String::new();
    for part in relative.components() {
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(part.as_os_str().to_str()?);
    }
    key::is_valid(&key).then_some(key)
}

/// Whether an error says that the path names no file: it is not there, or a
/// folder on the way is a file, or a name is longer than any file can have.
fn names_nothing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::InvalidFilename
    )
}

/// Puts a finished staged file under its key without replacing what another
/// write put there meanwhile.
fn publish(staged: &Path, target: &Path, key: &str) -> Result<()> {
    if let Some(parent) = target.parent() {
        match std::fs::create_dir_all(parent) {
            Ok(()) => {}
            // A file stands where a folder of the key's path has to be.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::AlreadyExists | ErrorKind::NotADirectory
                ) =>
            {
                return Err(Error::AlreadyExists(key.to_owned()));
            }
            Err(error) => return Err(error.into()),
        }
    }
    // A hard link fails when the name is taken, which is the one atomic
    // "create unless it exists" a rename does not give.
    match std::fs::hard_link(staged, target) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err(Error::AlreadyExists(key.to_owned()));
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::Unsupported | ErrorKind::PermissionDenied
            ) =>
        {
            // A file system without hard links. A rename is atomic but
            // replaces, so two writers racing for one key can both win here.
            if target.symlink_metadata().is_ok() {
                return Err(Error::AlreadyExists(key.to_owned()));
            }
            std::fs::rename(staged, target)?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    // The staged name has done its work. If it stays, the next start clears it.
    if let Err(error) = std::fs::remove_file(staged) {
        tracing::debug!(%error, "a staged file could not be removed after its move");
    }
    Ok(())
}

/// A staged file that is removed when the write that made it is given up,
/// whether it failed or its future was dropped.
struct Staged {
    path: PathBuf,
    armed: bool,
}

impl Staged {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.armed {
            // The next start clears the staging folder if this fails.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
