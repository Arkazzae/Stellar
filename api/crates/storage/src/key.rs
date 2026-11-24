//! What an object key may look like.

use crate::error::Error;

/// The longest key a store takes, in bytes: S3's own limit, applied to every
/// store so that objects move between them.
pub const MAX_KEY_BYTES: usize = 1024;

/// Where the folder store stages files before they get their key. No key may
/// start with it, in any store, so that a backup of one restores into the
/// other.
pub const INCOMING_FOLDER: &str = ".incoming";

/// A relative path of plain segments separated by `/`: no absolute path, no
/// backslash, no NUL, no empty segment and no step up or in place. Such a key
/// stays inside any folder it is joined to, which the folder store and an
/// unpacked backup both rely on.
pub fn is_safe(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('/')
        && !key.contains(['\\', '\0'])
        && !key
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."))
}

/// Whether the stores take `key`: safe, not longer than [`MAX_KEY_BYTES`] and
/// not inside the staging folder.
pub fn is_valid(key: &str) -> bool {
    is_safe(key)
        && key.len() <= MAX_KEY_BYTES
        && key != INCOMING_FOLDER
        && !key.starts_with(".incoming/")
}

pub(crate) fn check(key: &str) -> Result<(), Error> {
    if is_valid(key) {
        Ok(())
    } else {
        Err(Error::InvalidKey(show(key)))
    }
}

/// A key as an error message can show it: escaped, and cut short.
fn show(key: &str) -> String {
    const LIMIT: usize = 80;
    let escaped = key.escape_debug().to_string();
    if escaped.chars().count() <= LIMIT {
        escaped
    } else {
        let head: String = escaped.chars().take(LIMIT).collect();
        format!("{head}...")
    }
}
