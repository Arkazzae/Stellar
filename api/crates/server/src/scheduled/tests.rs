//! What becomes of a scheduled message when its time comes: it is posted once
//! whatever restarts or races meet it, a refusal names its reason, slow mode
//! only delays it, and its files are kept for as long as it waits.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::send::{self, Attempt};
use super::store::{self, Scheduled, Target};
use super::*;
use crate::auth::Caller;
use crate::database::Database;
use crate::events::{Event, EventPublisher, Unconnected};
use crate::http::Path;
use crate::messages::{Reason, Stop};
use crate::permissions::fixture::World;
use crate::permissions::privileges::VIEW_CHANNEL;
use crate::seal::Sealer;
use crate::settings::{DatabaseSettings, Settings};
use crate::testing::TempDir;
use crate::values::{Id, Timestamp};

const MINUTE: Duration = Duration::from_secs(60);
const DAY: Duration = Duration::from_secs(86_400);

/// Every event the routes and the worker published.
#[derive(Default)]
struct Recorder(Mutex<Vec<Event>>);

impl EventPublisher for Recorder {
    fn publish(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

/// A server with its people and channels, and the state the worker runs on.
struct Place {
    world: World,
    data: TempDir,
    state: AppState,
    events: Arc<Recorder>,
}

fn state_on(database: &Database, data: &TempDir) -> (AppState, Arc<Recorder>) {
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
        database.clone(),
        events.clone(),
        Arc::new(Unconnected),
        Sealer::from_key([5; 32]),
        Sealer::from_key([6; 32]),
    );
    (state, events)
}

impl Place {
    fn new() -> Place {
        let world = World::new();
        let data = TempDir::new();
        let (state, events) = state_on(&world.database, &data);
        Place {
            world,
            data,
            state,
            events,
        }
    }

    /// The server started again on the same files, with nothing of the last
    /// run's memory.
    fn restart(&mut self) {
        let database = Database::open(&DatabaseSettings {
            path: self.world.database.path().to_path_buf(),
            busy_timeout: Duration::from_secs(30),
        })
        .unwrap();
        (self.state, self.events) = state_on(&database, &self.data);
    }

    fn channel(&self, channel: Id) -> Target {
        Target::Channel {
            server: self.world.server,
            channel,
        }
    }

    /// Schedules a message two minutes ahead, through the checks the routes
    /// make.
    async fn schedule(
        &self,
        author: Id,
        target: Target,
        content: &str,
        attachments: Vec<Id>,
        reply_to: Option<Id>,
    ) -> Result<Id, Stop> {
        let messages = *self.state.messages();
        let now = Timestamp::now();
        let scheduled = Scheduled {
            id: Id::random(),
            author,
            target,
            content: content.to_string(),
            reply_to,
            send_at: now.plus(2 * MINUTE),
            created_at: now,
            revision: 0,
            failure: None,
            attachments,
        };
        let id = scheduled.id;
        self.state
            .database()
            .write(move |tx| super::routes::make(tx, &messages, &scheduled))
            .await
            .map(|_| id)
    }

    /// A message without files that the checks of the routes let through.
    async fn accepted(&self, author: Id, target: Target, content: &str, reply_to: Option<Id>) -> Id {
        self.schedule(author, target, content, Vec::new(), reply_to)
            .await
            .unwrap()
    }

    async fn in_general(&self, author: Id, content: &str) -> Id {
        self.accepted(author, self.channel(self.world.general), content, None)
            .await
    }

    /// Its time has come, as if the clock had moved on: it was made an hour
    /// ago for a second ago.
    fn due(&self, id: Id) {
        let now = Timestamp::now();
        self.world
            .db
            .execute(
                "UPDATE \"ScheduledMessages\" SET \"SendAt\" = ?2, \"DueAt\" = ?2, \"CreatedAt\" = ?3 WHERE \"Id\" = ?1",
                params![id, now.minus(Duration::from_secs(1)), now.minus(60 * MINUTE)],
            )
            .unwrap();
    }

    async fn run(&self) {
        pass(&self.state, Timestamp::now()).await.unwrap();
    }

    fn read(&self, id: Id) -> Scheduled {
        store::find(&self.world.db, author_of(&self.world.db, id), id)
            .unwrap()
            .unwrap()
    }

    /// The channel messages with this text, as their ids and times.
    fn posted(&self, content: &str) -> Vec<(Id, Timestamp)> {
        self.world
            .db
            .prepare("SELECT \"Id\", \"CreatedAt\" FROM \"Messages\" WHERE \"Content\" = ?1")
            .unwrap()
            .query_map([content], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// None once it is gone; its failure code while it is there, none while
    /// it is pending.
    fn left(&self, id: Id) -> Option<Option<String>> {
        self.world
            .db
            .query_row(
                "SELECT \"FailureCode\" FROM \"ScheduledMessages\" WHERE \"Id\" = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
    }

    fn events_named(&self, name: &str) -> Vec<(Value, Value)> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.name == name)
            .map(|event| {
                (
                    serde_json::to_value(&event.audience).unwrap(),
                    serde_json::from_str(&event.payload).unwrap(),
                )
            })
            .collect()
    }

    /// A message somebody wrote in a channel.
    fn message(&self, channel: Id, author: Id, content: &str) -> Id {
        let id = Id::random();
        self.world
            .db
            .execute(
                "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"EditedAt\", \"PinnedAt\", \"ReplyToMessageId\")
                 VALUES (?1, ?2, ?3, 0, ?4, ?5, NULL, NULL, NULL)",
                params![id, channel, author, content, Timestamp::now()],
            )
            .unwrap();
        id
    }

    fn conversation(&self, first: Id, second: Id) -> Id {
        let (a, b) = crate::social::pair(first, second);
        let id = Id::random();
        self.world
            .db
            .execute(
                "INSERT INTO \"DirectConversations\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\", \"EncryptionState\",
                     \"EncryptionRequestedById\", \"EncryptionRequestedAt\", \"EncryptionEnabledAt\", \"CurrentEpochId\")
                 VALUES (?1, ?2, ?3, ?4, 'Off', NULL, NULL, NULL, NULL)",
                params![id, a, b, Timestamp::now()],
            )
            .unwrap();
        id
    }

    /// A file a person uploaded at `at`.
    fn upload(&self, owner: Id, at: Timestamp) -> Id {
        let id = Id::random();
        self.world
            .db
            .execute(
                "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"IsEncrypted\", \"CreatedAt\")
                 VALUES (?1, ?2, ?3, 'plan.pdf', 'application/pdf', 100, 0, ?4)",
                params![id, owner, format!("test/{id}"), at],
            )
            .unwrap();
        id
    }

    fn file_exists(&self, id: Id) -> bool {
        self.world
            .db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM \"ImageAttachments\" WHERE \"Id\" = ?1)",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }
}

fn author_of(db: &rusqlite::Connection, id: Id) -> Id {
    db.query_row(
        "SELECT \"AuthorId\" FROM \"ScheduledMessages\" WHERE \"Id\" = ?1",
        [id],
        |row| row.get(0),
    )
    .unwrap()
}

#[tokio::test]
async fn a_due_message_goes_out_once_under_its_own_id_at_the_time_it_is_sent() {
    let place = Place::new();
    let ilya = place.world.ilya;
    let file = place.upload(ilya, Timestamp::now());
    let id = place
        .schedule(ilya, place.channel(place.world.general), "good morning", vec![file], None)
        .await
        .unwrap();
    place.due(id);
    let scheduled = place.read(id);
    let (kai, lea) = (place.world.user("kai"), place.world.user("lea"));
    let conversation = place.conversation(kai, lea);
    let direct = place
        .accepted(kai, Target::Conversation(conversation), "see you at nine", None)
        .await;
    place.due(direct);

    place.run().await;
    place.run().await;

    let posted = place.posted("good morning");
    assert_eq!(posted.len(), 1, "a second pass finds nothing left to send");
    let (message, created_at) = posted[0];
    assert_eq!(message, id);
    assert!(created_at >= scheduled.send_at);
    assert!(
        scheduled.created_at.until(created_at) > 59 * MINUTE,
        "it carries the time it went out, not the time it was scheduled"
    );
    let attached: Vec<Id> = place
        .world
        .db
        .prepare("SELECT \"AttachmentId\" FROM \"MessageAttachments\" WHERE \"MessageId\" = ?1")
        .unwrap()
        .query_map([id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(attached, vec![file]);
    assert_eq!(place.left(id), None);

    let sent: Vec<(Id, String)> = place
        .world
        .db
        .prepare("SELECT \"Id\", \"Content\" FROM \"DirectMessages\" WHERE \"ConversationId\" = ?1")
        .unwrap()
        .query_map([conversation], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(sent, vec![(direct, "see you at nine".to_string())]);
    assert_eq!(place.left(direct), None);

    // Neither was there to read what came before.
    let marked = |sql: &str, place_id: Id, user: Id| -> bool {
        place
            .world
            .db
            .query_row(sql, params![place_id, user], |_| Ok(()))
            .optional()
            .unwrap()
            .is_some()
    };
    assert!(!marked(
        "SELECT 1 FROM \"ChannelReads\" WHERE \"ChannelId\" = ?1 AND \"UserId\" = ?2",
        place.world.general,
        ilya
    ));
    assert!(!marked(
        "SELECT 1 FROM \"DirectConversationReads\" WHERE \"ConversationId\" = ?1 AND \"UserId\" = ?2",
        conversation,
        kai
    ));

    let mut removed = place.events_named(super::REMOVED);
    removed.sort_by_key(|(audience, _)| audience.to_string());
    let mut expected = vec![
        (json!({ "users": [ilya] }), json!({ "id": id, "sent": true })),
        (json!({ "users": [kai] }), json!({ "id": direct, "sent": true })),
    ];
    expected.sort_by_key(|(audience, _)| audience.to_string());
    assert_eq!(removed, expected);
    assert_eq!(place.events_named("MessageCreated").len(), 1);
    assert_eq!(place.events_named("DirectMessageCreated").len(), 1);
}

#[tokio::test]
async fn a_restart_between_posting_and_clearing_the_queue_posts_the_message_once() {
    let mut place = Place::new();
    let ilya = place.world.ilya;

    // The process dies after the message is written and before the
    // transaction commits: the message and the queue go back together.
    let id = place.in_general(ilya, "good morning").await;
    place.due(id);
    place
        .world
        .db
        .execute_batch(
            "CREATE TRIGGER PowerCut AFTER INSERT ON \"Messages\" WHEN new.\"Content\" = 'good morning'
             BEGIN SELECT RAISE(ABORT, 'the power went'); END;",
        )
        .unwrap();
    place.run().await;
    assert!(place.posted("good morning").is_empty());
    assert_eq!(place.left(id), Some(None), "it still waits");

    place.world.db.execute_batch("DROP TRIGGER PowerCut").unwrap();
    place.restart();
    // A fault of the server is tried again half a minute later.
    let later = Timestamp::now().plus(Duration::from_secs(31));
    pass(&place.state, later).await.unwrap();
    pass(&place.state, later).await.unwrap();
    assert_eq!(place.posted("good morning").len(), 1);
    assert_eq!(place.left(id), None);

    // What a crash between storing the message and clearing the row would
    // leave if they were two transactions: the message under the scheduled
    // id, and the row still there. The send meets the stored message and
    // only clears the row.
    let second = place.in_general(ilya, "second thoughts").await;
    let general = place.world.general;
    place
        .world
        .db
        .execute(
            "INSERT INTO \"Messages\" (\"Id\", \"ChannelId\", \"AuthorId\", \"IsSystem\", \"Content\", \"CreatedAt\", \"EditedAt\", \"PinnedAt\", \"ReplyToMessageId\")
             VALUES (?1, ?2, ?3, 0, 'second thoughts', ?4, NULL, NULL, NULL)",
            params![second, general, ilya, Timestamp::now()],
        )
        .unwrap();
    place.due(second);
    place.run().await;
    assert_eq!(place.posted("second thoughts").len(), 1);
    assert_eq!(place.left(second), None);
}

#[tokio::test]
async fn a_send_that_meets_another_send_an_edit_or_a_delete_posts_once_and_only_the_current_text() {
    let place = Place::new();
    let ilya = place.world.ilya;

    // "Send now" and the worker take the same message at the same moment.
    let id = place.in_general(ilya, "both at once").await;
    place.due(id);
    let row = place.read(id);
    let (first, second) = tokio::join!(
        send::attempt(&place.state, &row),
        send::attempt(&place.state, &row)
    );
    let sent = [&first, &second]
        .iter()
        .filter(|attempt| matches!(attempt, Attempt::Sent(_)))
        .count();
    assert_eq!(sent, 1);
    assert!(matches!(
        [&first, &second].iter().find(|attempt| !matches!(attempt, Attempt::Sent(_))),
        Some(Attempt::Replayed(_))
    ));
    assert_eq!(place.posted("both at once").len(), 1);
    assert_eq!(place.left(id), None);

    // The author changes the text after a send has read it: the old text is
    // not posted, and the new one goes at the next look.
    let id = place.in_general(ilya, "old text").await;
    let stale = place.read(id);
    let edited = Scheduled {
        content: "new text".to_string(),
        ..stale.clone()
    };
    assert!(store::edit(&place.world.db, &edited).unwrap());
    assert!(matches!(
        send::attempt(&place.state, &stale).await,
        Attempt::Refused(Stop::Refused(Reason::Withdrawn, _))
    ));
    assert!(place.posted("old text").is_empty());
    place.due(id);
    place.run().await;
    assert_eq!(place.posted("new text").len(), 1);

    // The author deletes it after a send has read it: nothing is posted.
    let id = place.in_general(ilya, "never mind").await;
    let stale = place.read(id);
    store::remove(&place.world.db, id).unwrap();
    assert!(matches!(
        send::attempt(&place.state, &stale).await,
        Attempt::Refused(Stop::Refused(Reason::Withdrawn, _))
    ));
    assert!(place.posted("never mind").is_empty());
}

#[tokio::test]
async fn a_conversation_encrypted_before_the_time_fails_the_message_and_gets_no_plain_text() {
    let place = Place::new();
    let (kai, lea) = (place.world.user("kai"), place.world.user("lea"));
    let conversation = place.conversation(kai, lea);
    let id = place
        .accepted(kai, Target::Conversation(conversation), "the plan", None)
        .await;

    // They switch encryption on before the time comes.
    place
        .world
        .db
        .execute(
            "UPDATE \"DirectConversations\" SET \"EncryptionState\" = 'On', \"EncryptionEnabledAt\" = ?2 WHERE \"Id\" = ?1",
            params![conversation, Timestamp::now()],
        )
        .unwrap();
    place.due(id);
    place.run().await;

    assert_eq!(place.left(id), Some(Some("encrypted".to_string())));
    let stored: i64 = place
        .world
        .db
        .query_row(
            "SELECT COUNT(*) FROM \"DirectMessages\" WHERE \"ConversationId\" = ?1",
            [conversation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0);
    let changed = place.events_named(super::CHANGED);
    assert_eq!(changed.len(), 1);
    let (audience, failed) = &changed[0];
    assert_eq!(audience, &json!({ "users": [kai] }));
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["failure"]["code"], "encrypted");
}

#[tokio::test]
async fn what_changed_before_the_time_is_the_reason_the_message_failed() {
    let place = Place::new();
    let w = &place.world;
    let hidden = w.channel(None);
    let doomed = w.channel(None);
    let root = place.message(w.general, w.ada, "the root");
    let (kai, lea, mo, nia) = (w.user("kai"), w.user("lea"), w.user("mo"), w.user("nia"));
    let kai_lea = place.conversation(kai, lea);
    let mo_lea = place.conversation(mo, lea);
    let lea_nia = place.conversation(lea, nia);

    let general = place.channel(w.general);
    let cases = [
        ("view taken away", place.accepted(w.ilya, place.channel(hidden), "a", None).await, "no_access"),
        ("banned", place.accepted(w.nils, general, "b", None).await, "no_access"),
        ("kicked", place.accepted(w.jo, general, "c", None).await, "no_access"),
        ("channel deleted", place.accepted(w.ada, place.channel(doomed), "d", None).await, "target_gone"),
        ("thread deleted", place.accepted(w.maja, general, "e", Some(root)).await, "thread_gone"),
        ("blocked", place.accepted(kai, Target::Conversation(kai_lea), "f", None).await, "no_access"),
        ("disabled", place.accepted(mo, Target::Conversation(mo_lea), "g", None).await, "no_access"),
        ("conversation gone", place.accepted(lea, Target::Conversation(lea_nia), "h", None).await, "target_gone"),
    ];

    w.member_override(false, hidden, w.ilya, 0, VIEW_CHANNEL);
    w.db.execute(
        "INSERT INTO \"ServerBans\" (\"ServerId\", \"UserId\", \"BannedById\", \"Reason\", \"BannedAt\") VALUES (?1, ?2, ?3, '', ?4)",
        params![w.server, w.nils, w.maja, Timestamp::now()],
    )
    .unwrap();
    crate::permissions::ownership::remove_member(&w.db, w.server, w.nils).unwrap();
    crate::permissions::ownership::remove_member(&w.db, w.server, w.jo).unwrap();
    w.db.execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [doomed]).unwrap();
    w.db.execute("DELETE FROM \"Messages\" WHERE \"Id\" = ?1", [root]).unwrap();
    w.db.execute(
        "INSERT INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
        params![lea, kai, Timestamp::now()],
    )
    .unwrap();
    w.disable(mo);
    w.db.execute("DELETE FROM \"DirectConversations\" WHERE \"Id\" = ?1", [lea_nia]).unwrap();

    for (_, id, _) in &cases {
        place.due(*id);
    }
    place.run().await;

    for (case, id, code) in &cases {
        assert_eq!(place.left(*id), Some(Some(code.to_string())), "{case}");
    }
    // The list still says where it was for, without the name of a channel
    // that is gone.
    let gone = store::dto(&w.db, w.ada, cases[3].1).unwrap().unwrap();
    let target = serde_json::to_value(&gone.target).unwrap();
    assert_eq!(target["serverName"], "Test");
    assert_eq!(target["channelId"], json!(doomed));
    assert_eq!(target["channelName"], Value::Null);
    for content in ["a", "b", "c", "d", "e"] {
        assert!(place.posted(content).is_empty(), "{content}");
    }
    let direct: i64 = w
        .db
        .query_row("SELECT COUNT(*) FROM \"DirectMessages\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(direct, 0);
}

#[tokio::test]
async fn slow_mode_holds_a_message_back_and_does_not_fail_it() {
    let place = Place::new();
    let (w, ilya) = (&place.world, place.world.ilya);
    let slow = w.channel(None);
    w.db.execute(
        "UPDATE \"Channels\" SET \"SlowModeSeconds\" = 3600 WHERE \"Id\" = ?1",
        [slow],
    )
    .unwrap();
    // Ilya has just posted there, and schedules the next one anyway.
    w.db.execute(
        "INSERT INTO \"ChannelPostTimes\" (\"ChannelId\", \"UserId\", \"LastPostedAt\") VALUES (?1, ?2, ?3)",
        params![slow, ilya, Timestamp::now()],
    )
    .unwrap();
    let id = place.accepted(ilya, place.channel(slow), "patience", None).await;
    place.due(id);
    let send_at = place.read(id).send_at;

    place.run().await;
    assert!(place.posted("patience").is_empty());
    assert_eq!(place.left(id), Some(None), "it still waits");
    let (kept, due_at): (Timestamp, Timestamp) = w
        .db
        .query_row(
            "SELECT \"SendAt\", \"DueAt\" FROM \"ScheduledMessages\" WHERE \"Id\" = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(kept, send_at, "the time the author chose stays");
    assert!(Timestamp::now().until(due_at) > 59 * MINUTE, "it is tried once the window has passed");
    assert!(place.events_named(super::CHANGED).is_empty());

    // The window passes.
    w.db.execute(
        "UPDATE \"ChannelPostTimes\" SET \"LastPostedAt\" = ?3 WHERE \"ChannelId\" = ?1 AND \"UserId\" = ?2",
        params![slow, ilya, Timestamp::now().minus(2 * 60 * MINUTE)],
    )
    .unwrap();
    pass(&place.state, due_at).await.unwrap();
    assert_eq!(place.posted("patience").len(), 1);
}

#[tokio::test]
async fn scheduled_files_outlive_housekeeping_and_go_with_their_message() {
    let place = Place::new();
    let ilya = place.world.ilya;
    let long_ago = Timestamp::now().minus(10 * DAY);
    let attached = place.upload(ilya, long_ago);
    let orphan = place.upload(ilya, long_ago);
    let id = place
        .schedule(ilya, place.channel(place.world.general), "", vec![attached], None)
        .await
        .unwrap();

    crate::housekeeping::run(&place.state, Timestamp::now(), &|| false)
        .await
        .unwrap();
    assert!(place.file_exists(attached), "a scheduled message holds its file");
    assert!(!place.file_exists(orphan));

    let answer = super::routes::delete(
        State(place.state.clone()),
        Caller {
            user: ilya,
            session: Id::random(),
        },
        Path(id),
    )
    .await
    .unwrap();
    assert_eq!(answer, StatusCode::NO_CONTENT);
    assert!(!place.file_exists(attached), "deleting the message takes its file");

    // A failed message keeps its file for its author to see, until it
    // expires thirty days after it failed; then the file is an orphan.
    let kept = place.upload(ilya, long_ago);
    let failed = place
        .schedule(ilya, place.channel(place.world.general), "", vec![kept], None)
        .await
        .unwrap();
    let pending = place.in_general(ilya, "still waiting").await;
    place
        .world
        .db
        .execute(
            "UPDATE \"ScheduledMessages\" SET \"FailureCode\" = 'no_access', \"FailedAt\" = ?2 WHERE \"Id\" = ?1",
            params![failed, Timestamp::now().minus(31 * DAY)],
        )
        .unwrap();
    crate::housekeeping::run(&place.state, Timestamp::now(), &|| false)
        .await
        .unwrap();
    assert_eq!(place.left(failed), None);
    assert_eq!(place.left(pending), Some(None), "a pending one is not touched");
    crate::housekeeping::run(&place.state, Timestamp::now(), &|| false)
        .await
        .unwrap();
    assert!(!place.file_exists(kept));
}
