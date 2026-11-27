//! Server events below the HTTP layer: the routes called as handlers, and the
//! worker's pass with the clock and the voice channels the test chooses.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State as Extract;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::params;
use serde_json::{Value, json};
use stellarr_timezone::TimeZone;

use super::model::Repeat;
use super::*;
use crate::auth::Caller;
use crate::delivery::outbox::Kind;
use crate::database::Database;
use crate::events::{Event, EventPublisher, Unconnected};
use crate::http::{ApiResult, Json, Path};
use crate::permissions::fixture::World;
use crate::permissions::membership::ActiveServer;
use crate::seal::Sealer;
use crate::settings::{DatabaseSettings, Settings};
use crate::state::AppState;
use crate::testing::TempDir;

const MINUTE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Recorder(Mutex<Vec<Event>>);

impl EventPublisher for Recorder {
    fn publish(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

struct Place {
    world: World,
    data: TempDir,
    state: AppState,
    events: Arc<Recorder>,
}

fn state_on(database: &Database, data: &TempDir) -> (AppState, Arc<Recorder>) {
    let settings = Settings::load(&[
        ("STELLARR_ENVIRONMENT".to_string(), "Testing".to_string()),
        ("STELLARR_DATA_ROOT".to_string(), data.path().display().to_string()),
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

async fn answer(result: ApiResult<Response>) -> (StatusCode, Value) {
    let response = result.unwrap_or_else(IntoResponse::into_response);
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

fn caller(user: Id) -> Caller {
    Caller {
        user,
        session: Id::random(),
    }
}

fn ahead(by: Duration) -> String {
    Timestamp::now().plus(by).to_wire_text()
}

impl Place {
    fn new() -> Place {
        let world = World::new();
        let data = TempDir::new();
        let (state, events) = state_on(&world.database, &data);
        Place { world, data, state, events }
    }

    fn restart(&mut self) {
        let database = Database::open(&DatabaseSettings {
            path: self.world.database.path().to_path_buf(),
            busy_timeout: Duration::from_secs(30),
        })
        .unwrap();
        (self.state, self.events) = state_on(&database, &self.data);
    }

    /// A member of the base role.
    fn member(&self, name: &str) -> Id {
        let id = self.world.user(name);
        self.world.admit(id, self.world.base_role);
        id
    }

    async fn create(&self, user: Id, body: Value) -> (StatusCode, Value) {
        answer(
            routes::create(
                Extract(self.state.clone()),
                caller(user),
                ActiveServer(self.world.server),
                Json(serde_json::from_value(body).unwrap()),
            )
            .await,
        )
        .await
    }

    async fn made(&self, user: Id, body: Value) -> Value {
        let (status, event) = self.create(user, body).await;
        assert_eq!(status, StatusCode::CREATED, "{event}");
        event
    }

    async fn sign_up(&self, user: Id, event: &Value) -> (StatusCode, Value) {
        answer(routes::sign_up(Extract(self.state.clone()), caller(user), Path(ids(event))).await).await
    }

    async fn sign_out(&self, user: Id, event: &Value) -> (StatusCode, Value) {
        answer(routes::sign_out(Extract(self.state.clone()), caller(user), Path(ids(event))).await).await
    }

    async fn edit(&self, user: Id, event: &Value, body: Value) -> (StatusCode, Value) {
        answer(
            routes::edit(
                Extract(self.state.clone()),
                caller(user),
                Path(ids(event).0),
                Json(serde_json::from_value(body).unwrap()),
            )
            .await,
        )
        .await
    }

    async fn cancel(&self, user: Id, event: &Value) -> StatusCode {
        match routes::cancel(Extract(self.state.clone()), caller(user), Path(ids(event).0)).await {
            Ok(status) => status,
            Err(error) => error.into_response().status(),
        }
    }

    async fn read(&self, user: Id, event: &Value) -> (StatusCode, Value) {
        answer(
            routes::one(Extract(self.state.clone()), caller(user), Path(ids(event).0))
                .await
                .map(IntoResponse::into_response),
        )
        .await
    }

    async fn list(&self, user: Id) -> Value {
        answer(
            routes::list(Extract(self.state.clone()), caller(user), ActiveServer(self.world.server))
                .await
                .map(IntoResponse::into_response),
        )
        .await
        .1
    }

    async fn run(&self, now: Timestamp, sitting: usize) {
        let occupancy = move |_: Id| sitting;
        pass(&self.state, now, &occupancy).await.unwrap();
    }

    fn sql(&self, sql: &str, values: &[&dyn rusqlite::ToSql]) {
        self.world.db.execute(sql, values).unwrap();
    }

    fn count(&self, sql: &str, values: &[&dyn rusqlite::ToSql]) -> i64 {
        self.world.db.query_row(sql, values, |row| row.get(0)).unwrap()
    }

    /// The inbox entries of a kind a person has about events.
    fn notices(&self, user: Id, module: &str) -> i64 {
        self.count(
            "SELECT COUNT(*) FROM \"UserNotifications\" n JOIN \"ServerEventNotices\" e ON e.\"NotificationId\" = n.\"Id\"
             WHERE n.\"UserId\" = ?1 AND n.\"SourceModuleId\" = ?2",
            &[&user, &module],
        )
    }

    fn published(&self, name: &str) -> Vec<(Value, Value)> {
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
}

fn ids(event: &Value) -> (Id, Id) {
    (
        Id::parse(event["id"].as_str().unwrap()).unwrap(),
        Id::parse(event["occurrenceId"].as_str().unwrap()).unwrap(),
    )
}

fn elsewhere(name: &str, starts: &str, seats: Option<i64>) -> Value {
    let start = Timestamp::parse(starts).unwrap();
    json!({
        "name": name,
        "place": { "kind": "elsewhere", "text": "The usual table" },
        "startsAt": starts,
        "endsAt": start.plus(120 * MINUTE).to_wire_text(),
        "timeZone": "Europe/Warsaw",
        "seats": seats,
    })
}

#[test]
fn repeats_keep_the_clock_time_across_daylight_saving_and_the_end_of_a_month() {
    let warsaw = TimeZone::find("Europe/Warsaw").unwrap();
    let first = Timestamp::parse("2026-10-24 18:00:00").unwrap(); // Saturday 20:00 in summer time
    let weekly: Vec<String> = (0..3)
        .map(|n| Repeat::Weekly.start_of(&warsaw, first, n).to_db_text())
        .collect();
    assert_eq!(weekly, ["2026-10-24 18:00:00", "2026-10-31 19:00:00", "2026-11-07 19:00:00"]);
    assert_eq!(Repeat::Biweekly.start_of(&warsaw, first, 1).to_db_text(), "2026-11-07 19:00:00");
    let last = Timestamp::parse("2027-01-31 19:00:00").unwrap(); // the 31st at 20:00 in winter
    let monthly: Vec<String> = (0..4)
        .map(|n| Repeat::Monthly.start_of(&warsaw, last, n).to_db_text())
        .collect();
    assert_eq!(
        monthly,
        ["2027-01-31 19:00:00", "2027-02-28 19:00:00", "2027-03-31 18:00:00", "2027-04-30 18:00:00"],
        "the last day where a month is shorter, the 31st again where it is not"
    );
    // 02:30 a week before the clocks go forward is 03:30 on that night.
    let early = Timestamp::parse("2027-03-21 01:30:00").unwrap();
    assert_eq!(Repeat::Weekly.start_of(&warsaw, early, 1).to_db_text(), "2027-03-28 01:30:00");
}

#[tokio::test]
async fn seats_are_never_given_twice_and_the_waitlist_moves_up_in_order() {
    let place = Place::new();
    let w = &place.world;
    let event = place.made(w.maja, elsewhere("Board games", &ahead(60 * MINUTE), Some(2))).await;
    let people: Vec<Id> = (0..6).map(|n| place.member(&format!("player{n}"))).collect();

    // Six at once for two seats.
    let signing = people.iter().map(|person| {
        let state = place.state.clone();
        let path = ids(&event);
        let person = *person;
        tokio::spawn(async move { answer(routes::sign_up(Extract(state), caller(person), Path(path)).await).await })
    });
    for joined in futures_all(signing).await {
        assert_eq!(joined.0, StatusCode::OK, "{}", joined.1);
    }
    let order: Vec<(Id, bool)> = w
        .db
        .prepare("SELECT \"UserId\", \"Seated\" FROM \"ServerEventSignups\" ORDER BY \"Number\"")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        order.iter().map(|(_, seated)| *seated).collect::<Vec<_>>(),
        [true, true, false, false, false, false],
        "the first two to sign up have the seats"
    );
    let (_, read) = place.read(order[4].0, &event).await;
    assert_eq!((read["mine"].clone(), read["myPosition"].clone()), (json!("waiting"), json!(3)));
    assert_eq!((read["signedUp"].clone(), read["waiting"].clone()), (json!(2), json!(4)));

    // A seat comes free: the first waiting gets it, with a notice, and
    // everyone behind moves up.
    let (status, _) = place.sign_out(order[0].0, &event).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(place.notices(order[2].0, "seat"), 1);
    let (_, read) = place.read(order[2].0, &event).await;
    assert_eq!(read["mine"], "signed");
    let moved: Vec<(Value, Value)> = place.published(routes::SIGNUP_CHANGED);
    for (person, position) in [(order[3].0, 1), (order[4].0, 2), (order[5].0, 3)] {
        let told = moved.iter().rev().find(|(audience, _)| audience == &json!({ "users": [person] })).unwrap();
        assert_eq!((told.1["mine"].clone(), told.1["myPosition"].clone()), (json!("waiting"), json!(position)));
    }

    // A lower limit takes nobody's seat; a higher one fills from the front.
    let (status, edited) = place.edit(w.maja, &event, json!({ "seats": 1 })).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!((edited["signedUp"].clone(), edited["waiting"].clone()), (json!(2), json!(3)));
    let (_, edited) = place.edit(w.maja, &event, json!({ "seats": 4 })).await;
    assert_eq!((edited["signedUp"].clone(), edited["waiting"].clone()), (json!(4), json!(1)));
    assert_eq!(place.notices(order[3].0, "seat"), 1);
    assert_eq!(place.notices(order[4].0, "seat"), 1);
    assert_eq!(place.notices(order[5].0, "seat"), 0);
    let (_, read) = place.read(order[5].0, &event).await;
    assert_eq!(read["myPosition"], 1);

    // Everybody hears the counts; nobody but the person hears where they stand.
    for (audience, shared) in place.published(routes::CHANGED) {
        assert_eq!(audience, json!({ "server": w.server }));
        assert_eq!(shared["mine"], Value::Null);
    }
}

async fn futures_all<T>(tasks: impl Iterator<Item = tokio::task::JoinHandle<T>>) -> Vec<T> {
    let mut done = Vec::new();
    for task in tasks {
        done.push(task.await.unwrap());
    }
    done
}

#[tokio::test]
async fn a_reminder_goes_once_to_those_with_a_seat_even_across_a_restart() {
    let mut place = Place::new();
    let (maja, ilya, nils, jo) = (place.world.maja, place.world.ilya, place.world.nils, place.world.jo);
    let standup = place.made(maja, elsewhere("Standup", &ahead(60 * MINUTE), Some(1))).await;
    place.sign_up(ilya, &standup).await;
    place.sign_up(nils, &standup).await;
    let starts = Timestamp::parse(standup["startsAt"].as_str().unwrap()).unwrap();

    place.run(starts.minus(20 * MINUTE), 0).await;
    assert_eq!(place.notices(ilya, "reminder"), 0, "not yet");
    let due = starts.minus(10 * MINUTE);
    place.run(due, 0).await;
    assert_eq!(place.notices(ilya, "reminder"), 1);
    assert_eq!(place.notices(nils, "reminder"), 0, "the waitlist is not reminded");
    place.run(due.plus(MINUTE), 0).await;
    place.restart();
    place.run(due.plus(2 * MINUTE), 0).await;
    assert_eq!(place.notices(ilya, "reminder"), 1, "once, whatever restarts");

    // Somebody who signs up within the last quarter of an hour has just seen
    // the event, and is not told it starts soon.
    let soon = place.made(maja, elsewhere("Soon", &ahead(10 * MINUTE), None)).await;
    place.sign_up(jo, &soon).await;
    place.run(Timestamp::now(), 0).await;
    assert_eq!(place.notices(jo, "reminder"), 0);

    // The server was down when the reminder was due and the start has passed:
    // a reminder then is no use.
    let late = place.made(maja, elsewhere("Late", &ahead(30 * MINUTE), None)).await;
    place.sign_up(ilya, &late).await;
    let started = Timestamp::now().minus(MINUTE);
    place.sql(
        "UPDATE \"ServerEventOccurrences\" SET \"StartsAt\" = ?2, \"EndsAt\" = ?3 WHERE \"Id\" = ?1",
        &[&ids(&late).1, &started, &started.plus(60 * MINUTE)],
    );
    place.run(Timestamp::now(), 0).await;
    assert_eq!(place.notices(ilya, "reminder"), 1);
}

#[tokio::test]
async fn a_voice_event_without_an_end_ends_by_who_sits_in_its_channel() {
    let place = Place::new();
    let (maja, voice) = (place.world.maja, place.world.voice);
    let start = Timestamp::now().minus(MINUTE);
    let open = |name: &str| {
        json!({
            "name": name,
            "place": { "kind": "voice", "channelId": voice },
            "startsAt": start.to_wire_text(),
            "endsAt": null,
            "timeZone": "Europe/Warsaw",
        })
    };
    let state_of = |event: &Value| -> (String, Option<Timestamp>, Option<Timestamp>) {
        place
            .world
            .db
            .query_row(
                "SELECT \"Phase\", \"LiveSince\", \"EndedAt\" FROM \"ServerEventOccurrences\" WHERE \"Id\" = ?1",
                [ids(event).1],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    };

    let game = place.made(maja, open("Game night")).await;
    place.run(start.plus(MINUTE), 0).await;
    assert_eq!(state_of(&game), ("live".to_string(), None, None));
    let came = start.plus(5 * MINUTE);
    place.run(came, 3).await;
    assert_eq!(state_of(&game).1, Some(came), "live since somebody came");
    let left = start.plus(30 * MINUTE);
    place.run(left, 0).await;
    place.run(left.plus(MINUTE), 0).await;
    assert_eq!(state_of(&game).0, "live", "a minute empty is a moment, as after a restart");
    place.run(left.plus(2 * MINUTE), 0).await;
    assert_eq!(state_of(&game), ("ended".to_string(), Some(came), Some(left)), "ended when it emptied");

    let nobody = place.made(maja, open("Nobody came")).await;
    place.run(start.plus(59 * MINUTE), 0).await;
    assert_eq!(state_of(&nobody).0, "live");
    place.run(start.plus(61 * MINUTE), 0).await;
    assert_eq!(state_of(&nobody), ("ended".to_string(), None, Some(start.plus(NOBODY_GRACE))));

    let endless = place.made(maja, open("Endless")).await;
    place.run(start.plus(2 * MINUTE), 4).await;
    place.run(start.plus(LONGEST_OPEN).plus(MINUTE), 4).await;
    assert_eq!(state_of(&endless).2, Some(start.plus(LONGEST_OPEN)), "twelve hours at most");

    let (_, card) = place.read(maja, &game).await;
    assert_eq!((card["state"].clone(), card["liveSince"].clone()), (json!("ended"), json!(came)));
    assert!(place.list(maja).await.as_array().unwrap().is_empty(), "ended events leave the list");
}

#[tokio::test]
async fn a_repeating_event_comes_back_at_its_next_time_with_nobody_signed_up() {
    let place = Place::new();
    let (maja, ilya) = (place.world.maja, place.world.ilya);
    let mut body = elsewhere("Sprint review", &ahead(60 * MINUTE), None);
    body["repeat"] = json!("weekly");
    let event = place.made(maja, body).await;
    place.sign_up(ilya, &event).await;
    // It ran a week ago, less an hour, and has ended.
    let first = Timestamp::from_unix_seconds(Timestamp::now().minus(7 * 24 * 60 * MINUTE).plus(60 * MINUTE).unix_seconds()).unwrap();
    place.sql(
        "UPDATE \"ServerEvents\" SET \"FirstStartsAt\" = ?2 WHERE \"Id\" = ?1",
        &[&ids(&event).0, &first],
    );
    place.sql(
        "UPDATE \"ServerEventOccurrences\" SET \"StartsAt\" = ?2, \"EndsAt\" = ?3 WHERE \"Id\" = ?1",
        &[&ids(&event).1, &first, &first.plus(120 * MINUTE)],
    );
    place.run(Timestamp::now(), 0).await;
    let (_, next) = place.read(maja, &event).await;
    let starts = Repeat::Weekly.start_of(&TimeZone::find("Europe/Warsaw").unwrap(), first, 1);
    assert_eq!(next["state"], "upcoming");
    assert_eq!(next["startsAt"], json!(starts), "a week on, at the same time on the clock");
    assert_eq!(next["endsAt"], json!(starts.plus(120 * MINUTE)));
    assert_ne!(next["occurrenceId"], event["occurrenceId"]);
    assert_eq!(next["signedUp"], 0, "sign-ups belong to the occurrence that ended");
    let told = place.published(routes::CHANGED);
    assert_eq!(told.last().unwrap().1["occurrenceId"], next["occurrenceId"]);
    let (status, refused) = place.sign_up(ilya, &event).await;
    assert_eq!((status, refused["code"].clone()), (StatusCode::CONFLICT, json!("event-occurrence-gone")));
}

#[tokio::test]
async fn a_voice_event_is_seen_only_by_those_who_see_its_channel() {
    let place = Place::new();
    let w = &place.world;
    let hidden = w.channel(None);
    w.db.execute("UPDATE \"Channels\" SET \"Kind\" = 1 WHERE \"Id\" = ?1", [hidden]).unwrap();
    w.deny_view_to_base(hidden, false);
    let in_hidden = json!({
        "name": "Staff call",
        "place": { "kind": "voice", "channelId": hidden },
        "startsAt": ahead(60 * MINUTE),
        "endsAt": null,
        "timeZone": "UTC",
        "announceChannelId": w.general,
    });
    let (status, refused) = place.create(w.maja, in_hidden.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "an open channel would show the card to all");
    assert!(refused["errors"]["announceChannelId"].is_array());

    let mut quiet = in_hidden;
    quiet["announceChannelId"] = Value::Null;
    let event = place.made(w.maja, quiet).await;
    assert_eq!(place.list(w.ilya).await, json!([]));
    assert_eq!(place.read(w.ilya, &event).await.0, StatusCode::NOT_FOUND);
    assert_eq!(place.sign_up(w.ilya, &event).await.0, StatusCode::NOT_FOUND);
    assert_eq!(place.list(w.maja).await.as_array().unwrap().len(), 1);
    let (audience, _) = place.published(routes::CHANGED).pop().unwrap();
    let told = audience["followers"]["users"].as_array().unwrap();
    assert!(told.contains(&json!(w.maja)) && !told.contains(&json!(w.ilya)), "only those who see the channel: {audience}");
}

#[tokio::test]
async fn deleting_takes_events_with_their_server_and_sign_ups_with_their_membership() {
    let place = Place::new();
    let w = &place.world;
    let mut announced = elsewhere("Meetup", &ahead(120 * MINUTE), Some(1));
    announced["announceChannelId"] = json!(w.general);
    let event = place.made(w.maja, announced).await;
    let message = Id::parse(event["announceMessageId"].as_str().unwrap()).unwrap();
    place.sign_up(w.ilya, &event).await;
    place.sign_up(w.nils, &event).await;

    // Leaving the server takes the sign-up, and the seat goes on.
    crate::permissions::ownership::remove_member(&w.db, w.server, w.ilya).unwrap();
    assert_eq!(place.count("SELECT COUNT(*) FROM \"ServerEventSignups\" WHERE \"UserId\" = ?1", &[&w.ilya]), 0);
    place.run(Timestamp::now(), 0).await;
    assert_eq!(place.notices(w.nils, "seat"), 1);

    // The announcement goes; the event stays.
    w.db.execute("DELETE FROM \"Messages\" WHERE \"Id\" = ?1", [message]).unwrap();
    let (_, read) = place.read(w.maja, &event).await;
    assert_eq!(read["announceMessageId"], Value::Null);

    // The voice channel of another event goes: that one is cancelled, and its
    // people are told why.
    let room = w.channel(None);
    w.db.execute("UPDATE \"Channels\" SET \"Kind\" = 1 WHERE \"Id\" = ?1", [room]).unwrap();
    let voice = place
        .made(
            w.maja,
            json!({ "name": "Call", "place": { "kind": "voice", "channelId": room }, "startsAt": ahead(60 * MINUTE), "endsAt": null, "timeZone": "UTC" }),
        )
        .await;
    place.sign_up(w.jo, &voice).await;
    w.db.execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [room]).unwrap();
    place.run(Timestamp::now(), 0).await;
    let (_, read) = place.read(w.jo, &voice).await;
    assert_eq!(read["state"], "cancelled");
    assert_eq!(place.notices(w.jo, "cancelled"), 1);

    w.db.execute("DELETE FROM \"Servers\" WHERE \"Id\" = ?1", [w.server]).unwrap();
    for table in ["ServerEvents", "ServerEventOccurrences", "ServerEventSignups", "ServerEventAnnouncements", "ServerEventNotices"] {
        assert_eq!(place.count(&format!("SELECT COUNT(*) FROM \"{table}\""), &[]), 0, "{table}");
    }
}

#[tokio::test]
async fn nobody_new_takes_a_seat_while_others_wait_for_it() {
    let place = Place::new();
    let w = &place.world;
    let event = place.made(w.maja, elsewhere("Quiz", &ahead(120 * MINUTE), Some(1))).await;
    place.sign_up(w.ilya, &event).await;
    place.sign_up(w.nils, &event).await;
    // Ilya is banned; the cascade frees the seat before the worker looks.
    crate::permissions::ownership::remove_member(&w.db, w.server, w.ilya).unwrap();
    let (_, newcomer) = place.sign_up(w.jo, &event).await;
    assert_eq!((newcomer["mine"].clone(), newcomer["myPosition"].clone()), (json!("waiting"), json!(1)));
    let (_, nils) = place.read(w.nils, &event).await;
    assert_eq!(nils["mine"], "signed", "the seat went to the one who waited");
    assert_eq!(place.notices(w.nils, "seat"), 1);
}

#[tokio::test]
async fn an_event_whose_hidden_channel_is_deleted_stays_hidden() {
    let place = Place::new();
    let w = &place.world;
    let hidden = w.channel(None);
    w.db.execute("UPDATE \"Channels\" SET \"Kind\" = 1 WHERE \"Id\" = ?1", [hidden]).unwrap();
    w.deny_view_to_base(hidden, false);
    w.member_override(false, hidden, w.nils, crate::permissions::privileges::VIEW_CHANNEL, 0);
    let event = place
        .made(
            w.maja,
            json!({ "name": "Secret", "place": { "kind": "voice", "channelId": hidden }, "startsAt": ahead(60 * MINUTE), "endsAt": null, "timeZone": "UTC" }),
        )
        .await;
    place.sign_up(w.nils, &event).await;
    w.db.execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [hidden]).unwrap();
    place.run(Timestamp::now(), 0).await;

    let (audience, cancelled) = place.published(routes::CHANGED).pop().unwrap();
    assert_eq!(cancelled["state"], "cancelled");
    let told = audience["users"].as_array().unwrap();
    assert!(told.contains(&json!(w.nils)) && told.contains(&json!(w.maja)) && !told.contains(&json!(w.ilya)), "{audience}");
    assert_eq!(place.read(w.ilya, &event).await.0, StatusCode::NOT_FOUND);
    assert_eq!(place.read(w.nils, &event).await.1["state"], "cancelled");
}

#[tokio::test]
async fn a_notice_goes_by_push_alone_and_is_of_no_use_after_the_start() {
    let place = Place::new();
    let w = &place.world;
    let ilya = w.ilya;
    let session = Id::random();
    let now = Timestamp::now();
    w.db.execute(
        "INSERT INTO \"UserPreferences\" (\"UserId\", \"DesktopNotifications\", \"MentionPopups\", \"CallRingtone\",
             \"EmailNotifications\", \"PushNotifications\", \"MentionsOnly\", \"MessageSound\", \"JoinLeaveSound\",
             \"AllowDirectMessages\", \"AllowFriendRequests\", \"ShowPresence\", \"DeveloperMode\", \"ReducedMotion\",
             \"MutedChannelIdsJson\", \"VadSensitivity\")
         VALUES (?1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, '[]', 60)",
        [ilya],
    )
    .unwrap();
    w.db.execute(
        "UPDATE \"Users\" SET \"Email\" = 'ilya@example.test', \"NormalizedEmail\" = 'ILYA@EXAMPLE.TEST' WHERE \"Id\" = ?1",
        [ilya],
    )
    .unwrap();
    w.db.execute(
        "INSERT INTO \"MfaMethods\" (\"Id\", \"UserId\", \"Type\", \"Name\", \"CreatedAt\", \"ConfirmedAt\", \"EmailAddress\")
         VALUES (?1, ?2, 'email', 'E-mail', ?3, ?3, 'ILYA@EXAMPLE.TEST')",
        params![Id::random(), ilya, now],
    )
    .unwrap();
    w.db.execute(
        "INSERT INTO \"UserSessions\" (\"Id\", \"UserId\", \"Device\", \"IpAddress\", \"CreatedAt\", \"LastSeenAt\", \"ExpiresAt\",
             \"AbsoluteExpiresAt\", \"PrimarySource\", \"SecurityVersion\", \"ShortLived\")
         VALUES (?1, ?2, 'test', '127.0.0.1', ?3, ?3, ?4, ?4, 'password', 0, 0)",
        params![session, ilya, now, now.plus(365 * 24 * 60 * MINUTE)],
    )
    .unwrap();
    w.db.execute(
        "INSERT INTO \"PushSubscriptions\" (\"Id\", \"UserId\", \"SessionId\", \"Endpoint\", \"P256dh\", \"Auth\", \"CreatedAt\")
         VALUES (?1, ?2, ?3, 'https://push.example.test/ilya', 'key', 'secret', ?4)",
        params![Id::random(), ilya, session, now],
    )
    .unwrap();
    let event = place.made(w.maja, elsewhere("Movie", &ahead(60 * MINUTE), None)).await;
    place.sign_up(ilya, &event).await;
    let starts = Timestamp::parse(event["startsAt"].as_str().unwrap()).unwrap();
    place.run(starts.minus(10 * MINUTE), 0).await;

    let rows: Vec<(String, String, Timestamp)> = w
        .db
        .prepare("SELECT \"Channel\", \"EventKind\", \"ExpiresAt\" FROM \"NotificationDeliveries\" WHERE \"RecipientId\" = ?1")
        .unwrap()
        .query_map([ilya], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, vec![("push".to_string(), "event-reminder".to_string(), starts)], "no mail, though mail is on");
    let id = ids(&event).0;
    assert_eq!(server_for(&w.db, id, ilya).unwrap(), Some(w.server), "the push opens the event");
    assert!(notice_wanted(&w.db, Kind::EventReminder, id, ilya, starts.minus(MINUTE)).unwrap());
    assert!(!notice_wanted(&w.db, Kind::EventReminder, id, ilya, starts.plus(MINUTE)).unwrap(), "after the start, no use");
    place.sign_out(ilya, &event).await;
    assert!(!notice_wanted(&w.db, Kind::EventReminder, id, ilya, starts.minus(MINUTE)).unwrap(), "nor once signed out");
}

#[tokio::test]
async fn managing_events_lets_a_member_change_and_cancel_anyone_s() {
    let place = Place::new();
    let w = &place.world;
    let event = place.made(w.maja, elsewhere("Owner's", &ahead(60 * MINUTE), None)).await;
    assert_eq!(place.edit(w.ilya, &event, json!({ "name": "Taken" })).await.0, StatusCode::FORBIDDEN);
    assert_eq!(place.cancel(w.ilya, &event).await, StatusCode::FORBIDDEN);
    let managers = w.role("Event crew", 1, crate::permissions::privileges::MANAGE_EVENTS);
    let crew = w.user("crew");
    w.member_of(crew, &[managers]);
    assert_eq!(place.edit(crew, &event, json!({ "name": "Renamed" })).await.0, StatusCode::OK);
    assert_eq!(place.cancel(crew, &event).await, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn the_limits_of_a_server_and_of_the_clock_are_kept() {
    let place = Place::new();
    let w = &place.world;
    // A hundred running or upcoming events on the server.
    let start = Timestamp::now().plus(60 * MINUTE);
    for n in 0..MAX_ACTIVE {
        let id = Id::random();
        w.db.execute(
            "INSERT INTO \"ServerEvents\" (\"Id\", \"ServerId\", \"Name\", \"VoiceChannelId\", \"PlaceText\", \"TimeZone\",
                 \"FirstStartsAt\", \"DurationSeconds\", \"Repeat\", \"CreatedById\", \"CreatedAt\")
             VALUES (?1, ?2, ?3, NULL, 'here', 'UTC', ?4, 3600, 'none', ?5, ?4)",
            params![id, w.server, format!("event {n}"), start, w.maja],
        )
        .unwrap();
        w.db.execute(
            "INSERT INTO \"ServerEventOccurrences\" (\"Id\", \"EventId\", \"Sequence\", \"StartsAt\", \"EndsAt\") VALUES (?1, ?2, 0, ?3, ?4)",
            params![Id::random(), id, start, start.plus(60 * MINUTE)],
        )
        .unwrap();
    }
    let (status, refused) = place.create(w.maja, elsewhere("One too many", &ahead(60 * MINUTE), None)).await;
    assert_eq!((status, refused["code"].clone()), (StatusCode::CONFLICT, json!("event-limit")));

    // An event whose end has passed takes no sign-up, before the worker says.
    let other = Place::new();
    let event = other.made(other.world.maja, elsewhere("Over", &ahead(60 * MINUTE), None)).await;
    let past = Timestamp::now().minus(MINUTE);
    other.sql(
        "UPDATE \"ServerEventOccurrences\" SET \"StartsAt\" = ?2, \"EndsAt\" = ?3 WHERE \"Id\" = ?1",
        &[&ids(&event).1, &past.minus(60 * MINUTE), &past],
    );
    let (status, refused) = other.sign_up(other.world.ilya, &event).await;
    assert_eq!((status, refused["code"].clone()), (StatusCode::CONFLICT, json!("event-over")));
}

#[tokio::test]
async fn an_event_in_a_hidden_channel_is_announced_where_every_reader_sees_it() {
    let place = Place::new();
    let w = &place.world;
    let room = w.channel(None);
    w.db.execute("UPDATE \"Channels\" SET \"Kind\" = 1 WHERE \"Id\" = ?1", [room]).unwrap();
    w.deny_view_to_base(room, false);
    let staff = w.channel(None);
    w.deny_view_to_base(staff, false);
    let event = place
        .made(
            w.maja,
            json!({ "name": "Staff call", "place": { "kind": "voice", "channelId": room }, "startsAt": ahead(60 * MINUTE),
                    "endsAt": null, "timeZone": "UTC", "announceChannelId": staff }),
        )
        .await;
    assert!(event["announceMessageId"].is_string(), "the staff read the card, and all of them see the room");
    let (status, again) = place.edit(w.maja, &event, json!({ "announceChannelId": staff })).await;
    assert_eq!((status, again["code"].clone()), (StatusCode::CONFLICT, json!("event-announced")), "one card per event");
}

#[test]
fn the_migration_stores_the_event_permissions_on_owner_and_administrator_roles() {
    let world = World::new();
    let admin = world.role("Admins", 1, crate::permissions::privileges::ADMINISTRATOR);
    // As they were stored before events.
    world.set_permissions(world.owner_role, 0x7fff);
    world.set_permissions(admin, crate::permissions::privileges::ADMINISTRATOR);
    let version: i64 = world.db.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    world.db.pragma_update(None, "user_version", version - 1).unwrap();
    drop(
        Database::open(&DatabaseSettings {
            path: world.database.path().to_path_buf(),
            busy_timeout: Duration::from_secs(30),
        })
        .unwrap(),
    );
    let mask = |role: Id| -> i64 {
        world
            .db
            .query_row("SELECT \"Permissions\" FROM \"ServerRoles\" WHERE \"Id\" = ?1", [role], |row| row.get(0))
            .unwrap()
    };
    use crate::permissions::privileges::{ALL, CREATE_EVENTS, MANAGE_EVENTS};
    assert_eq!(mask(world.owner_role), ALL);
    assert_eq!(mask(admin) & (CREATE_EVENTS | MANAGE_EVENTS), CREATE_EVENTS | MANAGE_EVENTS);
    assert_eq!(mask(world.base_role) & (CREATE_EVENTS | MANAGE_EVENTS), 0, "plain members are not given them");
    assert_eq!(mask(world.moderator_role) & CREATE_EVENTS, 0);
}

#[tokio::test]
async fn a_moved_time_is_told_to_those_signed_up_and_a_running_event_keeps_its_time() {
    let place = Place::new();
    let (maja, ilya, nils) = (place.world.maja, place.world.ilya, place.world.nils);
    let event = place.made(maja, elsewhere("Talk", &ahead(90 * MINUTE), Some(1))).await;
    place.sign_up(ilya, &event).await;
    place.sign_up(nils, &event).await;
    let later = Timestamp::now().plus(180 * MINUTE);
    let (status, moved) = place
        .edit(maja, &event, json!({ "startsAt": later.to_wire_text(), "endsAt": later.plus(60 * MINUTE).to_wire_text() }))
        .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["signedUp"], 1, "the sign-ups are kept");
    assert_eq!((place.notices(ilya, "changed"), place.notices(nils, "changed")), (1, 1));
    let (status, refused) = place.edit(ilya, &event, json!({ "name": "Mine now" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

    let started = Timestamp::now().minus(MINUTE);
    place.sql(
        "UPDATE \"ServerEventOccurrences\" SET \"StartsAt\" = ?2 WHERE \"Id\" = ?1",
        &[&ids(&event).1, &started],
    );
    let (status, refused) = place.edit(maja, &event, json!({ "startsAt": ahead(60 * MINUTE) })).await;
    assert_eq!((status, refused["code"].clone()), (StatusCode::CONFLICT, json!("event-live")));
    let (status, renamed) = place.edit(maja, &event, json!({ "name": "Talk, live" })).await;
    assert_eq!((status, renamed["name"].clone()), (StatusCode::OK, json!("Talk, live")));
}
