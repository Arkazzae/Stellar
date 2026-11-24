//! What an operator publishes under `{data root}/releases`: a JSON manifest
//! beside the files it describes.
//!
//! No manifest means nothing is published. A manifest that cannot be trusted
//! is [`Untrusted`], which the routes answer with 503: a 404 or a 204 would
//! hide a broken publication behind clients that merely never update.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::moment::parse_published_at;
use super::version;
use crate::values::Timestamp;

const SUPPORTED_SCHEMA_VERSION: i32 = 1;
/// Counted in UTF-16 units, as the clients count.
const MAX_NOTES_LENGTH: usize = 2_000;
/// A manifest is a few hundred bytes; one that is megabytes is not what an
/// operator wrote, and is not read into memory.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

/// A manifest or an artifact that cannot be trusted, or a failure of the
/// disk that gives no reason to think the publication is broken.
#[derive(Debug)]
pub enum Failure {
    /// The publication is broken: say so (503), and say nothing of how.
    Untrusted,
    /// The disk would not answer.
    Disk(io::Error),
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Failure {
        Failure::Disk(error)
    }
}

/// The folder one kind of release is published in.
pub fn directory(data_root: &Path, name: &str) -> PathBuf {
    let folder = data_root.join("releases").join(name);
    std::path::absolute(&folder).unwrap_or(folder)
}

/// The bytes of the manifest at `path`: None when there is none,
/// [`Failure::Untrusted`] when it is larger than any manifest is.
pub fn read(path: &Path) -> Result<Option<Vec<u8>>, Failure> {
    let mut bytes = Vec::new();
    let opened = File::open(path).and_then(|file| {
        // A folder opens, and fails on the first read.
        file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)
    });
    match opened {
        Ok(_) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(Failure::Disk(error)),
    }
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(Failure::Untrusted);
    }
    Ok(Some(bytes))
}

/// A moment that is missing, or is one the manifest can be trusted with.
/// None when it is written in a way that is no moment at all.
fn moment(text: Option<&str>) -> Option<Option<Timestamp>> {
    match text {
        None => Some(None),
        Some(text) => parse_published_at(text).map(Some),
    }
}

/// What every kind of manifest starts with, once it has been checked.
pub struct Header {
    pub version: String,
    pub published_at: Timestamp,
    pub notes: String,
}

/// The members of a manifest are read by their exact names, and a member of
/// the wrong type, written twice, or in quotes where a number belongs, makes
/// the whole manifest untrustworthy.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeaderWire {
    #[serde(default)]
    schema_version: i32,
    version: Option<String>,
    published_at: Option<String>,
    notes: Option<String>,
}

struct RawHeader {
    schema_version: i32,
    version: Option<String>,
    published_at: Option<Timestamp>,
    notes: Option<String>,
}

impl RawHeader {
    fn read(wire: HeaderWire) -> Option<RawHeader> {
        Some(RawHeader {
            schema_version: wire.schema_version,
            version: wire.version,
            published_at: moment(wire.published_at.as_deref())?,
            notes: wire.notes,
        })
    }

    fn check(self) -> Option<Header> {
        let (Some(version), Some(published_at), Some(notes)) =
            (self.version, self.published_at, self.notes)
        else {
            return None;
        };
        (self.schema_version == SUPPORTED_SCHEMA_VERSION
            && version::parse_stable(&version).is_some()
            && stellarr_unicode::utf16::length(&notes) <= MAX_NOTES_LENGTH)
            .then(|| Header {
                version: version.trim().to_string(),
                published_at,
                notes: notes.trim().to_string(),
            })
    }
}

/// The file an artifact names, as the manifest declares it.
pub struct Artifact {
    file_name: Option<String>,
    size_bytes: i64,
    sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactWire {
    file_name: Option<String>,
    #[serde(default)]
    size_bytes: i64,
    sha256: Option<String>,
}

impl From<ArtifactWire> for Artifact {
    fn from(wire: ArtifactWire) -> Artifact {
        Artifact {
            file_name: wire.file_name,
            size_bytes: wire.size_bytes,
            sha256: wire.sha256,
        }
    }
}

impl Artifact {
    /// The file this artifact names, which must be a plain file name inside
    /// `directory` and a file of exactly the declared size, so that a package
    /// still being copied is never served. What the manifest names is
    /// whatever its publisher wrote, so a name is never joined to the folder
    /// before it is known to hold no path.
    pub fn resolve(&self, directory: &Path) -> Result<(PathBuf, String), Failure> {
        let name = match &self.file_name {
            Some(name)
                if !name.trim().is_empty()
                    && !name.contains(['/', '\0'])
                    && name != "."
                    && name != ".." =>
            {
                name
            }
            _ => return Err(Failure::Untrusted),
        };
        let digest_ok = self.sha256.as_deref().is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        if self.size_bytes < 1 || !digest_ok {
            return Err(Failure::Untrusted);
        }
        let path = directory.join(name);
        match std::fs::metadata(&path) {
            Ok(file) if file.is_file() && i64::try_from(file.len()) == Ok(self.size_bytes) => {
                Ok((path, name.clone()))
            }
            // Missing, a folder, another size, or one that cannot be looked at.
            _ => Err(Failure::Untrusted),
        }
    }

    pub fn size_bytes(&self) -> i64 {
        self.size_bytes
    }

    pub fn sha256(&self) -> &str {
        self.sha256.as_deref().unwrap_or("")
    }
}

/// The manifest of a package a person downloads.
pub struct ReleaseManifest {
    pub header: Header,
    pub platform: Option<String>,
    pub architecture: Option<String>,
    pub artifact: Artifact,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseWire {
    #[serde(flatten)]
    header: HeaderWire,
    artifact: Option<ReleaseArtifactWire>,
}

#[derive(Deserialize)]
struct ReleaseArtifactWire {
    platform: Option<String>,
    architecture: Option<String>,
    #[serde(flatten)]
    file: ArtifactWire,
}

impl ReleaseManifest {
    /// None when the manifest cannot be trusted.
    pub fn from(document: &[u8]) -> Option<ReleaseManifest> {
        let wire: ReleaseWire = serde_json::from_slice(document).ok()?;
        let header = RawHeader::read(wire.header)?;
        let header = header.check()?;
        let artifact = wire.artifact?;
        Some(ReleaseManifest {
            header,
            platform: artifact.platform,
            architecture: artifact.architecture,
            artifact: artifact.file.into(),
        })
    }
}

/// One artifact of the updater's manifest.
pub struct UpdateArtifact {
    pub target: Option<String>,
    pub architecture: Option<String>,
    pub bundle: Option<String>,
    pub signature: Option<String>,
    pub artifact: Artifact,
}

#[derive(Deserialize)]
struct UpdateArtifactWire {
    target: Option<String>,
    architecture: Option<String>,
    bundle: Option<String>,
    signature: Option<String>,
    #[serde(flatten)]
    file: ArtifactWire,
}

/// The manifest of what an installed client may update itself to.
pub struct UpdateManifest {
    pub header: Option<Header>,
    pub channel: Option<String>,
    pub rollout_percent: i32,
    pub artifacts: Vec<Option<UpdateArtifact>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWire {
    #[serde(flatten)]
    header: HeaderWire,
    channel: Option<String>,
    #[serde(default)]
    rollout_percent: i32,
    artifacts: Option<Vec<Option<UpdateArtifactWire>>>,
}

impl UpdateManifest {
    /// None when the manifest is not even the right shape; a header that is
    /// not valid shows as `header: None`.
    pub fn from(document: &[u8]) -> Option<UpdateManifest> {
        let wire: UpdateWire = serde_json::from_slice(document).ok()?;
        let header = RawHeader::read(wire.header)?;
        let artifacts = wire
            .artifacts
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                item.map(|item| UpdateArtifact {
                    target: item.target,
                    architecture: item.architecture,
                    bundle: item.bundle,
                    signature: item.signature,
                    artifact: item.file.into(),
                })
            })
            .collect();
        Some(UpdateManifest {
            header: header.check(),
            channel: wire.channel,
            rollout_percent: wire.rollout_percent,
            artifacts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const GOOD: &str = r#"{"schemaVersion":1,"version":" 1.2.3 ","publishedAt":"2026-11-20T10:11:12Z",
        "notes":"  hello ","artifact":{"platform":"arch","architecture":"x86_64","fileName":"a.bin",
        "sizeBytes":16,"sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}}"#;

    fn release(text: &str) -> Option<ReleaseManifest> {
        ReleaseManifest::from(text.as_bytes())
    }

    #[test]
    fn a_manifest_is_read_and_its_header_trimmed() {
        let manifest = release(GOOD).unwrap();

        assert_eq!(manifest.platform.as_deref(), Some("arch"));
        assert_eq!(manifest.artifact.size_bytes(), 16);
        assert_eq!(manifest.header.version, "1.2.3");
        assert_eq!(manifest.header.notes, "hello");
    }

    #[test]
    fn names_match_exactly_and_a_name_written_twice_spoils_the_manifest() {
        let upper = GOOD.replace("schemaVersion", "SCHEMAVERSION");
        assert!(release(&upper).is_none(), "another case names nothing, and schema 0 is no schema");
        let twice = GOOD.replace("{\"schemaVersion\":1,", "{\"schemaVersion\":1,\"schemaVersion\":1,");
        assert!(release(&twice).is_none());
        let artifact_twice = GOOD.replace("\"platform\":\"arch\",", "\"platform\":\"arch\",\"platform\":\"arch\",");
        assert!(release(&artifact_twice).is_none());
        assert!(release(GOOD).is_some());
    }

    #[test]
    fn a_member_of_the_wrong_type_spoils_the_manifest() {
        let wrong = GOOD.replace("{\"schemaVersion\":1,", "{\"schemaVersion\":true,");
        assert!(release(&wrong).is_none());
    }

    #[test]
    fn numbers_are_numbers_and_not_text_and_not_fractions() {
        let number = |written: &str| {
            let text = GOOD.replace("\"schemaVersion\":1", &format!("\"schemaVersion\":{written}"));
            release(&text).is_some()
        };

        assert!(number("1"));
        for refused in ["\"1\"", "\"+1\"", "\" 1\"", "1.0", "1e0", "true", "null", "\"\"", "3000000000", "2"] {
            assert!(!number(refused), "{refused}");
        }
    }

    #[test]
    fn notes_are_counted_in_utf16_units() {
        let notes = |text: String| {
            let text = GOOD.replace("\"  hello \"", &format!("\"{text}\""));
            release(&text).is_some()
        };

        assert!(notes("x".repeat(2000)));
        assert!(!notes("x".repeat(2001)));
        assert!(notes("😀".repeat(1000)));
        assert!(!notes(format!("{}x", "😀".repeat(1000))));
        assert!(notes("ż".repeat(2000)));
    }

    #[test]
    fn a_header_that_is_not_valid_is_not_a_header() {
        for (from, to) in [
            ("\"version\":\" 1.2.3 \"", "\"version\":\"1.2.3-beta\""),
            ("\"version\":\" 1.2.3 \"", "\"version\":null"),
            ("\"publishedAt\":\"2026-11-20T10:11:12Z\"", "\"publishedAt\":\"0001-01-01T00:00:00\""),
            ("\"notes\":\"  hello \"", "\"notes\":null"),
        ] {
            let text = GOOD.replace(from, to);
            assert!(release(&text).is_none(), "{to}");
        }
    }

    #[test]
    fn an_artifact_that_is_not_there_is_untrusted() {
        let scratch = TempDir::new();
        let folder = scratch.path().to_path_buf();
        let sizes = |size: i64, name: &str, digest: &str| Artifact {
            file_name: Some(name.to_string()),
            size_bytes: size,
            sha256: Some(digest.to_string()),
        };
        let digest = "0".repeat(64);
        std::fs::write(folder.join("a.bin"), [0u8; 16]).unwrap();
        std::fs::create_dir(folder.join("dir")).unwrap();

        assert!(sizes(16, "a.bin", &digest).resolve(&folder).is_ok());
        for (size, name, digest) in [
            (15, "a.bin", digest.as_str()),
            (17, "a.bin", digest.as_str()),
            (0, "a.bin", digest.as_str()),
            (16, "missing.bin", digest.as_str()),
            (16, "dir", digest.as_str()),
            (16, "a.bin", "ABC"),
            (16, "a.bin", &"A".repeat(64)),
            (16, "a.bin", &"g".repeat(64)),
            (16, " ", digest.as_str()),
            (16, "", digest.as_str()),
            (16, ".", digest.as_str()),
            (16, "..", digest.as_str()),
            (16, "./a.bin", digest.as_str()),
            (16, "sub/../a.bin", digest.as_str()),
            (16, "../a.bin", digest.as_str()),
            (16, "/etc/passwd", digest.as_str()),
            (16, "a.bin\0", digest.as_str()),
            (16, "a\0.bin", digest.as_str()),
        ] {
            assert!(
                matches!(sizes(size, name, digest).resolve(&folder), Err(Failure::Untrusted)),
                "{size} {name:?} {digest}"
            );
        }
        // Folders only, never what lies outside them.
        let long = "x".repeat(10_000);
        assert!(matches!(sizes(16, &long, &digest).resolve(&folder), Err(Failure::Untrusted)));
    }

    #[test]
    fn a_manifest_file_is_read_as_it_is_and_a_byte_order_mark_spoils_it() {
        let scratch = TempDir::new();
        let folder = scratch.path().to_path_buf();
        let at = folder.join("m.json");

        assert!(matches!(read(&folder.join("nothing.json")), Ok(None)));
        std::fs::write(&at, GOOD).unwrap();
        let bytes = read(&at).unwrap().unwrap();
        assert!(ReleaseManifest::from(&bytes).is_some());
        let marked = [b"\xEF\xBB\xBF".as_slice(), GOOD.as_bytes()].concat();
        assert!(ReleaseManifest::from(&marked).is_none());
        let utf16: Vec<u8> = GOOD.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(ReleaseManifest::from(&[b"\xFF\xFE".as_slice(), &utf16].concat()).is_none());
        for broken in ["", "{ this is not json", "null", "[]", "5", "\"x\""] {
            assert!(ReleaseManifest::from(broken.as_bytes()).is_none(), "{broken}");
            assert!(UpdateManifest::from(broken.as_bytes()).is_none(), "{broken}");
        }
        std::fs::write(&at, [b"{}".as_slice(), &vec![b' '; 17 * 1024 * 1024]].concat()).unwrap();
        assert!(matches!(read(&at), Err(Failure::Untrusted)));
        // A folder where the manifest should be is the disk's trouble, not a publication.
        std::fs::remove_file(&at).unwrap();
        std::fs::create_dir(&at).unwrap();
        assert!(matches!(read(&at), Err(Failure::Disk(_))));
    }

    #[test]
    fn an_update_manifest_keeps_its_holes_and_reads_its_artifacts_by_exact_names() {
        let text = GOOD.replace(
            "\"artifact\":{\"platform\":\"arch\",\"architecture\":\"x86_64\",",
            "\"channel\":\"stable\",\"rolloutPercent\":5,\"artifacts\":[null,{\"target\":\"linux\",\"architecture\":\"x86_64\",\"bundle\":\"appimage\",\"signature\":\"s\",",
        );
        let text = format!("{}}}]}}", text.trim_end_matches('}'));
        let update = UpdateManifest::from(text.as_bytes()).unwrap();
        assert_eq!(update.rollout_percent, 5);
        assert!(update.artifacts[0].is_none());
        assert_eq!(update.artifacts[1].as_ref().unwrap().target.as_deref(), Some("linux"));
        assert!(UpdateManifest::from(text.replace("rolloutPercent", "RolloutPercent").as_bytes()).unwrap().rollout_percent == 0);
        assert!(UpdateManifest::from(text.replace("\"rolloutPercent\":5", "\"rolloutPercent\":\"5\"").as_bytes()).is_none());
    }
}
