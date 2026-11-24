//! What handlers tell the realtime hub, and how.
//!
//! A handler that changes something people are looking at commits the change
//! to the database first and publishes the event after, once `write` has
//! returned. An event that is lost cannot undo a change, and a change that
//! failed never announces itself.
//!
//! ```ignore
//! state.events().publish(Event::new(Audience::Server(server_id), "ChannelCreated", &dto));
//! ```
//!
//! The hub implements [`EventPublisher`]. Until it is plugged in the process
//! holds [`Unconnected`], which has nobody to send to.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;

use crate::values::Id;

/// Who an event is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Audience {
    /// Everyone connected who is on the server.
    Server(Id),
    /// These people, on whichever connections they have.
    Users(Vec<Id>),
    /// One connection, by the identifier the hub gave it.
    Connection(String),
}

/// A named event with the JSON the clients receive for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub audience: Audience,
    pub name: String,
    /// The payload, already written as JSON.
    pub payload: String,
}

impl Event {
    pub fn new(audience: Audience, name: &str, payload: &impl Serialize) -> Event {
        // A payload that cannot be written as JSON is a mistake in the code
        // that built it; the request that hit it fails like any other bug.
        let payload = serde_json::to_string(payload).expect("an event payload is written as JSON");
        Event {
            audience,
            name: name.to_string(),
            payload,
        }
    }
}

pub trait EventPublisher: Send + Sync {
    /// Hands the event over. It does not wait for delivery and cannot fail.
    fn publish(&self, event: Event);
}

/// What the hub knows of who is connected, which the API reads to say whether
/// a person is `online`, `idle`, `dnd` or `offline` to themselves.
pub trait Presence: Send + Sync {
    /// The account's own view of its status; being disconnected wins over any
    /// choice it made.
    fn presence_of(&self, user: Id) -> String;
}

impl Presence for Unconnected {
    fn presence_of(&self, _user: Id) -> String {
        "offline".to_string()
    }
}

/// The publisher of a process that has no hub: nobody is connected, so there
/// is nobody to tell.
#[derive(Debug, Default)]
pub struct Unconnected;

impl EventPublisher for Unconnected {
    fn publish(&self, _event: Event) {}
}

/// Keeps what was published, for tests of handlers that run in the same
/// process.
#[derive(Debug, Default)]
pub struct RecordingEvents {
    events: Mutex<Vec<Event>>,
}

impl RecordingEvents {
    /// Everything published so far, oldest first.
    pub fn published(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Everything published so far, forgotten after.
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

impl EventPublisher for RecordingEvents {
    fn publish(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }
}

/// For the integration tests, which run the real binary and so cannot look
/// into its memory: appends each event to a file as one line of JSON,
/// `{"audience":{"server":"..."},"name":"...","payload":{...}}`. It exists
/// only in the Testing environment (`Testing:EventLog`).
#[derive(Debug)]
pub struct EventFile {
    path: PathBuf,
    gate: Mutex<()>,
}

impl EventFile {
    pub fn new(path: PathBuf) -> EventFile {
        EventFile {
            path,
            gate: Mutex::new(()),
        }
    }
}

#[derive(Serialize)]
struct Line<'a> {
    audience: &'a Audience,
    name: &'a str,
    payload: serde_json::Value,
}

impl EventPublisher for EventFile {
    fn publish(&self, event: Event) {
        let payload = serde_json::from_str(&event.payload).unwrap_or(serde_json::Value::Null);
        let Ok(mut line) = serde_json::to_string(&Line {
            audience: &event.audience,
            name: &event.name,
            payload,
        }) else {
            return;
        };
        line.push('\n');
        let _turn = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if let Err(error) = written {
            tracing::warn!(
                "the event log {} could not be written: {error}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Renamed {
        channel_id: Id,
        name: String,
    }

    fn payload() -> Renamed {
        Renamed {
            channel_id: Id::parse("8F62A4D1-BDCB-4D86-BD1C-6E351900E629").unwrap(),
            name: "general".to_string(),
        }
    }

    #[test]
    fn an_event_carries_its_payload_as_the_clients_will_read_it() {
        let server = Id::random();

        let event = Event::new(Audience::Server(server), "ChannelRenamed", &payload());

        assert_eq!(
            event.payload,
            "{\"channelId\":\"8f62a4d1-bdcb-4d86-bd1c-6e351900e629\",\"name\":\"general\"}"
        );
    }

    #[test]
    fn the_recorder_keeps_events_in_the_order_they_were_published() {
        let recorder = RecordingEvents::default();
        let (maya, jo) = (Id::random(), Id::random());

        recorder.publish(Event::new(
            Audience::Users(vec![maya, jo]),
            "Typing",
            &payload(),
        ));
        recorder.publish(Event::new(
            Audience::Connection("abc".to_string()),
            "Hello",
            &payload(),
        ));

        let published = recorder.published();
        assert_eq!(published.len(), 2);
        assert_eq!(published[0].audience, Audience::Users(vec![maya, jo]));
        assert_eq!(published[1].name, "Hello");
        assert_eq!(recorder.take().len(), 2);
        assert!(recorder.published().is_empty());
    }

    #[test]
    fn the_file_gets_one_line_of_json_for_each_event() {
        let folder = TempDir::new();
        let file = EventFile::new(folder.path().join("events.jsonl"));
        let server = Id::parse("3d82da44-8aa5-4bdf-9ee1-2902df99880c").unwrap();

        file.publish(Event::new(
            Audience::Server(server),
            "ChannelRenamed",
            &payload(),
        ));
        file.publish(Event::new(
            Audience::Connection("abc".to_string()),
            "Hello",
            &payload(),
        ));

        let text = std::fs::read_to_string(folder.path().join("events.jsonl")).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0]["audience"]["server"],
            "3d82da44-8aa5-4bdf-9ee1-2902df99880c"
        );
        assert_eq!(lines[0]["name"], "ChannelRenamed");
        assert_eq!(lines[0]["payload"]["name"], "general");
        assert_eq!(lines[1]["audience"]["connection"], "abc");
    }
}
