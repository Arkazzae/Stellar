//! Who a message calls on: by name, and through a group when its author may
//! call on the channel; what is stamped on the message, and what an edit
//! leaves of it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State as Extract;
use axum::response::IntoResponse;
use serde_json::{Value, json};
use stellarr_messages::{Language, Messages};

use super::*;
use crate::auth::Caller;
use crate::database::Database;
use crate::events::{Event, EventPublisher, Presence, Unconnected};
use crate::http::{Json, Path};
use crate::messages::post::{Posted, Stored, Submission, post};
use crate::permissions::fixture::{MODERATOR, World};
use crate::permissions::membership::ActiveServer;
use crate::permissions::privileges::{ADMINISTRATOR, ALL, MENTION_EVERYONE, NONE};
use crate::seal::Sealer;
use crate::settings::{DatabaseSettings, Settings};
use crate::state::AppState;
use crate::testing::TempDir;

/// Who is online, by the status each account would give itself; an account
/// it does not list has no connection.
#[derive(Default)]
struct Statuses(HashMap<Id, &'static str>);

impl Presence for Statuses {
    fn presence_of(&self, user: Id) -> String {
        self.0.get(&user).copied().unwrap_or(OFFLINE).to_string()
    }
}

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

fn submission(channel: Id, author: Id, content: &str) -> Submission {
    Submission {
        channel,
        author,
        content: content.to_string(),
        attachments: Vec::new(),
        reply_to: None,
        client_id: None,
        scheduled: None,
        poll: None,
        event: None,
    }
}

/// Posts through the post path, as the route does.
fn send(world: &World, presence: &dyn Presence, channel: Id, author: Id, content: &str) -> Stored {
    match post(&world.db, &messages(), presence, world.server, &submission(channel, author, content)) {
        Ok(Posted::Stored(stored)) => *stored,
        other => panic!("the message was not posted: {other:?}"),
    }
}

/// Who got an inbox entry for the message.
fn told(stored: &Stored) -> Vec<Id> {
    stored.mentions.iter().map(|entry| entry.user).collect()
}

fn sorted(mut ids: Vec<Id>) -> Vec<Id> {
    ids.sort();
    ids
}

fn stamped(world: &World, message: Id) -> i64 {
    world
        .db
        .query_row("SELECT \"GroupMention\" FROM \"Messages\" WHERE \"Id\" = ?1", [message], |row| row.get(0))
        .unwrap()
}

/// Everyone in the fixture's server but its owner, Maja.
fn all_but_maja(world: &World) -> Vec<Id> {
    sorted(vec![world.ada, world.ilya, world.nils, world.jo])
}

#[test]
fn the_reader_of_mentions_reads_the_words_of_groups_as_it_reads_names() {
    let read = |content: &str| stellarr_mentions::extract_normalized_usernames(content);
    assert_eq!(read("@everyone"), BTreeSet::from(["EVERYONE".to_string()]));
    assert_eq!(groups_in(&read("@everyone")), EVERYONE);
    assert_eq!(groups_in(&read("hej @Wszyscy!")), EVERYONE);
    assert_eq!(groups_in(&read("@HERE i @obecni")), HERE);
    assert_eq!(groups_in(&read("@everyone, @here")), EVERYONE | HERE);
    // Code, an escape, a link's label, an address and a longer word call
    // nobody, as they name nobody.
    for quiet in [
        "`@everyone`",
        "\\@everyone",
        "[@everyone](https://example.test)",
        "everyone@here.test",
        "@everyones",
        "@here_and_now",
    ] {
        assert_eq!(groups_in(&read(quiet)), 0, "{quiet}");
    }
    // The reader takes a full stop as part of a name; the group is read from
    // the name without its trailing dots, the name itself stays as it is.
    assert_eq!(read("Spotkanie o 20 @wszyscy."), BTreeSet::from(["WSZYSCY.".to_string()]));
    for word in ["EVERYONE", "WSZYSCY", "HERE", "OBECNI"] {
        assert!(is_group_name(word), "{word}");
    }
    assert!(!is_group_name("everyone"), "names come upper-cased");
    assert!(!is_group_name("ADA"));
}

/// The cases the client's parser is held to as well
/// (`web/src/stellarr/lib/messages/groupMentions.test.ts`), so that a pill is
/// drawn exactly where the server called a group: content, and the groups it
/// calls on.
const SENTENCE_ENDS: [(&str, i64); 10] = [
    ("Spotkanie o 20 @wszyscy.", EVERYONE),
    ("@EVERYONE...", EVERYONE),
    ("Kto jest, @obecni.", HERE),
    ("(@here.)", HERE),
    ("@everyone. @here.", EVERYONE | HERE),
    ("@wszyscy!", EVERYONE),
    ("@wszyscy.x", 0),
    ("@wszyscy.-", 0),
    ("`@wszyscy.`", 0),
    ("\\@wszyscy.", 0),
];

#[test]
fn a_sentence_ending_on_a_groups_word_calls_the_group() {
    for (content, groups) in SENTENCE_ENDS {
        let names = stellarr_mentions::extract_normalized_usernames(content);
        assert_eq!(groups_in(&names), groups, "{content:?}");
        assert_eq!(groups_kept(EVERYONE | HERE, content), groups, "{content:?}");
    }
    // A name made of dots alone, or a group's word with more after its dots,
    // is no group.
    assert_eq!(groups_in(&BTreeSet::from([".".to_string(), "HERE.NOW".to_string()])), 0);
    // The reserved names are the words themselves; a username may end with a
    // dot, so `wszyscy.` is a name an account can have.
    assert!(!is_group_name("WSZYSCY."));
}

#[test]
fn an_account_whose_name_ends_with_a_dot_after_a_groups_word_is_still_named_by_it() {
    let world = World::new();
    let dotted = world.user("wszyscy.");
    world.admit(dotted, world.base_role);

    // Without the permission it is a mention of the account and nothing more.
    let plain = send(&world, &Unconnected, world.general, world.ilya, "Spotkanie o 20 @wszyscy.");
    assert_eq!(told(&plain), vec![dotted]);
    assert_eq!(stamped(&world, plain.message.id), 0);

    // With it the group is called too, and the account is told as named.
    let content = "Spotkanie o 20 @wszyscy.";
    let names = stellarr_mentions::extract_normalized_usernames(content);
    let called = notify(
        &world.db,
        &Mentioning {
            server: world.server,
            channel: world.general,
            author: world.maja,
            author_name: "Maja",
            content,
            created_at: Timestamp::now(),
            names: &names,
            groups: groups_in(&names),
            presence: &Unconnected,
        },
    )
    .unwrap();
    assert_eq!(groups_in(&names), EVERYONE);
    assert_eq!(called.named, vec![dotted]);
    assert_eq!(called.grouped, all_but_maja(&world));
}

#[test]
fn an_edit_keeps_a_groups_call_only_while_its_word_is_there_and_never_adds_one() {
    assert_eq!(groups_kept(EVERYONE | HERE, "@here only"), HERE);
    assert_eq!(groups_kept(EVERYONE | HERE, "@wszyscy @obecni"), EVERYONE | HERE);
    assert_eq!(groups_kept(HERE, "@everyone @here"), HERE);
    assert_eq!(groups_kept(0, "@everyone"), 0);
    assert_eq!(groups_kept(EVERYONE, "`@everyone`"), 0);
}

#[test]
fn a_plain_members_call_on_everyone_is_plain_text_that_tells_nobody() {
    let world = World::new();

    let stored = send(&world, &Unconnected, world.general, world.ilya, "@everyone @here @wszyscy @obecni");

    assert!(told(&stored).is_empty());
    assert_eq!(stamped(&world, stored.message.id), 0);
    assert!(!stored.message.mentions_everyone && !stored.message.mentions_here);
}

#[test]
fn an_owners_call_on_everyone_tells_every_member_who_sees_the_channel_but_the_author() {
    let world = World::new();

    for word in ["Hej @everyone", "@WSZYSCY, zobaczcie", "Spotkanie o 20 @wszyscy."] {
        let stored = send(&world, &Unconnected, world.general, world.maja, word);

        assert_eq!(told(&stored), all_but_maja(&world), "{word}");
        assert_eq!(stamped(&world, stored.message.id), EVERYONE, "{word}");
        assert!(stored.message.mentions_everyone && !stored.message.mentions_here, "{word}");
        let wire = serde_json::to_value(&stored.message).unwrap();
        assert_eq!((&wire["mentionsEveryone"], &wire["mentionsHere"]), (&json!(true), &json!(false)));
    }
}

#[test]
fn a_call_on_those_here_tells_only_those_online_idle_or_busy() {
    let world = World::new();
    let presence = Statuses(HashMap::from([
        (world.ilya, "online"),
        (world.nils, "idle"),
        (world.jo, "dnd"),
        // Connected, but appearing offline.
        (world.ada, "offline"),
    ]));

    for word in ["@here", "@Obecni"] {
        let stored = send(&world, &presence, world.general, world.maja, word);

        assert_eq!(told(&stored), sorted(vec![world.ilya, world.nils, world.jo]), "{word}");
        assert_eq!(stamped(&world, stored.message.id), HERE);
        assert!(stored.message.mentions_here && !stored.message.mentions_everyone);
    }
    // Both at once reach everybody, and both are stamped.
    let both = send(&world, &presence, world.general, world.maja, "@everyone @here");
    assert_eq!(told(&both), all_but_maja(&world));
    assert_eq!(stamped(&world, both.message.id), EVERYONE | HERE);
}

#[test]
fn the_permission_is_decided_in_the_channel_so_an_override_gives_or_takes_it() {
    let world = World::new();
    world.override_of(false, world.projects, world.base_role, MENTION_EVERYONE, NONE);
    world.set_permissions(world.moderator_role, MODERATOR | MENTION_EVERYONE);
    world.override_of(false, world.projects, world.moderator_role, NONE, MENTION_EVERYONE);

    let given = send(&world, &Unconnected, world.projects, world.ilya, "@everyone");
    assert_eq!(told(&given), sorted(vec![world.maja, world.ada, world.nils, world.jo]));
    let elsewhere = send(&world, &Unconnected, world.general, world.ilya, "@everyone");
    assert!(told(&elsewhere).is_empty());

    let taken = send(&world, &Unconnected, world.projects, world.ada, "@everyone");
    assert!(told(&taken).is_empty());
    assert_eq!(stamped(&world, taken.message.id), 0);
    let held = send(&world, &Unconnected, world.general, world.ada, "@everyone");
    assert_eq!(told(&held), sorted(vec![world.maja, world.ilya, world.nils, world.jo]));
}

#[test]
fn somebody_named_and_in_the_group_is_told_once_as_somebody_named() {
    let world = World::new();

    let stored = send(&world, &Unconnected, world.general, world.maja, "@everyone @ilya @ilya");
    assert_eq!(told(&stored), all_but_maja(&world));

    let content = "@everyone @ilya";
    let names = stellarr_mentions::extract_normalized_usernames(content);
    let split = notify(
        &world.db,
        &Mentioning {
            server: world.server,
            channel: world.general,
            author: world.maja,
            author_name: "Maja",
            content,
            created_at: Timestamp::now(),
            names: &names,
            groups: EVERYONE,
            presence: &Unconnected,
        },
    )
    .unwrap();
    assert_eq!(split.named, vec![world.ilya]);
    assert_eq!(split.grouped, sorted(vec![world.ada, world.nils, world.jo]));
    assert_eq!(split.entries.len(), 4);
}

#[test]
fn those_who_do_not_see_the_channel_and_switched_off_accounts_are_left_out() {
    let world = World::new();
    let staff = world.channel(None);
    world.deny_view_to_base(staff, false);
    world.allow_view_to(staff, false, world.moderator_role);

    let hidden = send(&world, &Unconnected, staff, world.maja, "@everyone");
    assert_eq!(told(&hidden), vec![world.ada]);

    world.disable(world.nils);
    let open = send(&world, &Unconnected, world.general, world.maja, "@everyone");
    assert_eq!(told(&open), sorted(vec![world.ada, world.ilya, world.jo]));
}

#[test]
fn an_account_named_like_a_group_is_never_called_by_its_name() {
    // Somebody who took the name before it was reserved: the group meaning
    // wins, so only a call on the group reaches them, and as one of it.
    let world = World::new();
    let legacy = world.user("everyone");
    world.admit(legacy, world.base_role);

    let plain = send(&world, &Unconnected, world.general, world.ilya, "@everyone");
    assert!(told(&plain).is_empty());

    let content = "@everyone";
    let names = stellarr_mentions::extract_normalized_usernames(content);
    let called = notify(
        &world.db,
        &Mentioning {
            server: world.server,
            channel: world.general,
            author: world.maja,
            author_name: "Maja",
            content,
            created_at: Timestamp::now(),
            names: &names,
            groups: EVERYONE,
            presence: &Unconnected,
        },
    )
    .unwrap();
    assert!(called.named.is_empty());
    assert!(called.grouped.contains(&legacy));
}

#[test]
fn the_migration_gives_the_permission_to_owners_and_administrators_and_adds_the_column_once() {
    let world = World::new();
    let admins = world.role("Admins", 1, ADMINISTRATOR);
    // As they were stored before group mentions.
    world.set_permissions(world.owner_role, 0x1_ffff);
    let reopen = |back: i64| {
        let version: i64 = world.db.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        world.db.pragma_update(None, "user_version", version - back).unwrap();
        drop(
            Database::open(&DatabaseSettings {
                path: world.database.path().to_path_buf(),
                busy_timeout: Duration::from_secs(30),
            })
            .unwrap(),
        );
    };
    let mask = |role: Id| -> i64 {
        world
            .db
            .query_row("SELECT \"Permissions\" FROM \"ServerRoles\" WHERE \"Id\" = ?1", [role], |row| row.get(0))
            .unwrap()
    };

    // The step runs again on a file that has the column already, and so do
    // the three after it (the activity dates and update checks), which
    // change nothing they have not changed already.
    reopen(4);

    assert_eq!(mask(world.owner_role), ALL);
    assert_eq!(mask(admins) & MENTION_EVERYONE, MENTION_EVERYONE);
    assert_eq!(mask(world.base_role) & MENTION_EVERYONE, 0, "plain members are not given it");
    assert_eq!(mask(world.moderator_role) & MENTION_EVERYONE, 0);
    let stored = send(&world, &Unconnected, world.general, world.ilya, "hej");
    assert_eq!(stamped(&world, stored.message.id), 0, "a message calls on nobody by default");
}

// ---- Through the routes, with the state's own presence

struct Place {
    world: World,
    _data: TempDir,
    state: AppState,
    events: Arc<Recorder>,
}

impl Place {
    fn new(presence: impl Fn(&World) -> Statuses) -> Place {
        let world = World::new();
        let data = TempDir::new();
        let settings = Settings::load(&[
            ("STELLARR_ENVIRONMENT".to_string(), "Testing".to_string()),
            ("STELLARR_DATA_ROOT".to_string(), data.path().display().to_string()),
        ])
        .unwrap();
        let events = Arc::new(Recorder::default());
        let state = AppState::new(
            settings,
            world.database.clone(),
            events.clone(),
            Arc::new(presence(&world)),
            Sealer::from_key([5; 32]),
            Sealer::from_key([6; 32]),
        );
        Place {
            world,
            _data: data,
            state,
            events,
        }
    }

    fn caller(user: Id) -> Caller {
        Caller {
            user,
            session: Id::random(),
        }
    }

    async fn answer(response: axum::response::Response) -> Value {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(status.is_success(), "{status}: {}", String::from_utf8_lossy(&bytes));
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn say(&self, author: Id, content: &str) -> Value {
        let response = crate::messages::post::create(
            Extract(self.state.clone()),
            Self::caller(author),
            ActiveServer(self.world.server),
            Path(self.world.general),
            Json(serde_json::from_value(json!({ "content": content })).unwrap()),
        )
        .await
        .unwrap_or_else(IntoResponse::into_response);
        Self::answer(response).await
    }

    async fn edit(&self, author: Id, message: &Value, content: &str) -> Value {
        let id = Id::parse(message["id"].as_str().unwrap()).unwrap();
        let response = crate::messages::edit::edit(
            Extract(self.state.clone()),
            Self::caller(author),
            ActiveServer(self.world.server),
            Path(id),
            Json(serde_json::from_value(json!({ "content": content })).unwrap()),
        )
        .await
        .unwrap_or_else(IntoResponse::into_response);
        Self::answer(response).await
    }

    fn published(&self, name: &str) -> Vec<Value> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.name == name)
            .map(|event| serde_json::from_str(&event.payload).unwrap())
            .collect()
    }
}

fn flags(message: &Value) -> (bool, bool) {
    (
        message["mentionsEveryone"].as_bool().unwrap(),
        message["mentionsHere"].as_bool().unwrap(),
    )
}

#[tokio::test]
async fn a_post_reads_who_is_here_from_the_state_and_its_event_carries_the_stamp() {
    let place = Place::new(|world| Statuses(HashMap::from([(world.jo, "online")])));
    let world = &place.world;

    let message = place.say(world.maja, "@obecni").await;

    assert_eq!(flags(&message), (false, true));
    let created = place.published("MessageCreated");
    assert_eq!(flags(&created[0]), (false, true));
    let entries: Vec<Id> = world
        .db
        .prepare("SELECT \"UserId\" FROM \"UserNotifications\"")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(entries, vec![world.jo]);
    // History reads the same stamp.
    let id = Id::parse(message["id"].as_str().unwrap()).unwrap();
    let loaded = crate::messages::dto::load(&world.db, &messages(), &[id], None).unwrap();
    assert!(loaded[0].mentions_here && !loaded[0].mentions_everyone);
}

#[tokio::test]
async fn an_edit_shrinks_the_stamp_to_the_words_left_and_never_grows_it_or_tells_anyone() {
    let place = Place::new(|_| Statuses::default());
    let world = &place.world;
    let message = place.say(world.maja, "@everyone @here").await;
    assert_eq!(flags(&message), (true, true));
    let told_before: i64 = world
        .db
        .query_row("SELECT COUNT(*) FROM \"UserNotifications\"", [], |row| row.get(0))
        .unwrap();

    let shrunk = place.edit(world.maja, &message, "only @here now").await;
    assert_eq!(flags(&shrunk), (false, true));
    let regrown = place.edit(world.maja, &message, "@everyone @here again").await;
    assert_eq!(flags(&regrown), (false, true));
    let updated = place.published("MessageUpdated");
    assert_eq!(flags(updated.last().unwrap()), (false, true));

    // A plain member's words are never stamped, edited in or not.
    let plain = place.say(world.ilya, "hej").await;
    assert_eq!(flags(&place.edit(world.ilya, &plain, "@everyone").await), (false, false));

    let told_after: i64 = world
        .db
        .query_row("SELECT COUNT(*) FROM \"UserNotifications\"", [], |row| row.get(0))
        .unwrap();
    assert_eq!(told_after, told_before, "an edit tells nobody");
}
