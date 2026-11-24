//! Old files in a folder of the data root: backups and logs.

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use tracing::warn;

use super::TARGET;
use crate::values::Timestamp;

/// Deletes the files directly in `directory` that were last written before
/// `cutoff`, and says how many. A file that cannot be deleted is logged and
/// left; a folder that is not there has nothing to prune.
pub async fn prune(directory: PathBuf, cutoff: Timestamp, kind: &'static str) -> usize {
    tokio::task::spawn_blocking(move || prune_blocking(&directory, cutoff, kind))
        .await
        .unwrap_or(0)
}

fn prune_blocking(directory: &std::path::Path, cutoff: Timestamp, kind: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let Ok(ticks) = u64::try_from(cutoff.ticks()) else {
        return 0;
    };
    let limit = UNIX_EPOCH + Duration::from_nanos(ticks.saturating_mul(100));
    let mut pruned = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        // A file, as a link to one is, and not a folder.
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.modified().is_ok_and(|written| written >= limit) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => pruned += 1,
            Err(error) => {
                warn!(target: TARGET, "Deleting expired {kind} file {} failed: {error}", path.display());
            }
        }
    }
    pruned
}
