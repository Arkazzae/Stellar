//! The statuses a development seed leaves for accounts it cannot keep
//! connected.
//!
//! Presence lives only in this process and follows live connections; a seed
//! that inserts a roomful of accounts has no way to open one for each, so it
//! writes a small document into the data folder instead. Only the
//! Development environment reads it: a production instance must never invent
//! a status for someone who is not connected. Connecting as a seeded account
//! promotes it to a real session and drops the seeded status.

use std::path::Path;

use serde_json::Value;

use super::names::parse_guid;
use super::presence::{PresenceTracker, is_supported};
use crate::settings::Environment;
use crate::values::Id;

/// The name of the document in the data folder.
pub const FILE_NAME: &str = "seed-presence.json";

/// Reads the document, if there is one and this is development, and gives the
/// tracker what it says. A document that cannot be read is logged and leaves
/// everybody offline.
pub fn apply(environment: &Environment, data_root: &Path, tracker: &PresenceTracker) {
    if !environment.is_development() {
        return;
    }
    let path = data_root.join(FILE_NAME);
    if !path.exists() {
        return;
    }
    match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| read(&text))
    {
        Ok(presences) => {
            tracing::info!(
                target: "Stellarr.Api.Realtime",
                "Restored {} seeded presence entries from {}.",
                presences.len(),
                path.display()
            );
            tracker.seed(presences);
        }
        Err(error) => tracing::warn!(
            target: "Stellarr.Api.Realtime",
            "Could not read the seeded presence from {}: {error}",
            path.display()
        ),
    }
}

/// The entries of the document that name an account and a status; the others
/// are skipped. Property names are read without regard to case.
fn read(text: &str) -> Result<Vec<(Id, String)>, String> {
    let document: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let users = match &document {
        Value::Object(members) => member(members, "users"),
        Value::Null => None,
        _ => return Err("the document is not an object".to_string()),
    };
    let entries = match users {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(entries)) => entries,
        Some(_) => return Err("users is not a list".to_string()),
    };
    let mut presences = Vec::new();
    for entry in entries {
        let Value::Object(members) = entry else {
            return Err("an entry is not an object".to_string());
        };
        let id = text_member(members, "id")?;
        let presence = text_member(members, "presence")?;
        if let (Some(id), Some(presence)) = (id, presence)
            && let Some(id) = parse_guid(id)
            && is_supported(presence)
        {
            presences.push((id, presence.to_string()));
        }
    }
    Ok(presences)
}

fn member<'a>(members: &'a serde_json::Map<String, Value>, name: &str) -> Option<&'a Value> {
    members
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

fn text_member<'a>(
    members: &'a serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<&'a str>, String> {
    match member(members, name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(format!("{name} is not text")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn tracker_after(environment: &str, document: Option<&str>) -> (PresenceTracker, TempDir) {
        let folder = TempDir::new();
        if let Some(document) = document {
            std::fs::write(folder.path().join(FILE_NAME), document).unwrap();
        }
        let tracker = PresenceTracker::default();
        apply(&Environment::new(environment), folder.path(), &tracker);
        (tracker, folder)
    }

    #[test]
    fn a_seeded_document_restores_presence_in_development() {
        let user = Id::random();
        let document = format!(
            r#"{{"version":1,"users":[
                {{"id":"{user}","presence":"idle"}},
                {{"id":"not-a-guid","presence":"dnd"}},
                {{"id":"{}","presence":"offline"}}]}}"#,
            Id::random()
        );

        let (tracker, _folder) = tracker_after("Development", Some(&document));

        assert_eq!(tracker.presence_of(user), "idle");
    }

    #[test]
    fn production_ignores_a_seeded_document() {
        let user = Id::random();
        let document = format!(r#"{{"users":[{{"id":"{user}","presence":"idle"}}]}}"#);

        let (tracker, _folder) = tracker_after("Production", Some(&document));

        assert_eq!(tracker.presence_of(user), "offline");
    }

    #[test]
    fn a_malformed_document_leaves_everyone_offline() {
        let (tracker, _folder) = tracker_after("Development", Some("{ this is not json"));

        assert_eq!(tracker.presence_of(Id::random()), "offline");
    }

    #[test]
    fn no_document_is_no_seed() {
        let (tracker, _folder) = tracker_after("Development", None);

        assert_eq!(tracker.presence_of(Id::random()), "offline");
    }

    #[test]
    fn names_are_read_in_any_case_and_a_wrong_kind_makes_the_whole_document_unreadable() {
        let user = Id::random();
        let upper = format!(r#"{{"Users":[{{"ID":"{user}","Presence":"dnd"}}]}}"#);
        let (tracker, _folder) = tracker_after("development", Some(&upper));
        assert_eq!(tracker.presence_of(user), "dnd");

        let wrong = format!(
            r#"{{"users":[{{"id":"{user}","presence":"idle"}},{{"id":5,"presence":"idle"}}]}}"#
        );
        let (tracker, _folder) = tracker_after("Development", Some(&wrong));
        assert_eq!(tracker.presence_of(user), "offline");

        let none = r#"{"users":null}"#;
        let (tracker, _folder) = tracker_after("Development", Some(none));
        assert_eq!(tracker.presence_of(user), "offline");
    }
}
