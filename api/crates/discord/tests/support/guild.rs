//! A Discord server as data, answering the bot's reads as Discord does: its
//! channels, people and threads, and each channel's history, newest first,
//! paged by `before`. The reader reads it for real, so what a writer is
//! tested against is what the reader really produces.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{Answer, Recorded};

pub const ID: &str = "41771983423143936";

#[derive(Default)]
struct Data {
    channels: Vec<Value>,
    roles: Vec<Value>,
    members: Vec<Value>,
    threads: Vec<Value>,
    messages: HashMap<String, Vec<Value>>,
    pins: HashMap<String, Vec<(String, Value)>>,
}

/// A fake server; clones share the same data, so a test can change it while
/// the server answers from it.
#[derive(Clone)]
pub struct FakeGuild {
    name: String,
    data: Arc<Mutex<Data>>,
}

/// What can be said about a channel.
#[derive(Default)]
pub struct Ch<'a> {
    /// Discord's type: 0 text, 2 voice, 5 announcements, 13 stage, 15 forum, 16 media.
    pub kind: i64,
    pub parent: Option<&'a str>,
    pub position: i64,
    pub topic: Option<&'a str>,
    pub slow_mode: i64,
    pub user_limit: i64,
    pub hidden: bool,
}

/// What can be said about a message.
#[derive(Default)]
pub struct Msg<'a> {
    pub username: Option<&'a str>,
    pub global_name: Option<&'a str>,
    pub kind: i64,
    pub reply_to: Option<&'a str>,
    pub pinned: bool,
    pub edited_minute: Option<i64>,
    pub bot: bool,
    pub webhook_id: Option<&'a str>,
    pub attachments: usize,
    pub sticker: bool,
    pub embed: bool,
    pub mention: Option<(&'a str, &'a str)>,
    pub embed_link: Option<&'a str>,
    pub forwarded: Option<&'a str>,
}

impl FakeGuild {
    pub fn new(name: &str) -> FakeGuild {
        let mut data = Data::default();
        // @everyone sees every channel unless an overwrite says otherwise.
        data.roles
            .push(json!({"id": ID, "name": "@everyone", "position": 0, "permissions": "1024"}));
        FakeGuild {
            name: name.to_owned(),
            data: Arc::new(Mutex::new(data)),
        }
    }

    pub fn role(&self, id: &str, name: &str) -> &Self {
        let mut data = self.data.lock().unwrap();
        let position = data.roles.len();
        data.roles
            .push(json!({"id": id, "name": name, "position": position, "permissions": "0"}));
        self
    }

    pub fn category(&self, id: &str, name: &str, position: i64) -> &Self {
        self.data
            .lock()
            .unwrap()
            .channels
            .push(json!({"id": id, "type": 4, "name": name, "position": position}));
        self
    }

    pub fn channel(&self, id: &str, name: &str, options: Ch<'_>) -> &Self {
        let mut channel = json!({
            "id": id,
            "type": options.kind,
            "name": name,
            "position": options.position,
            "parent_id": options.parent,
            "topic": options.topic,
            "rate_limit_per_user": options.slow_mode,
            "user_limit": options.user_limit,
        });
        if options.hidden {
            channel["permission_overwrites"] =
                json!([{"id": ID, "type": 0, "allow": "0", "deny": "1024"}]);
        }
        let mut data = self.data.lock().unwrap();
        data.channels.push(channel);
        data.messages.entry(id.to_owned()).or_default();
        self
    }

    pub fn text(&self, id: &str, name: &str) -> &Self {
        self.channel(id, name, Ch::default())
    }

    pub fn member(
        &self,
        id: &str,
        username: &str,
        global_name: Option<&str>,
        nick: Option<&str>,
    ) -> &Self {
        self.data.lock().unwrap().members.push(json!({
            "user": {"id": id, "username": username, "global_name": global_name},
            "nick": nick,
            "roles": [],
            "joined_at": "2023-01-01T00:00:00+00:00",
        }));
        self
    }

    /// A thread: 11 public, 12 private. A public thread has the id of the
    /// message it grew from.
    pub fn thread(&self, id: &str, parent: &str, name: &str, kind: i64) -> &Self {
        let mut data = self.data.lock().unwrap();
        data.threads.push(json!({
            "id": id,
            "type": kind,
            "parent_id": parent,
            "name": name,
            "thread_metadata": {"archived": false, "locked": false},
        }));
        data.messages.entry(id.to_owned()).or_default();
        self
    }

    /// Adds a message to a channel's history; `minute` is when, in minutes
    /// after 2024-01-01.
    pub fn message(
        &self,
        id: &str,
        channel: &str,
        author: &str,
        content: &str,
        minute: i64,
        options: Msg<'_>,
    ) -> &Self {
        let mut message = json!({
            "id": id,
            "channel_id": channel,
            "type": if options.reply_to.is_some() { 19 } else { options.kind },
            "content": content,
            "mentions": options.mention.map(|(id, name)| vec![json!({
                "id": id, "username": name, "global_name": "Ula"
            })]).unwrap_or_default(),
            "timestamp": at(minute),
            "edited_timestamp": options.edited_minute.map(at),
            "pinned": options.pinned,
            "webhook_id": options.webhook_id,
            "author": {
                "id": options.webhook_id.unwrap_or(author),
                "username": options.username.unwrap_or("jo"),
                "global_name": options.global_name,
                "bot": options.bot || options.webhook_id.is_some(),
            },
            "attachments": (0..options.attachments).map(|index| json!({
                "id": format!("{id}{index}"),
                "filename": format!("photo{index}.png"),
                "size": 1024,
                "url": format!("https://cdn.discordapp.com/attachments/{channel}/{id}{index}/photo{index}.png"),
                "content_type": "image/png",
            })).collect::<Vec<_>>(),
            "embeds": if let Some(url) = options.embed_link {
                json!([{"type": "gifv", "url": url, "title": "A wave",
                    "video": {"url": "https://static.klipy.com/ii/wave.mp4"},
                    "thumbnail": {
                    "url": "https://example.test/wave.gif",
                    "proxy_url": "https://images-ext-1.discordapp.net/external/wave.gif"
                }}])
            } else if options.embed {
                json!([{"type": "rich", "title": "Status", "description": "All good"}])
            } else {
                json!([])
            },
            "sticker_items": if options.sticker {
                json!([{"id": "749054660769218631", "name": "Wave", "format_type": 1}])
            } else {
                json!([])
            },
        });
        if let Some(reply) = options.reply_to {
            message["message_reference"] =
                json!({"type": 0, "message_id": reply, "channel_id": channel});
        } else if let Some(forwarded) = options.forwarded {
            message["message_reference"] =
                json!({"type": 1, "message_id": "1", "channel_id": channel});
            message["message_snapshots"] =
                json!([{"message": {"content": forwarded, "attachments": []}}]);
        }
        let mut data = self.data.lock().unwrap();
        data.messages
            .entry(channel.to_owned())
            .or_default()
            .push(message.clone());
        if options.pinned {
            data.pins
                .entry(channel.to_owned())
                .or_default()
                .push((at(minute), message));
        }
        self
    }

    /// Removes a message, as somebody deleting it on Discord would.
    pub fn delete(&self, channel: &str, id: &str) {
        if let Some(list) = self.data.lock().unwrap().messages.get_mut(channel) {
            list.retain(|message| message["id"] != id);
        }
    }

    pub fn handler(&self) -> impl Fn(&Recorded, usize) -> Answer + Send + Sync + 'static {
        let guild = self.clone();
        move |request, _| guild.respond(request)
    }

    fn respond(&self, request: &Recorded) -> Answer {
        let path = request.path().replace("/api/v10/", "");
        let parts: Vec<&str> = path.split('/').collect();
        let data = self.data.lock().unwrap();
        let guild = format!("guilds/{ID}");
        if path == guild {
            return json_answer(json!({"id": ID, "name": self.name}));
        }
        if path == format!("{guild}/channels") {
            return json_answer(Value::Array(data.channels.clone()));
        }
        if path == format!("{guild}/roles") {
            return json_answer(Value::Array(data.roles.clone()));
        }
        if path == format!("{guild}/emojis") || path == format!("{guild}/stickers") {
            return json_answer(json!([]));
        }
        if path == format!("{guild}/members") {
            let after = request.query("after");
            let members: Vec<Value> = data
                .members
                .iter()
                .filter(|member| {
                    after.as_deref().is_none_or(|after| {
                        compare(member["user"]["id"].as_str().unwrap(), after) == Ordering::Greater
                    })
                })
                .cloned()
                .collect();
            return json_answer(Value::Array(members));
        }
        if path == format!("{guild}/threads/active") {
            return json_answer(json!({"threads": data.threads}));
        }
        match parts.as_slice() {
            ["channels", _, "threads", "archived", _] => {
                json_answer(json!({"has_more": false, "threads": []}))
            }
            ["channels", channel, "messages", "pins"] => {
                let mut pins = data.pins.get(*channel).cloned().unwrap_or_default();
                pins.sort_by(|a, b| b.0.cmp(&a.0));
                json_answer(json!({
                    "has_more": false,
                    "items": pins.iter().map(|(pinned_at, message)| json!({"pinned_at": pinned_at, "message": message})).collect::<Vec<_>>(),
                }))
            }
            ["channels", channel, "messages"] => {
                let limit: usize = request.query("limit").unwrap().parse().unwrap();
                let before = request.query("before");
                let mut messages: Vec<Value> = data
                    .messages
                    .get(*channel)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|message| {
                        before.as_deref().is_none_or(|before| {
                            compare(message["id"].as_str().unwrap(), before) == Ordering::Less
                        })
                    })
                    .collect();
                messages
                    .sort_by(|a, b| compare(b["id"].as_str().unwrap(), a["id"].as_str().unwrap()));
                messages.truncate(limit);
                json_answer(Value::Array(messages))
            }
            _ => Answer::error(404, 0, &format!("Not expected: {path}")),
        }
    }
}

fn json_answer(body: Value) -> Answer {
    Answer::json(body.to_string())
}

fn compare(left: &str, right: &str) -> Ordering {
    left.parse::<u64>()
        .unwrap()
        .cmp(&right.parse::<u64>().unwrap())
}

/// `2024-01-01T00:00:00+00:00` and `minute` minutes on.
pub fn at(minute: i64) -> String {
    let total = minute * 60;
    let days = total.div_euclid(86_400);
    let of_day = total.rem_euclid(86_400);
    // January 2024 is long enough for every test; further on, count months by hand.
    let (month, day) = month_day(days);
    format!(
        "2024-{month:02}-{day:02}T{:02}:{:02}:00+00:00",
        of_day / 3600,
        of_day % 3600 / 60
    )
}

fn month_day(mut days: i64) -> (i64, i64) {
    for (month, length) in [
        (1, 31),
        (2, 29),
        (3, 31),
        (4, 30),
        (5, 31),
        (6, 30),
        (7, 31),
        (8, 31),
        (9, 30),
        (10, 31),
        (11, 30),
        (12, 31),
    ] {
        if days < length {
            return (month, days + 1);
        }
        days -= length;
    }
    (12, 31)
}
