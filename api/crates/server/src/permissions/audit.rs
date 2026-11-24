//! The audit log of a server: one line for every change, written in the same
//! transaction as the change it records.
//!
//! The line has a sentence in the server's language, as it was when written,
//! and the structured fields a client words in its reader's language. The
//! `ChangesJson` column holds the fields as JSON: camelCase, a member that is
//! null left out, and text as it is.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use stellarr_unicode::utf16;

use crate::values::{Id, Timestamp};

const MAX_SUMMARY: usize = 500;
const MAX_TARGET_NAME: usize = 200;

/// One field an audited action changed. Values are what a reader's client
/// words in its own language: names as they were at the time, and fixed
/// tokens such as permission names or `public` and `invite-only`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditChange {
    #[serde(default)]
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_color: Option<String>,
}

impl AuditChange {
    /// A change of one value, or nothing when it stayed the same.
    pub fn changed(field: &str, from: Option<&str>, to: Option<&str>) -> Option<AuditChange> {
        (from != to).then(|| AuditChange {
            field: field.to_string(),
            from: from.map(str::to_string),
            to: to.map(str::to_string),
            ..AuditChange::default()
        })
    }

    pub fn added(field: &str, names: Vec<String>) -> AuditChange {
        AuditChange {
            field: field.to_string(),
            added: Some(names),
            ..AuditChange::default()
        }
    }
}

/// What happened, for the audit log. `sentence` is the line in the server's
/// language without the actor's name, for readers that know only the
/// sentence; the rest lets a client say it in its own words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditEvent {
    /// What was done, such as `role.update`.
    pub action: String,
    pub sentence: String,
    /// What it was done to: `role`, `member`, `channel` and so on.
    pub target_type: Option<String>,
    pub target_id: Option<Id>,
    pub target_name: Option<String>,
    pub changes: Vec<AuditChange>,
}

/// The changes that happened, without the fields that did not change.
pub fn only(changes: impl IntoIterator<Item = Option<AuditChange>>) -> Vec<AuditChange> {
    changes.into_iter().flatten().collect()
}

/// Writes a line of the audit log. Call it in the transaction of the change
/// it records, so that both are kept or neither is. An actor who is no account
/// is a fault, since the line names them.
pub fn record(db: &Connection, server: Id, actor: Id, event: &AuditEvent) -> rusqlite::Result<Id> {
    let name: String = db.query_row(
        "SELECT \"DisplayName\" FROM \"Users\" WHERE \"Id\" = ?1",
        [actor],
        |row| row.get(0),
    )?;
    let id = Id::random();
    let changes = (!event.changes.is_empty()).then(|| changes_json(&event.changes));
    db.execute(
        "INSERT INTO \"ServerAuditEntries\" (\"Id\", \"ServerId\", \"ActorId\", \"Summary\", \"CreatedAt\", \"Action\", \"TargetType\", \"TargetId\", \"TargetName\", \"ChangesJson\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            server,
            actor,
            clip(&format!("{name} {}", event.sentence), MAX_SUMMARY),
            Timestamp::now(),
            event.action,
            event.target_type,
            event.target_id,
            event
                .target_name
                .as_deref()
                .map(|name| clip(name, MAX_TARGET_NAME)),
            changes,
        ],
    )?;
    Ok(id)
}

/// The text cut to `max` UTF-16 units, the last of which is an ellipsis when
/// it was cut. A pair the cut falls inside is left out whole.
pub fn clip(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.to_string();
    }
    let mut kept = utf16::prefix(text, max - 1).to_string();
    kept.push('\u{2026}');
    kept
}

/// The fields as the column holds them.
pub fn changes_json(changes: &[AuditChange]) -> String {
    // Text and lists of text always write.
    serde_json::to_string(changes).expect("changes are written as JSON")
}

/// The changes of an entry, or none when it has none or they cannot be read.
/// A damaged line must not make the log unreadable, so a line with anything of
/// the wrong kind in it is read as having no changes at all.
pub fn read_changes(json: Option<&str>) -> Vec<AuditChange> {
    json.filter(|json| !json.is_empty())
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_written_as_they_are_and_read_back() {
        let changes = vec![
            AuditChange {
                field: "name".into(),
                from: Some("Zażółć & <b>".into()),
                to: Some("\u{1F600} 'x' + `y`".into()),
                ..AuditChange::default()
            },
            AuditChange::added("roles", vec!["a".into()]),
        ];
        let written = changes_json(&changes);
        assert!(written.contains("Zażółć & <b>"), "{written}");
        assert!(written.contains("\u{1F600} 'x' + `y`"), "{written}");
        assert!(!written.contains("\\u"), "{written}");
        assert!(!written.contains("null"), "{written}");
        assert_eq!(read_changes(Some(&written)), changes);
    }

    #[test]
    fn a_damaged_line_has_no_changes_and_names_match_exactly() {
        for damaged in [
            "",
            "{}",
            "[1]",
            "[{\"field\":5}]",
            "[{\"field\":\"a\",\"field\":\"b\"}]",
        ] {
            assert!(read_changes(Some(damaged)).is_empty(), "{damaged}");
        }
        let cased = read_changes(Some("[{\"Field\":\"a\",\"field\":\"b\"}]"));
        assert_eq!(cased[0].field, "b");
        assert!(read_changes(None).is_empty());
    }
}
