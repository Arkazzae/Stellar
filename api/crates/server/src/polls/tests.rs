//! Polls below the routes: what deleting takes with it, what a page of polls
//! costs, and the worker that ends them on time.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::params;
use serde_json::{Value, json};
use stellarr_messages::{Language, Messages};

use super::*;
use crate::events::{Event, EventPublisher, Unconnected};
use crate::messages::dto;
use crate::messages::post::{Posted, Submission, post};
use crate::permissions::fixture::World;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::state::AppState;
use crate::testing::TempDir;

/// Every event the worker published.
#[derive(Default)]
struct Recorder(Mutex<Vec<Event>>);

impl EventPublisher for Recorder {
    fn publish(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

fn messages() -> Messages {
    Messages::new(Language::English)
}

fn draft(question: &str, answers: usize) -> Draft {
    Draft {
        question: question.to_string(),
        answers: (1..=answers).map(|n| (format!("answer {n}"), None)).collect(),
        multiple: false,
        length: Length::Hour,
    }
}

/// Posts a poll through the post path, as the route does.
fn posted(world: &World, channel: Id, author: Id, question: &str) -> Id {
    let submission = Submission {
        channel,
        author,
        content: String::new(),
        attachments: Vec::new(),
        reply_to: None,
        client_id: None,
        scheduled: None,
        poll: Some(draft(question, 3)),
        event: None,
    };
    match post(&world.db, &messages(), &Unconnected, world.server, &submission) {
        Ok(Posted::Stored(stored)) => stored.message.id,
        other => panic!("the poll was not posted: {other:?}"),
    }
}

fn vote(world: &World, message: Id, user: Id, answer: i64) {
    world
        .db
        .execute(
            "INSERT INTO \"PollVotes\" (\"MessageId\", \"AnswerId\", \"UserId\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4)",
            params![message, answer, user, Timestamp::now()],
        )
        .unwrap();
}

/// How many rows of a message's poll are left: the poll, its answers, its
/// votes.
fn left(world: &World, message: Id) -> (i64, i64, i64) {
    let count = |table: &str| -> i64 {
        world
            .db
            .query_row(
                &format!("SELECT COUNT(*) FROM \"{table}\" WHERE \"MessageId\" = ?1"),
                [message],
                |row| row.get(0),
            )
            .unwrap()
    };
    (count("Polls"), count("PollAnswers"), count("PollVotes"))
}

#[test]
fn deleting_a_message_its_channel_or_its_server_takes_its_poll_and_votes() {
    let world = World::new();
    let doomed = world.channel(None);
    let by_message = posted(&world, world.general, world.maja, "message");
    let by_channel = posted(&world, doomed, world.maja, "channel");
    let by_server = posted(&world, world.projects, world.maja, "server");
    for message in [by_message, by_channel, by_server] {
        vote(&world, message, world.ilya, 1);
        vote(&world, message, world.nils, 2);
        assert_eq!(left(&world, message), (1, 3, 2));
    }

    world
        .db
        .execute("DELETE FROM \"Messages\" WHERE \"Id\" = ?1", [by_message])
        .unwrap();
    assert_eq!(left(&world, by_message), (0, 0, 0));
    assert_eq!(left(&world, by_channel), (1, 3, 2), "the others stay");

    world
        .db
        .execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [doomed])
        .unwrap();
    assert_eq!(left(&world, by_channel), (0, 0, 0));
    assert_eq!(left(&world, by_server), (1, 3, 2));

    world
        .db
        .execute("DELETE FROM \"Servers\" WHERE \"Id\" = ?1", [world.server])
        .unwrap();
    assert_eq!(left(&world, by_server), (0, 0, 0));
}

#[test]
fn a_page_of_polls_costs_the_statements_of_one_and_a_page_without_none() {
    let mut world = World::new();
    let plain: Vec<Id> = (0..5)
        .map(|n| {
            let submission = Submission {
                channel: world.general,
                author: world.maja,
                content: format!("plain {n}"),
                attachments: Vec::new(),
                reply_to: None,
                client_id: None,
                scheduled: None,
                poll: None,
                event: None,
            };
            match post(&world.db, &messages(), &Unconnected, world.server, &submission) {
                Ok(Posted::Stored(stored)) => stored.message.id,
                other => panic!("not posted: {other:?}"),
            }
        })
        .collect();
    let polls: Vec<Id> = (0..5)
        .map(|n| posted(&world, world.general, world.maja, &format!("poll {n}")))
        .collect();
    for (n, message) in polls.iter().enumerate() {
        vote(&world, *message, world.ilya, 1 + (n as i64 % 3));
    }

    let mut cost = |ids: &[Id]| {
        world.count_statements();
        let loaded = dto::load(&world.db, &messages(), ids, Some(world.ilya)).unwrap();
        assert_eq!(loaded.len(), ids.len());
        world.statements().len()
    };
    let without = cost(&plain);
    let one = cost(&polls[..1]);
    let all = cost(&polls);
    assert_eq!(all, one, "five polls cost what one does");
    assert_eq!(one, without + 3, "the polls, their answers and the reader's votes");
    assert_eq!(cost(&plain[..1]), without, "a page without a poll reads none");
}

#[tokio::test]
async fn the_worker_ends_a_due_poll_once_at_its_time_and_leaves_the_others() {
    let world = World::new();
    let data = TempDir::new();
    let settings = Settings::load(&[
        ("STELLARR_ENVIRONMENT".to_string(), "Testing".to_string()),
        (
            "STELLARR_DATA_ROOT".to_string(),
            data.path().display().to_string(),
        ),
    ])
    .unwrap();
    let events = Arc::new(Recorder::default());
    let state = AppState::new(
        settings,
        world.database.clone(),
        events.clone(),
        Arc::new(Unconnected),
        Sealer::from_key([5; 32]),
        Sealer::from_key([6; 32]),
    );
    let due = posted(&world, world.general, world.maja, "due");
    let early = posted(&world, world.general, world.maja, "ended early");
    let running = posted(&world, world.general, world.maja, "running");
    vote(&world, due, world.ilya, 2);
    let was_due = Timestamp::now().minus(Duration::from_secs(600));
    world
        .db
        .execute(
            "UPDATE \"Polls\" SET \"EndsAt\" = ?2 WHERE \"MessageId\" IN (?1, ?3)",
            params![due, was_due, early],
        )
        .unwrap();
    let ended_early = was_due.minus(Duration::from_secs(60));
    world
        .db
        .execute(
            "UPDATE \"Polls\" SET \"EndedAt\" = ?2, \"EndedById\" = ?3 WHERE \"MessageId\" = ?1",
            params![early, ended_early, world.maja],
        )
        .unwrap();
    let ends: Timestamp = world
        .db
        .query_row(
            "SELECT \"EndsAt\" FROM \"Polls\" WHERE \"MessageId\" = ?1",
            [running],
            |row| row.get(0),
        )
        .unwrap();

    let next = pass(&state, Timestamp::now()).await.unwrap();
    assert_eq!(next, Some(ends), "it sleeps until the running poll ends");
    assert_eq!(pass(&state, Timestamp::now()).await.unwrap(), Some(ends));

    let ended = |message: Id| -> (Option<Timestamp>, Option<Id>) {
        world
            .db
            .query_row(
                "SELECT \"EndedAt\", \"EndedById\" FROM \"Polls\" WHERE \"MessageId\" = ?1",
                [message],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(ended(due), (Some(was_due), None), "ended at the time it was due");
    assert_eq!(ended(early), (Some(ended_early), Some(world.maja)));
    assert_eq!(ended(running), (None, None));

    let recorded = events.0.lock().unwrap();
    let updates: Vec<(Value, Value)> = recorded
        .iter()
        .filter(|event| event.name == "PollUpdated")
        .map(|event| {
            (
                serde_json::to_value(&event.audience).unwrap(),
                serde_json::from_str(&event.payload).unwrap(),
            )
        })
        .collect();
    assert_eq!(updates.len(), 1, "one end, told once");
    let (audience, update) = &updates[0];
    assert_eq!(audience, &json!({ "server": world.server }));
    assert_eq!(update["messageId"], json!(due));
    assert_eq!(update["endedAt"], json!(was_due));
    assert_eq!(update["endedBy"], Value::Null);
    assert_eq!(update["voters"], 1);
    assert_eq!(update["answers"], json!([{ "id": 1, "votes": 0 }, { "id": 2, "votes": 1 }, { "id": 3, "votes": 0 }]));
    assert!(update.get("mine").is_none(), "the end names nobody's choice");
    assert!(recorded.iter().all(|event| event.name != "MessageCreated"), "no message is posted");
}
