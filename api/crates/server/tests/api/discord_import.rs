//! The import's writer, fed by the real reader from a Discord server held in
//! memory: the structure it makes, the history it restores, and that the
//! history behaves as if it had always been there.
//!
//! The writer runs in the test's own process on a second handle to the
//! database of a real binary, which is how a route that starts an import would
//! have it next to the routes that serve people. What people see is read from
//! the binary's own routes.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::database::Database;
use stellarr_api::discord_import::{
    Import, ImportError, ImportWriter, LinkResult, PageResult, SkipReason, StructureResult,
    message_id,
};
use stellarr_api::settings::DatabaseSettings;
use stellarr_api::values::{Id, Timestamp};
use stellarr_discord::{BotReader, DiscordChannelType, MessageSnapshot, ServerSnapshot};

use crate::support::{Person, Request, Server};

/// The reader's own fake Discord, which its tests use: the writer is tested
/// against what the reader really produces.
#[path = "../../../discord/tests/support/mod.rs"]
pub(crate) mod discord;

use discord::Rig;
use discord::guild::{Ch, FakeGuild, ID, Msg};

/// A real binary, a writer on its database, and a Discord to read.
struct World {
    server: Server,
    ada: Person,
    people: HashMap<String, Person>,
    writer: ImportWriter,
    server_id: Id,
    rig: Rig,
    /// When a message was pinned, where the fake Discord says it was pinned
    /// when it was sent.
    pin_times: HashMap<String, stellarr_discord::Timestamp>,
}

/// Where an import stands after its structure is written.
struct Started {
    import: Import,
    snapshot: ServerSnapshot,
    bot: BotReader,
    structure: StructureResult,
}

/// What a whole run came to, page by page.
struct Run {
    import: Import,
    structure: StructureResult,
    pages: Vec<PageResult>,
    links: Vec<LinkResult>,
}

impl Run {
    fn written(&self) -> usize {
        self.pages.iter().map(|p| p.written).sum()
    }
    fn already_seen(&self) -> usize {
        self.pages.iter().map(|p| p.already_seen).sum()
    }
    fn placeholders_made(&self) -> usize {
        self.pages.iter().map(|p| p.placeholders_made).sum()
    }
    fn skipped(&self) -> BTreeMap<SkipReason, usize> {
        let mut all = BTreeMap::new();
        for page in &self.pages {
            for (reason, count) in &page.skipped {
                *all.entry(*reason).or_insert(0) += count;
            }
        }
        all
    }
    fn notices(&self) -> BTreeMap<i32, usize> {
        let mut all = BTreeMap::new();
        for page in &self.pages {
            for (kind, count) in &page.notices_by_type {
                *all.entry(*kind).or_insert(0) += count;
            }
        }
        all
    }
}

#[derive(Clone, Copy)]
struct Order {
    page_size: usize,
    oldest_first: bool,
    threads_first: bool,
}

impl Default for Order {
    fn default() -> Order {
        Order {
            page_size: 100,
            oldest_first: false,
            threads_first: false,
        }
    }
}

impl World {
    /// The instance's server, owned by `ada`, with `members` registered in it.
    async fn new(guild: FakeGuild, members: &[&str]) -> World {
        let server = Server::start().await;
        let ada = server.found("ada").await;
        let mut people = HashMap::new();
        for name in members {
            people.insert((*name).to_string(), server.register(name).await);
        }
        let server_id: String = server
            .database()
            .query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0))
            .unwrap();
        let database = Database::open(&DatabaseSettings {
            path: server.database_path(),
            busy_timeout: Duration::from_secs(30),
        })
        .expect("a second handle to the database");
        let rig = Rig::new(guild.handler()).await;
        World {
            writer: ImportWriter::new(database),
            server_id: Id::parse(&server_id).unwrap(),
            server,
            ada,
            people,
            rig,
            pin_times: HashMap::new(),
        }
    }

    fn db(&self) -> Connection {
        self.server.database()
    }

    fn ada_id(&self) -> Id {
        Id::parse(&self.ada.id).unwrap()
    }

    async fn start(&self) -> Started {
        let bot = self.rig.bot();
        let snapshot = bot.read_server(ID).await.expect("the server reads");
        let import = self
            .writer
            .start(self.server_id, self.ada_id(), &snapshot)
            .await
            .unwrap();
        let structure = self.writer.write_structure(&import, &snapshot).await.unwrap();
        Started {
            import,
            snapshot,
            bot,
            structure,
        }
    }

    /// The pages of one channel as the reader gives them, cut to `size`.
    async fn pages_of(
        &self,
        started: &Started,
        source: &str,
        order: Order,
    ) -> (Vec<Vec<MessageSnapshot>>, HashMap<String, stellarr_discord::Timestamp>) {
        let mut pins = HashMap::new();
        for pin in started.bot.pins(source).unwrap().collect_all().await.unwrap() {
            pins.insert(pin.message.id.clone(), pin.pinned_at);
        }
        pins.extend(self.pin_times.clone());
        let history = started
            .bot
            .messages(source, None)
            .unwrap()
            .collect_all()
            .await
            .unwrap();
        let mut pages: Vec<Vec<MessageSnapshot>> =
            history.chunks(order.page_size).map(<[_]>::to_vec).collect();
        if order.oldest_first {
            pages.reverse();
        }
        (pages, pins)
    }

    /// Imports every channel and thread in pages as the next stage will, each
    /// completed after its last page.
    async fn import_all(&self, started: Started, order: Order) -> Run {
        let channels: Vec<String> = started.snapshot.channels.iter().map(|c| c.id.clone()).collect();
        let threads: Vec<String> = started.snapshot.threads.iter().map(|t| t.id.clone()).collect();
        let sources: Vec<String> = if order.threads_first {
            threads.into_iter().chain(channels).collect()
        } else {
            channels.into_iter().chain(threads).collect()
        };
        let (mut pages, mut links) = (Vec::new(), Vec::new());
        for source in &sources {
            let (history, pins) = self.pages_of(&started, source, order).await;
            for page in history {
                pages.push(
                    self.writer
                        .write_messages(&started.import, &started.snapshot, source, &page, Some(&pins))
                        .await
                        .unwrap(),
                );
            }
            links.push(self.writer.complete_channel(&started.import, source).await.unwrap());
        }
        Run {
            import: started.import,
            structure: started.structure,
            pages,
            links,
        }
    }

    async fn run(&self) -> Run {
        self.run_ordered(Order::default()).await
    }

    async fn run_ordered(&self, order: Order) -> Run {
        let started = self.start().await;
        self.import_all(started, order).await
    }

    fn channel(&self, discord_id: &str) -> String {
        channel_of(&self.db(), self.server_id, discord_id)
    }

    async fn get(&self, who: &Person, path: &str) -> Value {
        let response = who.client.get(path).await;
        assert_eq!(response.status, 200, "{path}: {}", response.text());
        response.json()
    }

    fn message(&self, discord_id: &str) -> Id {
        message_id(self.server_id, discord_id)
    }

    fn content_of(&self, discord_id: &str) -> Option<String> {
        self.db()
            .query_row(
                "SELECT \"Content\" FROM \"Messages\" WHERE \"Id\" = ?1",
                [self.message(discord_id)],
                |row| row.get(0),
            )
            .ok()
    }
}

/// The Stellarr channel an imported Discord channel became, as clients write it.
fn channel_of(db: &Connection, server: Id, discord_id: &str) -> String {
    let id: String = db
        .query_row(
            "SELECT \"ChannelId\" FROM \"DiscordImportChannels\" WHERE \"ServerId\" = ?1 AND \"DiscordId\" = ?2",
            params![server, discord_id],
            |row| row.get(0),
        )
        .unwrap();
    id.to_lowercase()
}

fn count(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// 2024-01-01 and `minute` minutes on, as the fake Discord says it.
fn utc(minute: i64) -> Timestamp {
    Timestamp::from_unix_seconds(1_704_067_200 + minute * 60).unwrap()
}

fn plain() -> Msg<'static> {
    Msg::default()
}

fn with_reply(to: &str) -> Msg<'_> {
    Msg {
        reply_to: Some(to),
        ..Msg::default()
    }
}

#[tokio::test]
async fn the_structure_comes_in_discords_order_and_a_second_run_makes_nothing() {
    let guild = FakeGuild::new("Krew");
    guild
        .category("900", "Talk", 1)
        .category("800", "Games & Fun", 0)
        .channel(
            "110",
            "General",
            Ch {
                parent: Some("900"),
                topic: Some("  Hello  "),
                slow_mode: 30,
                ..Ch::default()
            },
        )
        .channel("100", "general", Ch { parent: Some("900"), ..Ch::default() })
        .channel("120", "Lounge", Ch { kind: 2, parent: Some("900"), user_limit: 5, ..Ch::default() })
        .channel(
            "130",
            "help",
            Ch {
                kind: 15,
                parent: Some("800"),
                topic: Some(&"t".repeat(1500)),
                ..Ch::default()
            },
        )
        .channel("140", "Town Hall", Ch { kind: 13, parent: Some("800"), user_limit: 10000, ..Ch::default() })
        .channel("150", "📢・news", Ch { kind: 5, ..Ch::default() })
        .channel("160", "staff", Ch { parent: Some("900"), position: 3, hidden: true, ..Ch::default() })
        .thread("170", "130", "How do I join?", 11);
    let world = World::new(guild, &[]).await;
    let before: Vec<(String, i64)> = {
        let db = world.db();
        let mut statement = db.prepare("SELECT \"Name\", \"Kind\" FROM \"Channels\" ORDER BY \"Position\"").unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };

    let first = world.run().await;
    let second = world.run().await;

    let db = world.db();
    let sections: Vec<String> = db
        .prepare("SELECT \"Name\" FROM \"Sections\" ORDER BY \"Position\"")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(sections, ["Games & Fun", "Talk"]);
    let channels: Vec<(String, i64, String, i64, i64)> = db
        .prepare("SELECT \"Name\", \"Kind\", \"Topic\", \"SlowModeSeconds\", \"UserLimit\" FROM \"Channels\" ORDER BY \"Position\"")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    // The server's own channels stay first, and a name taken already gets a number.
    let imported: Vec<_> = channels[before.len()..].to_vec();
    let names: Vec<&str> = imported.iter().map(|c| c.0.as_str()).collect();
    assert_eq!(names, ["news", "help", "town-hall", "general-2", "general-3", "staff", "lounge"]);
    let kinds: Vec<i64> = imported.iter().map(|c| c.1).collect();
    assert_eq!(kinds, [0, 0, 1, 0, 0, 0, 1]);
    let by_name = |name: &str| imported.iter().find(|c| c.0 == name).unwrap().clone();
    assert_eq!((by_name("general-3").2, by_name("general-3").3), ("Hello".to_string(), 30));
    assert_eq!(by_name("help").2.chars().count(), 1024);
    assert_eq!(by_name("lounge").4, 5);
    assert_eq!(by_name("town-hall").4, 0);
    let talk: String = db
        .query_row("SELECT \"Id\" FROM \"Sections\" WHERE \"Name\" = 'Talk'", [], |row| row.get(0))
        .unwrap();
    let general_section: String = db
        .query_row("SELECT \"SectionId\" FROM \"Channels\" WHERE \"Name\" = 'general-3'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(talk, general_section);

    assert_eq!(
        (first.structure.sections_made, first.structure.channels_made, first.structure.threads_recorded),
        (2, 7, 1)
    );
    assert_eq!(
        first.structure.simplified,
        BTreeMap::from([
            ("130".to_string(), DiscordChannelType::Forum),
            ("140".to_string(), DiscordChannelType::Stage),
            ("150".to_string(), DiscordChannelType::Announcements),
        ])
    );
    assert_eq!(first.structure.hidden_from_everyone, ["160"]);
    assert_eq!(
        (second.structure.sections_made, second.structure.channels_made, second.structure.threads_recorded, second.structure.already_imported),
        (0, 0, 0, 10)
    );

    let (kind, parent, channel): (i64, String, String) = db
        .query_row(
            "SELECT \"Kind\", \"ParentDiscordId\", \"ChannelId\" FROM \"DiscordImportChannels\" WHERE \"DiscordId\" = '170'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let forum: String = db
        .query_row("SELECT \"ChannelId\" FROM \"DiscordImportChannels\" WHERE \"DiscordId\" = '130'", [], |row| row.get(0))
        .unwrap();
    assert_eq!((kind, parent.as_str(), channel), (1, "130", forum));
    let private: bool = db
        .query_row("SELECT \"IsPrivate\" FROM \"DiscordImportChannels\" WHERE \"DiscordId\" = '160'", [], |row| row.get(0))
        .unwrap();
    assert!(private);
}

#[tokio::test]
async fn history_keeps_its_authors_and_times_and_a_second_run_writes_nothing() {
    let guild = FakeGuild::new("Krew");
    guild
        .text("100", "general")
        .member("11", "jo", Some("Jo Global"), Some("Jo on Krew"))
        .member("12", "ada", None, None)
        .role("5", "Mods");
    guild.message("1001", "100", "11", "first, by the person Discord calls jo", 1, plain());
    guild.message(
        "1002",
        "100",
        "12",
        "hi <@11> and <@13>, see <#100> and <@&5>",
        2,
        Msg { username: Some("ada"), edited_minute: Some(5), ..plain() },
    );
    guild.message("1003", "100", "11", "pinned", 3, Msg { pinned: true, ..plain() });
    let pinned_at = stellarr_discord::parse_timestamp(&discord::guild::at(4)).unwrap();
    // A webhook names every message as it likes; its placeholder keeps the name
    // the import met first, which is the newest.
    guild.message("1004", "100", "", "from the hook", 6, Msg { username: Some("Other name"), webhook_id: Some("77"), ..plain() });
    let long = "W".repeat(80);
    guild.message("1005", "100", "", "from the hook again", 7, Msg { username: Some(&long), webhook_id: Some("77"), ..plain() });
    let mut world = World::new(guild, &["nils"]).await;
    world.pin_times.insert("1003".to_string(), pinned_at);

    let first = world.run().await;
    let db = world.db();
    let counts = |db: &Connection| {
        (
            count(db, "SELECT COUNT(*) FROM \"Messages\""),
            count(db, "SELECT COUNT(*) FROM \"Users\""),
            count(db, "SELECT COUNT(*) FROM \"MessageChanges\""),
        )
    };
    let held = counts(&db);
    let second = world.run().await;

    assert_eq!((first.written(), first.already_seen(), first.placeholders_made()), (5, 0, 3));
    assert_eq!((second.written(), second.already_seen(), second.placeholders_made()), (0, 5, 0));
    assert_eq!(held, counts(&db));
    let channel = world.channel("100");
    type Row = (String, String, String, String, bool, Option<String>, Option<String>);
    let rows: Vec<Row> = db
        .prepare(
            "SELECT m.\"Id\", u.\"Username\", u.\"DisplayName\", m.\"Content\", u.\"IsDisabled\" AND u.\"PasswordHash\" IS NULL AND u.\"Email\" IS NULL AND u.\"Origin\" = 'discord',
                    m.\"EditedAt\", m.\"PinnedAt\"
               FROM \"Messages\" m JOIN \"Users\" u ON u.\"Id\" = m.\"AuthorId\"
              WHERE m.\"ChannelId\" = ?1 ORDER BY m.\"CreatedAt\"",
        )
        .unwrap()
        .query_map([channel.to_uppercase()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let usernames: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(usernames, ["discord:11", "discord:12", "discord:11", "discord:77", "discord:77"]);
    let shown: Vec<&str> = rows.iter().map(|r| r.2.as_str()).collect();
    assert_eq!(shown, ["Jo on Krew", "ada", "Jo on Krew", &"W".repeat(64), &"W".repeat(64)]);
    assert!(rows.iter().all(|r| r.4), "every author is a disabled placeholder with no password or address");
    assert_eq!(rows[1].5.as_deref(), Some(utc(5).to_db_text().as_str()));
    assert_eq!(rows[2].6.as_deref(), Some(utc(4).to_db_text().as_str()));
    assert_eq!(rows[0].6, None);
    assert_eq!(rows[1].0, world.message("1002").to_db_text());
    // Mentions are labels with the name shown; nobody is mentioned, not even the real jo.
    assert_eq!(rows[1].3, r"hi \@Jo on Krew and \@unknown-user, see #general-2 and \@Mods");
    assert!(
        rows.iter().all(|r| stellarr_discord::extract_normalized_usernames(&r.3).is_empty()),
        "no imported message mentions anybody"
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM \"ServerMembers\" WHERE \"UserId\" IN (SELECT \"AuthorId\" FROM \"Messages\" WHERE \"IsSystem\" = 0 AND \"AuthorId\" IN (SELECT \"UserId\" FROM \"DiscordImportAccounts\"))"),
        0
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"DiscordImportAccounts\""), 3);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM \"DiscordImportAccounts\" WHERE \"DiscordId\" = '77' AND \"IsWebhook\" = 1"), 1);

    // The routes that show history name the author as a placeholder.
    let history = world.get(&world.ada, &format!("/api/channels/{channel}/messages")).await;
    for message in history["messages"].as_array().unwrap() {
        assert_eq!(message["author"]["origin"], "discord", "{message}");
    }
}

#[tokio::test]
async fn a_reply_chain_ends_on_its_root_whichever_order_the_pages_came_in() {
    for oldest_first in [false, true] {
        let guild = FakeGuild::new("Krew");
        guild.text("500", "general");
        // Ids across a digit boundary, so text order would be wrong.
        guild.message("98", "500", "21", "a: the start", 1, plain());
        guild.message("99", "500", "22", "b: answers a", 2, with_reply("98"));
        guild.message("100", "500", "21", "c: answers b", 3, with_reply("99"));
        guild.message("101", "500", "22", "d: answers something deleted", 4, with_reply("50"));
        let world = World::new(guild, &[]).await;

        world
            .run_ordered(Order { page_size: 1, oldest_first, ..Order::default() })
            .await;

        let channel = world.channel("500");
        let db = world.db();
        let replies: BTreeMap<String, Option<String>> = db
            .prepare("SELECT SUBSTR(\"Content\", 1, 1), \"ReplyToMessageId\" FROM \"Messages\" WHERE \"ChannelId\" = ?1")
            .unwrap()
            .query_map([channel.to_uppercase()], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let a = Some(world.message("98").to_db_text());
        assert_eq!(
            replies,
            BTreeMap::from([
                ("a".to_string(), None),
                ("b".to_string(), a.clone()),
                ("c".to_string(), a),
                ("d".to_string(), None),
            ]),
            "oldest first: {oldest_first}"
        );

        let threads = world.get(&world.ada, &format!("/api/channels/{channel}/threads")).await;
        let thread = &threads.as_array().unwrap()[0];
        assert_eq!(threads.as_array().unwrap().len(), 1);
        assert_eq!(thread["root"]["id"], world.message("98").to_string());
        assert_eq!(thread["replyCount"], 2);
    }
}

#[tokio::test]
async fn a_thread_answers_its_starter_and_a_forum_post_is_headed_by_its_name() {
    for threads_first in [true, false] {
        let guild = FakeGuild::new("Krew");
        guild
            .text("100", "general")
            .channel("200", "help", Ch { kind: 15, ..Ch::default() })
            .member("34", "jo", None, None);
        guild.message("1000", "100", "31", "question", 1, plain());
        // The thread grew from 1001, which answers 1000: its replies go to 1000.
        guild.message("1001", "100", "32", "starter", 2, with_reply("1000"));
        guild.thread("1001", "100", "side talk", 11);
        guild.message("1002", "1001", "", "", 3, Msg { kind: 21, ..plain() });
        guild.message("1003", "1001", "33", "in the thread", 4, plain());
        guild.message("1004", "1001", "31", "answers in the thread", 5, with_reply("1003"));
        // A forum post: its first message opens it.
        guild.thread("2001", "200", "@jo **x** <@34>", 11);
        guild.message("2001", "2001", "33", "how do I <@34>?", 6, plain());
        guild.message("2002", "2001", "31", "like this", 7, plain());
        // A private thread grows from nothing.
        guild.thread("3001", "100", "private", 12);
        guild.message("3002", "3001", "32", "secret start", 8, plain());
        guild.message("3003", "3001", "31", "secret reply", 9, plain());
        let world = World::new(guild, &[]).await;

        let run = world
            .run_ordered(Order { threads_first, ..Order::default() })
            .await;
        // Completing again changes nothing, and puts no second name on a root.
        let changes = count(&world.db(), "SELECT COUNT(*) FROM \"MessageChanges\"");
        for source in ["100", "200", "1001", "2001", "3001"] {
            world.writer.complete_channel(&run.import, source).await.unwrap();
        }

        let db = world.db();
        assert_eq!(changes, count(&db, "SELECT COUNT(*) FROM \"MessageChanges\""));
        let reply_of = |discord_id: &str| -> Option<String> {
            db.query_row(
                "SELECT \"ReplyToMessageId\" FROM \"Messages\" WHERE \"Id\" = ?1",
                [world.message(discord_id)],
                |row| row.get(0),
            )
            .unwrap()
        };
        let id = |discord_id: &str| Some(world.message(discord_id).to_db_text());
        assert_eq!(reply_of("1001"), id("1000"));
        assert_eq!(reply_of("1003"), id("1000"));
        assert_eq!(reply_of("1004"), id("1000"));
        assert_eq!(world.content_of("1002"), None);

        assert_eq!(reply_of("2001"), None);
        assert_eq!(reply_of("2002"), id("2001"));
        let post = world.content_of("2001").unwrap();
        assert_eq!(post, "**\\@jo \\*\\*x\\*\\* <\\@34>**\nhow do I \\@jo?");
        assert!(stellarr_discord::extract_normalized_usernames(&post).is_empty());

        assert_eq!(reply_of("3002"), None);
        assert!(world.content_of("3002").unwrap().starts_with("**private**\n"));
        assert_eq!(reply_of("3003"), id("3002"));
        let roots: Vec<String> = db
            .prepare("SELECT \"RootMessageId\" FROM \"DiscordImportChannels\" WHERE \"Kind\" = 1 ORDER BY \"DiscordId\"")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            roots,
            [world.message("1000"), world.message("2001"), world.message("3002")].map(|id| id.to_db_text())
        );

        if threads_first {
            // The text channel's thread had to wait for its channel.
            assert!(run.links.iter().any(|link| link.waiting));
        }
    }
}

#[tokio::test]
async fn a_thread_completed_before_its_channel_waits_for_the_whole_history() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general");
    guild.message("1000", "100", "35", "the question", 1, plain());
    guild.message("1001", "100", "36", "the starter, which answers it", 2, with_reply("1000"));
    guild.thread("1001", "100", "side talk", 11);
    guild.message("1003", "1001", "35", "in the thread", 3, plain());
    let world = World::new(guild, &[]).await;
    let started = world.start().await;
    let (import, snapshot) = (&started.import, &started.snapshot);
    let (history, _) = world.pages_of(&started, "100", Order::default()).await;
    let history = history.concat();
    let (thread, _) = world.pages_of(&started, "1001", Order::default()).await;

    // The newest page holds the starter but not what the starter answers.
    world.writer.write_messages(import, snapshot, "100", &history[..1], None).await.unwrap();
    world.writer.write_messages(import, snapshot, "1001", &thread.concat(), None).await.unwrap();
    let early = world.writer.complete_channel(import, "1001").await.unwrap();
    world.writer.write_messages(import, snapshot, "100", &history[1..], None).await.unwrap();
    let late = world.writer.complete_channel(import, "100").await.unwrap();

    assert!(early.waiting);
    assert_eq!(late.threads_linked, 1);
    for id in ["1001", "1003"] {
        let reply: Option<String> = world
            .db()
            .query_row(
                "SELECT \"ReplyToMessageId\" FROM \"Messages\" WHERE \"Id\" = ?1",
                [world.message(id)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reply, Some(world.message("1000").to_db_text()), "{id}");
    }
}

#[tokio::test]
async fn an_id_already_taken_or_a_text_too_long_is_left_out_and_counted() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general");
    guild.message("1001", "100", "38", "somebody sent a message under my id first", 1, plain());
    guild.message("1002", "100", "38", &"x".repeat(20_001), 2, plain());
    guild.message("1003", "100", "38", "fine", 3, plain());
    let world = World::new(guild, &[]).await;
    let general: String = world
        .db()
        .query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Name\" = 'general'", [], |row| row.get(0))
        .unwrap();

    // A client chooses the id of what it sends.
    let squatted = world
        .ada
        .client
        .send(
            Request::new(Method::POST, &format!("/api/channels/{}/messages", general.to_lowercase()))
                .json(&json!({ "content": "mine", "clientMessageId": world.message("1001").to_string() })),
        )
        .await;
    assert!(squatted.status.is_success(), "{}", squatted.text());

    let run = world.run().await;

    assert_eq!(run.written(), 1);
    assert_eq!(
        run.skipped(),
        BTreeMap::from([(SkipReason::TooLong, 1), (SkipReason::IdTaken, 1)])
    );
    assert_eq!(world.content_of("1001").as_deref(), Some("mine"));
}

#[tokio::test]
async fn a_deleted_channel_stays_deleted_and_the_record_outlives_its_server() {
    let guild = FakeGuild::new("Krew");
    guild
        .category("900", "Talk", 0)
        .channel("100", "general", Ch { parent: Some("900"), ..Ch::default() })
        .text("101", "other");
    guild.message("1001", "100", "39", "kept", 1, plain());
    guild.message("1101", "101", "39", "in the channel that goes", 2, plain());
    let mut world = World::new(guild.clone(), &[]).await;
    // The instance's own server cannot be deleted, so this one is made.
    let made = world
        .ada
        .client
        .post_json("/api/servers", &json!({ "name": "Imported", "textChannelName": "general", "voiceChannelName": "voice" }))
        .await;
    assert_eq!(made.status, 201, "{}", made.text());
    let header = made.json::<Value>()["id"].as_str().unwrap().to_string();
    world.server_id = Id::parse(&header).unwrap();
    let first = world.run().await;
    let other = world.channel("101");
    let removed = world
        .ada
        .client
        .send(Request::new(Method::DELETE, &format!("/api/admin/channels/{other}")).header("X-Stellarr-Server", &header))
        .await;
    assert_eq!(removed.status, 204, "{}", removed.text());
    guild.message("1102", "101", "39", "said on Discord after the channel went here", 3, plain());

    let again = world.run().await;

    assert_eq!((again.structure.channels_made, again.written()), (0, 0));
    assert_eq!(again.skipped(), BTreeMap::from([(SkipReason::ChannelRemoved, 1)]));
    let db = world.db();
    let gone: Option<String> = db
        .query_row("SELECT \"ChannelId\" FROM \"DiscordImportChannels\" WHERE \"DiscordId\" = '101'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(gone, None);

    let deleted = world
        .ada
        .client
        .send(Request::new(Method::DELETE, "/api/admin/server").header("X-Stellarr-Server", &header))
        .await;
    assert_eq!(deleted.status, 204, "{}", deleted.text());
    let servers: Vec<Option<String>> = db
        .prepare("SELECT \"ServerId\" FROM \"DiscordImports\" WHERE \"Id\" IN (?1, ?2)")
        .unwrap()
        .query_map(params![first.import.id, again.import.id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(servers, [None, None]);
    for table in ["DiscordImportChannels", "DiscordImportSections", "DiscordImportMessages"] {
        assert_eq!(count(&db, &format!("SELECT COUNT(*) FROM \"{table}\"")), 0, "{table}");
    }
    // Accounts are not removed with a server: the placeholder and what made it stay for an undo.
    let (import, user): (String, String) = db
        .query_row("SELECT \"ImportId\", \"UserId\" FROM \"DiscordImportAccounts\" WHERE \"DiscordId\" = '39'", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(import, first.import.id.to_db_text());
    let origin: Option<String> = db
        .query_row("SELECT \"Origin\" FROM \"Users\" WHERE \"Id\" = ?1", [user], |row| row.get(0))
        .unwrap();
    assert_eq!(origin.as_deref(), Some("discord"));
}

#[tokio::test]
async fn what_has_no_shape_is_left_out_and_counted_once() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general").channel("110", "Lounge", Ch { kind: 2, ..Ch::default() });
    guild.message("1001", "100", "41", "", 1, Msg { kind: 7, ..plain() });
    guild.message("1002", "100", "41", "", 2, Msg { kind: 7, ..plain() });
    guild.message("1003", "100", "41", "", 3, Msg { kind: 6, ..plain() });
    guild.message("1004", "100", "41", "", 4, Msg { kind: 8, ..plain() });
    guild.message("1005", "100", "41", "", 5, Msg { forwarded: Some("somebody else's words"), ..plain() });
    guild.message("1006", "100", "41", "", 6, Msg { sticker: true, ..plain() });
    guild.message("1007", "100", "41", "", 7, Msg { embed: true, bot: true, ..plain() });
    guild.message("1008", "100", "41", "   ", 8, plain());
    guild.message("1009", "100", "41", "", 9, Msg { attachments: 2, ..plain() });
    guild.message("1010", "100", "41", "look at this", 10, Msg { attachments: 1, ..plain() });
    guild.message("1011", "100", "", "nobody", 11, plain());
    guild.message("1012", "110", "41", "said in voice", 12, plain());
    let world = World::new(guild, &[]).await;

    let first = world.run().await;
    let again = world.run().await;

    let sum = |f: fn(&PageResult) -> usize| first.pages.iter().map(f).sum::<usize>();
    assert_eq!(
        (first.written(), sum(|p| p.attachments_pending), sum(|p| p.awaiting_attachments)),
        (1, 1, 1)
    );
    assert_eq!(
        first.skipped(),
        BTreeMap::from([
            (SkipReason::Notice, 4),
            (SkipReason::Forward, 1),
            (SkipReason::OnlySticker, 1),
            (SkipReason::OnlyEmbeds, 1),
            (SkipReason::Empty, 1),
            (SkipReason::VoiceChannel, 1),
            (SkipReason::NoAuthor, 1),
        ])
    );
    assert_eq!(first.notices(), BTreeMap::from([(6, 1), (7, 2), (8, 1)]));
    assert_eq!((again.written(), again.already_seen()), (0, 12));

    let tally = world.writer.tally(world.server_id).await.unwrap();
    assert_eq!(
        (tally.written, tally.attachments_pending, tally.awaiting_attachments, tally.placeholders),
        (1, 1, 1, 1)
    );
    assert_eq!(tally.skipped, first.skipped());
    assert_eq!(tally.notices_by_type, first.notices());
    let (outcome, message, pending): (i64, Option<String>, bool) = world
        .db()
        .query_row(
            "SELECT \"Outcome\", \"MessageId\", \"AttachmentsPending\" FROM \"DiscordImportMessages\" WHERE \"DiscordId\" = '1009'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((outcome, message, pending), (2, None, true));
}

#[tokio::test]
async fn imported_history_is_not_unread_with_or_without_a_marker() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general");
    for minute in 1..=30 {
        guild.message(&(1000 + minute).to_string(), "100", "51", &format!("old news {minute}"), minute, plain());
    }
    let world = World::new(guild, &["nils", "ilya"]).await;
    let started = world.start().await;
    // nils opened the channel long before anything in it was written; ilya never did.
    let channel = world.channel("100");
    world
        .db()
        .execute(
            "INSERT INTO \"ChannelReads\" (\"ChannelId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)",
            params![channel.to_uppercase(), world.people["nils"].id.to_uppercase(), Timestamp::from_unix_seconds(1_577_836_800).unwrap()],
        )
        .unwrap();

    let run = world.import_all(started, Order::default()).await;

    assert_eq!(run.written(), 30);
    let markers: Vec<Timestamp> = world
        .db()
        .prepare("SELECT \"LastReadAt\" FROM \"ChannelReads\" WHERE \"ChannelId\" = ?1")
        .unwrap()
        .query_map([channel.to_uppercase()], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!markers.is_empty());
    assert!(markers.iter().all(|marker| *marker >= utc(30)), "every marker is at the newest imported message");
    // Whoever has none gets one when they first load the channel, which is later than anything imported.
    for name in ["nils", "ilya"] {
        let history = world.get(&world.people[name], &format!("/api/channels/{channel}/messages?limit=100")).await;
        assert_eq!(history["messages"].as_array().unwrap().len(), 30, "{name}");
    }
}

#[tokio::test]
async fn imported_history_is_found_by_search() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general").member("61", "jo", None, Some("Joanna"));
    guild.message("1001", "100", "61", "the quokka migration is on friday", 1, plain());
    guild.message("1002", "100", "61", "nothing to see", 2, plain());
    let world = World::new(guild, &[]).await;

    world.run().await;

    let channel = world.channel("100");
    let found = world.get(&world.ada, &format!("/api/channels/{channel}/messages/search?query=quokka")).await;
    let ids: Vec<&str> = found.as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [world.message("1001").to_string()]);
    let by_author = world.get(&world.ada, &format!("/api/channels/{channel}/messages/search?query=Joanna")).await;
    assert_eq!(by_author.as_array().unwrap().len(), 2);
}

async fn cursor_of(world: &World, channel: &str) -> String {
    world.get(&world.ada, &format!("/api/channels/{channel}/messages?limit=1")).await["syncCursor"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_client_reloads_the_channel_once_instead_of_replaying_every_imported_message() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general");
    for index in 0..600 {
        guild.message(&(10_000 + index).to_string(), "100", "71", &format!("message {index}"), index, plain());
    }
    let world = World::new(guild, &[]).await;
    let started = world.start().await;
    let channel = world.channel("100");
    let before = cursor_of(&world, &channel).await;

    let began = Instant::now();
    let (pages, pins) = world.pages_of(&started, "100", Order { page_size: 300, ..Order::default() }).await;
    let mut between = None;
    let mut written = 0;
    for (number, page) in pages.iter().enumerate() {
        written += world
            .writer
            .write_messages(&started.import, &started.snapshot, "100", page, Some(&pins))
            .await
            .unwrap()
            .written;
        if number == 0 {
            between = Some(cursor_of(&world, &channel).await);
        }
    }
    world.writer.complete_channel(&started.import, "100").await.unwrap();
    let elapsed = began.elapsed();

    assert_eq!(written, 600);
    for cursor in [before, between.unwrap()] {
        let sync = world.get(&world.ada, &format!("/api/channels/{channel}/messages/sync?after={cursor}")).await;
        assert_eq!(sync["resetRequired"], true);
        let changes = sync["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["messageId"], Id::from_bytes([0; 16]).to_string());
    }
    let after = cursor_of(&world, &channel).await;
    let quiet = world.get(&world.ada, &format!("/api/channels/{channel}/messages/sync?after={after}")).await;
    assert!(quiet["changes"].as_array().unwrap().is_empty());
    assert_eq!(quiet["resetRequired"], false);
    let kinds: Vec<i64> = world
        .db()
        .prepare("SELECT \"Kind\" FROM \"MessageChanges\" WHERE \"ChannelId\" = ?1")
        .unwrap()
        .query_map([channel.to_uppercase()], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(kinds, [2]);

    // 600 messages in pages of 300 are several transactions, each after a pause.
    assert!(elapsed >= Duration::from_millis(600), "the pauses were kept: {elapsed:?}");

    // A live change afterwards is journalled as ever.
    let removed = world
        .ada
        .client
        .send(Request::new(Method::DELETE, &format!("/api/messages/{}", world.message("10000"))))
        .await;
    assert_eq!(removed.status, 204, "{}", removed.text());
    let live = world.get(&world.ada, &format!("/api/channels/{channel}/messages/sync?after={after}")).await;
    let changes = live["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["kind"], "delete");
}

#[tokio::test]
async fn nothing_a_send_does_happens_and_a_deleted_message_stays_deleted() {
    let guild = FakeGuild::new("Krew");
    guild.channel("100", "general", Ch { slow_mode: 600, ..Ch::default() }).member("81", "jo", None, None);
    guild.message("1001", "100", "81", "hey @jo and @everyone, read https://example.com/news", 1, plain());
    guild.message("1002", "100", "81", "<@81> again", 2, plain());
    let world = World::new(guild, &["joey"]).await;
    let tables = [
        "UserNotifications",
        "NotificationDeliveries",
        "ChannelPostTimes",
        "ServerAuditEntries",
        "MessageEmbeds",
    ];
    let counted = |db: &Connection| -> Vec<i64> {
        tables.iter().map(|t| count(db, &format!("SELECT COUNT(*) FROM \"{t}\""))).collect()
    };
    let before = counted(&world.db());

    let run = world.run().await;

    assert_eq!(run.written(), 2);
    assert_eq!(before, counted(&world.db()));
    assert_eq!(
        world.content_of("1001").as_deref(),
        Some(r"hey \@jo and \@everyone, read https://example.com/news")
    );

    let removed = world
        .ada
        .client
        .send(Request::new(Method::DELETE, &format!("/api/messages/{}", world.message("1002"))))
        .await;
    assert_eq!(removed.status, 204, "{}", removed.text());

    let again = world.run().await;

    assert_eq!((again.written(), again.already_seen()), (0, 2));
    assert_eq!(world.content_of("1002"), None);
    let linked: Option<String> = world
        .db()
        .query_row(
            "SELECT \"MessageId\" FROM \"DiscordImportMessages\" WHERE \"DiscordId\" = '1002'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(linked, None);
}

#[tokio::test]
async fn a_template_cannot_be_imported_and_a_page_must_be_of_its_channel() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general").text("101", "other");
    guild.message("1001", "101", "91", "elsewhere", 1, plain());
    let world = World::new(guild, &[]).await;
    let started = world.start().await;
    let (page, _) = world.pages_of(&started, "101", Order::default()).await;
    let page = page.concat();
    let mut template = started.snapshot.clone();
    template.source = stellarr_discord::SnapshotSource::Template;

    let refused = world.writer.start(world.server_id, world.ada_id(), &template).await;
    assert!(matches!(refused, Err(ImportError::Argument(_))), "{refused:?}");
    let wrong = world
        .writer
        .write_messages(&started.import, &started.snapshot, "100", &page, None)
        .await;
    assert!(matches!(wrong, Err(ImportError::Argument(_))), "{wrong:?}");
    let unknown = world
        .writer
        .write_messages(&started.import, &started.snapshot, "999", &[], None)
        .await;
    assert!(matches!(unknown, Err(ImportError::Argument(_))), "{unknown:?}");
}

#[tokio::test]
async fn a_database_opened_again_keeps_its_tables_and_its_accounts_are_people() {
    let world = World::new(FakeGuild::new("Krew"), &["nils"]).await;
    let shape = |db: &Connection| -> Vec<String> {
        let mut shape = Vec::new();
        for table in [
            "DiscordImports",
            "DiscordImportAccounts",
            "DiscordImportSections",
            "DiscordImportChannels",
            "DiscordImportMessages",
            "Users",
        ] {
            let mut statement = db.prepare(&format!("PRAGMA table_info(\"{table}\")")).unwrap();
            shape.extend(
                statement
                    .query_map([], |row| Ok(format!("{table}.{}:{}", row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
                    .unwrap()
                    .map(Result::unwrap),
            );
        }
        shape
    };
    let held = shape(&world.db());
    assert!(held.iter().any(|column| column == "Users.Origin:TEXT"), "{held:?}");

    // Opening it a second and a third time changes nothing.
    for _ in 0..2 {
        Database::open(&DatabaseSettings {
            path: world.server.database_path(),
            busy_timeout: Duration::from_secs(10),
        })
        .unwrap();
    }

    assert_eq!(held, shape(&world.db()));
    assert_eq!(count(&world.db(), "SELECT COUNT(*) FROM \"Users\" WHERE \"Origin\" IS NOT NULL"), 0);
    assert_eq!(count(&world.db(), "SELECT COUNT(*) FROM \"Users\""), 2);
}

/// Every file in a folder, by path below it, without the database's own and
/// the test log's.
fn files_below(folder: &std::path::Path) -> Vec<String> {
    fn walk(base: &std::path::Path, at: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(at).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, found);
            } else {
                found.push(path.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut found = Vec::new();
    walk(folder, folder, &mut found);
    found.retain(|name| !name.starts_with("stellarr.db") && name != "events.jsonl");
    found.sort();
    found
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

#[tokio::test]
async fn hostile_snapshots_are_written_without_harm() {
    let guild = FakeGuild::new("Krew");
    guild.text("100", "general");
    guild.message("1001", "100", "11", "seed", 1, plain());
    let world = World::new(guild, &[]).await;
    let started = world.start().await;
    let (pages, _) = world.pages_of(&started, "100", Order::default()).await;
    let template = pages[0][0].clone();
    let make = |id: &str, content: &str| {
        let mut message = template.clone();
        message.id = id.to_string();
        message.content = content.to_string();
        message
    };
    let before = files_below(world.server.folder());

    // The same message twice in a page, and again in the next page.
    let repeated = make("2001", "said once");
    // Control characters and an override of the writing direction stay in the
    // text as the text, and stay out of nothing else.
    let controls = make("2002", "nul\u{0} esc\u{1b} bidi \u{202e}txt \u{feff}end");
    // An author with a name of five thousand characters, controls among them.
    let mut loud = make("2003", "loud");
    loud.author.id = "9001".into();
    loud.author.username = format!("a\u{0}b{}", "n".repeat(5000));
    loud.author.display_name = Some("   ".into());
    loud.author.avatar_url = Some(format!("https://cdn.discordapp.com/{}", "x".repeat(300)));
    // A name that would be cut in the middle of a pair.
    let mut pair = make("2004", "pair");
    pair.author.id = "9002".into();
    pair.author.display_name = Some(format!("{}😀tail", "a".repeat(63)));
    // An id that is not a number, and an author's id that is a path.
    let mut odd = make("not-a-number", "odd");
    odd.author.id = "../../etc/passwd".into();
    // Attachments with names that are paths and sizes that are lies: the
    // writer writes none of them and touches no file.
    let mut attached = make("2005", "with files");
    attached.attachments = vec![stellarr_discord::AttachmentSnapshot {
        id: "../../x".into(),
        file_name: "../../etc/passwd".into(),
        size: i64::MAX,
        content_type: Some("text/html\r\nX-Evil: 1".into()),
        url: "https://cdn.discordapp.com/attachments/1/2/../../../x".into(),
        width: Some(-1),
        height: Some(i32::MAX),
        duration_seconds: Some(f64::NAN),
    }];
    // Exactly as long as a message may be, and one unit more.
    let longest = make("2006", &"😀".repeat(10_000));
    let too_long = make("2007", &format!("{}a", "😀".repeat(10_000)));
    let mut cut = longest.clone();
    cut.id = "2008".into();
    cut.content = format!("a{}", "😀".repeat(9_999));

    let page = vec![
        repeated.clone(),
        repeated.clone(),
        controls,
        loud,
        pair,
        odd,
        attached,
        longest,
        too_long,
        cut,
    ];
    let first = world
        .writer
        .write_messages(&started.import, &started.snapshot, "100", &page, None)
        .await
        .unwrap();
    let again = world
        .writer
        .write_messages(&started.import, &started.snapshot, "100", &[repeated], None)
        .await
        .unwrap();

    assert_eq!((first.written, first.already_seen), (8, 1));
    assert_eq!(first.skipped, BTreeMap::from([(SkipReason::TooLong, 1)]));
    assert_eq!((again.written, again.already_seen), (0, 1));
    assert_eq!(
        world.content_of("2002").as_deref(),
        Some("nul\u{0} esc\u{1b} bidi \u{202e}txt \u{feff}end")
    );
    assert_eq!(files_below(world.server.folder()), before, "no file was written");

    let db = world.db();
    let names: Vec<(String, String)> = db
        .prepare("SELECT \"Username\", \"DisplayName\" FROM \"Users\" WHERE \"Origin\" = 'discord'")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(names.len(), 4, "{names:?}");
    for (username, shown) in &names {
        assert!(units(shown) <= 64, "{username}");
        assert!(!shown.is_empty());
    }
    let loud_name = names.iter().find(|n| n.0 == "discord:9001").unwrap();
    // A blank display name falls back to the username, cut to what fits.
    assert_eq!(units(&loud_name.1), 64);
    assert!(loud_name.1.starts_with("a\u{0}bn"));
    let pair_name = names.iter().find(|n| n.0 == "discord:9002").unwrap();
    assert_eq!(pair_name.1, "a".repeat(63), "never cut inside a pair");
    let recorded: i64 = count(
        &db,
        "SELECT COUNT(*) FROM \"DiscordImportAccounts\" WHERE LENGTH(\"DiscordUsername\") > 100 OR LENGTH(\"DiscordAvatarUrl\") > 256",
    );
    assert_eq!(recorded, 0);
    let pending: bool = db
        .query_row(
            "SELECT \"AttachmentsPending\" FROM \"DiscordImportMessages\" WHERE \"DiscordId\" = '2005'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(pending);
    // What was written is what the tally counts.
    let tally = world.writer.tally(world.server_id).await.unwrap();
    assert_eq!((tally.written, tally.attachments_pending), (8, 1));
}
