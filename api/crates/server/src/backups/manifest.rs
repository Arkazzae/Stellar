//! The contents list at the root of every archive.
//!
//! It is written as JSON with camelCase members in a fixed order, indented by
//! two spaces, text as it is.

use serde::{Deserialize, Serialize};

use crate::values::Timestamp;

pub const FORMAT: &str = "stellarr-backup";
pub const CURRENT_VERSION: i64 = 1;

/// The most a manifest may take to read: a listing of a few million missing
/// objects would be more, and nothing a person made.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// `environment_secrets` names those of the secrets the panel can hold that
/// this instance reads from its environment instead: their values stay on the
/// host, in `.env`, and are not in the archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub version: i64,
    pub created_at: Timestamp,
    #[serde(default)]
    pub instance_name: String,
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub database_bytes: i64,
    #[serde(default)]
    pub object_count: i64,
    #[serde(default)]
    pub object_bytes: i64,
    #[serde(default)]
    pub missing_objects: Vec<String>,
    #[serde(default)]
    pub key_files: i64,
    #[serde(default)]
    pub environment_secrets: Vec<String>,
    #[serde(default)]
    pub instance_files: i64,
}

impl Manifest {
    /// The bytes of `manifest.json`.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("a manifest is written as JSON")
    }

    /// Reads a manifest of a Stellarr backup, or none when the bytes are not
    /// one: not JSON of this shape, or of another format.
    pub fn read(bytes: &[u8]) -> Option<Manifest> {
        let manifest: Manifest = serde_json::from_slice(bytes).ok()?;
        (manifest.format == FORMAT).then_some(manifest)
    }

    /// Whether the archive says it has the part.
    pub fn has_part(&self, name: &str) -> bool {
        self.parts.iter().any(|part| part == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            format: FORMAT.into(),
            version: 1,
            created_at: Timestamp::parse("2026-11-24T10:20:30.1234567Z").unwrap(),
            instance_name: "Jaś & <Małgosia>".into(),
            parts: vec!["database".into(), "objects".into()],
            database_bytes: 4096,
            object_count: 2,
            object_bytes: 10,
            missing_objects: vec![],
            key_files: 1,
            environment_secrets: vec!["STELLARR_X".into()],
            instance_files: 0,
        }
    }

    #[test]
    fn the_json_is_written_indented_with_its_text_as_it_is() {
        let json = String::from_utf8(sample().to_json()).unwrap();
        assert_eq!(
            json,
            "{\n  \"format\": \"stellarr-backup\",\n  \"version\": 1,\n  \"createdAt\": \"2026-11-24T10:20:30.1234567Z\",\n  \"instanceName\": \"Ja\u{15b} & <Ma\u{142}gosia>\",\n  \"parts\": [\n    \"database\",\n    \"objects\"\n  ],\n  \"databaseBytes\": 4096,\n  \"objectCount\": 2,\n  \"objectBytes\": 10,\n  \"missingObjects\": [],\n  \"keyFiles\": 1,\n  \"environmentSecrets\": [\n    \"STELLARR_X\"\n  ],\n  \"instanceFiles\": 0\n}"
        );
    }

    #[test]
    fn what_is_written_reads_back_and_other_formats_do_not() {
        let manifest = sample();
        assert_eq!(Manifest::read(&manifest.to_json()), Some(manifest));
        assert!(Manifest::read(br#"{"format":"other","createdAt":"2026-11-24T10:20:30Z"}"#).is_none());
        assert!(Manifest::read(b"not json").is_none());
        assert!(Manifest::read(br#"{"format":"stellarr-backup","createdAt":"yesterday"}"#).is_none());
        let minimal = Manifest::read(br#"{"format":"stellarr-backup","createdAt":"2026-11-24T10:20:30Z"}"#).unwrap();
        assert_eq!(minimal.instance_files, 0);
        assert!(minimal.parts.is_empty());
    }
}
