//! What an operator publishes under `{data root}/releases`: a JSON manifest
//! beside the files it describes.
//!
//! No manifest means nothing is published. A manifest that cannot be trusted
//! is [`Untrusted`], which the routes answer with 503: a 404 or a 204 would
//! hide a broken publication behind clients that merely never update.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::json::{self, Value};
use super::moment::parse_published_at;
use super::version;
use crate::values::Timestamp;

const SUPPORTED_SCHEMA_VERSION: i32 = 1;
/// Counted in UTF-16 units, as the first server counted them.
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

/// The manifest at `path` as JSON: None when there is none, [`Failure::Untrusted`]
/// when there is one that is not JSON.
pub fn read(path: &Path) -> Result<Option<Value>, Failure> {
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
    match json::parse(&decode(&bytes)) {
        Some(Value::Object(members)) => Ok(Some(Value::Object(members))),
        _ => Err(Failure::Untrusted),
    }
}

/// The text of a file as its byte order mark says, as UTF-8 when it has none;
/// what is not valid is replaced, not refused.
fn decode(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], big: bool| {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| {
                if big {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The members of one object, looked up the way the first server's reader did:
/// a name matches whatever its letter case, and the last of a name written
/// twice decides. Anything of the wrong type makes the whole manifest
/// untrustworthy, even a member that a later one would have replaced.
pub struct Members<'a>(&'a [(String, Value)]);

impl<'a> Members<'a> {
    fn of(value: &'a Value) -> Option<Members<'a>> {
        match value {
            Value::Object(members) => Some(Members(members)),
            _ => None,
        }
    }

    fn all(&self, name: &str) -> Vec<&'a Value> {
        let wanted = stellarr_unicode::to_upper(name);
        self.0
            .iter()
            .filter(|(key, _)| stellarr_unicode::to_upper(key) == wanted)
            .map(|(_, value)| value)
            .collect()
    }

    /// A text, or none when it is missing or null.
    fn text(&self, name: &str) -> Option<Option<String>> {
        let mut found = None;
        for value in self.all(name) {
            found = Some(match value {
                Value::String(text) => Some(text.clone()),
                Value::Null => None,
                _ => return None,
            });
        }
        Some(found.flatten())
    }

    /// A whole number: written as one, or as text holding one with an
    /// optional sign. Missing is 0.
    fn number(&self, name: &str, within: std::ops::RangeInclusive<i64>) -> Option<i64> {
        let mut found = 0;
        for value in self.all(name) {
            let text = match value {
                Value::Number(text) | Value::String(text) => text.as_str(),
                _ => return None,
            };
            let (negative, digits) = match text.as_bytes().first() {
                Some(b'-') => (true, &text[1..]),
                Some(b'+') if matches!(value, Value::String(_)) => (false, &text[1..]),
                _ => (false, text),
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let magnitude: i64 = digits.parse().ok()?;
            found = Some(if negative { -magnitude } else { magnitude })
                .filter(|n| within.contains(n))?;
        }
        Some(found)
    }

    fn int(&self, name: &str) -> Option<i32> {
        self.number(name, i64::from(i32::MIN)..=i64::from(i32::MAX))
            .and_then(|n| i32::try_from(n).ok())
    }

    fn long(&self, name: &str) -> Option<i64> {
        self.number(name, i64::MIN..=i64::MAX)
    }

    /// A moment; None inside when it is missing, which stands for "no moment".
    fn moment(&self, name: &str) -> Option<Option<Timestamp>> {
        let mut found = None;
        for value in self.all(name) {
            match value {
                Value::String(text) => found = Some(parse_published_at(text)?),
                _ => return None,
            }
        }
        Some(found)
    }

    /// An object, or none when it is missing or null.
    fn object(&self, name: &str) -> Option<Option<Members<'a>>> {
        let mut found = None;
        for value in self.all(name) {
            found = Some(match value {
                Value::Null => None,
                other => Some(Members::of(other)?),
            });
        }
        Some(found.flatten())
    }
}

/// What every kind of manifest starts with, once it has been checked.
pub struct Header {
    pub version: String,
    pub published_at: Timestamp,
    pub notes: String,
}

struct RawHeader {
    schema_version: i32,
    version: Option<String>,
    published_at: Option<Timestamp>,
    notes: Option<String>,
}

impl RawHeader {
    fn read(members: &Members) -> Option<RawHeader> {
        Some(RawHeader {
            schema_version: members.int("schemaVersion")?,
            version: members.text("version")?,
            published_at: members.moment("publishedAt")?,
            notes: members.text("notes")?,
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
            && notes.encode_utf16().count() <= MAX_NOTES_LENGTH)
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

impl Artifact {
    fn read(members: &Members) -> Option<Artifact> {
        Some(Artifact {
            file_name: members.text("fileName")?,
            size_bytes: members.long("sizeBytes")?,
            sha256: members.text("sha256")?,
        })
    }

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

impl ReleaseManifest {
    /// None when the manifest cannot be trusted.
    pub fn from(document: &Value) -> Option<ReleaseManifest> {
        let members = Members::of(document)?;
        let header = RawHeader::read(&members)?;
        let artifact = members.object("artifact")?;
        let header = header.check()?;
        let artifact = artifact?;
        Some(ReleaseManifest {
            header,
            platform: artifact.text("platform")?,
            architecture: artifact.text("architecture")?,
            artifact: Artifact::read(&artifact)?,
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

/// The manifest of what an installed client may update itself to.
pub struct UpdateManifest {
    pub header: Option<Header>,
    pub channel: Option<String>,
    pub rollout_percent: i32,
    pub artifacts: Vec<Option<UpdateArtifact>>,
}

impl UpdateManifest {
    /// None when the manifest is not even the right shape; a header that is
    /// not valid shows as `header: None`.
    pub fn from(document: &Value) -> Option<UpdateManifest> {
        let members = Members::of(document)?;
        let header = RawHeader::read(&members)?;
        let channel = members.text("channel")?;
        let rollout_percent = members.int("rolloutPercent")?;
        let mut artifacts = Vec::new();
        for list in members.all("artifacts") {
            artifacts.clear();
            match list {
                Value::Null => {}
                Value::Array(items) => {
                    for item in items {
                        artifacts.push(match item {
                            Value::Null => None,
                            other => {
                                let item = Members::of(other)?;
                                Some(UpdateArtifact {
                                    target: item.text("target")?,
                                    architecture: item.text("architecture")?,
                                    bundle: item.text("bundle")?,
                                    signature: item.text("signature")?,
                                    artifact: Artifact::read(&item)?,
                                })
                            }
                        });
                    }
                }
                _ => return None,
            }
        }
        Some(UpdateManifest {
            header: header.check(),
            channel,
            rollout_percent,
            artifacts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn document(text: &str) -> Value {
        json::parse(text).unwrap()
    }

    const GOOD: &str = r#"{"schemaVersion":1,"version":" 1.2.3 ","publishedAt":"2026-11-20T10:11:12Z",
        "notes":"  hello ","artifact":{"platform":"arch","architecture":"x86_64","fileName":"a.bin",
        "sizeBytes":16,"sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}}"#;

    #[test]
    fn a_manifest_is_read_and_its_header_trimmed() {
        let manifest = ReleaseManifest::from(&document(GOOD)).unwrap();

        assert_eq!(manifest.platform.as_deref(), Some("arch"));
        assert_eq!(manifest.artifact.size_bytes(), 16);
        assert_eq!(
            RawHeader::read(&Members::of(&document(GOOD)).unwrap())
                .and_then(RawHeader::check)
                .map(|header| (header.version, header.notes)),
            Some(("1.2.3".to_string(), "hello".to_string()))
        );
    }

    #[test]
    fn names_match_in_any_letter_case_and_the_last_one_decides() {
        let text = GOOD
            .replace("schemaVersion", "SCHEMAVERSION")
            .replace("\"version\"", "\"Version\"")
            .replace("{\"SCHEMAVERSION\":1,", "{\"schemaVersion\":7,\"SCHEMAVERSION\":1,");

        assert!(ReleaseManifest::from(&document(&text)).is_some());
        let last_is_bad = GOOD.replace("{\"schemaVersion\":1,", "{\"schemaVersion\":1,\"schemaVersion\":7,");
        assert!(RawHeader::read(&Members::of(&document(&last_is_bad)).unwrap())
            .and_then(RawHeader::check)
            .is_none());
    }

    #[test]
    fn a_member_of_the_wrong_type_spoils_the_manifest_even_when_overwritten() {
        let overwritten = GOOD.replace("{\"schemaVersion\":1,", "{\"schemaVersion\":true,\"schemaVersion\":1,");

        assert!(ReleaseManifest::from(&document(&overwritten)).is_none());
    }

    #[test]
    fn numbers_may_be_written_as_text_but_not_with_a_fraction() {
        let number = |written: &str| {
            let text = GOOD.replace("\"schemaVersion\":1", &format!("\"schemaVersion\":{written}"));
            ReleaseManifest::from(&document(&text)).is_some()
        };

        assert!(number("1"));
        assert!(number("\"1\""));
        assert!(number("\"+1\""));
        assert!(number("\"01\""));
        assert!(!number("\" 1\""));
        assert!(!number("1.0"));
        assert!(!number("1e0"));
        assert!(!number("true"));
        assert!(!number("null"));
        assert!(!number("\"\""));
        assert!(!number("3000000000"));
        // The type is fine, the value is not a version of the schema.
        assert!(number("1") && !number("2"));
    }

    #[test]
    fn notes_are_counted_in_utf16_units() {
        let notes = |text: String| {
            let text = GOOD.replace("\"  hello \"", &format!("\"{text}\""));
            ReleaseManifest::from(&document(&text)).is_some()
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
            assert!(ReleaseManifest::from(&document(&text)).is_none(), "{to}");
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
    fn a_manifest_file_is_read_whatever_its_byte_order_mark() {
        let scratch = TempDir::new();
        let folder = scratch.path().to_path_buf();
        let at = folder.join("m.json");
        let utf16: Vec<u8> = GOOD.encode_utf16().flat_map(u16::to_le_bytes).collect();

        assert!(matches!(read(&folder.join("nothing.json")), Ok(None)));
        std::fs::write(&at, GOOD).unwrap();
        assert!(matches!(read(&at), Ok(Some(_))));
        std::fs::write(&at, [b"\xEF\xBB\xBF".as_slice(), GOOD.as_bytes()].concat()).unwrap();
        assert!(matches!(read(&at), Ok(Some(_))));
        std::fs::write(&at, [b"\xFF\xFE".as_slice(), &utf16].concat()).unwrap();
        assert!(matches!(read(&at), Ok(Some(_))));
        std::fs::write(&at, &utf16).unwrap();
        assert!(matches!(read(&at), Err(Failure::Untrusted)));
        for broken in ["", "{ this is not json", "null", "[]", "5", "\"x\""] {
            std::fs::write(&at, broken).unwrap();
            assert!(matches!(read(&at), Err(Failure::Untrusted)), "{broken}");
        }
        std::fs::write(&at, " ".repeat(3 * 1024 * 1024)).unwrap();
        assert!(matches!(read(&at), Err(Failure::Untrusted)), "blank, so not JSON");
        std::fs::write(&at, [b"{}".as_slice(), &vec![b' '; 17 * 1024 * 1024]].concat()).unwrap();
        assert!(matches!(read(&at), Err(Failure::Untrusted)));
        // A folder where the manifest should be is the disk's trouble, not a publication.
        std::fs::remove_file(&at).unwrap();
        std::fs::create_dir(&at).unwrap();
        assert!(matches!(read(&at), Err(Failure::Disk(_))));
    }
}
