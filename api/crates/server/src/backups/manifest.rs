//! The contents list at the root of every archive.
//!
//! It is written the way the first server wrote it (camelCase members in a
//! fixed order, indented by two spaces, every character outside printable
//! ASCII and `"&'+<>` and the backtick as `\uXXXX`), so an archive made by
//! either server reads on the other.

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
        let mut out = String::from("{\n");
        let mut member = |name: &str, value: String, last: bool| {
            out.push_str("  \"");
            out.push_str(name);
            out.push_str("\": ");
            out.push_str(&value);
            out.push_str(if last { "\n" } else { ",\n" });
        };
        member("format", text(&self.format), false);
        member("version", self.version.to_string(), false);
        member("createdAt", text(&self.created_at.to_wire_text()), false);
        member("instanceName", text(&self.instance_name), false);
        member("parts", list(&self.parts), false);
        member("databaseBytes", self.database_bytes.to_string(), false);
        member("objectCount", self.object_count.to_string(), false);
        member("objectBytes", self.object_bytes.to_string(), false);
        member("missingObjects", list(&self.missing_objects), false);
        member("keyFiles", self.key_files.to_string(), false);
        member("environmentSecrets", list(&self.environment_secrets), false);
        member("instanceFiles", self.instance_files.to_string(), true);
        out.push('}');
        out.into_bytes()
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

fn text(value: &str) -> String {
    let mut out = String::from("\"");
    for unit in value.encode_utf16().collect::<Vec<_>>().iter().copied() {
        match unit {
            0x22 => out.push_str("\\u0022"),
            0x5c => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0c => out.push_str("\\f"),
            0x0d => out.push_str("\\r"),
            0x26 | 0x27 | 0x2b | 0x3c | 0x3e | 0x60 => out.push_str(&format!("\\u{unit:04X}")),
            0x20..=0x7e => out.push(unit as u8 as char),
            _ => out.push_str(&format!("\\u{unit:04X}")),
        }
    }
    out.push('"');
    out
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let mut out = String::from("[\n");
    for (at, item) in items.iter().enumerate() {
        out.push_str("    ");
        out.push_str(&text(item));
        out.push_str(if at + 1 == items.len() { "\n" } else { ",\n" });
    }
    out.push_str("  ]");
    out
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
    fn the_json_is_written_as_the_first_server_wrote_it() {
        let json = String::from_utf8(sample().to_json()).unwrap();
        assert_eq!(
            json,
            "{\n  \"format\": \"stellarr-backup\",\n  \"version\": 1,\n  \"createdAt\": \"2026-11-24T10:20:30.1234567Z\",\n  \"instanceName\": \"Ja\\u015B \\u0026 \\u003CMa\\u0142gosia\\u003E\",\n  \"parts\": [\n    \"database\",\n    \"objects\"\n  ],\n  \"databaseBytes\": 4096,\n  \"objectCount\": 2,\n  \"objectBytes\": 10,\n  \"missingObjects\": [],\n  \"keyFiles\": 1,\n  \"environmentSecrets\": [\n    \"STELLARR_X\"\n  ],\n  \"instanceFiles\": 0\n}"
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
