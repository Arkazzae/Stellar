//! Polls through the real binary: their limits, voting, ending them, who
//! chose what, where they are refused, and the worker that ends them on time.
//! The counts go to everyone who sees the channel; a person's own choice goes
//! to that person alone.

use std::time::Duration;

use http::Method;
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::{Id, Timestamp};

use crate::support::{Person, Request, Response, Server, TempDir};

/// An instance with its owner, two members, and its first text channel.
struct World {
    server: Server,
    owner: Person,
    maya: Person,
    joe: Person,
    general: String,
    instance: String,
}

async fn world() -> World {
    let server = Server::start().await;
    arrange(server).await
}

async fn arrange(server: Server) -> World {
    let owner = server.found("owner").await;
    let maya = server.register("maya").await;
    let joe = server.register("joe").await;
    let db = server.database();
    let general: String = db
        .query_row(
            "SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let instance: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
    World {
        server,
        owner,
        maya,
        joe,
        general: general.to_lowercase(),
        instance: instance.to_lowercase(),
    }
}

fn three(multiple: bool) -> Value {
    json!({
        "question": "When do we play?",
        "answers": [{ "text": "Friday" }, { "text": "Saturday", "emoji": "🎲" }, { "text": "Sunday" }],
        "multiple": multiple,
    })
}

impl World {
    async fn post(&self, who: &Person, channel: &str, body: Value) -> Response {
        who.client.post_json(&format!("/api/channels/{channel}/messages"), &body).await
    }

    /// A poll in general, and its message as the author was answered.
    async fn poll(&self, who: &Person, poll: Value) -> Value {
        let response = self.post(who, &self.general, json!({ "content": "", "poll": poll })).await;
        assert_eq!(response.status, 201, "{}", response.text());
        response.json()
    }

    async fn vote(&self, who: &Person, message: &Value, answers: Value) -> Response {
        who.client
            .send(
                Request::new(Method::PUT, &format!("/api/messages/{}/poll/vote", message["id"].as_str().unwrap()))
                    .json(&json!({ "answers": answers })),
            )
            .await
    }

    /// Votes and says the poll as the voter was answered.
    async fn voted(&self, who: &Person, message: &Value, answers: Value) -> Value {
        let response = self.vote(who, message, answers).await;
        assert_eq!(response.status, 200, "{}", response.text());
        response.json()
    }

    async fn end(&self, who: &Person, message: &Value) -> Response {
        who.client
            .send(Request::new(Method::POST, &format!("/api/messages/{}/poll/end", message["id"].as_str().unwrap())))
            .await
    }

    fn events(&self, name: &str) -> Vec<Value> {
        self.server.events().into_iter().filter(|event| event["name"] == name).collect()
    }

    fn count(&self, sql: &str) -> i64 {
        self.server.database().query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// Where `who` resumes listening to general's journal.
    async fn sync_cursor(&self, who: &Person) -> String {
        let page: Value = who.client.get(&format!("/api/channels/{}/messages", self.general)).await.json();
        page["syncCursor"].as_str().unwrap().to_string()
    }

    /// The poll of a message as `who` catches up on general after `cursor`.
    async fn synced(&self, who: &Person, cursor: &str, message: &Value) -> Value {
        let page: Value = who
            .client
            .get(&format!("/api/channels/{}/messages/sync?after={cursor}", self.general))
            .await
            .json();
        page["changes"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|change| change["messageId"] == message["id"])
            .unwrap_or_else(|| panic!("no change of the poll: {page}"))["message"]["poll"]
            .clone()
    }

    /// The poll of a message in general, as `who` reads the channel.
    async fn read(&self, who: &Person, message: &Value) -> Value {
        let page: Value = who.client.get(&format!("/api/channels/{}/messages", self.general)).await.json();
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == message["id"])
            .unwrap()["poll"]
            .clone()
    }
}

fn votes(poll: &Value) -> Vec<i64> {
    poll["answers"].as_array().unwrap().iter().map(|answer| answer["votes"].as_i64().unwrap()).collect()
}

#[tokio::test]
async fn a_poll_is_refused_past_each_limit_and_taken_at_them() {
    let w = world().await;
    let answers = |count: usize, length: usize| -> Value {
        (0..count).map(|n| json!({ "text": format!("{n}{}", "a".repeat(length - 1)) })).collect()
    };
    let refused = [
        (json!({ "question": "q".repeat(301), "answers": answers(2, 5) }), "poll.question"),
        (json!({ "question": "  ", "answers": answers(2, 5) }), "poll.question"),
        (json!({ "question": "Which?", "answers": answers(1, 5) }), "poll.answers"),
        (json!({ "question": "Which?", "answers": answers(11, 5) }), "poll.answers"),
        (json!({ "question": "Which?", "answers": [{ "text": "fine" }, { "text": "b".repeat(56) }] }), "poll.answers[1]"),
        (json!({ "question": "Which?", "answers": [{ "text": "fine" }, { "text": " " }] }), "poll.answers[1]"),
        (json!({ "question": "Which?", "answers": [{ "text": "fine", "emoji": "🎲 🎲" }, { "text": "ok" }] }), "poll.answers[0]"),
        (json!({ "question": "Which?", "answers": [{ "text": "fine", "emoji": "x".repeat(65) }, { "text": "ok" }] }), "poll.answers[0]"),
        (json!({ "question": "Which?", "answers": answers(2, 5), "duration": "2d" }), "poll.duration"),
    ];
    for (poll, field) in refused {
        let response = w.post(&w.owner, &w.general, json!({ "poll": poll })).await;
        assert_eq!(response.status, 400, "{field}: {}", response.text());
        assert!(response.json::<Value>()["errors"][field].is_array(), "{field}: {}", response.text());
    }
    let upload = Id::random();
    w.server
        .database()
        .execute(
            "INSERT INTO \"ImageAttachments\" (\"Id\", \"UploadedById\", \"ObjectKey\", \"FileName\", \"ContentType\", \"SizeBytes\", \"IsEncrypted\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, 'a.pdf', 'application/pdf', 1, 0, ?4)",
            params![upload, Id::parse(&w.owner.id).unwrap(), format!("test/{upload}"), Timestamp::now()],
        )
        .unwrap();
    let with_file = w.post(&w.owner, &w.general, json!({ "poll": three(false), "attachmentIds": [upload] })).await;
    assert_eq!(with_file.status, 400);
    assert!(with_file.json::<Value>()["errors"]["attachmentIds"].is_array());
    assert_eq!(w.count("SELECT COUNT(*) FROM \"Polls\""), 0, "nothing refused was stored");

    // At every limit, it is taken.
    let emoji = "🎲".repeat(32);
    let mut longest: Vec<Value> = (0..10).map(|n| json!({ "text": format!("{n}{}", "a".repeat(54)) })).collect();
    longest[0]["emoji"] = json!(emoji);
    let made = w
        .poll(&w.owner, json!({ "question": "q".repeat(300), "answers": longest, "duration": "1w" }))
        .await;
    let poll = &made["poll"];
    assert_eq!(poll["answers"].as_array().unwrap().len(), 10);
    assert_eq!(poll["answers"][0]["id"], 1);
    assert_eq!(poll["answers"][0]["emoji"], json!(emoji));
    let created = Timestamp::parse(made["createdAt"].as_str().unwrap()).unwrap();
    let ends = Timestamp::parse(poll["endsAt"].as_str().unwrap()).unwrap();
    assert_eq!(created.until(ends), Duration::from_secs(7 * 24 * 3600));
    assert_eq!(made["content"], "");
}

#[tokio::test]
async fn votes_count_once_per_person_change_until_the_end_and_only_the_voter_hears_their_choice() {
    let w = world().await;
    let single = w.poll(&w.owner, three(false)).await;
    let multiple = w.poll(&w.owner, three(true)).await;
    assert_eq!(single["poll"]["mine"], json!([]));
    let announced = w.events("MessageCreated").into_iter().find(|event| event["payload"]["id"] == single["id"]).unwrap();
    assert_eq!(announced["payload"]["poll"]["mine"], Value::Null, "an event for everyone says nobody's choice");
    let messages = w.count("SELECT COUNT(*) FROM \"Messages\"");
    let created = w.events("MessageCreated").len();
    let cursor = w.sync_cursor(&w.maya).await;

    let poll = w.voted(&w.maya, &single, json!([2])).await;
    assert_eq!((votes(&poll), poll["voters"].as_i64(), poll["mine"].clone()), (vec![0, 1, 0], Some(1), json!([2])));
    let two = w.vote(&w.maya, &single, json!([1, 2])).await;
    assert_eq!(two.status, 400, "a single-answer poll takes one");
    assert!(two.json::<Value>()["errors"]["answers"].is_array());
    assert_eq!(w.vote(&w.maya, &single, json!([4])).await.status, 400, "no such answer");
    let poll = w.voted(&w.maya, &single, json!([3])).await;
    assert_eq!((votes(&poll), poll["mine"].clone()), (vec![0, 0, 1], json!([3])), "a changed vote replaces the old one");
    let poll = w.voted(&w.joe, &single, json!([3])).await;
    assert_eq!((votes(&poll), poll["voters"].as_i64()), (vec![0, 0, 2], Some(2)));
    let poll = w.voted(&w.joe, &single, json!([])).await;
    assert_eq!((votes(&poll), poll["voters"].as_i64(), poll["mine"].clone()), (vec![0, 0, 1], Some(1), json!([])));

    let poll = w.voted(&w.maya, &multiple, json!([1, 3, 3])).await;
    assert_eq!((votes(&poll), poll["voters"].as_i64(), poll["mine"].clone()), (vec![1, 0, 1], Some(1), json!([1, 3])));
    let poll = w.voted(&w.joe, &multiple, json!([1])).await;
    assert_eq!((votes(&poll), poll["voters"].as_i64()), (vec![2, 0, 1], Some(2)), "a person counts once");
    let updates = w.events("PollUpdated").len();
    w.voted(&w.joe, &multiple, json!([1])).await;
    assert_eq!(w.events("PollUpdated").len(), updates, "the same vote again changes nothing");

    // Each reads their own choice, and the counts are the same for all.
    for (person, mine_single, mine_multiple) in [(&w.maya, json!([3]), json!([1, 3])), (&w.joe, json!([]), json!([1])), (&w.owner, json!([]), json!([]))] {
        let read = w.read(person, &single).await;
        assert_eq!((votes(&read), read["mine"].clone()), (vec![0, 0, 1], mine_single), "{}", person.username);
        let read = w.read(person, &multiple).await;
        assert_eq!((votes(&read), read["mine"].clone()), (vec![2, 0, 1], mine_multiple), "{}", person.username);
    }

    // Everyone who sees the channel hears the counts and nobody's choice;
    // the choice goes to the voter's own connections alone.
    let updates = w.events("PollUpdated");
    assert_eq!(updates.len(), 6);
    let mut last = 0;
    for update in &updates {
        assert_eq!(update["audience"], json!({ "server": w.instance }));
        let mut keys: Vec<&str> = update["payload"].as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["answers", "channelId", "endedAt", "endedBy", "messageId", "sequence", "voters"]);
        let sequence = update["payload"]["sequence"].as_i64().unwrap();
        assert!(sequence > last, "the updates are in order");
        last = sequence;
    }
    let choices = w.events("PollVoteChanged");
    assert_eq!(choices.len(), 6);
    for choice in &choices {
        let voter = if choice["audience"] == json!({ "users": [w.maya.id] }) { &w.maya } else { &w.joe };
        assert_eq!(choice["audience"], json!({ "users": [voter.id] }));
    }
    let maya_last = choices.iter().rev().find(|choice| choice["audience"] == json!({ "users": [w.maya.id] })).unwrap();
    assert_eq!(maya_last["payload"]["mine"], json!([1, 3]));

    // A client that missed the events catches up through the journal, with
    // its own choice.
    let synced = w.synced(&w.maya, &cursor, &multiple).await;
    assert_eq!((votes(&synced), synced["mine"].clone()), (vec![2, 0, 1], json!([1, 3])));

    // The poll changed in place: no message was posted for any of it.
    assert_eq!(w.count("SELECT COUNT(*) FROM \"Messages\""), messages);
    assert_eq!(w.events("MessageCreated").len(), created);

    // A poll with no text of its own is quoted by its question.
    let reply = w.post(&w.joe, &w.general, json!({ "content": "Sunday, surely", "replyToMessageId": single["id"] })).await;
    assert_eq!(reply.status, 201, "{}", reply.text());
    assert_eq!(reply.json::<Value>()["replyTo"]["preview"], "When do we play?");
    let threads: Value = w.joe.client.get(&format!("/api/channels/{}/threads", w.general)).await.json();
    let root = threads.as_array().unwrap().iter().find(|thread| thread["root"]["id"] == single["id"]).unwrap();
    assert_eq!(root["root"]["preview"], "When do we play?");
}

#[tokio::test]
async fn only_the_author_or_whoever_manages_messages_ends_a_poll_and_no_vote_comes_after_the_end() {
    let w = world().await;
    let poll = w.poll(&w.maya, three(false)).await;
    w.voted(&w.joe, &poll, json!([1])).await;

    assert_eq!(w.end(&w.joe, &poll).await.status, 403, "neither its author nor a moderator");
    let ended = w.end(&w.owner, &poll).await;
    assert_eq!(ended.status, 200, "{}", ended.text());
    let ended: Value = ended.json();
    assert_eq!(ended["endedBy"], w.owner.id.as_str());
    assert!(ended["endedAt"].is_string());
    for refused in [w.vote(&w.joe, &poll, json!([2])).await, w.end(&w.maya, &poll).await] {
        assert_eq!(refused.status, 409);
        assert_eq!(refused.json::<Value>()["code"], "poll-ended");
    }
    assert_eq!(votes(&w.read(&w.joe, &poll).await), vec![1, 0, 0], "the vote after the end did not count");

    let own = w.poll(&w.maya, three(false)).await;
    let ended = w.end(&w.maya, &own).await;
    assert_eq!(ended.status, 200, "its author ends it");
    assert_eq!(ended.json::<Value>()["endedBy"], w.maya.id.as_str());

    // Joe may manage messages in general, and nothing else.
    let theirs = w.poll(&w.maya, three(false)).await;
    assert_eq!(w.end(&w.joe, &theirs).await.status, 403);
    w.server
        .database()
        .execute(
            "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 16, 0)",
            params![Id::random(), w.instance.to_uppercase(), w.general.to_uppercase(), w.joe.id.to_uppercase()],
        )
        .unwrap();
    let ended = w.end(&w.joe, &theirs).await;
    assert_eq!(ended.status, 200, "{}", ended.text());
    assert_eq!(ended.json::<Value>()["endedBy"], w.joe.id.as_str());

    // A poll whose time is up has ended, before anything marks it so.
    let late = w.poll(&w.maya, three(false)).await;
    let ends_at = Timestamp::now().minus(Duration::from_secs(5));
    w.server
        .database()
        .execute(
            "UPDATE \"Polls\" SET \"EndsAt\" = ?2 WHERE \"MessageId\" = ?1",
            params![Id::parse(late["id"].as_str().unwrap()).unwrap(), ends_at],
        )
        .unwrap();
    let refused = w.vote(&w.joe, &late, json!([1])).await;
    assert_eq!(refused.status, 409);
    assert_eq!(refused.json::<Value>()["code"], "poll-ended");
    let read = w.read(&w.joe, &late).await;
    assert_eq!(read["endedAt"], json!(ends_at));
    assert_eq!(read["endedBy"], Value::Null);

    // A poll is not edited after it is sent.
    let edited = w
        .maya
        .client
        .send(Request::new(Method::PATCH, &format!("/api/messages/{}", own["id"].as_str().unwrap())).json(&json!({ "content": "something else" })))
        .await;
    assert_eq!(edited.status, 409);
    assert_eq!(edited.json::<Value>()["code"], "poll-not-editable");

    let ends: Vec<Value> = w.events("PollUpdated").into_iter().filter(|update| update["payload"]["endedBy"].is_string()).collect();
    assert_eq!(ends.len(), 3, "each end is told once, in place");
}

#[tokio::test]
async fn who_chose_an_answer_is_listed_to_those_who_see_the_channel() {
    let w = world().await;
    let poll = w.poll(&w.owner, three(true)).await;
    w.voted(&w.maya, &poll, json!([1])).await;
    w.voted(&w.joe, &poll, json!([1, 2])).await;
    let id = poll["id"].as_str().unwrap();
    let names = |page: &Value| -> Vec<String> {
        page["voters"].as_array().unwrap().iter().map(|voter| voter["username"].as_str().unwrap().to_string()).collect()
    };

    let page: Value = w.owner.client.get(&format!("/api/messages/{id}/poll/answers/1/voters")).await.json();
    assert_eq!(names(&page), ["maya", "joe"], "the earliest vote first");
    assert_eq!(page["nextCursor"], Value::Null);
    let first: Value = w.owner.client.get(&format!("/api/messages/{id}/poll/answers/1/voters?limit=1")).await.json();
    assert_eq!(names(&first), ["maya"]);
    let cursor = first["nextCursor"].as_str().unwrap();
    let second: Value = w.owner.client.get(&format!("/api/messages/{id}/poll/answers/1/voters?limit=1&cursor={cursor}")).await.json();
    assert_eq!((names(&second), second["nextCursor"].clone()), (vec!["joe".to_string()], Value::Null));
    let page: Value = w.maya.client.get(&format!("/api/messages/{id}/poll/answers/2/voters")).await.json();
    assert_eq!(names(&page), ["joe"]);
    assert_eq!(w.owner.client.get(&format!("/api/messages/{id}/poll/answers/4/voters")).await.status, 404);

    // In a channel Joe does not see, the poll is not there for him at all.
    let hidden = Id::random();
    let db = w.server.database();
    db.execute(
        "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\")
         VALUES (?1, ?2, 'staff', 0, 30, '', 0, 0, 0, NULL)",
        params![hidden, w.instance.to_uppercase()],
    )
    .unwrap();
    let base: String = db
        .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
        .unwrap();
    db.execute(
        "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
        params![Id::random(), w.instance.to_uppercase(), hidden, base],
    )
    .unwrap();
    let staff = w.post(&w.owner, &hidden.to_string(), json!({ "poll": three(false) })).await;
    assert_eq!(staff.status, 201, "{}", staff.text());
    let staff: Value = staff.json();
    w.voted(&w.owner, &staff, json!([1])).await;
    let staff_id = staff["id"].as_str().unwrap();
    assert_eq!(w.joe.client.get(&format!("/api/messages/{staff_id}/poll/answers/1/voters")).await.status, 404);
    assert_eq!(w.vote(&w.joe, &staff, json!([2])).await.status, 404);
    assert_eq!(w.end(&w.joe, &staff).await.status, 404);
    let page: Value = w.owner.client.get(&format!("/api/messages/{staff_id}/poll/answers/1/voters")).await.json();
    assert_eq!(names(&page), ["owner"]);
    let told = w.events("PollUpdated");
    let update = told.iter().find(|update| update["payload"]["messageId"] == staff["id"]).unwrap();
    assert_eq!(
        update["audience"],
        json!({ "followers": { "server": w.instance, "users": [w.owner.id] } }),
        "only those who see the channel hear of its polls"
    );
}

#[tokio::test]
async fn a_poll_is_refused_in_a_conversation_and_in_a_scheduled_message() {
    let w = world().await;
    let opened = w.maya.client.post_json("/api/social/conversations", &json!({ "userId": w.owner.id })).await;
    assert_eq!(opened.status, 200, "{}", opened.text());
    let conversation = opened.json::<Value>()["id"].as_str().unwrap().to_string();

    let direct = w
        .maya
        .client
        .post_json(&format!("/api/social/conversations/{conversation}/messages"), &json!({ "content": "vote!", "poll": three(false) }))
        .await;
    assert_eq!(direct.status, 400);
    assert!(direct.json::<Value>()["errors"]["poll"].is_array());
    assert_eq!(w.count("SELECT COUNT(*) FROM \"DirectMessages\""), 0, "nothing went out as plain text");
    let plain = w
        .maya
        .client
        .post_json(&format!("/api/social/conversations/{conversation}/messages"), &json!({ "content": "hello" }))
        .await;
    assert_eq!(plain.status, 201);
    assert_eq!(plain.json::<Value>()["poll"], Value::Null);

    let send_at = Timestamp::now().plus(Duration::from_secs(600)).to_wire_text();
    for path in [format!("/api/channels/{}/scheduled", w.general), format!("/api/social/conversations/{conversation}/scheduled")] {
        let scheduled = w.maya.client.post_json(&path, &json!({ "content": "later", "sendAt": send_at, "poll": three(false) })).await;
        assert_eq!(scheduled.status, 400, "{path}");
        assert!(scheduled.json::<Value>()["errors"]["poll"].is_array(), "{path}");
    }
    assert_eq!(w.count("SELECT COUNT(*) FROM \"ScheduledMessages\""), 0);
}

#[tokio::test]
async fn the_worker_ends_a_poll_on_time_and_one_that_fell_due_while_the_server_was_down() {
    let folder = TempDir::new();
    std::fs::create_dir_all(folder.path().join("db")).unwrap();
    let workers = [("Testing__Workers", "poll-ends")];
    let w = arrange(Server::start_in(folder.path(), &workers).await).await;
    let due = w.poll(&w.owner, three(false)).await;
    w.voted(&w.maya, &due, json!([2])).await;
    let at = |message: &Value| Id::parse(message["id"].as_str().unwrap()).unwrap();
    let ends = |server: &Server, message: &Value| -> (Timestamp, Option<Timestamp>) {
        server
            .database()
            .query_row("SELECT \"EndsAt\", \"EndedAt\" FROM \"Polls\" WHERE \"MessageId\" = ?1", [at(message)], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
    };
    async fn until_ended(server: &Server, message: Id) {
        for _ in 0..200 {
            let ended: Option<Timestamp> = server
                .database()
                .query_row("SELECT \"EndedAt\" FROM \"Polls\" WHERE \"MessageId\" = ?1", [message], |row| row.get(0))
                .unwrap();
            if ended.is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the poll was not ended");
    }

    // Its time comes; a new poll makes the worker look again.
    let was_due = Timestamp::now().minus(Duration::from_secs(1));
    w.server
        .database()
        .execute("UPDATE \"Polls\" SET \"EndsAt\" = ?2 WHERE \"MessageId\" = ?1", params![at(&due), was_due])
        .unwrap();
    let later = w.poll(&w.owner, three(false)).await;
    until_ended(&w.server, at(&due)).await;
    assert_eq!(ends(&w.server, &due), (was_due, Some(was_due)));
    let told = w.events("PollUpdated");
    let end = told.iter().rev().find(|update| update["payload"]["messageId"] == due["id"]).unwrap();
    assert_eq!(end["payload"]["endedAt"], json!(was_due));
    assert_eq!(end["payload"]["endedBy"], Value::Null);
    assert_eq!(ends(&w.server, &later).1, None, "the other one still runs");

    // The server is down when the other one falls due, and ends it once it is
    // back, at the time it was due.
    let messages = w.count("SELECT COUNT(*) FROM \"Messages\"");
    let cursor = w.sync_cursor(&w.owner).await;
    let general = w.general.clone();
    let World { server, owner, .. } = w;
    assert!(server.stop().await);
    let overdue = Timestamp::now().minus(Duration::from_secs(300));
    rusqlite::Connection::open(folder.path().join("db/stellarr.db"))
        .unwrap()
        .execute("UPDATE \"Polls\" SET \"EndsAt\" = ?2 WHERE \"MessageId\" = ?1", params![at(&later), overdue])
        .unwrap();
    let again = Server::start_in(folder.path(), &workers).await;
    until_ended(&again, at(&later)).await;
    assert_eq!(ends(&again, &later), (overdue, Some(overdue)));
    let count: i64 = again.database().query_row("SELECT COUNT(*) FROM \"Messages\"", [], |row| row.get(0)).unwrap();
    assert_eq!(count, messages, "the ends posted nothing");

    // The end is in the journal, for a client that was not connected.
    let client = again.sign_in(&owner).await;
    let page: Value = client.get(&format!("/api/channels/{general}/messages/sync?after={cursor}")).await.json();
    let change = page["changes"].as_array().unwrap().iter().rev().find(|change| change["messageId"] == later["id"]).unwrap();
    assert_eq!(change["message"]["poll"]["endedAt"], json!(overdue));
}
