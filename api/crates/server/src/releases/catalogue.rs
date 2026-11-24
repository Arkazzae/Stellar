//! Which packages are on offer, and which update an installed client may take.
//!
//! The deploy scripts publish a package beside its manifest on the data
//! volume, so that a release of a native client needs neither a new image nor
//! a restart: every question reads the manifest again.

use std::path::{Path, PathBuf};

use serde::Serialize;
use stellarr_messages::MessageKey;

use super::manifest::{self, Failure, ReleaseManifest, UpdateManifest};
use super::version;
use crate::values::Timestamp;

/// Where one platform's release is published and what its manifest declares.
#[derive(Debug)]
pub struct Kind {
    /// The folder under `{data root}/releases`.
    pub directory: &'static str,
    pub manifest: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    /// Where this server serves the package.
    pub download_path: &'static str,
    pub content_type: &'static str,
    /// What a client is told when the manifest cannot be trusted.
    pub unavailable: MessageKey,
}

/// The Arch Linux package of the desktop client, which a person downloads in
/// a browser.
pub const DESKTOP: Kind = Kind {
    directory: "desktop",
    manifest: "desktop-release.json",
    platform: "arch",
    architecture: "x86_64",
    download_path: "/api/desktop/release/arch",
    content_type: "application/zstd",
    unavailable: MessageKey::DesktopReleaseUnavailable,
};

/// The Android package. Signing material stays on the build host and never
/// reaches the data volume or the image.
pub const ANDROID: Kind = Kind {
    directory: "android",
    manifest: "android-release.json",
    platform: "android",
    architecture: "arm64-v8a",
    download_path: "/api/android/release/apk",
    content_type: "application/vnd.android.package-archive",
    unavailable: MessageKey::AndroidReleaseUnavailable,
};

/// A package of a native client that this server offers for download.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDto {
    pub version: String,
    pub published_at: Timestamp,
    pub notes: String,
    pub platform: String,
    pub architecture: String,
    pub file_name: String,
    pub size_bytes: i64,
    /// Lower case hex.
    pub sha256: String,
    /// Where this server serves the package.
    pub download_url: String,
}

/// A release whose manifest has been checked against the package on disk.
#[derive(Debug, Clone)]
pub struct Published {
    pub version: String,
    pub published_at: Timestamp,
    pub notes: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub sha256: String,
    /// Where the package lies on the data volume.
    pub file_path: PathBuf,
    kind: &'static Kind,
}

impl Published {
    pub fn kind(&self) -> &'static Kind {
        self.kind
    }

    pub fn dto(&self) -> ReleaseDto {
        ReleaseDto {
            version: self.version.clone(),
            published_at: self.published_at,
            notes: self.notes.clone(),
            platform: self.kind.platform.to_string(),
            architecture: self.kind.architecture.to_string(),
            file_name: self.file_name.clone(),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            download_url: self.kind.download_path.to_string(),
        }
    }
}

/// The published release of one platform: None when there is none, and
/// [`Failure::Untrusted`] when a manifest is published but cannot be trusted.
/// Reads the disk, so it belongs on a blocking thread.
pub fn current(kind: &'static Kind, data_root: &Path) -> Result<Option<Published>, Failure> {
    let directory = manifest::directory(data_root, kind.directory);
    let Some(document) = manifest::read(&directory.join(kind.manifest))? else {
        return Ok(None);
    };
    let release = ReleaseManifest::from(&document).ok_or(Failure::Untrusted)?;
    if release.platform.as_deref() != Some(kind.platform)
        || release.architecture.as_deref() != Some(kind.architecture)
    {
        return Err(Failure::Untrusted);
    }
    let (file_path, file_name) = release.artifact.resolve(&directory)?;
    Ok(Some(Published {
        version: release.header.version,
        published_at: release.header.published_at,
        notes: release.header.notes,
        file_name,
        size_bytes: release.artifact.size_bytes(),
        sha256: release.artifact.sha256().to_string(),
        file_path,
        kind,
    }))
}

/// One update artifact, checked against the file on the data volume.
#[derive(Debug, Clone)]
pub struct UpdateCandidate {
    pub version: String,
    pub published_at: Timestamp,
    pub notes: String,
    /// The share of clients, counted in buckets from 0, that may take it.
    pub rollout_percent: i32,
    /// What the client verifies the download against.
    pub signature: String,
    pub file_name: String,
    pub file_path: PathBuf,
    pub size_bytes: i64,
}

pub fn is_known_channel(channel: &str) -> bool {
    matches!(channel, "stable" | "beta")
}

const STABLE_MANIFEST: &str = "desktop-updater.json";
const BETA_MANIFEST: &str = "desktop-updater-beta.json";

/// The artifact an installed client may update to, from a manifest of its own,
/// apart from the one behind the downloadable package: that one is offered to
/// a person in a browser, this one is a contract that a program verifies with
/// a key, so a broken one cannot take the other down. None when nothing is
/// published for these arguments. Reads the disk.
pub fn find_update(
    data_root: &Path,
    channel: &str,
    target: &str,
    architecture: &str,
    bundle: &str,
) -> Result<Option<UpdateCandidate>, Failure> {
    if !is_known_channel(channel) {
        return Ok(None);
    }
    let directory = manifest::directory(data_root, "desktop");
    let file = if channel == "beta" {
        BETA_MANIFEST
    } else {
        STABLE_MANIFEST
    };
    let Some(document) = manifest::read(&directory.join(file))? else {
        return Ok(None);
    };
    let update = UpdateManifest::from(&document).ok_or(Failure::Untrusted)?;
    let header = update.header.ok_or(Failure::Untrusted)?;
    if update.channel.as_deref() != Some(channel)
        || !(0..=100).contains(&update.rollout_percent)
        || update.artifacts.is_empty()
    {
        return Err(Failure::Untrusted);
    }
    // Only the artifact asked for is checked, so that a broken build for
    // another platform does not take updates from the platforms that work.
    let Some(found) = update.artifacts.iter().flatten().find(|candidate| {
        candidate.target.as_deref() == Some(target)
            && candidate.architecture.as_deref() == Some(architecture)
            && candidate.bundle.as_deref() == Some(bundle)
    }) else {
        return Ok(None);
    };
    let signature = found
        .signature
        .as_deref()
        .filter(|signature| is_signature(signature))
        .ok_or(Failure::Untrusted)?;
    let (file_path, file_name) = found.artifact.resolve(&directory)?;
    Ok(Some(UpdateCandidate {
        version: header.version,
        published_at: header.published_at,
        notes: header.notes,
        rollout_percent: update.rollout_percent,
        signature: signature.to_string(),
        file_name,
        file_path,
        size_bytes: found.artifact.size_bytes(),
    }))
}

/// A minisign signature is base64 together with the comment lines the
/// bundler writes around it: 64 to 4096 of the characters of both.
fn is_signature(text: &str) -> bool {
    let allowed = |b: u8| b.is_ascii_alphanumeric() || b"+/=\r\n:. _-".contains(&b);
    text.bytes().all(allowed) && (64..=4096).contains(&text.len())
}

/// A version the desktop client has to be at least, or is advised to be:
/// what the operator saved wins over what is configured, which wins over the
/// default of the whole product.
pub fn desktop_version(saved: Option<&str>, configured: Option<&str>) -> String {
    const FALLBACK: &str = "0.1.0";
    [saved, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|version| !version.is_empty())
        .unwrap_or(FALLBACK)
        .to_string()
}

/// Whether `candidate` is newer than what a client has installed.
pub fn is_newer(candidate: &str, installed: (u32, u32, u32)) -> bool {
    version::parse_stable(candidate).is_some_and(|published| published > installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_has_the_characters_and_the_length_the_bundler_writes() {
        let comment = "untrusted comment: signature from minisign secret key\n";
        let real = format!(
            "{comment}RWSRGejCl7EEOExTHJ2LUMF6MF1yBuIBWKvgYWuFE0duj3HfmJq9rj4BAAAAAAAAAAA="
        );

        assert!(is_signature(&real));
        assert!(is_signature(&"A".repeat(64)));
        assert!(is_signature(&"A".repeat(4096)));
        assert!(!is_signature(&"A".repeat(63)));
        assert!(!is_signature(&"A".repeat(4097)));
        assert!(is_signature(&format!("{}\n", "A".repeat(4095))));
        assert!(!is_signature(&format!("{}\n", "A".repeat(4096))), "a line break counts like any character");
        assert!(!is_signature(&format!("{}\n\n", "A".repeat(4096))));
        assert!(!is_signature(&format!("{}\r", "A".repeat(4096))));
        assert!(!is_signature(&format!("{real}!")));
        assert!(!is_signature(&format!("{real}ü")));
        assert!(!is_signature("RWQ="));
        assert!(!is_signature(""));
    }

    #[test]
    fn what_the_operator_saved_wins_over_what_is_configured() {
        assert_eq!(desktop_version(Some(" 0.3.0 "), Some("0.2.0")), "0.3.0");
        assert_eq!(desktop_version(None, Some("0.2.0")), "0.2.0");
        assert_eq!(desktop_version(Some("  "), Some(" 0.2.0")), "0.2.0");
        assert_eq!(desktop_version(Some(""), None), "0.1.0");
        assert_eq!(desktop_version(None, None), "0.1.0");
    }
}
