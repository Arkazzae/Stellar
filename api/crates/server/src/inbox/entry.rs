//! One entry of a person's inbox, how it is written, and how it is announced.
//!
//! Every group that puts something in an inbox (an invitation, a mention)
//! writes it with [`add`] inside its own transaction and, once that has
//! committed, tells the person with [`announce`]. The delivery by mail or push
//! that a message asks for is the message routes' own business.

use rusqlite::{Connection, Row, params};
use serde::Serialize;

use crate::events::{Audience, Event, EventPublisher};
use crate::values::{Id, Timestamp};

/// What is written for a new entry. Its id, position and creation time (unless
/// given) are made here; it starts unread.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub server: Id,
    pub user: Id,
    pub source: String,
    pub source_package_id: String,
    pub source_package_name: String,
    pub source_module_id: String,
    pub source_module_name: String,
    pub title: String,
    pub body: String,
    pub tone: String,
    pub target_channel: Option<Id>,
    pub target_invite_code: Option<String>,
    pub created_at: Timestamp,
}

/// An entry as stored.
#[derive(Debug, Clone)]
pub struct Entry {
    pub sequence: i64,
    pub id: Id,
    pub server: Id,
    pub user: Id,
    pub source: String,
    pub source_package_id: String,
    pub source_package_name: String,
    pub source_module_id: String,
    pub source_module_name: String,
    pub title: String,
    pub body: String,
    pub tone: String,
    pub target_channel: Option<Id>,
    pub target_invite_code: Option<String>,
    pub created_at: Timestamp,
    pub read_at: Option<Timestamp>,
}

pub(super) const COLUMNS: &str = "\"Sequence\", \"Id\", \"ServerId\", \"UserId\", \"Source\", \"SourcePackageId\",
    \"SourcePackageName\", \"SourceModuleId\", \"SourceModuleName\", \"Title\", \"Body\", \"Tone\",
    \"TargetChannelId\", \"TargetInviteCode\", \"CreatedAt\", \"ReadAt\"";

impl Entry {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Entry> {
        Ok(Entry {
            sequence: row.get(0)?,
            id: row.get(1)?,
            server: row.get(2)?,
            user: row.get(3)?,
            source: row.get(4)?,
            source_package_id: row.get(5)?,
            source_package_name: row.get(6)?,
            source_module_id: row.get(7)?,
            source_module_name: row.get(8)?,
            title: row.get(9)?,
            body: row.get(10)?,
            tone: row.get(11)?,
            target_channel: row.get(12)?,
            target_invite_code: row.get(13)?,
            created_at: row.get(14)?,
            read_at: row.get(15)?,
        })
    }
}

/// Writes a new, unread entry and returns it with its position. Call it in the
/// transaction of whatever caused the entry, and announce after the commit.
pub fn add(connection: &Connection, new: &NewEntry) -> rusqlite::Result<Entry> {
    let id = Id::random();
    connection.execute(
        "INSERT INTO \"UserNotifications\" (\"Id\", \"ServerId\", \"UserId\", \"Source\", \"SourcePackageId\",
             \"SourcePackageName\", \"SourceModuleId\", \"SourceModuleName\", \"Title\", \"Body\", \"Tone\",
             \"TargetChannelId\", \"TargetInviteCode\", \"CreatedAt\", \"ReadAt\")
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL)",
        params![
            id,
            new.server,
            new.user,
            new.source,
            new.source_package_id,
            new.source_package_name,
            new.source_module_id,
            new.source_module_name,
            new.title,
            new.body,
            new.tone,
            new.target_channel,
            new.target_invite_code,
            new.created_at,
        ],
    )?;
    Ok(Entry {
        sequence: connection.last_insert_rowid(),
        id,
        server: new.server,
        user: new.user,
        source: new.source.clone(),
        source_package_id: new.source_package_id.clone(),
        source_package_name: new.source_package_name.clone(),
        source_module_id: new.source_module_id.clone(),
        source_module_name: new.source_module_name.clone(),
        title: new.title.clone(),
        body: new.body.clone(),
        tone: new.tone.clone(),
        target_channel: new.target_channel,
        target_invite_code: new.target_invite_code.clone(),
        created_at: new.created_at,
        read_at: None,
    })
}

/// The payload of `NotificationInboxChanged`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Changed {
    server_id: Id,
    sequence: Option<i64>,
    global: bool,
}

/// The same event for a change that names no single entry: the clients tell an
/// absent `sequence` from a null one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InvitationsChanged {
    server_id: Id,
    global: bool,
}

const EVENT: &str = "NotificationInboxChanged";

/// Tells the person's own connections that an entry was added, changed or
/// removed. An invitation is global: it shows on every server.
pub fn announce(events: &dyn EventPublisher, entry: &Entry) {
    events.publish(Event::new(
        Audience::Users(vec![entry.user]),
        EVENT,
        &Changed {
            server_id: entry.server,
            sequence: Some(entry.sequence),
            global: entry.target_invite_code.is_some(),
        },
    ));
}

/// Tells the person's connections that several entries changed, of any server.
pub(super) fn announce_all_read(events: &dyn EventPublisher, user: Id, server: Id) {
    events.publish(Event::new(
        Audience::Users(vec![user]),
        EVENT,
        &Changed {
            server_id: server,
            sequence: None,
            global: true,
        },
    ));
}

/// Tells the person's connections to reload the inbox they see on every
/// server, after invitations to `server` came or went. The payload has no
/// `sequence` at all.
pub fn announce_invitations_changed(events: &dyn EventPublisher, user: Id, server: Id) {
    events.publish(Event::new(
        Audience::Users(vec![user]),
        EVENT,
        &InvitationsChanged {
            server_id: server,
            global: true,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingEvents;
    use crate::testing::new_database;

    fn new_entry(user: Id, server: Id, invite: Option<&str>) -> NewEntry {
        NewEntry {
            server,
            user,
            source: "system".into(),
            source_package_id: "stellarr.mention".into(),
            source_package_name: "Stellarr".into(),
            source_module_id: "mention".into(),
            source_module_name: "Maja".into(),
            title: "Maja mentioned you".into(),
            body: "hello".into(),
            tone: "info".into(),
            target_channel: None,
            target_invite_code: invite.map(str::to_string),
            created_at: Timestamp::now(),
        }
    }

    #[test]
    fn an_added_entry_gets_the_next_position_and_starts_unread() {
        let (_folder, database) = new_database();
        // With foreign keys off, the entry can stand without the account and
        // server it would belong to.
        let connection = Connection::open(database.path()).unwrap();
        connection.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        let (user, server) = (Id::random(), Id::random());

        let first = add(&connection, &new_entry(user, server, None)).unwrap();
        let second = add(&connection, &new_entry(user, server, Some("code"))).unwrap();

        assert_eq!(second.sequence, first.sequence + 1);
        assert!(first.read_at.is_none());
        assert_ne!(first.id, second.id);
        let stored: (String, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT \"Title\", \"TargetInviteCode\", \"ReadAt\" FROM \"UserNotifications\" WHERE \"Sequence\" = ?1",
                [second.sequence],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored, ("Maja mentioned you".to_string(), Some("code".to_string()), None));
    }

    #[test]
    fn the_three_shapes_of_the_event_are_written_as_the_clients_read_them() {
        let events = RecordingEvents::default();
        let (user, server) = (Id::random(), Id::random());
        let mention = Entry {
            sequence: 7,
            id: Id::random(),
            server,
            user,
            source: "system".into(),
            source_package_id: String::new(),
            source_package_name: String::new(),
            source_module_id: String::new(),
            source_module_name: String::new(),
            title: String::new(),
            body: String::new(),
            tone: String::new(),
            target_channel: None,
            target_invite_code: None,
            created_at: Timestamp::now(),
            read_at: None,
        };
        let invitation = Entry {
            target_invite_code: Some("code".into()),
            ..mention.clone()
        };

        announce(&events, &mention);
        announce(&events, &invitation);
        announce_all_read(&events, user, server);
        announce_invitations_changed(&events, user, server);

        let payloads: Vec<String> = events.take().into_iter().map(|e| e.payload).collect();
        assert_eq!(
            payloads,
            [
                format!("{{\"serverId\":\"{server}\",\"sequence\":7,\"global\":false}}"),
                format!("{{\"serverId\":\"{server}\",\"sequence\":7,\"global\":true}}"),
                format!("{{\"serverId\":\"{server}\",\"sequence\":null,\"global\":true}}"),
                format!("{{\"serverId\":\"{server}\",\"global\":true}}"),
            ]
        );
    }
}
