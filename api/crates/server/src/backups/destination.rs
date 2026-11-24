//! Somewhere outside this server that a completed archive is copied to. The
//! backup schedule knows only this contract; the concrete destination, a
//! bucket on Cloudflare R2 or the relay in front of one, lives in
//! [`super::offsite`].

use std::path::Path;

use stellarr_storage::BoxFuture;

use super::BackupFailed;
use crate::values::Id;

pub trait Destination: Send + Sync {
    /// Whether this destination should receive the next archive.
    fn enabled(&self) -> BoxFuture<'_, bool>;

    /// Copies the archive to the destination, and checks it arrived whole.
    /// Sending the same archive again is harmless: one that is already there
    /// with the same size and checksum counts as sent.
    fn upload<'a>(
        &'a self,
        archive: &'a Path,
        server: Id,
    ) -> BoxFuture<'a, Result<(), BackupFailed>>;
}
