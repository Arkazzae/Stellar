//! A whole fake server read through the real reader, as the writer is meant
//! to be tested: what it hands over is what the snapshot of a server and the
//! pages of a channel's history look like.

mod support;

use stellarr_discord::{
    ChannelKind, DiscordChannelType, StickerFormat, Timestamp, hidden_from_everyone, is_cdn_url,
};
use support::Rig;
use support::guild::{Ch, FakeGuild, ID, Msg};

fn krew() -> FakeGuild {
    let guild = FakeGuild::new("Krew");
    guild
        .category("900", "Talk", 0)
        .channel(
            "100",
            "chat",
            Ch {
                parent: Some("900"),
                topic: Some("Hello"),
                slow_mode: 10,
                ..Ch::default()
            },
        )
        .channel(
            "101",
            "secret",
            Ch {
                parent: Some("900"),
                position: 1,
                hidden: true,
                ..Ch::default()
            },
        )
        .channel(
            "102",
            "Lounge",
            Ch {
                kind: 2,
                parent: Some("900"),
                user_limit: 4,
                ..Ch::default()
            },
        )
        .channel(
            "103",
            "stage",
            Ch {
                kind: 13,
                user_limit: 5000,
                ..Ch::default()
            },
        )
        .channel(
            "104",
            "help",
            Ch {
                kind: 15,
                ..Ch::default()
            },
        )
        .channel(
            "105",
            "news",
            Ch {
                kind: 5,
                ..Ch::default()
            },
        )
        .channel(
            "106",
            "art",
            Ch {
                kind: 16,
                ..Ch::default()
            },
        )
        .role("500", "Mods")
        .member("1", "jo", Some("Jo"), Some("Joanna"))
        .member("2", "ada", None, None)
        .thread("1002", "100", "a thread", 11)
        .thread("1003", "100", "a secret", 12);
    guild
}

#[tokio::test]
async fn the_structure_the_people_and_the_threads_come_out_as_the_snapshot() {
    let guild = krew();
    let rig = Rig::new(guild.handler()).await;

    let snapshot = rig.bot().read_server(ID).await.unwrap();

    assert_eq!(snapshot.name, "Krew");
    assert_eq!(snapshot.sections.len(), 1);
    assert_eq!(
        snapshot
            .channels
            .iter()
            .map(|c| (
                c.id.as_str(),
                c.kind,
                c.discord_type,
                c.section_id.as_deref(),
                c.user_limit
            ))
            .collect::<Vec<_>>(),
        [
            (
                "100",
                ChannelKind::Text,
                DiscordChannelType::Conversation,
                Some("900"),
                0
            ),
            (
                "101",
                ChannelKind::Text,
                DiscordChannelType::Conversation,
                Some("900"),
                0
            ),
            (
                "102",
                ChannelKind::Voice,
                DiscordChannelType::Conversation,
                Some("900"),
                4
            ),
            (
                "103",
                ChannelKind::Voice,
                DiscordChannelType::Stage,
                None,
                5000
            ),
            ("104", ChannelKind::Text, DiscordChannelType::Forum, None, 0),
            (
                "105",
                ChannelKind::Text,
                DiscordChannelType::Announcements,
                None,
                0
            ),
            ("106", ChannelKind::Text, DiscordChannelType::Media, None, 0),
        ]
    );
    assert_eq!(snapshot.channels[0].topic.as_deref(), Some("Hello"));
    assert_eq!(snapshot.channels[0].slow_mode_seconds, 10);

    // Which channels are hidden from everyone: the one with the overwrite.
    let everyone = snapshot.roles.iter().find(|role| role.is_everyone);
    assert_eq!(everyone.map(|role| role.id.as_str()), Some(ID));
    let hidden: Vec<_> = snapshot
        .channels
        .iter()
        .filter(|channel| hidden_from_everyone(channel, everyone))
        .map(|channel| channel.id.as_str())
        .collect();
    assert_eq!(hidden, ["101"]);

    assert_eq!(snapshot.members.len(), 2);
    assert_eq!(snapshot.members[0].nickname.as_deref(), Some("Joanna"));
    assert_eq!(
        snapshot.members[0].joined_at.map(Timestamp::unix_seconds),
        Some(1_672_531_200)
    );
    assert_eq!(snapshot.roles.len(), 2);
    assert_eq!(
        snapshot
            .threads
            .iter()
            .map(|t| (t.id.as_str(), t.private, t.starter_message_id.is_some()))
            .collect::<Vec<_>>(),
        [("1002", false, true), ("1003", true, false)]
    );
    assert!(snapshot.gaps.is_empty());
}

#[tokio::test]
async fn a_channels_history_comes_in_pages_with_everything_a_message_carries() {
    let guild = krew();
    // Two hundred and fifty plain messages, then a few of every kind.
    for number in 0..250 {
        guild.message(
            &(1_000 + number).to_string(),
            "100",
            "1",
            &format!("plain {number}"),
            number,
            Msg::default(),
        );
    }
    guild
        .message(
            "2000",
            "100",
            "1",
            "a reply",
            300,
            Msg {
                reply_to: Some("1001"),
                ..Msg::default()
            },
        )
        .message(
            "2001",
            "100",
            "2",
            "edited and pinned",
            301,
            Msg {
                edited_minute: Some(305),
                pinned: true,
                ..Msg::default()
            },
        )
        .message(
            "2002",
            "100",
            "9",
            "from a hook",
            302,
            Msg {
                webhook_id: Some("777"),
                username: Some("Weather"),
                ..Msg::default()
            },
        )
        .message(
            "2003",
            "100",
            "1",
            "with files",
            303,
            Msg {
                attachments: 2,
                sticker: true,
                embed: true,
                ..Msg::default()
            },
        )
        .message(
            "2004",
            "100",
            "1",
            "",
            304,
            Msg {
                forwarded: Some("look at this"),
                ..Msg::default()
            },
        )
        .message(
            "2005",
            "100",
            "1",
            "joined",
            305,
            Msg {
                kind: 7,
                ..Msg::default()
            },
        );
    let rig = Rig::new(guild.handler()).await;
    let bot = rig.bot();

    let mut pages = bot.messages("100", None).unwrap();
    let mut sizes = Vec::new();
    let mut all = Vec::new();
    while let Some(page) = pages.next_page().await.unwrap() {
        sizes.push(page.len());
        all.extend(page);
    }

    assert_eq!(sizes, [100, 100, 56]);
    assert_eq!(all.len(), 256);
    // Newest first, by id as a number.
    assert_eq!(all[0].id, "2005");
    assert_eq!(all[255].id, "1000");

    let by_id = |id: &str| all.iter().find(|m| m.id == id).unwrap();
    let reply = by_id("2000");
    assert_eq!(reply.reply_to_message_id.as_deref(), Some("1001"));
    assert_eq!(reply.kind, 19);
    let pinned = by_id("2001");
    assert!(pinned.pinned);
    assert_eq!(pinned.author.id, "2");
    assert_eq!(
        pinned.edited_at.map(Timestamp::unix_seconds).unwrap() - pinned.created_at.unix_seconds(),
        4 * 60
    );
    let hook = by_id("2002");
    assert_eq!(hook.author.id, "777");
    assert_eq!(hook.author.webhook_id.as_deref(), Some("777"));
    assert!(hook.author.is_bot);
    let rich = by_id("2003");
    assert_eq!(rich.attachments.len(), 2);
    assert!(rich.attachments.iter().all(|a| is_cdn_url(Some(&a.url))));
    assert_eq!(rich.stickers[0].format, StickerFormat::Png);
    assert_eq!(rich.embeds[0].title.as_deref(), Some("Status"));
    assert_eq!(
        by_id("2004").forwarded.as_ref().map(|f| f.content.as_str()),
        Some("look at this")
    );
    assert!(by_id("2005").is_system_notice);
    assert!(!by_id("1000").is_system_notice);
    assert_eq!(by_id("1000").created_at.unix_seconds(), 1_704_067_200);

    // Every request asked for a hundred at most.
    assert!(
        rig.server
            .recorded()
            .iter()
            .all(|r| r.query("limit").as_deref() == Some("100"))
    );
}

#[tokio::test]
async fn a_read_that_was_stopped_goes_on_even_when_messages_were_deleted_meanwhile() {
    let guild = krew();
    for number in 0..150 {
        guild.message(
            &(1_000 + number).to_string(),
            "100",
            "1",
            "x",
            number,
            Msg::default(),
        );
    }
    let rig = Rig::new(guild.handler()).await;
    let bot = rig.bot();

    let mut first = bot.messages("100", None).unwrap();
    let page = first.next_page().await.unwrap().unwrap();
    assert_eq!(
        (page[0].id.as_str(), page[99].id.as_str()),
        ("1149", "1050")
    );
    let resume = first.resume_before().unwrap().to_owned();
    assert_eq!(resume, "1050");

    // Somebody deletes the message the read would have started at, and another further on.
    guild.delete("100", "1049");
    guild.delete("100", "1010");
    let rest = bot
        .messages("100", Some(&resume))
        .unwrap()
        .collect_all()
        .await
        .unwrap();

    assert_eq!(rest.len(), 48);
    assert_eq!(
        (rest[0].id.as_str(), rest[47].id.as_str()),
        ("1048", "1000")
    );
}

#[tokio::test]
async fn pins_come_with_their_messages_latest_first() {
    let guild = krew();
    guild
        .message(
            "1",
            "100",
            "1",
            "first",
            1,
            Msg {
                pinned: true,
                ..Msg::default()
            },
        )
        .message(
            "2",
            "100",
            "1",
            "second",
            2,
            Msg {
                pinned: true,
                ..Msg::default()
            },
        )
        .message("3", "100", "1", "not pinned", 3, Msg::default());
    let rig = Rig::new(guild.handler()).await;

    let pins = rig.bot().pins("100").unwrap().collect_all().await.unwrap();

    assert_eq!(
        pins.iter()
            .map(|p| p.message.id.as_str())
            .collect::<Vec<_>>(),
        ["2", "1"]
    );
    assert!(pins[0].pinned_at > pins[1].pinned_at);
}
