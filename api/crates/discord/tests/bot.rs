//! The bot's reads: which route each asks, how its pages are walked, and how
//! a whole server comes together.

mod support;

use serde_json::json;
use stellarr_discord::{
    ChannelKind, DiscordChannelType, FailureKind, OverwriteTarget, PermissionOverwriteSnapshot,
    ReactionEmoji, ServerSummary, SnapshotGap, SnapshotSource,
};
use support::{Answer, Rig};

const GUILD: &str = "41771983423143936";
const CHANNEL: &str = "290926798999357250";

fn message(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "channel_id": CHANNEL,
        "type": 0,
        "content": format!("message {id}"),
        "timestamp": "2024-01-01T00:00:00+00:00",
        "author": {"id": "1", "username": "jo"},
    })
}

fn member(id: &str) -> serde_json::Value {
    json!({"user": {"id": id, "username": format!("user{id}")}, "roles": [], "joined_at": "2024-01-01T00:00:00+00:00"})
}

fn thread(id: &str, archived_at: &str, parent: &str) -> serde_json::Value {
    json!({
        "id": id, "type": 11, "parent_id": parent, "name": format!("thread {id}"),
        "thread_metadata": {"archived": true, "archive_timestamp": archived_at},
    })
}

fn array(items: impl IntoIterator<Item = serde_json::Value>) -> String {
    serde_json::Value::Array(items.into_iter().collect()).to_string()
}

#[tokio::test]
async fn messages_are_read_newest_first_a_page_of_a_hundred_at_a_time() {
    // 130 messages with ids 1001..1130; Discord answers newest first.
    let rig = Rig::new(|request, _| {
        let before: i64 = request
            .query("before")
            .map_or(1131, |text| text.parse().unwrap());
        let limit: i64 = request.query("limit").unwrap().parse().unwrap();
        let ids = (0..limit)
            .map(|step| before - 1 - step)
            .filter(|&id| id >= 1001);
        Answer::json(array(ids.map(|id| message(&id.to_string()))))
    })
    .await;

    let messages = rig
        .bot()
        .messages(CHANNEL, None)
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(messages.len(), 130);
    assert_eq!(messages[0].id, "1130");
    assert_eq!(messages[129].id, "1001");
    assert_eq!(
        rig.server.targets(),
        [
            format!("/api/v10/channels/{CHANNEL}/messages?limit=100"),
            format!("/api/v10/channels/{CHANNEL}/messages?limit=100&before=1031"),
        ]
    );
}

#[tokio::test]
async fn a_message_read_goes_a_page_at_a_time_and_can_be_picked_up_again() {
    let rig = Rig::new(|request, _| {
        let before: i64 = request
            .query("before")
            .map_or(1131, |text| text.parse().unwrap());
        let limit: i64 = request.query("limit").unwrap().parse().unwrap();
        let ids = (0..limit)
            .map(|step| before - 1 - step)
            .filter(|&id| id >= 1001);
        Answer::json(array(ids.map(|id| message(&id.to_string()))))
    })
    .await;
    let bot = rig.bot();

    let mut pages = bot.messages(CHANNEL, None).unwrap();
    assert_eq!(pages.resume_before(), None);
    let first = pages.next_page().await.unwrap().unwrap();
    assert_eq!(first.len(), 100);
    // The page is in hand: nothing more was asked for yet.
    assert_eq!(rig.server.hits(), 1);
    let resume = pages.resume_before().unwrap().to_owned();
    assert_eq!(resume, "1031");
    drop(pages);

    // Later, from where it stopped: the other thirty, and then the end.
    let mut again = bot.messages(CHANNEL, Some(&resume)).unwrap();
    let rest = again.next_page().await.unwrap().unwrap();
    assert_eq!(
        (rest.len(), rest[0].id.as_str(), rest[29].id.as_str()),
        (30, "1030", "1001")
    );
    // A short page is the last one: the read knows it without asking again.
    assert!(again.is_finished());
    assert_eq!(again.next_page().await.unwrap(), None);
    assert_eq!(again.next_page().await.unwrap(), None);
    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn a_message_read_can_go_on_from_where_it_stopped() {
    let rig = Rig::new(|_, _| Answer::json(array([message("5"), message("4")]))).await;

    let messages = rig
        .bot()
        .messages(CHANNEL, Some("6"))
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["5", "4"]
    );
    assert_eq!(
        rig.server.recorded()[0].query("before").as_deref(),
        Some("6")
    );
}

#[tokio::test]
async fn members_are_paged_by_the_highest_id_as_a_number() {
    // Ids 1..1000 then 1001: as text, "999" would sort after "1000".
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::json(array((1..=1000).map(|id| member(&id.to_string()))))
        } else {
            Answer::json(array([member("1001")]))
        }
    })
    .await;

    let members = rig
        .bot()
        .members(GUILD)
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(members.len(), 1001);
    assert_eq!(
        rig.server.targets(),
        [
            format!("/api/v10/guilds/{GUILD}/members?limit=1000"),
            format!("/api/v10/guilds/{GUILD}/members?limit=1000&after=1000"),
        ]
    );
}

#[tokio::test]
async fn a_refused_member_list_is_a_missing_intent_only_when_the_app_says_so() {
    // The app's flags decide, not the code: the docs name no code for a missing intent.
    let cases: [(i32, Option<&str>, FailureKind); 5] = [
        (50001, Some("0"), FailureKind::MissingIntent),
        (50013, Some("0"), FailureKind::MissingIntent),
        (50001, Some("32768"), FailureKind::MissingAccess),
        (50001, Some("16384"), FailureKind::MissingAccess),
        (50001, None, FailureKind::MissingAccess),
    ];
    for (code, flags, expected) in cases {
        let rig = Rig::new(move |request, _| match (request.path(), flags) {
            ("/api/v10/applications/@me", Some(flags)) => {
                Answer::json(format!(r#"{{"id":"1","name":"Importer","flags":{flags}}}"#))
            }
            ("/api/v10/applications/@me", None) => Answer::error(403, 50001, "Missing Access"),
            _ => Answer::error(403, code, "Missing Access"),
        })
        .await;

        let failure = rig
            .bot()
            .members(GUILD)
            .unwrap()
            .next_page()
            .await
            .unwrap_err();

        assert_eq!(failure.kind(), expected, "{code} {flags:?}");
        assert_eq!(failure.discord_code(), Some(code));
        assert_eq!(failure.status(), Some(403));
        assert_eq!(failure.route(), format!("GET guilds/{GUILD}/members"));
        let paths: Vec<_> = rig
            .server
            .recorded()
            .into_iter()
            .map(|r| r.path().to_owned())
            .collect();
        assert_eq!(
            paths,
            [
                format!("/api/v10/guilds/{GUILD}/members"),
                "/api/v10/applications/@me".to_owned()
            ]
        );
    }
}

#[tokio::test]
async fn reaction_users_are_paged_and_the_emoji_escaped() {
    let rig = Rig::new(|_, index| match index {
        0 => Answer::json(array(
            (1..=100).map(|id| json!({"id": id.to_string(), "username": format!("user{id}")})),
        )),
        1 => Answer::json(array([
            json!({"id": "101", "username": "user101"}),
            json!({"id": "102", "username": "user102"}),
        ])),
        _ => Answer::json("[]"),
    })
    .await;
    let bot = rig.bot();

    let fire = bot
        .reaction_users(
            CHANNEL,
            "7",
            &ReactionEmoji {
                id: None,
                name: "🔥".into(),
                animated: false,
            },
            false,
        )
        .unwrap()
        .collect_all()
        .await
        .unwrap();
    let custom = bot
        .reaction_users(
            CHANNEL,
            "7",
            &ReactionEmoji {
                id: Some("41771983429993937".into()),
                name: "LUL".into(),
                animated: true,
            },
            true,
        )
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(fire.len(), 102);
    assert!(custom.is_empty());
    assert_eq!(
        rig.server.targets(),
        [
            format!(
                "/api/v10/channels/{CHANNEL}/messages/7/reactions/%F0%9F%94%A5?type=0&limit=100"
            ),
            format!(
                "/api/v10/channels/{CHANNEL}/messages/7/reactions/%F0%9F%94%A5?type=0&limit=100&after=100"
            ),
            format!(
                "/api/v10/channels/{CHANNEL}/messages/7/reactions/LUL%3A41771983429993937?type=1&limit=100"
            ),
        ]
    );
}

#[tokio::test]
async fn a_deleted_custom_emoji_without_a_name_is_asked_for_by_its_id() {
    let rig = Rig::new(|_, _| Answer::json("[]")).await;
    rig.bot()
        .reaction_users(
            "1",
            "2",
            &ReactionEmoji {
                id: Some("3".into()),
                name: String::new(),
                animated: false,
            },
            false,
        )
        .unwrap()
        .collect_all()
        .await
        .unwrap();
    assert_eq!(
        rig.server.targets(),
        ["/api/v10/channels/1/messages/2/reactions/_%3A3?type=0&limit=100"]
    );
}

#[tokio::test]
async fn archived_threads_are_paged_by_their_archive_time() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::json(
                json!({"has_more": true, "threads": [
                    thread("21", "2024-05-02T10:00:00.000000+00:00", CHANNEL),
                    thread("20", "2024-05-01T10:00:00.000000+00:00", CHANNEL)]})
                .to_string(),
            )
        } else {
            Answer::json(json!({"has_more": false, "threads": [thread("19", "2024-04-01T10:00:00.000000+00:00", CHANNEL)]}).to_string())
        }
    })
    .await;

    let threads = rig
        .bot()
        .archived_threads(CHANNEL, false)
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(
        threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["21", "20", "19"]
    );
    assert!(threads.iter().all(|thread| thread.archived));
    let targets = rig.server.targets();
    assert_eq!(
        targets[0],
        format!("/api/v10/channels/{CHANNEL}/threads/archived/public")
    );
    // The offset's + must arrive as a plus, not as a space.
    assert_eq!(
        targets[1],
        format!(
            "/api/v10/channels/{CHANNEL}/threads/archived/public?before=2024-05-01T10%3A00%3A00.000000%2B00%3A00"
        )
    );
    assert_eq!(
        rig.server.recorded()[1].query("before").as_deref(),
        Some("2024-05-01T10:00:00.000000+00:00")
    );
}

#[tokio::test]
async fn pins_come_with_when_they_were_pinned() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::json(
                json!({"has_more": true, "items": [
                    {"pinned_at": "2024-02-02T00:00:00+00:00", "message": message("9")},
                    {"pinned_at": "2024-02-01T00:00:00+00:00", "message": message("8")}]})
                .to_string(),
            )
        } else {
            Answer::json(json!({"has_more": false, "items": [{"pinned_at": "2024-01-01T00:00:00+00:00", "message": message("3")}]}).to_string())
        }
    })
    .await;

    let pins = rig
        .bot()
        .pins(CHANNEL)
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(
        pins.iter()
            .map(|pin| pin.message.id.as_str())
            .collect::<Vec<_>>(),
        ["9", "8", "3"]
    );
    assert_eq!(pins[0].pinned_at.unix_seconds(), 1_706_832_000);
    assert_eq!(
        rig.server.targets()[0],
        format!("/api/v10/channels/{CHANNEL}/messages/pins?limit=50")
    );
    assert_eq!(
        rig.server.recorded()[1].query("before").as_deref(),
        Some("2024-02-01T00:00:00+00:00")
    );
}

#[tokio::test]
async fn a_full_page_that_does_not_move_the_cursor_ends_the_read() {
    // A server that answers every page with the same hundred messages.
    let rig =
        Rig::new(|_, _| Answer::json(array((1..=100).map(|id| message(&id.to_string()))))).await;

    let messages = rig
        .bot()
        .messages(CHANNEL, None)
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(messages.len(), 200);
    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn pages_that_loop_or_run_on_for_ever_end_every_kind_of_read() {
    // Members: the same thousand again and again.
    let rig =
        Rig::new(|_, _| Answer::json(array((1..=1000).map(|id| member(&id.to_string()))))).await;
    assert_eq!(
        rig.bot()
            .members(GUILD)
            .unwrap()
            .collect_all()
            .await
            .unwrap()
            .len(),
        2000
    );
    assert_eq!(rig.server.hits(), 2);

    // Archived threads: has_more without a time to go on from, and the same time again.
    let rig = Rig::new(|_, _| {
        Answer::json(
            json!({"has_more": true, "threads": [{"id": "2", "type": 11, "parent_id": "1"}]})
                .to_string(),
        )
    })
    .await;
    assert_eq!(
        rig.bot()
            .archived_threads("1", false)
            .unwrap()
            .collect_all()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(rig.server.hits(), 1);
    let rig = Rig::new(|_, _| {
        Answer::json(
            json!({"has_more": true, "threads": [thread("2", "2024-01-01T00:00:00+00:00", "1")]})
                .to_string(),
        )
    })
    .await;
    assert_eq!(
        rig.bot()
            .archived_threads("1", true)
            .unwrap()
            .collect_all()
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(rig.server.hits(), 2);

    // Pins: the same.
    let rig = Rig::new(|_, _| Answer::json(json!({"has_more": true, "items": [{"pinned_at": "2024-01-01T00:00:00+00:00", "message": message("1")}]}).to_string())).await;
    assert_eq!(
        rig.bot()
            .pins("1")
            .unwrap()
            .collect_all()
            .await
            .unwrap()
            .len(),
        2
    );

    // Servers: a page of ids that does not go up.
    let rig = Rig::new(|_, _| {
        Answer::json(array(
            (1..=200).map(|id| json!({"id": id.to_string(), "name": "s"})),
        ))
    })
    .await;
    assert_eq!(rig.bot().list_servers().await.unwrap().len(), 400);

    // Reaction users.
    let rig =
        Rig::new(|_, _| Answer::json(array((1..=100).map(|id| json!({"id": id.to_string()})))))
            .await;
    let emoji = ReactionEmoji {
        id: None,
        name: "x".into(),
        animated: false,
    };
    assert_eq!(
        rig.bot()
            .reaction_users("1", "2", &emoji, false)
            .unwrap()
            .collect_all()
            .await
            .unwrap()
            .len(),
        200
    );
}

#[tokio::test]
async fn an_id_from_discord_that_is_not_a_number_cannot_add_to_the_query() {
    let rig = Rig::new(|_, index| {
        if index == 0 {
            Answer::json(array((1..=1000).map(|id| {
                if id == 1000 {
                    member("1&limit=5#x/../y")
                } else {
                    member(&id.to_string())
                }
            })))
        } else {
            Answer::json("[]")
        }
    })
    .await;

    // Whatever the text, the second request is for the one next page and nothing else.
    let _ = rig.bot().members(GUILD).unwrap().collect_all().await;

    let second = &rig.server.targets()[1];
    assert!(
        second.starts_with(&format!(
            "/api/v10/guilds/{GUILD}/members?limit=1000&after="
        )),
        "{second}"
    );
    assert!(
        !second.contains('#') && !second.contains("limit=5") && second.matches('&').count() == 1,
        "{second}"
    );
}

#[tokio::test]
async fn the_servers_the_bot_is_in_come_with_their_size() {
    let rig = Rig::new(|_, _| {
        Answer::json(format!(
            r#"[{{ "id": "{GUILD}", "name": "1337 Krew", "icon": "8342729096ea3675442027381ff50dfe", "approximate_member_count": 3268 }}, null, {{"name": "no id"}}]"#
        ))
    })
    .await;

    let servers = rig.bot().list_servers().await.unwrap();

    assert_eq!(
        servers,
        [ServerSummary {
            id: GUILD.into(),
            name: "1337 Krew".into(),
            icon_url: Some(format!(
                "https://cdn.discordapp.com/icons/{GUILD}/8342729096ea3675442027381ff50dfe.png"
            )),
            approximate_member_count: Some(3268),
        }]
    );
    assert_eq!(
        rig.server.targets(),
        ["/api/v10/users/@me/guilds?limit=200&with_counts=true"]
    );
}

#[tokio::test]
async fn an_id_that_is_not_a_snowflake_is_refused_before_any_request() {
    let rig = Rig::new(|_, _| Answer::json("{}")).await;
    let bot = rig.bot();
    let emoji = ReactionEmoji {
        id: None,
        name: "x".into(),
        animated: false,
    };

    for id in [
        "../users/@me",
        "12a",
        "",
        "123456789012345678901",
        "-1",
        "1 2",
        "١٢٣",
        "1/2",
        "1?x",
        "\u{0}",
    ] {
        let refusals = [
            bot.server(id).await.unwrap_err(),
            bot.channels(id).await.unwrap_err(),
            bot.roles(id).await.unwrap_err(),
            bot.emoji(id).await.unwrap_err(),
            bot.stickers(id).await.unwrap_err(),
            bot.active_threads(id).await.unwrap_err(),
            bot.read_server(id).await.unwrap_err(),
            bot.members(id).unwrap_err(),
            bot.archived_threads(id, false).unwrap_err(),
            bot.messages(id, None).unwrap_err(),
            bot.messages(CHANNEL, Some(id)).unwrap_err(),
            bot.pins(id).unwrap_err(),
            bot.reaction_users(id, "1", &emoji, false).unwrap_err(),
            bot.reaction_users("1", id, &emoji, false).unwrap_err(),
            bot.reaction_users(
                "1",
                "1",
                &ReactionEmoji {
                    id: Some(id.into()),
                    name: "x".into(),
                    animated: false,
                },
                false,
            )
            .unwrap_err(),
        ];
        for refusal in refusals {
            assert_eq!(refusal.kind(), FailureKind::InvalidArgument, "{id:?}");
        }
    }
    assert_eq!(rig.server.hits(), 0);
    // Twenty digits is a snowflake, and a very large one.
    assert!(bot.messages("12345678901234567890", None).is_ok());
}

#[tokio::test]
async fn a_whole_server_comes_together_and_says_what_it_could_not_read() {
    let (text, forum, voice) = ("100", "101", "102");
    let rig = Rig::new(move |request, _| {
        let path = request.path().replace("/api/v10/", "");
        let body = match path.as_str() {
            p if p == format!("guilds/{GUILD}") => {
                json!({"id": GUILD, "name": "Krew", "description": "Hi", "icon": "abc", "approximate_member_count": 2}).to_string()
            }
            p if p == format!("guilds/{GUILD}/channels") => json!([
                {"id": "99", "type": 4, "name": "Talk", "position": 0},
                {"id": text, "type": 0, "name": "general", "parent_id": "99", "topic": "Hello", "rate_limit_per_user": 5,
                 "permission_overwrites": [{"id": GUILD, "type": 0, "allow": "0", "deny": "2048"}]},
                {"id": forum, "type": 15, "name": "help", "parent_id": "99"},
                {"id": voice, "type": 2, "name": "Lounge", "user_limit": 5}
            ])
            .to_string(),
            p if p == format!("guilds/{GUILD}/roles") => json!([
                {"id": GUILD, "name": "@everyone", "position": 0, "permissions": "104324673"},
                {"id": "5", "name": "Mods", "position": 1, "permissions": "8", "colors": {"primary_color": 3447003}, "mentionable": true}
            ])
            .to_string(),
            p if p == format!("guilds/{GUILD}/emojis") => r#"[{ "id": "41771983429993937", "name": "LUL", "animated": true }]"#.to_owned(),
            p if p == format!("guilds/{GUILD}/stickers") => r#"[{ "id": "6", "name": "Wave", "format_type": 1 }]"#.to_owned(),
            p if p == format!("guilds/{GUILD}/members") => array([member("7"), member("8")]),
            p if p == format!("guilds/{GUILD}/threads/active") => {
                json!({"threads": [{"id": "30", "type": 11, "parent_id": forum, "name": "Question"}], "members": []}).to_string()
            }
            p if p == format!("channels/{text}/threads/archived/public") => {
                json!({"has_more": false, "threads": [thread("31", "2024-01-01T00:00:00+00:00", text)]}).to_string()
            }
            p if p == format!("channels/{text}/threads/archived/private") => {
                return Answer::error(403, 50013, "Missing Permissions");
            }
            // The active thread shows up again among the archived ones; it is one thread.
            p if p == format!("channels/{forum}/threads/archived/public") => {
                json!({"has_more": false, "threads": [{"id": "30", "type": 11, "parent_id": forum, "name": "Question"}]}).to_string()
            }
            other => return Answer::error(404, 0, &format!("Not expected: {other}")),
        };
        Answer::json(body)
    })
    .await;

    let snapshot = rig.bot().read_server(GUILD).await.unwrap();

    assert_eq!(snapshot.source, SnapshotSource::Bot);
    assert_eq!(
        (
            snapshot.source_id.as_str(),
            snapshot.name.as_str(),
            snapshot.description.as_deref()
        ),
        (GUILD, "Krew", Some("Hi"))
    );
    assert_eq!(
        snapshot.icon_url.as_deref(),
        Some(format!("https://cdn.discordapp.com/icons/{GUILD}/abc.png").as_str())
    );
    assert_eq!(snapshot.sections.len(), 1);
    assert_eq!(snapshot.sections[0].name, "Talk");
    assert_eq!(
        snapshot
            .channels
            .iter()
            .map(|c| (c.id.as_str(), c.kind, c.discord_type))
            .collect::<Vec<_>>(),
        [
            (text, ChannelKind::Text, DiscordChannelType::Conversation),
            (forum, ChannelKind::Text, DiscordChannelType::Forum),
            (voice, ChannelKind::Voice, DiscordChannelType::Conversation),
        ]
    );
    assert_eq!(snapshot.channels[0].slow_mode_seconds, 5);
    assert_eq!(
        snapshot.channels[0].overwrites,
        [PermissionOverwriteSnapshot {
            target: OverwriteTarget::Role,
            target_id: GUILD.into(),
            allow: "0".into(),
            deny: "2048".into()
        }]
    );
    assert_eq!(
        snapshot
            .roles
            .iter()
            .map(|r| r.is_everyone)
            .collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(snapshot.roles[1].color, Some(3447003));
    assert_eq!(snapshot.emoji[0].name, "LUL");
    assert_eq!(snapshot.stickers[0].name, "Wave");
    assert_eq!(
        snapshot
            .members
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["7", "8"]
    );
    let mut threads: Vec<_> = snapshot.threads.iter().map(|t| t.id.as_str()).collect();
    threads.sort_unstable();
    assert_eq!(threads, ["30", "31"]);
    assert_eq!(
        snapshot.gaps,
        [SnapshotGap {
            what: "private archived threads".into(),
            channel_id: Some(text.into()),
            kind: FailureKind::MissingAccess,
            discord_code: Some(50013)
        }]
    );
    // Voice channels have no threads, and a forum has no private ones.
    let targets = rig.server.targets();
    assert!(
        !targets
            .iter()
            .any(|t| t.contains(&format!("channels/{voice}/")))
    );
    assert!(
        !targets
            .iter()
            .any(|t| t.contains(&format!("channels/{forum}/threads/archived/private")))
    );
    // Seven requests for the server, one for each text channel's archives.
    assert_eq!(targets.len(), 7 + 3);
}

#[tokio::test]
async fn a_whole_server_without_the_members_intent_still_reads() {
    let rig = Rig::new(
        |request, _| match request.path().replace("/api/v10/", "").as_str() {
            p if p == format!("guilds/{GUILD}") => {
                Answer::json(format!(r#"{{ "id": "{GUILD}", "name": "Krew" }}"#))
            }
            p if p == format!("guilds/{GUILD}/members") => {
                Answer::error(403, 50001, "Missing Access")
            }
            "applications/@me" => Answer::json(r#"{ "id": "1", "name": "Importer", "flags": 0 }"#),
            p if p == format!("guilds/{GUILD}/threads/active") => {
                Answer::json(r#"{ "threads": [] }"#)
            }
            _ => Answer::json("[]"),
        },
    )
    .await;

    let snapshot = rig.bot().read_server(GUILD).await.unwrap();

    assert!(snapshot.members.is_empty());
    assert_eq!(
        snapshot.gaps,
        [SnapshotGap {
            what: "members".into(),
            channel_id: None,
            kind: FailureKind::MissingIntent,
            discord_code: Some(50001)
        }]
    );
}

#[tokio::test]
async fn a_bad_token_fails_the_whole_read() {
    let rig = Rig::new(|_, _| Answer::error(401, 0, "401: Unauthorized")).await;

    let failure = rig.bot().read_server(GUILD).await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::InvalidToken);
    assert_eq!(rig.server.hits(), 1);
}

#[tokio::test]
async fn a_failure_other_than_a_refusal_fails_the_whole_read_even_in_a_part() {
    let rig = Rig::new(|request, _| {
        if request.path().ends_with("/members") {
            Answer::status(500, "")
        } else {
            Answer::json(if request.path().ends_with(GUILD) {
                format!(r#"{{"id":"{GUILD}"}}"#)
            } else {
                "[]".to_owned()
            })
        }
    })
    .await;

    let failure = rig.bot().read_server(GUILD).await.unwrap_err();

    assert_eq!(failure.kind(), FailureKind::Unreachable);
}
