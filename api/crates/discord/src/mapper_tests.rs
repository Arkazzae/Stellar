//! Discord's JSON as the snapshot reads it, starting from a real template
//! answer saved in `tests/fixtures`.

use serde::Deserialize;

use crate::cdn::StickerFormat;
use crate::mapper;
use crate::snapshot::*;
use crate::time::Timestamp;
use crate::wire::{
    Obj, WireChannel, WireEmoji, WireError, WireMember, WireMessage, WireSticker, WireTemplate,
    nests_deeper_than,
};

fn read<T: for<'de> Deserialize<'de>>(json: &str) -> T {
    serde_json::from_str::<Obj<T>>(json)
        .unwrap_or_else(|error| panic!("{error}: {json}"))
        .0
        .expect("not null")
}

fn fails<T: for<'de> Deserialize<'de>>(json: &str) -> bool {
    serde_json::from_str::<Obj<T>>(json).is_err()
}

fn seconds(unix: i64) -> Option<Timestamp> {
    Timestamp::from_unix_seconds(unix)
}

fn millis(unix_millis: i64) -> Option<Timestamp> {
    Timestamp::from_unix_millis(unix_millis)
}

fn template_fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/template-hgM48av5Q69A.json"
    ))
    .unwrap()
}

#[test]
fn a_template_becomes_sections_channels_and_roles() {
    let snapshot = mapper::from_template(&read::<WireTemplate>(&template_fixture()));

    assert_eq!(snapshot.source, SnapshotSource::Template);
    assert_eq!(snapshot.source_id, "678070694164299796");
    assert_eq!(snapshot.name, "Friends & Family");
    assert_eq!(snapshot.description, None);
    assert_eq!(snapshot.icon_url, None);

    assert_eq!(
        snapshot
            .sections
            .iter()
            .map(|s| (
                s.id.as_str(),
                s.name.as_str(),
                s.position,
                s.overwrites.len()
            ))
            .collect::<Vec<_>>(),
        [("1", "Text Channels", 1, 0), ("5", "Voice Channels", 2, 0)]
    );

    assert_eq!(
        snapshot
            .channels
            .iter()
            .map(|c| (
                c.id.as_str(),
                c.name.as_str(),
                c.kind,
                c.section_id.as_deref(),
                c.position
            ))
            .collect::<Vec<_>>(),
        [
            ("2", "general", ChannelKind::Text, Some("1"), 1),
            ("3", "games", ChannelKind::Text, Some("1"), 2),
            ("4", "music", ChannelKind::Text, Some("1"), 5),
            ("6", "Lounge", ChannelKind::Voice, Some("5"), 0),
            ("7", "Stream Room", ChannelKind::Voice, Some("5"), 1),
        ]
    );
    for channel in &snapshot.channels {
        assert_eq!(channel.discord_type, DiscordChannelType::Conversation);
        assert_eq!(channel.topic, None);
        assert_eq!(channel.slow_mode_seconds, 0);
        assert!(channel.overwrites.is_empty());
    }

    let [everyone] = snapshot.roles.as_slice() else {
        panic!("{:?}", snapshot.roles)
    };
    assert_eq!(everyone.id, "0");
    assert_eq!(everyone.name, "@everyone");
    assert_eq!(everyone.permissions, "2248329584434769");
    assert!(everyone.is_everyone);
    assert_eq!(everyone.color, None);

    assert!(snapshot.members.is_empty());
    assert!(snapshot.emoji.is_empty());
    assert!(snapshot.threads.is_empty());
}

#[test]
fn tolerates_numbers_for_text_missing_fields_and_fields_it_does_not_know() {
    // The documentation's own example gives permissions as a number.
    let template = read::<WireTemplate>(
        r#"{
          "code": "abc",
          "source_guild_id": 678070694164299796,
          "something_new": { "nested": [1, 2] },
          "serialized_source_guild": {
            "name": "Quiet",
            "icon_hash": "f749bb0cbeeb26ef21eca719337d20f1",
            "roles": [
              { "id": 0, "name": "@everyone", "permissions": 104324689 },
              { "id": 1, "name": "Mods", "permissions": "8", "color": 3447003, "hoist": true, "colors": null },
              { "name": "no id, left out" }
            ],
            "channels": [
              { "id": 10, "type": 4, "name": "Talk" },
              { "id": 11, "type": 0, "name": "news", "parent_id": 10, "topic": "Read me", "rate_limit_per_user": "30",
                "permission_overwrites": [
                  { "id": 1, "type": 0, "allow": 2048, "deny": "0" },
                  { "id": "268473310986240001", "type": 1, "allow": "0", "deny": "1024" },
                  { "id": 2, "type": 7, "allow": "1", "deny": "0" }
                ] },
              { "id": 12, "type": 5, "name": "updates", "parent_id": 99 },
              { "id": 13, "type": 15, "name": "help" },
              { "id": 14, "type": 16, "name": "art" },
              { "id": 15, "type": 13, "name": "stage", "user_limit": 40 },
              { "id": 16, "type": 14, "name": "directory" },
              { "id": 17, "type": 99, "name": "from the future" },
              { "type": 0, "name": "no id" }
            ]
          }
        }"#,
    );

    let snapshot = mapper::from_template(&template);

    assert_eq!(
        snapshot.icon_url.as_deref(),
        Some(
            "https://cdn.discordapp.com/icons/678070694164299796/f749bb0cbeeb26ef21eca719337d20f1.png"
        )
    );
    assert_eq!(
        snapshot
            .roles
            .iter()
            .map(|r| r.permissions.as_str())
            .collect::<Vec<_>>(),
        ["104324689", "8"]
    );
    assert_eq!(
        snapshot
            .roles
            .iter()
            .map(|r| r.is_everyone)
            .collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(
        snapshot
            .roles
            .iter()
            .map(|r| r.position)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(snapshot.roles[1].color, Some(3447003));
    assert!(snapshot.roles[1].hoisted);

    assert_eq!(
        snapshot
            .channels
            .iter()
            .map(|c| (
                c.id.as_str(),
                c.kind,
                c.discord_type,
                c.section_id.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            (
                "11",
                ChannelKind::Text,
                DiscordChannelType::Conversation,
                Some("10")
            ),
            (
                "12",
                ChannelKind::Text,
                DiscordChannelType::Announcements,
                None
            ),
            ("13", ChannelKind::Text, DiscordChannelType::Forum, None),
            ("14", ChannelKind::Text, DiscordChannelType::Media, None),
            ("15", ChannelKind::Voice, DiscordChannelType::Stage, None),
        ]
    );

    let news = &snapshot.channels[0];
    assert_eq!(news.topic.as_deref(), Some("Read me"));
    assert_eq!(news.slow_mode_seconds, 30);
    assert_eq!(
        news.overwrites,
        [
            PermissionOverwriteSnapshot {
                target: OverwriteTarget::Role,
                target_id: "1".into(),
                allow: "2048".into(),
                deny: "0".into()
            },
            PermissionOverwriteSnapshot {
                target: OverwriteTarget::Member,
                target_id: "268473310986240001".into(),
                allow: "0".into(),
                deny: "1024".into()
            },
        ]
    );
    assert_eq!(snapshot.channels[4].user_limit, 40);
}

#[test]
fn a_message_keeps_its_author_reply_files_embeds_stickers_and_reactions() {
    let message = mapper::message(&read::<WireMessage>(
        r#"{
          "id": "1200000000000000001",
          "channel_id": "290926798999357250",
          "type": 19,
          "content": "Supa <@53908099506183680>",
          "mentions": [{ "id": "53908099506183680", "username": "ula", "global_name": "Ula", "member": { "nick": "Ula z Krew" } }, null],
          "timestamp": "2017-07-11T17:27:07.299000+00:00",
          "edited_timestamp": "2017-07-11T17:30:00.000000+00:00",
          "pinned": true,
          "flags": 32,
          "webhook_id": "777",
          "author": { "id": "777", "username": "Weather", "avatar": "abc", "bot": true },
          "message_reference": { "type": 0, "message_id": "1100000000000000000", "channel_id": "290926798999357250" },
          "attachments": [
            { "id": "9", "filename": "song.ogg", "size": 12345, "content_type": "audio/ogg",
              "url": "https://cdn.discordapp.com/attachments/1/9/song.ogg?ex=1&is=2&hm=3", "duration_secs": 3.5 },
            { "id": "10", "filename": "pic.png", "size": 1, "url": "https://cdn.discordapp.com/attachments/1/10/pic.png",
              "width": 640, "height": 480 },
            { "filename": "no id" }
          ],
          "embeds": [{ "type": "article", "url": "https://example.test/a", "title": "A", "thumbnail": { "url": "https://example.test/t.png", "proxy_url": "https://images-ext-1.discordapp.net/t.png" } }],
          "sticker_items": [{ "id": "749054660769218631", "name": "Wave", "format_type": 3 }],
          "reactions": [
            { "count": 3, "count_details": { "burst": 1, "normal": 2 }, "emoji": { "id": null, "name": "🔥" } },
            { "count": 1, "emoji": { "id": "41771983429993937", "name": "LUL", "animated": true } }
          ]
        }"#,
    ))
    .unwrap();

    assert_eq!(message.id, "1200000000000000001");
    assert_eq!(message.content, "Supa <@53908099506183680>");
    assert_eq!(message.mentions.len(), 1);
    assert_eq!(message.mentions[0].id, "53908099506183680");
    assert_eq!(message.mentions[0].username.as_deref(), Some("ula"));
    assert_eq!(message.mentions[0].display_name.as_deref(), Some("Ula"));
    assert_eq!(message.mentions[0].nickname.as_deref(), Some("Ula z Krew"));
    assert_eq!(Some(message.created_at), millis(1_499_794_027_299));
    assert_eq!(message.edited_at, seconds(1_499_794_200));
    assert_eq!(
        message.reply_to_message_id.as_deref(),
        Some("1100000000000000000")
    );
    assert!(message.pinned);
    assert!(!message.is_system_notice);
    assert_eq!(
        message.started_thread_id.as_deref(),
        Some("1200000000000000001")
    );

    assert_eq!(message.author.username, "Weather");
    assert_eq!(message.author.webhook_id.as_deref(), Some("777"));
    assert_eq!(
        message.author.avatar_url.as_deref(),
        Some("https://cdn.discordapp.com/avatars/777/abc.png")
    );

    assert_eq!(
        message
            .attachments
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["9", "10"]
    );
    assert_eq!(message.attachments[0].duration_seconds, Some(3.5));
    assert_eq!(
        message.attachments[0].content_type.as_deref(),
        Some("audio/ogg")
    );
    assert_eq!(message.attachments[0].size, 12345);
    assert_eq!(
        (message.attachments[1].width, message.attachments[1].height),
        (Some(640), Some(480))
    );

    assert_eq!(
        message.embeds,
        [EmbedSummary {
            kind: Some("article".into()),
            url: Some("https://example.test/a".into()),
            title: Some("A".into()),
            description: None,
            image_url: Some("https://example.test/t.png".into()),
            image_proxy_url: Some("https://images-ext-1.discordapp.net/t.png".into()),
            video_url: None,
        }]
    );
    assert_eq!(
        message.stickers,
        [StickerReference {
            id: "749054660769218631".into(),
            name: "Wave".into(),
            format: StickerFormat::Lottie,
            image_url: "https://cdn.discordapp.com/stickers/749054660769218631.json".into(),
        }]
    );
    assert_eq!(
        message.reactions,
        [
            ReactionSnapshot {
                emoji: ReactionEmoji {
                    id: None,
                    name: "🔥".into(),
                    animated: false
                },
                count: 3,
                burst_count: 1
            },
            ReactionSnapshot {
                emoji: ReactionEmoji {
                    id: Some("41771983429993937".into()),
                    name: "LUL".into(),
                    animated: true
                },
                count: 1,
                burst_count: 0
            },
        ]
    );
}

#[test]
fn a_notice_a_forward_and_a_message_without_a_time_read_too() {
    let pin = mapper::message(&read::<WireMessage>(
        r#"{ "id": "175928847299117063", "channel_id": "1", "type": 6, "author": { "id": "2", "username": "jo" },
             "message_reference": { "message_id": "3" } }"#,
    ))
    .unwrap();
    let forward = mapper::message(&read::<WireMessage>(
        r#"{ "id": "4", "channel_id": "1", "type": 0, "content": "", "timestamp": "2024-01-01T00:00:00+00:00",
             "author": { "id": "2", "username": "jo" },
             "message_reference": { "type": 1, "message_id": "5", "channel_id": "6" },
             "message_snapshots": [{ "message": { "content": "look", "attachments": [
               { "id": "7", "filename": "a.png", "size": 5, "url": "https://cdn.discordapp.com/attachments/6/7/a.png" }] } }] }"#,
    ))
    .unwrap();

    assert!(pin.is_system_notice);
    assert_eq!(pin.reply_to_message_id, None);
    // Without a timestamp, the time comes out of the id.
    assert_eq!(Some(pin.created_at), millis(1_462_015_105_796));

    let copy = forward.forwarded.as_ref().unwrap();
    assert_eq!(copy.content, "look");
    assert_eq!(copy.attachments.len(), 1);
    assert_eq!(copy.attachments[0].id, "7");
    assert_eq!(forward.reply_to_message_id, None);
    assert!(mapper::message(&read::<WireMessage>(r#"{ "channel_id": "1" }"#)).is_none());
}

#[test]
fn members_threads_emoji_and_stickers_read() {
    let member = mapper::member(
        &read::<WireMember>(
            r#"{ "user": { "id": "80351110224678912", "username": "nelly", "global_name": "Nelly", "avatar": "8342729096ea3675442027381ff50dfe" },
                 "nick": "NOT API SUPPORT", "avatar": "a_1269e74af4df7417b13759eae50c83dc", "roles": ["41771983423143936", "1"],
                 "joined_at": "2015-04-26T06:26:56.936000+00:00" }"#,
        ),
        "1",
    )
    .unwrap();
    let thread = mapper::thread(&read::<WireChannel>(
        r#"{ "id": "41771983423143937", "parent_id": "41771983423143936", "name": "don't buy dota-2", "type": 11,
             "thread_metadata": { "archived": true, "locked": false, "archive_timestamp": "2021-04-12T23:40:39.855793+00:00" } }"#,
    ))
    .unwrap();
    let private = mapper::thread(&read::<WireChannel>(
        r#"{ "id": "9", "parent_id": "8", "name": "secret", "type": 12, "thread_metadata": { "create_timestamp": "2023-01-01T00:00:00+00:00" } }"#,
    ))
    .unwrap();
    let emoji = mapper::emoji(&read::<WireEmoji>(
        r#"{ "id": "41771983429993937", "name": "LUL", "animated": false }"#,
    ))
    .unwrap();
    let sticker = mapper::sticker(&read::<WireSticker>(
        r#"{ "id": "5", "name": "Wave", "format_type": 4 }"#,
    ))
    .unwrap();

    assert_eq!(
        (
            member.username.as_str(),
            member.display_name.as_deref(),
            member.nickname.as_deref()
        ),
        ("nelly", Some("Nelly"), Some("NOT API SUPPORT"))
    );
    assert_eq!(
        member.avatar_url.as_deref(),
        Some(
            "https://cdn.discordapp.com/avatars/80351110224678912/8342729096ea3675442027381ff50dfe.png"
        )
    );
    assert_eq!(
        member.server_avatar_url.as_deref(),
        Some(
            "https://cdn.discordapp.com/guilds/1/users/80351110224678912/avatars/a_1269e74af4df7417b13759eae50c83dc.webp?animated=true"
        )
    );
    assert_eq!(member.role_ids, ["41771983423143936"]);
    assert_eq!(member.joined_at, millis(1_430_029_616_936));

    assert_eq!(
        (
            thread.parent_channel_id.as_str(),
            thread.starter_message_id.as_deref(),
            thread.archived,
            thread.private
        ),
        ("41771983423143936", Some("41771983423143937"), true, false)
    );
    assert_eq!(private.starter_message_id, None);
    assert!(private.private);
    assert_eq!(private.created_at, seconds(1_672_531_200));

    assert_eq!(
        emoji.image_url,
        "https://cdn.discordapp.com/emojis/41771983429993937.webp"
    );
    assert!(emoji.available);
    assert_eq!(
        sticker.image_url,
        "https://media.discordapp.net/stickers/5.gif"
    );
}

/* ------------------------------------------------------- what is not trusted */

#[test]
fn a_value_of_the_wrong_kind_fails_the_answer_or_reads_as_absent() {
    // Where text is expected, an object or a list is absent and the rest reads.
    let user =
        read::<crate::wire::WireUser>(r#"{"id": [1, 2], "username": {"a": 1}, "bot": null}"#);
    assert_eq!(user.id.get(), None);
    assert_eq!(user.username.get(), None);
    // A number or a boolean is text.
    let user = read::<crate::wire::WireUser>(r#"{"id": 12, "username": true, "global_name": 1.5}"#);
    assert_eq!(
        (user.id.get(), user.username.get(), user.global_name.get()),
        (Some("12"), Some("true"), Some("1.5"))
    );

    // Where a number is expected, anything but a whole number in range fails.
    for json in [
        r#"{"type": "x"}"#,
        r#"{"type": 1.5}"#,
        r#"{"type": 3.0}"#,
        r#"{"type": 3000000000}"#,
        r#"{"type": -3000000000}"#,
        r#"{"type": []}"#,
        r#"{"type": {}}"#,
        r#"{"type": true}"#,
        r#"{"type": "1e3"}"#,
        r#"{"type": "+1"}"#,
        r#"{"type": " 1"}"#,
        r#"{"type": ""}"#,
        r#"{"type": "-"}"#,
    ] {
        assert!(fails::<WireChannel>(json), "{json}");
    }
    assert_eq!(read::<WireChannel>(r#"{"type": "13"}"#).kind.0, Some(13));
    assert_eq!(read::<WireChannel>(r#"{"type": "-13"}"#).kind.0, Some(-13));
    assert_eq!(read::<WireChannel>(r#"{"type": null}"#).kind.0, None);

    // Where an object is expected, a list, a string or a number fails.
    for json in [
        r#"{"author": []}"#,
        r#"{"author": [1, 2, 3]}"#,
        r#"{"author": "x"}"#,
        r#"{"author": 1}"#,
        r#"{"attachments": {}}"#,
        r#"{"attachments": "x"}"#,
        r#"{"attachments": [1]}"#,
        r#"{"attachments": [[]]}"#,
        r#"{"embeds": [{"image": []}]}"#,
        r#"{"flags": "x"}"#,
        r#"{"pinned": "true"}"#,
        r#"{"pinned": 1}"#,
    ] {
        assert!(fails::<WireMessage>(json), "{json}");
    }
    // Null is absent, in a list as well.
    let message = read::<WireMessage>(
        r#"{"author": null, "attachments": null, "embeds": [null, {}], "pinned": null}"#,
    );
    assert!(message.author.0.is_none());
    assert_eq!(message.embeds.0.len(), 2);
    assert_eq!(message.embeds.present().count(), 1);

    // A list read as an object is not an object with its items for fields.
    assert!(fails::<WireMember>("[1, 2]"));
    assert!(fails::<WireMember>(r#"[{"id": "1"}, "nick"]"#));
    // Unknown fields, however deep, are ignored.
    assert!(
        read::<WireMember>(r#"{"new": {"a": [1, {"b": null}]}, "nick": "x"}"#)
            .nick
            .get()
            == Some("x")
    );
}

#[test]
fn a_whole_error_body_reads_as_far_as_it_can() {
    let error = read::<WireError>(
        r#"{"message": "Nope", "code": 50001, "retry_after": 1.5, "global": true}"#,
    );
    assert_eq!(error.message.get(), Some("Nope"));
    assert_eq!(error.code.0, Some(50001));
    assert_eq!(error.retry_after.0, Some(1.5));
    assert_eq!(error.global.0, Some(true));
    // Numbers as strings, as Discord's own documentation sometimes writes them.
    let error = read::<WireError>(r#"{"code": "10003", "retry_after": "2.25"}"#);
    assert_eq!(
        (error.code.0, error.retry_after.0),
        (Some(10003), Some(2.25))
    );
    assert!(fails::<WireError>(r#"{"retry_after": "NaN"}"#));
    assert!(fails::<WireError>(r#"{"retry_after": "inf"}"#));
}

#[test]
fn nesting_is_counted_and_strings_do_not_count() {
    assert!(!nests_deeper_than(b"[[[[]]]]", 4));
    assert!(nests_deeper_than(b"[[[[]]]]", 3));
    assert!(!nests_deeper_than(br#"["[[[[[[[[", "\"[[[[[["]"#, 1));
    assert!(nests_deeper_than(&b"[".repeat(100_000), 64));
    assert!(!nests_deeper_than(br#"{"a": [{"b": [1]}]}"#, 4));
}

#[test]
fn a_record_without_its_id_is_left_out_and_the_rest_keeps_its_place() {
    // A number in a list of objects is a failure, not a record to skip.
    assert!(fails::<WireTemplate>(
        r#"{"serialized_source_guild": {"roles": [5]}}"#
    ));
    let template = read::<WireTemplate>(
        r#"{"serialized_source_guild": {"roles": [null, {"name": "no id"}, {"id": "7", "name": "kept"}]}}"#,
    );
    let snapshot = mapper::from_template(&template);
    // A template gives no positions, so a role's place is its place in the list.
    assert_eq!(
        snapshot
            .roles
            .iter()
            .map(|r| (r.id.as_str(), r.position))
            .collect::<Vec<_>>(),
        [("7", 2)]
    );
}

#[test]
fn colours_follow_the_new_field_then_the_old_one_and_zero_is_none() {
    let cases = [
        (r#"{"id": "1"}"#, None),
        (r#"{"id": "1", "color": 0}"#, None),
        (r#"{"id": "1", "color": 5}"#, Some(5)),
        (
            r#"{"id": "1", "colors": {"primary_color": 7}, "color": 5}"#,
            Some(7),
        ),
        (
            r#"{"id": "1", "colors": {"primary_color": 0}, "color": 5}"#,
            None,
        ),
        (r#"{"id": "1", "colors": {}, "color": 5}"#, Some(5)),
        (r#"{"id": "1", "color": -5}"#, None),
    ];
    for (json, expected) in cases {
        let guild = format!(r#"{{"serialized_source_guild": {{"roles": [{json}]}}}}"#);
        let snapshot = mapper::from_template(&read::<WireTemplate>(&guild));
        assert_eq!(snapshot.roles[0].color, expected, "{json}");
    }
}

#[test]
fn a_reply_is_a_reply_only_to_a_message_and_a_thread_comes_from_flag_or_object() {
    let message = |json: &str| mapper::message(&read::<WireMessage>(json)).unwrap();
    let base = r#""id": "10", "channel_id": "1""#;
    assert_eq!(
        message(&format!(
            r#"{{{base}, "type": 19, "message_reference": {{"message_id": "9"}}}}"#
        ))
        .reply_to_message_id
        .as_deref(),
        Some("9")
    );
    // A forward's reference is not a reply, and neither is a reply's own type missing.
    assert_eq!(
        message(&format!(
            r#"{{{base}, "type": 19, "message_reference": {{"type": 1, "message_id": "9"}}}}"#
        ))
        .reply_to_message_id,
        None
    );
    assert_eq!(
        message(&format!(
            r#"{{{base}, "type": 0, "message_reference": {{"message_id": "9"}}}}"#
        ))
        .reply_to_message_id,
        None
    );
    assert_eq!(
        message(&format!(r#"{{{base}, "flags": 32}}"#))
            .started_thread_id
            .as_deref(),
        Some("10")
    );
    assert_eq!(
        message(&format!(r#"{{{base}, "flags": 31}}"#)).started_thread_id,
        None
    );
    assert_eq!(
        message(&format!(r#"{{{base}, "thread": {{"id": "77"}}}}"#))
            .started_thread_id
            .as_deref(),
        Some("77")
    );
    // Types that are somebody speaking, and the ones that are not.
    for (kind, notice) in [
        (0, false),
        (19, false),
        (20, false),
        (23, false),
        (6, true),
        (7, true),
        (21, true),
        (-1, true),
    ] {
        assert_eq!(
            message(&format!(r#"{{{base}, "type": {kind}}}"#)).is_system_notice,
            notice,
            "{kind}"
        );
    }
    // The first snapshot is the forward, even if it is not the first present.
    let forwarded = message(&format!(
        r#"{{{base}, "message_reference": {{"type": 1}}, "message_snapshots": [null, {{"message": {{"content": "x"}}}}]}}"#
    ));
    assert!(forwarded.forwarded.is_none());
}

#[test]
fn a_webhook_message_is_shown_as_the_webhook_and_a_missing_author_as_nobody() {
    let webhook = mapper::message(&read::<WireMessage>(
        r#"{"id": "1", "channel_id": "2", "webhook_id": "99"}"#,
    ))
    .unwrap();
    assert_eq!(webhook.author.id, "99");
    assert_eq!(webhook.author.webhook_id.as_deref(), Some("99"));
    assert_eq!(webhook.author.avatar_url, None);
    let nobody =
        mapper::message(&read::<WireMessage>(r#"{"id": "1", "channel_id": "2"}"#)).unwrap();
    assert_eq!(nobody.author.id, "");
    assert_eq!(nobody.author.avatar_url, None);
}

#[test]
fn a_time_that_does_not_read_falls_back_to_the_id_and_then_to_the_epoch() {
    let odd = mapper::message(&read::<WireMessage>(
        r#"{"id": "175928847299117063", "channel_id": "2", "timestamp": "last tuesday", "edited_timestamp": "never"}"#,
    ))
    .unwrap();
    assert_eq!(Some(odd.created_at), millis(1_462_015_105_796));
    assert_eq!(odd.edited_at, None);
    let none =
        mapper::message(&read::<WireMessage>(r#"{"id": "abc", "channel_id": "2"}"#)).unwrap();
    assert_eq!(none.created_at, Timestamp::UNIX_EPOCH);
}
