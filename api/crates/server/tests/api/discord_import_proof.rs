//! The runners of the import's measurement. Each test here is ignored and
//! driven by environment variables, by `tests/discord-import/bulk.sh`; none of
//! them means anything alone.
//!
//! - `proof_base` makes a database a founded instance would have, with ada,
//!   the owner, and copies it to `$DISCORDW_OUT/base.db`.
//! - `proof_bulk` writes `$DISCORDW_MESSAGES` generated messages into one
//!   channel and measures what that costs: how long a call takes, and how long a
//!   second connection waits for the write lock meanwhile.

use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as Atomic};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{Value, json};
use stellarr_api::database::Database;
use stellarr_api::discord_import::ImportWriter;
use stellarr_api::settings::DatabaseSettings;
use stellarr_api::values::Id;
use stellarr_discord::{BotReader, BotToken, DiscordClient, ServerSnapshot};

use crate::discord_import::discord;
use crate::support::Server;
use discord::{Answer, FakeClock, Recorded, config_for};

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn number(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[tokio::test]
#[ignore = "driven by tests/discord-import/"]
async fn proof_base() {
    let out = std::path::PathBuf::from(var("DISCORDW_OUT"));
    let folder = out.join("founding");
    std::fs::create_dir_all(&folder).unwrap();
    let server = Server::start_in(&folder, &[]).await;
    let ada = server.found("ada").await;
    server.register("nils").await;
    let database = server.database_path();
    assert!(server.stop().await, "the server leaves cleanly");
    let db = Connection::open(&database).unwrap();
    let server_id: String = db.query_row("SELECT \"Id\" FROM \"Servers\"", [], |r| r.get(0)).unwrap();
    let _ = std::fs::remove_file(out.join("base.db"));
    db.execute("VACUUM INTO ?1", [out.join("base.db").display().to_string()]).unwrap();
    drop(db);
    std::fs::remove_dir_all(&folder).unwrap();
    std::fs::write(
        out.join("base.json"),
        json!({ "server": server_id.to_lowercase(), "user": ada.id }).to_string(),
    )
    .unwrap();
}

/// Discord's answers, from the generated server.
fn respond(guild: &Value, request: &Recorded) -> Answer {
    let path = request.path().replace("/api/v10/", "");
    let parts: Vec<&str> = path.split('/').collect();
    let prefix = format!("guilds/{}", discord::guild::ID);
    let json = |value: Value| Answer::json(value.to_string());
    let list = |name: &str| guild[name].clone();
    if path == prefix {
        return json(json!({ "id": discord::guild::ID, "name": "Krew" }));
    }
    if path == format!("{prefix}/channels") {
        return json(list("channels"));
    }
    if path == format!("{prefix}/roles") {
        return json(list("roles"));
    }
    if path == format!("{prefix}/emojis") || path == format!("{prefix}/stickers") {
        return json(json!([]));
    }
    let compare = |a: &str, b: &str| a.parse::<u64>().unwrap().cmp(&b.parse::<u64>().unwrap());
    if path == format!("{prefix}/members") {
        let after = request.query("after");
        let members: Vec<Value> = guild["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| after.as_deref().is_none_or(|a| compare(m["user"]["id"].as_str().unwrap(), a) == Ordering::Greater))
            .cloned()
            .collect();
        return json(Value::Array(members));
    }
    if path == format!("{prefix}/threads/active") {
        return json(json!({ "threads": list("threads") }));
    }
    match parts.as_slice() {
        ["channels", _, "threads", "archived", _] => json(json!({ "has_more": false, "threads": [] })),
        ["channels", channel, "messages", "pins"] => {
            let mut pins = guild["pins"][*channel].as_array().cloned().unwrap_or_default();
            pins.sort_by(|a, b| b[0].as_str().cmp(&a[0].as_str()));
            json(json!({
                "has_more": false,
                "items": pins.iter().map(|p| json!({ "pinned_at": p[0], "message": p[1] })).collect::<Vec<_>>(),
            }))
        }
        ["channels", channel, "messages"] => {
            let limit: usize = request.query("limit").unwrap().parse().unwrap();
            let before = request.query("before");
            let mut messages: Vec<Value> = guild["messages"][*channel]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| before.as_deref().is_none_or(|b| compare(m["id"].as_str().unwrap(), b) == Ordering::Less))
                .collect();
            messages.sort_by(|a, b| compare(b["id"].as_str().unwrap(), a["id"].as_str().unwrap()));
            messages.truncate(limit);
            json(Value::Array(messages))
        }
        _ => Answer::error(404, 0, &format!("Not expected: {path}")),
    }
}

async fn reader(guild: Value) -> (discord::Server, BotReader) {
    let server = discord::Server::start(move |request, _| respond(&guild, request)).await;
    let config = config_for(server.addr.port(), FakeClock::new());
    let bot = DiscordClient::with_config(config).for_bot(BotToken::new(discord::TOKEN).unwrap());
    (server, bot)
}

/// Messages made up as they are written, so that what is held is the writer's
/// and not the test's.
fn made_up(template: &stellarr_discord::MessageSnapshot, channel: &str, number: usize) -> stellarr_discord::MessageSnapshot {
    let mut message = template.clone();
    message.id = (1_200_000_000_000_000_000u64 + number as u64 * 1000).to_string();
    message.channel_id = channel.to_string();
    message.content = format!("message {number} about the quokka migration <@{}> and <#{channel}>", 2000 + number % 40);
    message.author.id = (3000 + number % 500).to_string();
    message.author.username = format!("person{}", number % 500);
    message.created_at = stellarr_discord::Timestamp::from_unix_seconds(1_704_067_200 + number as i64).unwrap();
    message.reply_to_message_id = (number.is_multiple_of(5) && number > 5)
        .then(|| (1_200_000_000_000_000_000u64 + (number as u64 - 3) * 1000).to_string());
    message
}

#[tokio::test]
#[ignore = "driven by tests/discord-import/"]
async fn proof_bulk() {
    let guild: Value = serde_json::from_str(&std::fs::read_to_string(var("DISCORDW_GUILD")).unwrap()).unwrap();
    let path = std::path::PathBuf::from(var("DISCORDW_DB"));
    let server = Id::parse(&var("DISCORDW_SERVER")).unwrap();
    let user = Id::parse(&var("DISCORDW_USER")).unwrap();
    let total = number("DISCORDW_MESSAGES", 100_000);
    let page_size = number("DISCORDW_PAGE", 100);
    let (_discord, bot) = reader(guild).await;
    let snapshot: ServerSnapshot = bot.read_server(discord::guild::ID).await.unwrap();
    let template = bot
        .messages("100", None)
        .unwrap()
        .collect_all()
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the generated server has a message in channel 100");
    let settings = DatabaseSettings { path: path.clone(), busy_timeout: Duration::from_secs(60) };
    let database = Database::open(&settings).unwrap();
    let writer = ImportWriter::new(database.clone());
    // `$DISCORDW_PAUSE=0` writes without the pause between transactions, a
    // writer of its own for each call, so that a call is one transaction.
    let pause = number("DISCORDW_PAUSE", 1) != 0;
    let import = writer.start(server, user, &snapshot).await.unwrap();
    writer.write_structure(&import, &snapshot).await.unwrap();

    // A second connection asks for the write lock every few milliseconds, as a
    // person sending a message would, and notes how long it waited.
    let running = Arc::new(AtomicBool::new(true));
    let probe = {
        let (running, path) = (running.clone(), path.clone());
        std::thread::spawn(move || {
            let db = Connection::open(path).unwrap();
            db.busy_timeout(Duration::from_secs(60)).unwrap();
            let mut waits = Vec::new();
            while running.load(Atomic::SeqCst) {
                let began = Instant::now();
                db.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").unwrap();
                waits.push(began.elapsed());
                std::thread::sleep(Duration::from_millis(5));
            }
            waits
        })
    };

    let began = Instant::now();
    let mut calls: Vec<Duration> = Vec::new();
    let mut number = 0;
    while number < total {
        let page: Vec<_> = (number..(number + page_size).min(total))
            .rev()
            .map(|n| made_up(&template, "100", n + 1))
            .collect();
        number += page.len();
        let call = Instant::now();
        if pause {
            writer.write_messages(&import, &snapshot, "100", &page, None).await.unwrap();
        } else {
            let fresh = ImportWriter::new(database.clone());
            fresh.write_messages(&import, &snapshot, "100", &page, None).await.unwrap();
        }
        calls.push(call.elapsed());
    }
    writer.complete_channel(&import, "100").await.unwrap();
    let elapsed = began.elapsed();
    // How the cost grows with what is already there: the mean call of each tenth.
    let tenth = (calls.len() / 10).max(1);
    let growth: Vec<String> = calls
        .chunks(tenth)
        .map(|c| format!("{:.0}", c.iter().sum::<Duration>().as_secs_f64() * 1000.0 / c.len() as f64))
        .collect();
    println!("mean call in ms by tenth: {}", growth.join(" "));
    let mut slowest: Vec<(usize, Duration)> = calls.iter().copied().enumerate().collect();
    slowest.sort_by_key(|(_, call)| std::cmp::Reverse(*call));
    let slowest: Vec<String> =
        slowest.iter().take(5).map(|(at, call)| format!("#{at}:{}ms", call.as_millis())).collect();
    println!("mean slowest calls: {}", slowest.join(" "));
    running.store(false, Atomic::SeqCst);
    let mut waits = probe.join().unwrap();
    waits.sort();
    calls.sort();
    let pick = |sorted: &[Duration], share: f64| sorted[((sorted.len() as f64 * share) as usize).min(sorted.len() - 1)];
    println!(
        "bulk messages={total} seconds={:.1} per_second={:.0} calls={} call_p50_ms={:.1} call_p99_ms={:.1} call_max_ms={:.1} \
         waiters={} wait_p50_ms={:.2} wait_p99_ms={:.1} wait_max_ms={:.1}",
        elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64(),
        calls.len(),
        pick(&calls, 0.5).as_secs_f64() * 1000.0,
        pick(&calls, 0.99).as_secs_f64() * 1000.0,
        calls.last().unwrap().as_secs_f64() * 1000.0,
        waits.len(),
        pick(&waits, 0.5).as_secs_f64() * 1000.0,
        pick(&waits, 0.99).as_secs_f64() * 1000.0,
        waits.last().unwrap().as_secs_f64() * 1000.0,
    );
}
