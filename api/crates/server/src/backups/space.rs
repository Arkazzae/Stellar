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
    let mut child = Command::new("df")
        .arg("-Pk")
        .arg(&probe)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // As long as the upload gate waits for `df`, and no longer: a `df` that
    // hangs on a dead mount must not hang a backup.
    let deadline = std::time::Instant::now() + crate::admission::DF_TIMEOUT;
    let status = loop {
        match child.try_wait().ok()? {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    };
    if !status.success() {
        return None;
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    u64::try_from(crate::admission::parse_df(&text)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_does_not_exist_yet_is_asked_about_through_its_parent() {
        let folder = crate::testing::TempDir::new();
        assert!(free_bytes(&folder.path().join("not/yet")).is_some());
    }
}
