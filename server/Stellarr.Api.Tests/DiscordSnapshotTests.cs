using System.Text.Json;
using Stellarr.Api.Domain;
using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

/// <summary>
/// Discord's JSON as the snapshot reads it, starting from a real template
/// answer saved in Fixtures/Discord.
/// </summary>
public sealed class DiscordSnapshotTests
{
    internal static string TemplateFixture() =>
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Fixtures", "Discord", "template-hgM48av5Q69A.json"));

    [Fact]
    public void ATemplateBecomesSectionsChannelsAndRoles()
    {
        var snapshot = DiscordSnapshotMapper.FromTemplate(Read<WireTemplate>(TemplateFixture()));

        Assert.Equal(SnapshotSource.Template, snapshot.Source);
        Assert.Equal("678070694164299796", snapshot.SourceId);
        Assert.Equal("Friends & Family", snapshot.Name);
        Assert.Null(snapshot.Description);
        Assert.Null(snapshot.IconUrl);

        Assert.Equal(
            [new("1", "Text Channels", 1, []), new("5", "Voice Channels", 2, [])],
            snapshot.Sections.Select(section => section with { Overwrites = [] }));

        Assert.Equal(
            [
                ("2", "general", ChannelKind.Text, "1", 1),
                ("3", "games", ChannelKind.Text, "1", 2),
                ("4", "music", ChannelKind.Text, "1", 5),
                ("6", "Lounge", ChannelKind.Voice, "5", 0),
                ("7", "Stream Room", ChannelKind.Voice, "5", 1),
            ],
            snapshot.Channels.Select(channel => (channel.Id, channel.Name, channel.Kind, channel.SectionId, channel.Position)));
        Assert.All(snapshot.Channels, channel =>
        {
            Assert.Equal(DiscordChannelType.Conversation, channel.DiscordType);
            Assert.Null(channel.Topic);
            Assert.Equal(0, channel.SlowModeSeconds);
            Assert.Empty(channel.Overwrites);
        });

        var everyone = Assert.Single(snapshot.Roles);
        Assert.Equal("0", everyone.Id);
        Assert.Equal("@everyone", everyone.Name);
        Assert.Equal("2248329584434769", everyone.Permissions);
        Assert.True(everyone.IsEveryone);
        Assert.Null(everyone.Color);

        Assert.Empty(snapshot.Members);
        Assert.Empty(snapshot.Emoji);
        Assert.Empty(snapshot.Threads);
    }

    [Fact]
    public void ToleratesNumbersForTextMissingFieldsAndFieldsItDoesNotKnow()
    {
        // The documentation's own example gives permissions as a number.
        var template = Read<WireTemplate>("""
            {
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
            }
            """);

        var snapshot = DiscordSnapshotMapper.FromTemplate(template);

        Assert.Equal(
            "https://cdn.discordapp.com/icons/678070694164299796/f749bb0cbeeb26ef21eca719337d20f1.png",
            snapshot.IconUrl);
        Assert.Equal(["104324689", "8"], snapshot.Roles.Select(role => role.Permissions));
        Assert.Equal([true, false], snapshot.Roles.Select(role => role.IsEveryone));
        Assert.Equal([0, 1], snapshot.Roles.Select(role => role.Position));
        Assert.Equal(3447003, snapshot.Roles[1].Color);
        Assert.True(snapshot.Roles[1].Hoisted);

        Assert.Equal(
            [
                ("11", ChannelKind.Text, DiscordChannelType.Conversation, "10"),
                ("12", ChannelKind.Text, DiscordChannelType.Announcements, null),
                ("13", ChannelKind.Text, DiscordChannelType.Forum, null),
                ("14", ChannelKind.Text, DiscordChannelType.Media, null),
                ("15", ChannelKind.Voice, DiscordChannelType.Stage, null),
            ],
            snapshot.Channels.Select(channel => (channel.Id, channel.Kind, channel.DiscordType, channel.SectionId)));

        var news = snapshot.Channels[0];
        Assert.Equal("Read me", news.Topic);
        Assert.Equal(30, news.SlowModeSeconds);
        Assert.Equal(
            [
                new PermissionOverwriteSnapshot(OverwriteTarget.Role, "1", "2048", "0"),
                new PermissionOverwriteSnapshot(OverwriteTarget.Member, "268473310986240001", "0", "1024"),
            ],
            news.Overwrites);
        Assert.Equal(40, snapshot.Channels[4].UserLimit);
    }

    [Fact]
    public void AMessageKeepsItsAuthorReplyFilesEmbedsStickersAndReactions()
    {
        var message = DiscordSnapshotMapper.Message(Read<WireMessage>("""
            {
              "id": "1200000000000000001",
              "channel_id": "290926798999357250",
              "type": 19,
              "content": "Supa <@53908099506183680>",
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
              "embeds": [{ "type": "article", "url": "https://example.test/a", "title": "A", "thumbnail": { "url": "https://example.test/t.png" } }],
              "sticker_items": [{ "id": "749054660769218631", "name": "Wave", "format_type": 3 }],
              "reactions": [
                { "count": 3, "count_details": { "burst": 1, "normal": 2 }, "emoji": { "id": null, "name": "🔥" } },
                { "count": 1, "emoji": { "id": "41771983429993937", "name": "LUL", "animated": true } }
              ]
            }
            """))!;

        Assert.Equal("1200000000000000001", message.Id);
        Assert.Equal("Supa <@53908099506183680>", message.Content);
        Assert.Equal(new DateTimeOffset(2017, 7, 11, 17, 27, 7, 299, TimeSpan.Zero), message.CreatedAt);
        Assert.Equal(new DateTimeOffset(2017, 7, 11, 17, 30, 0, TimeSpan.Zero), message.EditedAt);
        Assert.Equal("1100000000000000000", message.ReplyToMessageId);
        Assert.True(message.Pinned);
        Assert.False(message.IsSystemNotice);
        Assert.Equal("1200000000000000001", message.StartedThreadId);

        Assert.Equal("Weather", message.Author.Username);
        Assert.Equal("777", message.Author.WebhookId);
        Assert.Equal("https://cdn.discordapp.com/avatars/777/abc.png", message.Author.AvatarUrl);

        Assert.Equal(["9", "10"], message.Attachments.Select(attachment => attachment.Id));
        Assert.Equal(3.5, message.Attachments[0].DurationSeconds);
        Assert.Equal("audio/ogg", message.Attachments[0].ContentType);
        Assert.Equal((640, 480), (message.Attachments[1].Width, message.Attachments[1].Height));

        Assert.Equal(
            new EmbedSummary("article", "https://example.test/a", "A", null, "https://example.test/t.png"),
            Assert.Single(message.Embeds));
        Assert.Equal(
            new StickerReference("749054660769218631", "Wave", StickerFormat.Lottie,
                "https://cdn.discordapp.com/stickers/749054660769218631.json"),
            Assert.Single(message.Stickers));
        Assert.Equal(
            [
                new ReactionSnapshot(new ReactionEmoji(null, "🔥", false), 3, 1),
                new ReactionSnapshot(new ReactionEmoji("41771983429993937", "LUL", true), 1, 0),
            ],
            message.Reactions);
    }

    [Fact]
    public void ANoticeAForwardAndAMessageWithoutATimeReadToo()
    {
        var pin = DiscordSnapshotMapper.Message(Read<WireMessage>("""
            { "id": "175928847299117063", "channel_id": "1", "type": 6, "author": { "id": "2", "username": "jo" },
              "message_reference": { "message_id": "3" } }
            """))!;
        var forward = DiscordSnapshotMapper.Message(Read<WireMessage>("""
            { "id": "4", "channel_id": "1", "type": 0, "content": "", "timestamp": "2024-01-01T00:00:00+00:00",
              "author": { "id": "2", "username": "jo" },
              "message_reference": { "type": 1, "message_id": "5", "channel_id": "6" },
              "message_snapshots": [{ "message": { "content": "look", "attachments": [
                { "id": "7", "filename": "a.png", "size": 5, "url": "https://cdn.discordapp.com/attachments/6/7/a.png" }] } }] }
            """))!;

        Assert.True(pin.IsSystemNotice);
        Assert.Null(pin.ReplyToMessageId);
        // Without a timestamp, the time comes out of the id.
        Assert.Equal(new DateTimeOffset(2016, 4, 30, 11, 18, 25, 796, TimeSpan.Zero), pin.CreatedAt);

        Assert.Equal("look", forward.Forwarded?.Content);
        Assert.Equal("7", Assert.Single(forward.Forwarded!.Attachments).Id);
        Assert.Null(forward.ReplyToMessageId);
        Assert.Null(DiscordSnapshotMapper.Message(Read<WireMessage>("""{ "channel_id": "1" }""")));
    }

    [Fact]
    public void MembersThreadsEmojiAndStickersRead()
    {
        var member = DiscordSnapshotMapper.Member(Read<WireMember>("""
            { "user": { "id": "80351110224678912", "username": "nelly", "global_name": "Nelly", "avatar": "8342729096ea3675442027381ff50dfe" },
              "nick": "NOT API SUPPORT", "avatar": "a_1269e74af4df7417b13759eae50c83dc", "roles": ["41771983423143936", "1"],
              "joined_at": "2015-04-26T06:26:56.936000+00:00" }
            """), serverId: "1")!;
        var thread = DiscordSnapshotMapper.Thread(Read<WireChannel>("""
            { "id": "41771983423143937", "parent_id": "41771983423143936", "name": "don't buy dota-2", "type": 11,
              "thread_metadata": { "archived": true, "locked": false, "archive_timestamp": "2021-04-12T23:40:39.855793+00:00" } }
            """))!;
        var privateThread = DiscordSnapshotMapper.Thread(Read<WireChannel>("""
            { "id": "9", "parent_id": "8", "name": "secret", "type": 12, "thread_metadata": { "create_timestamp": "2023-01-01T00:00:00+00:00" } }
            """))!;
        var emoji = DiscordSnapshotMapper.Emoji(Read<WireEmoji>("""{ "id": "41771983429993937", "name": "LUL", "animated": false }"""))!;
        var sticker = DiscordSnapshotMapper.Sticker(Read<WireSticker>("""{ "id": "5", "name": "Wave", "format_type": 4 }"""))!;

        Assert.Equal(("nelly", "Nelly", "NOT API SUPPORT"), (member.Username, member.DisplayName, member.Nickname));
        Assert.Equal("https://cdn.discordapp.com/avatars/80351110224678912/8342729096ea3675442027381ff50dfe.png", member.AvatarUrl);
        Assert.Equal(
            "https://cdn.discordapp.com/guilds/1/users/80351110224678912/avatars/a_1269e74af4df7417b13759eae50c83dc.webp?animated=true",
            member.ServerAvatarUrl);
        Assert.Equal(["41771983423143936"], member.RoleIds);
        Assert.Equal(new DateTimeOffset(2015, 4, 26, 6, 26, 56, 936, TimeSpan.Zero), member.JoinedAt);

        Assert.Equal(("41771983423143936", "41771983423143937", true, false), (thread.ParentChannelId, thread.StarterMessageId, thread.Archived, thread.Private));
        Assert.Null(privateThread.StarterMessageId);
        Assert.True(privateThread.Private);
        Assert.Equal(new DateTimeOffset(2023, 1, 1, 0, 0, 0, TimeSpan.Zero), privateThread.CreatedAt);

        Assert.Equal("https://cdn.discordapp.com/emojis/41771983429993937.webp", emoji.ImageUrl);
        Assert.True(emoji.Available);
        Assert.Equal("https://media.discordapp.net/stickers/5.gif", sticker.ImageUrl);
    }

    private static T Read<T>(string json) => JsonSerializer.Deserialize<T>(json, DiscordJson.Options)!;
}
