//! How much room a volume has, for the checks made before a big write.

use std::path::Path;
use std::process::{Command, Stdio};

/// What a refusal says when there is no figure: a check that cannot be made is
/// not passed. The words are the operator's tool's own, for the same case.
pub fn unknown(what: &str, needed: u64) -> String {
    format!(
        "The free space on the data volume could not be read (`df` failed), and {what} needs {needed} bytes, so it was not done."
    )
}

/// The bytes available on the volume that holds `path`, as `df` reports them;
/// none when `df` is missing, fails or is not understood. It blocks, so it is
/// called from a blocking thread. The standard library cannot say, and this
/// code may not call into the system itself.
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(test)]
    if let Some(told) = fake::told(path) {
        return told;
    }
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

/// A way for a test to say what the disk reports for a folder (and below it),
/// including nothing at all.
#[cfg(test)]
pub mod fake {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static TOLD: Mutex<Vec<(PathBuf, Option<u64>)>> = Mutex::new(Vec::new());

    pub fn tell(folder: &Path, free: Option<u64>) {
        let folder = std::path::absolute(folder).unwrap();
        let mut told = TOLD.lock().unwrap();
        told.retain(|(have, _)| *have != folder);
        told.push((folder, free));
    }

    pub(super) fn told(path: &Path) -> Option<Option<u64>> {
        let path = std::path::absolute(path).ok()?;
        let told = TOLD.lock().unwrap();
        told.iter()
            .find(|(folder, _)| path.starts_with(folder))
            .map(|(_, free)| *free)
    }
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
