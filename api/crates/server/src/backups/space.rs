//! How much room a volume has, for the checks made before a big write.

use std::path::Path;
use std::process::{Command, Stdio};

/// The bytes available on the volume that holds `path`, as `df` reports them;
/// none when `df` is missing, fails or is not understood. It blocks, so it is
/// called from a blocking thread. The standard library cannot say, and this
/// code may not call into the system itself.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let mut probe = std::path::absolute(path).ok()?;
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let output = Command::new("df")
        .arg("-Pk")
        .arg(&probe)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_df(&String::from_utf8_lossy(&output.stdout))
}

/// The Available column, in kilobytes, of the last line of `df -P`, in bytes.
fn parse_df(text: &str) -> Option<u64> {
    let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let columns: Vec<&str> = line.split_whitespace().collect();
    // A device or a mount point may hold spaces, so the Available column is
    // the one before the percentage.
    let at = columns.iter().rposition(|column| column.ends_with('%'))?;
    columns
        .get(at.checked_sub(1)?)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_available_column() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s1 489620264 13000000 6000000 69% /System/Volumes/Data\n";
        assert_eq!(parse_df(text), Some(6_000_000 * 1024));
        assert_eq!(parse_df("nothing"), None);
    }

    #[test]
    fn a_folder_that_does_not_exist_yet_is_asked_about_through_its_parent() {
        let folder = crate::testing::TempDir::new();
        assert!(free_bytes(&folder.path().join("not/yet")).is_some());
    }
}
