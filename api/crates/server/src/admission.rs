//! Room for an upload, reserved before any of its bytes are written to the
//! store.
//!
//! The process is the only writer to the database, so reserving here closes
//! the gap between reading the stored totals and committing the row that adds
//! to them. A reservation holds one of a few slots as well: only so many
//! uploads move bytes into the store at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, params};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::warn;

use crate::database::{Database, DbError};
use crate::settings::UploadSettings;
use crate::values::Id;

/// Why an upload was not let in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    UserQuota,
    InstanceQuota,
    LowDiskSpace,
}

#[derive(Clone)]
pub struct AdmissionGate {
    inner: Arc<Inner>,
}

struct Inner {
    limits: UploadSettings,
    data_root: PathBuf,
    slots: Arc<Semaphore>,
    /// Makes the check and the reservation one step.
    turnstile: tokio::sync::Mutex<()>,
    reserved: Mutex<Reserved>,
}

impl Inner {
    fn reserved(&self) -> MutexGuard<'_, Reserved> {
        self.reserved.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct Reserved {
    total: i64,
    by_user: HashMap<Id, i64>,
}

/// Holds the reservation and the slot until it is dropped.
pub struct Lease {
    gate: Arc<Inner>,
    user: Id,
    bytes: i64,
    _slot: OwnedSemaphorePermit,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut reserved = self.gate.reserved();
        reserved.total -= self.bytes;
        let left = reserved.by_user.get(&self.user).copied().unwrap_or(0) - self.bytes;
        if left <= 0 {
            reserved.by_user.remove(&self.user);
        } else {
            reserved.by_user.insert(self.user, left);
        }
    }
}

impl AdmissionGate {
    pub fn new(limits: &UploadSettings, data_root: PathBuf) -> AdmissionGate {
        let slots = limits.max_concurrent_uploads.clamp(1, 32) as usize;
        AdmissionGate {
            inner: Arc::new(Inner {
                limits: limits.clone(),
                data_root,
                slots: Arc::new(Semaphore::new(slots)),
                turnstile: tokio::sync::Mutex::new(()),
                reserved: Mutex::new(Reserved::default()),
            }),
        }
    }

    /// Waits for a free slot, then reserves `bytes` for `user` unless a quota
    /// or the disk says there is no room.
    pub async fn enter(
        &self,
        database: &Database,
        user: Id,
        bytes: i64,
    ) -> Result<Result<Lease, Refusal>, DbError> {
        let inner = &self.inner;
        let slot = Arc::clone(&inner.slots)
            .acquire_owned()
            .await
            .map_err(|_| DbError::Task("the upload slots were closed".to_string()))?;
        let _turn = inner.turnstile.lock().await;
        let stored = database
            .read(move |db| -> Result<Stored, DbError> { Ok(Stored::of(db, user)?) })
            .await?;
        let (user_reserved, total_reserved) = {
            let reserved = inner.reserved();
            (
                reserved.by_user.get(&user).copied().unwrap_or(0),
                reserved.total,
            )
        };
        let limits = &inner.limits;
        if would_exceed(
            stored.user,
            user_reserved,
            bytes,
            limits.max_stored_bytes_per_user,
        ) {
            return Ok(Err(Refusal::UserQuota));
        }
        if would_exceed(stored.total, total_reserved, bytes, limits.max_stored_bytes) {
            return Ok(Err(Refusal::InstanceQuota));
        }
        if !has_disk_headroom(
            &inner.data_root,
            limits.minimum_free_data_root_bytes,
            bytes,
            total_reserved,
        )
        .await
        {
            return Ok(Err(Refusal::LowDiskSpace));
        }
        let mut reserved = inner.reserved();
        reserved.total = reserved.total.saturating_add(bytes);
        let mine = reserved.by_user.entry(user).or_insert(0);
        *mine = mine.saturating_add(bytes);
        drop(reserved);
        Ok(Ok(Lease {
            gate: Arc::clone(&self.inner),
            user,
            bytes,
            _slot: slot,
        }))
    }
}

/// What is stored already: the sizes of every attachment's object and its
/// poster and preview, and of every emoji.
struct Stored {
    user: i64,
    total: i64,
}

impl Stored {
    fn of(db: &Connection, user: Id) -> rusqlite::Result<Stored> {
        const ATTACHMENTS: &str = "SELECT COALESCE(SUM(\"SizeBytes\" + COALESCE(\"PosterSizeBytes\", 0) + COALESCE(\"PreviewSizeBytes\", 0)), 0) FROM \"ImageAttachments\"";
        const EMOJI: &str = "SELECT COALESCE(SUM(\"SizeBytes\"), 0) FROM \"ServerEmoji\"";
        let of_user = |sql: &str| -> rusqlite::Result<i64> {
            db.query_row(
                &format!("{sql} WHERE \"UploadedById\" = ?1"),
                params![user],
                |row| row.get(0),
            )
        };
        let all = |sql: &str| -> rusqlite::Result<i64> { db.query_row(sql, [], |row| row.get(0)) };
        Ok(Stored {
            user: of_user(ATTACHMENTS)?.saturating_add(of_user(EMOJI)?),
            total: all(ATTACHMENTS)?.saturating_add(all(EMOJI)?),
        })
    }
}

/// Whether stored, reserved and requested bytes together pass the limit,
/// written as subtractions so that no sum can overflow. A limit of zero or
/// less is no limit.
fn would_exceed(stored: i64, reserved: i64, requested: i64, limit: i64) -> bool {
    limit > 0
        && (requested > limit
            || stored > limit - requested
            || reserved > limit - requested - stored)
}

/// Whether the volume of the data root has the free space the upload and the
/// uploads that are already moving need, and the minimum besides. A volume
/// that will not say how much it has is not a reason to refuse.
async fn has_disk_headroom(
    data_root: &Path,
    minimum: i64,
    requested: i64,
    reserved_total: i64,
) -> bool {
    let minimum = minimum.max(0);
    if minimum == 0 {
        return true;
    }
    if requested > i64::MAX - minimum || reserved_total > i64::MAX - minimum - requested {
        return false;
    }
    match free_bytes(data_root).await {
        Some(free) => free >= minimum + requested + reserved_total,
        None => true,
    }
}

/// How long `df` may take. The turnstile is held while it runs, so a `df` that
/// hangs would stall every upload.
const DF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// The space available on the volume that holds `path`, as `df` reports it.
/// The standard library cannot say, and this code may not call into the
/// system itself. When `df` is missing, fails or is too slow there is no
/// answer, which is logged: the reserve is then not enforced.
async fn free_bytes(path: &Path) -> Option<i64> {
    // The data root may not exist yet on a first start; its nearest parent is
    // on the same volume.
    let mut probe = std::path::absolute(path).ok()?;
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let run = tokio::process::Command::new("df")
        .arg("-Pk")
        .arg(&probe)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(DF_TIMEOUT, run).await {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            warn!(
                "df failed ({}): the free space reserve is not enforced",
                output.status
            );
            return None;
        }
        Ok(Err(error)) => {
            warn!("df cannot be run ({error}): the free space reserve is not enforced");
            return None;
        }
        Err(_) => {
            warn!("df did not answer in {DF_TIMEOUT:?}: the free space reserve is not enforced");
            return None;
        }
    };
    let free = parse_df(&String::from_utf8_lossy(&output.stdout));
    if free.is_none() {
        warn!("the answer of df was not understood: the free space reserve is not enforced");
    }
    free
}

/// The Available column, in kilobytes, of the last line of `df -P`, in bytes.
fn parse_df(text: &str) -> Option<i64> {
    let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let columns: Vec<&str> = line.split_whitespace().collect();
    // Filesystem, 1024-blocks, Used, Available, Capacity, Mounted on. A device
    // or a mount point may hold spaces, so the Available column is the one
    // that is followed by a percentage.
    let at = columns.iter().rposition(|column| column.ends_with('%'))?;
    columns
        .get(at.checked_sub(1)?)?
        .parse::<i64>()
        .ok()?
        .checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_is_passed_by_stored_reserved_and_requested_bytes_together() {
        assert!(!would_exceed(5, 3, 2, 10));
        assert!(would_exceed(5, 3, 3, 10));
        assert!(would_exceed(0, 0, 11, 10));
        assert!(!would_exceed(0, 0, 10, 10));
    }

    #[test]
    fn a_limit_of_zero_or_less_is_no_limit() {
        assert!(!would_exceed(i64::MAX, i64::MAX, i64::MAX, 0));
        assert!(!would_exceed(1, 1, 1, -1));
    }

    #[test]
    fn no_sum_overflows() {
        assert!(would_exceed(i64::MAX, 0, 1, i64::MAX));
        assert!(would_exceed(0, i64::MAX, 1, i64::MAX));
    }

    #[test]
    fn the_available_space_is_read_from_the_last_line() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s5 482344960 13000000 15728640 47% /System/Volumes/Data\n";
        assert_eq!(parse_df(text), Some(15_728_640 * 1024));
        let spaced = "Filesystem 1024-blocks Used Available Capacity Mounted on\nmy disk 100 10 90 10% /mnt/a b\n";
        assert_eq!(parse_df(spaced), Some(90 * 1024));
        assert_eq!(parse_df("nothing useful"), None);
        assert_eq!(parse_df(""), None);
    }
}
