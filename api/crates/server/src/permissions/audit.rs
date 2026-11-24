//! The audit log of a server: one line for every change, written in the same
//! transaction as the change it records.
//!
//! The line has a sentence in the server's language, as it was when written,
//! and the structured fields a client words in its reader's language. The
//! `ChangesJson` column holds the fields as the first server wrote them
//! (camelCase, a member that is null left out, and every character outside
//! printable ASCII, and `"&'+<>` and the backtick, escaped as `\uXXXX`), so
//! either server reads the other's lines.

use rusqlite::{Connection, params};
use stellarr_unicode::utf16;

use crate::values::{Id, Timestamp};

const MAX_SUMMARY: usize = 500;
const MAX_TARGET_NAME: usize = 200;

/// One field an audited action changed. Values are what a reader's client
/// words in its own language: names as they were at the time, and fixed
/// tokens such as permission names or `public` and `invite-only`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditChange {
    pub field: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub added: Option<Vec<String>>,
    pub removed: Option<Vec<String>>,
    pub from_color: Option<String>,
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
/// it was cut. A cut that falls inside a surrogate pair leaves half of it,
/// which is stored as the replacement character.
pub fn clip(text: &str, max: usize) -> String {
    let units = utf16::units(text);
    if units.len() <= max {
        return text.to_string();
    }
    let mut kept = utf16::text(&units[..max - 1]);
    kept.push('\u{2026}');
    kept
}

/// The fields as the first server wrote them.
pub fn changes_json(changes: &[AuditChange]) -> String {
    let mut out = String::from("[");
    for (index, change) in changes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        let mut first = true;
        let mut member = |name: &str, value: Value<'_>| {
            let text = match value {
                Value::Missing => return,
                Value::Text(text) => string(text),
                Value::List(items) => {
                    let items: Vec<String> = items.iter().map(|item| string(item)).collect();
                    format!("[{}]", items.join(","))
                }
            };
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&string(name));
            out.push(':');
            out.push_str(&text);
        };
        member("field", Value::Text(&change.field));
        member(
            "from",
            change.from.as_deref().map_or(Value::Missing, Value::Text),
        );
        member(
            "to",
            change.to.as_deref().map_or(Value::Missing, Value::Text),
        );
        member(
            "added",
            change.added.as_deref().map_or(Value::Missing, Value::List),
        );
        member(
            "removed",
            change
                .removed
                .as_deref()
                .map_or(Value::Missing, Value::List),
        );
        member(
            "fromColor",
            change
                .from_color
                .as_deref()
                .map_or(Value::Missing, Value::Text),
        );
        member(
            "toColor",
            change
                .to_color
                .as_deref()
                .map_or(Value::Missing, Value::Text),
        );
        out.push('}');
    }
    out.push(']');
    out
}

enum Value<'a> {
    Missing,
    Text(&'a str),
    List(&'a [String]),
}

/// A JSON string the way the first server's writer makes one: anything that is
/// not printable ASCII, and the characters a page could mistake for markup,
/// as `\uXXXX` with capital hex digits.
fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for unit in text.encode_utf16() {
        match unit {
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0c => out.push_str("\\f"),
            0x0d => out.push_str("\\r"),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e if !matches!(unit, 0x22 | 0x26 | 0x27 | 0x2b | 0x3c | 0x3e | 0x60) => {
                out.push(char::from(u8::try_from(unit).unwrap_or(b'?')));
            }
            _ => out.push_str(&format!("\\u{unit:04X}")),
        }
    }
    out.push('"');
    out
}

/// The changes of an entry, or none when it has none or they cannot be read.
/// A damaged line must not make the log unreadable, so a line with anything of
/// the wrong kind in it is read as having no changes at all. Names are matched
/// in any letter case, as the first server read them.
pub fn read_changes(json: Option<&str>) -> Vec<AuditChange> {
    let Some(json) = json.filter(|json| !json.is_empty()) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    items
        .iter()
        .map(read_change)
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

fn read_change(item: &serde_json::Value) -> Option<AuditChange> {
    use serde_json::Value;
    let object = item.as_object()?;
    let find = |name: &str| {
        object
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    };
    let text = |name: &str| -> Option<Option<String>> {
        match find(name) {
            None | Some(Value::Null) => Some(None),
            Some(Value::String(text)) => Some(Some(text.clone())),
            Some(_) => None,
        }
    };
    let list = |name: &str| -> Option<Option<Vec<String>>> {
        match find(name) {
            None | Some(Value::Null) => Some(None),
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| match value {
                    Value::String(text) => Some(text.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .map(Some),
            Some(_) => None,
        }
    };
    Some(AuditChange {
        field: text("field")?.unwrap_or_default(),
        from: text("from")?,
        to: text("to")?,
        added: list("added")?,
        removed: list("removed")?,
        from_color: text("fromColor")?,
        to_color: text("toColor")?,
    })
}
