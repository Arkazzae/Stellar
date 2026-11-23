using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Auth;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Endpoints;
using Stellarr.Api.Integrations.Discord;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Tests;

/// <summary>
/// The import's writer, fed by the real reader from a Discord server held in
/// memory: the structure it makes, the history it restores, and that the
/// history behaves as if it had always been there.
/// </summary>
public sealed class DiscordImportWriterTests(StellarrWebApplicationFactory factory)
    : IClassFixture<StellarrWebApplicationFactory>
{
    private static readonly Dictionary<string, (Guid Id, string Password)> Accounts = new()
    {
        ["maja"] = (Guid.Parse("30000000-0000-0000-0000-000000000001"), "Aurora!2026"),
        ["ada"] = (Guid.Parse("30000000-0000-0000-0000-000000000002"), "Quasar!2026"),
        ["ilya"] = (Guid.Parse("30000000-0000-0000-0000-000000000003"), "Meteor!2026"),
        ["nils"] = (Guid.Parse("30000000-0000-0000-0000-000000000004"), "Zenith!2026"),
        ["jo"] = (Guid.Parse("30000000-0000-0000-0000-000000000005"), "Corona!2026"),
    };

    [Fact]
    public async Task TheStructureComesInDiscordsOrderAndASecondRunMakesNothing()
    {
        var guild = new DiscordFakeGuild()
            .Category("900", "Talk", position: 1)
            .Category("800", "Games & Fun", position: 0)
            .Channel("110", "General", parent: "900", position: 0, topic: "  Hello  ", slowMode: 30)
            .Channel("100", "general", parent: "900", position: 0)
            .Channel("120", "Lounge", type: 2, parent: "900", position: 0, userLimit: 5)
            .Channel("130", "help", type: 15, parent: "800", position: 0, topic: new string('t', 1500))
            .Channel("140", "Town Hall", type: 13, parent: "800", userLimit: 10000)
            .Channel("150", "📢・news", type: 5)
            .Channel("160", "staff", parent: "900", position: 3, hidden: true)
            .Thread("170", "130", "How do I join?");
        var serverId = await NewServerAsync("ada");

        var first = await RunAsync(serverId, guild);
        var second = await RunAsync(serverId, guild);

        await using var scope = factory.OpenScope(out var db);
        var sections = await db.Sections.AsNoTracking().Where(x => x.ServerId == serverId)
            .OrderBy(x => x.Position).Select(x => x.Name).ToListAsync();
        Assert.Equal(["Games & Fun", "Talk"], sections);
        var channels = await db.Channels.AsNoTracking().Where(x => x.ServerId == serverId)
            .OrderBy(x => x.Position).ToListAsync();
        // The server's own starter channels stay first; a name taken already gets a number.
        var imported = channels.Where(x => x.Position >= channels.Count - 7).ToList();
        Assert.Equal(
            ["news", "help", "town-hall", "general-2", "general-3", "staff", "lounge"],
            imported.Select(x => x.Name));
        Assert.Equal(
            [ChannelKind.Text, ChannelKind.Text, ChannelKind.Voice, ChannelKind.Text, ChannelKind.Text, ChannelKind.Text, ChannelKind.Voice],
            imported.Select(x => x.Kind));
        var general = imported.Single(x => x.Name == "general-3");
        Assert.Equal(("Hello", 30), (general.Topic, general.SlowModeSeconds));
        Assert.Equal(1024, imported.Single(x => x.Name == "help").Topic.Length);
        Assert.Equal(5, imported.Single(x => x.Name == "lounge").UserLimit);
        Assert.Equal(0, imported.Single(x => x.Name == "town-hall").UserLimit);
        var talk = await db.Sections.AsNoTracking().SingleAsync(x => x.ServerId == serverId && x.Name == "Talk");
        Assert.Equal(talk.Id, general.SectionId);
        Assert.All(imported, channel => Assert.True(ChannelEndpoints.IsValidName(channel.Name)));

        Assert.Equal((2, 7, 1), (first.Structure.SectionsMade, first.Structure.ChannelsMade, first.Structure.ThreadsRecorded));
        Assert.Equal(
            new Dictionary<string, DiscordChannelType>
            {
                ["130"] = DiscordChannelType.Forum,
                ["140"] = DiscordChannelType.Stage,
                ["150"] = DiscordChannelType.Announcements,
            },
            first.Structure.Simplified);
        Assert.Equal(["160"], first.Structure.HiddenFromEveryone);
        Assert.Equal((0, 0, 0, 10), (
            second.Structure.SectionsMade, second.Structure.ChannelsMade, second.Structure.ThreadsRecorded,
            second.Structure.AlreadyImported));

        var records = await db.DiscordImportChannels.AsNoTracking().Where(x => x.ServerId == serverId).ToListAsync();
        Assert.Equal(DiscordChannelType.Forum, records.Single(x => x.DiscordId == "130").DiscordType);
        var thread = records.Single(x => x.DiscordId == "170");
        Assert.Equal(
            (DiscordImportChannelKind.Thread, "130", records.Single(x => x.DiscordId == "130").ChannelId),
            (thread.Kind, thread.ParentDiscordId, thread.ChannelId));
        Assert.True(records.Single(x => x.DiscordId == "160").IsPrivate);
    }

    [Fact]
    public async Task HistoryKeepsItsAuthorsAndTimesAndASecondRunWritesNothing()
    {
        var guild = new DiscordFakeGuild()
            .Channel("100", "general")
            .Member("11", "jo", globalName: "Jo Global", nick: "Jo on Krew")
            .Member("12", "ada", globalName: null);
        guild.Message("1001", "100", "11", "first, by the person Discord calls jo", minute: 1);
        guild.Message("1002", "100", "12", "hi <@11> and <@13>, see <#100> and <@&5>", minute: 2, username: "ada", editedMinute: 5);
        var pinned = guild.Message("1003", "100", "11", "pinned", minute: 3, pinned: true);
        guild.Pin(pinned, minute: 4);
        // A webhook names every message as it likes; its placeholder keeps the
        // name the import met first, which is the newest.
        guild.Message("1004", "100", "", "from the hook", minute: 6, username: "Other name", webhookId: "77");
        guild.Message("1005", "100", "", "from the hook again", minute: 7, username: new string('W', 80), webhookId: "77");
        guild.Role("5", "Mods");
        var serverId = await NewServerAsync("ada");

        var first = await RunAsync(serverId, guild);
        int messages, users, changes;
        await using (var scope = factory.OpenScope(out var db))
        {
            (messages, users, changes) = (await db.Messages.CountAsync(), await db.Users.CountAsync(), await db.MessageChanges.CountAsync());
        }

        var second = await RunAsync(serverId, guild);

        Assert.Equal((5, 0, 3), (first.Written, first.AlreadySeen, first.PlaceholdersMade));
        Assert.Equal((0, 5, 0), (second.Written, second.AlreadySeen, second.PlaceholdersMade));
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.Equal((messages, users, changes), (await db.Messages.CountAsync(), await db.Users.CountAsync(), await db.MessageChanges.CountAsync()));
            var channelId = await ChannelIdAsync(db, serverId, "100");
            var stored = await db.Messages.AsNoTracking().Include(x => x.Author)
                .Where(x => x.ChannelId == channelId).OrderBy(x => x.CreatedAt).ToListAsync();
            Assert.Equal(
                ["discord:11", "discord:12", "discord:11", "discord:77", "discord:77"],
                stored.Select(x => x.Author.Username));
            Assert.Equal(["Jo on Krew", "ada", "Jo on Krew", new string('W', 64), new string('W', 64)], stored.Select(x => x.Author.DisplayName));
            Assert.All(stored, x => Assert.True(x.Author is { IsDisabled: true, PasswordHash: null, Email: null, Origin: AccountOrigin.Discord }));
            Assert.Equal(DiscordFakeGuild.Utc(2), stored[1].CreatedAt);
            Assert.Equal(DiscordFakeGuild.Utc(5), stored[1].EditedAt);
            Assert.Equal(DiscordFakeGuild.Utc(4), stored[2].PinnedAt);
            Assert.Null(stored[0].PinnedAt);
            Assert.Equal(DiscordImportNames.MessageId(serverId, "1002"), stored[1].Id);
            // Mentions are labels with the name shown; nobody is mentioned, not even the real jo.
            Assert.Equal(@"hi \@Jo on Krew and \@unknown-user, see #general-2 and \@Mods", stored[1].Content);
            Assert.All(stored, x => Assert.Empty(MessageMentions.ExtractNormalizedUsernames(x.Content)));
            Assert.False(await db.ServerMembers.AnyAsync(x => stored.Select(s => s.AuthorId).Contains(x.UserId)));
            Assert.Equal(3, await db.DiscordImportAccounts.CountAsync(x => stored.Select(s => s.AuthorId).Contains(x.UserId)));
            Assert.True((await db.DiscordImportAccounts.SingleAsync(x => x.DiscordId == "77")).IsWebhook);
        }

        // The routes that show history name the author as a placeholder.
        using var ada = await SignInAsync("ada");
        var history = await GetAsync(ada, serverId, $"/api/channels/{await ChannelIdAsync(serverId, "100")}/messages");
        Assert.All(history.GetProperty("messages").EnumerateArray(),
            x => Assert.Equal("discord", x.GetProperty("author").GetProperty("origin").GetString()));
    }

    [Fact]
    public async Task AReplyChainEndsOnItsRootWhicheverOrderThePagesCameIn()
    {
        foreach (var oldestFirst in new[] { false, true })
        {
            var guild = new DiscordFakeGuild().Channel("500", "general");
            // Ids across a digit boundary, so text order would be wrong.
            guild.Message("98", "500", "21", "a: the start", minute: 1);
            guild.Message("99", "500", "22", "b: answers a", minute: 2, replyTo: "98");
            guild.Message("100", "500", "21", "c: answers b", minute: 3, replyTo: "99");
            guild.Message("101", "500", "22", "d: answers something deleted", minute: 4, replyTo: "50");
            var serverId = await NewServerAsync("ada");

            await RunAsync(serverId, guild, pageSize: 1, oldestFirst: oldestFirst);

            var channel = await ChannelIdAsync(serverId, "500");
            await using var scope = factory.OpenScope(out var db);
            var replies = (await db.Messages.AsNoTracking().Where(x => x.ChannelId == channel).ToListAsync())
                .ToDictionary(x => x.Content[..1], x => x.ReplyToMessageId);
            var a = DiscordImportNames.MessageId(serverId, "98");
            Assert.Equal(
                new Dictionary<string, Guid?> { ["a"] = null, ["b"] = a, ["c"] = a, ["d"] = null },
                replies);

            using var ada = await SignInAsync("ada");
            var threads = await GetAsync(ada, serverId, $"/api/channels/{channel}/threads");
            var thread = Assert.Single(threads.EnumerateArray());
            Assert.Equal(a.ToString(), thread.GetProperty("root").GetProperty("id").GetString());
            Assert.Equal(2, thread.GetProperty("replyCount").GetInt32());
        }
    }

    [Fact]
    public async Task AThreadAnswersItsStarterAndAForumPostIsHeadedByItsName()
    {
        foreach (var threadsFirst in new[] { true, false })
        {
            var guild = new DiscordFakeGuild()
                .Channel("100", "general")
                .Channel("200", "help", type: 15)
                .Member("34", "jo");
            guild.Message("1000", "100", "31", "question", minute: 1);
            // The thread grew from 1001, which answers 1000: its replies go to 1000.
            guild.Message("1001", "100", "32", "starter", minute: 2, replyTo: "1000");
            guild.Thread("1001", "100", "side talk");
            guild.Message("1002", "1001", "", "", minute: 3, type: 21);
            guild.Message("1003", "1001", "33", "in the thread", minute: 4);
            guild.Message("1004", "1001", "31", "answers in the thread", minute: 5, replyTo: "1003");
            // A forum post: its first message opens it.
            guild.Thread("2001", "200", "@jo **x** <@34>");
            guild.Message("2001", "2001", "33", "how do I <@34>?", minute: 6);
            guild.Message("2002", "2001", "31", "like this", minute: 7);
            // A private thread grows from nothing.
            guild.Thread("3001", "100", "private", type: 12);
            guild.Message("3002", "3001", "32", "secret start", minute: 8);
            guild.Message("3003", "3001", "31", "secret reply", minute: 9);
            var serverId = await NewServerAsync("ada");

            var run = await RunAsync(serverId, guild, threadsFirst: threadsFirst);
            // Completing again changes nothing, and puts no second name on a root.
            int changes;
            await using (var scope = factory.OpenScope(out var db))
            {
                changes = await db.MessageChanges.CountAsync();
            }

            await CompleteAgainAsync(serverId, run.Import, "100", "200", "1001", "2001", "3001");

            await using (var scope = factory.OpenScope(out var db))
            {
                Assert.Equal(changes, await db.MessageChanges.CountAsync());
                var stored = await db.Messages.AsNoTracking()
                    .Where(x => db.Channels.Any(c => c.Id == x.ChannelId && c.ServerId == serverId))
                    .ToDictionaryAsync(x => x.Id);
                Guid Id(string discordId) => DiscordImportNames.MessageId(serverId, discordId);
                Assert.Equal(Id("1000"), stored[Id("1001")].ReplyToMessageId);
                Assert.Equal(Id("1000"), stored[Id("1003")].ReplyToMessageId);
                Assert.Equal(Id("1000"), stored[Id("1004")].ReplyToMessageId);
                Assert.False(stored.ContainsKey(Id("1002")));

                var post = stored[Id("2001")];
                Assert.Null(post.ReplyToMessageId);
                Assert.Equal(Id("2001"), stored[Id("2002")].ReplyToMessageId);
                Assert.Equal("**\\@jo \\*\\*x\\*\\* <\\@34>**\nhow do I \\@jo?", post.Content);
                Assert.Empty(MessageMentions.ExtractNormalizedUsernames(post.Content));

                Assert.Null(stored[Id("3002")].ReplyToMessageId);
                Assert.StartsWith("**private**\n", stored[Id("3002")].Content, StringComparison.Ordinal);
                Assert.Equal(Id("3002"), stored[Id("3003")].ReplyToMessageId);
                Assert.Equal(
                    [Id("1000"), Id("2001"), Id("3002")],
                    await db.DiscordImportChannels.AsNoTracking()
                        .Where(x => x.ServerId == serverId && x.Kind == DiscordImportChannelKind.Thread)
                        .OrderBy(x => x.DiscordId).Select(x => x.RootMessageId!.Value).ToListAsync());
            }

            if (threadsFirst)
            {
                // The text channel's thread had to wait for its channel.
                Assert.Contains(run.Links, x => x.Waiting);
            }
        }
    }

    [Fact]
    public async Task AThreadCompletedBeforeItsChannelWaitsForTheWholeHistory()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general");
        guild.Message("1000", "100", "35", "the question", minute: 1);
        guild.Message("1001", "100", "36", "the starter, which answers it", minute: 2, replyTo: "1000");
        guild.Thread("1001", "100", "side talk");
        guild.Message("1003", "1001", "35", "in the thread", minute: 3);
        var serverId = await NewServerAsync("ada");
        var bot = guild.Bot();
        var snapshot = await bot.ReadServerAsync(DiscordFakeGuild.Id, CancellationToken.None);
        await using var scope = factory.OpenScope(out var db);
        var writer = new DiscordImportWriter(db, new DiscordTestClock());
        var import = await writer.StartAsync(serverId, Accounts["ada"].Id, snapshot, CancellationToken.None);
        await writer.WriteStructureAsync(import, snapshot, CancellationToken.None);
        var history = await ToListAsync(bot.ReadMessagesAsync("100"));

        // The newest page holds the starter but not what the starter answers.
        await writer.WriteMessagesAsync(import, snapshot, "100", history.Take(1).ToList(), null, CancellationToken.None);
        await writer.WriteMessagesAsync(
            import, snapshot, "1001", await ToListAsync(bot.ReadMessagesAsync("1001")), null, CancellationToken.None);
        var early = await writer.CompleteChannelAsync(import, "1001", CancellationToken.None);
        await writer.WriteMessagesAsync(import, snapshot, "100", history.Skip(1).ToList(), null, CancellationToken.None);
        var late = await writer.CompleteChannelAsync(import, "100", CancellationToken.None);

        Assert.True(early.Waiting);
        Assert.Equal(1, late.ThreadsLinked);
        var root = DiscordImportNames.MessageId(serverId, "1000");
        foreach (var id in new[] { "1001", "1003" })
        {
            Assert.Equal(root, (await db.Messages.AsNoTracking()
                .SingleAsync(x => x.Id == DiscordImportNames.MessageId(serverId, id))).ReplyToMessageId);
        }
    }

    [Fact]
    public async Task AnIdAlreadyTakenOrATextTooLongIsLeftOutAndCounted()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general");
        guild.Message("1001", "100", "38", "somebody sent a message under my id first", minute: 1);
        guild.Message("1002", "100", "38", new string('x', MessageLimits.MaxContentLength + 1), minute: 2);
        guild.Message("1003", "100", "38", "fine", minute: 3);
        var serverId = await NewServerAsync("ada");
        using var ada = await SignInAsync("ada");
        Guid general;
        await using (var scope = factory.OpenScope(out var db))
        {
            general = await db.Channels.Where(x => x.ServerId == serverId && x.Name == "general").Select(x => x.Id).SingleAsync();
        }

        // A client chooses the id of what it sends.
        var squatted = await ada.SendAsync(new HttpRequestMessage(HttpMethod.Post, $"/api/channels/{general}/messages")
        {
            Content = JsonContent.Create(new { content = "mine", clientMessageId = DiscordImportNames.MessageId(serverId, "1001") }),
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        });
        squatted.EnsureSuccessStatusCode();

        var run = await RunAsync(serverId, guild);

        Assert.Equal(1, run.Written);
        Assert.Equal(
            new Dictionary<DiscordSkipReason, int> { [DiscordSkipReason.IdTaken] = 1, [DiscordSkipReason.TooLong] = 1 },
            run.Skipped);
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.Equal("mine", (await db.Messages.SingleAsync(x => x.Id == DiscordImportNames.MessageId(serverId, "1001"))).Content);
        }
    }

    [Fact]
    public async Task ADeletedChannelStaysDeletedAndTheRecordOutlivesItsServer()
    {
        var guild = new DiscordFakeGuild().Category("900", "Talk", 0).Channel("100", "general", parent: "900").Channel("101", "other");
        guild.Message("1001", "100", "39", "kept", minute: 1);
        guild.Message("1101", "101", "39", "in the channel that goes", minute: 2);
        var serverId = await NewServerAsync("ada");
        var first = await RunAsync(serverId, guild);
        using var ada = await SignInAsync("ada");
        var other = await ChannelIdAsync(serverId, "101");
        Assert.Equal(HttpStatusCode.NoContent, (await ada.SendAsync(new HttpRequestMessage(HttpMethod.Delete, $"/api/admin/channels/{other}")
        {
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        })).StatusCode);
        guild.Message("1102", "101", "39", "said on Discord after the channel went here", minute: 3);

        var again = await RunAsync(serverId, guild);

        Assert.Equal((0, 0), (again.Structure.ChannelsMade, again.Written));
        Assert.Equal(new Dictionary<DiscordSkipReason, int> { [DiscordSkipReason.ChannelRemoved] = 1 }, again.Skipped);
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.Null((await db.DiscordImportChannels.SingleAsync(x => x.ServerId == serverId && x.DiscordId == "101")).ChannelId);
        }

        Assert.Equal(HttpStatusCode.NoContent, (await ada.SendAsync(new HttpRequestMessage(HttpMethod.Delete, "/api/admin/server")
        {
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        })).StatusCode);
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.Equal(
                [(Guid?)null, null],
                await db.DiscordImports.Where(x => x.Id == first.Import.Id || x.Id == again.Import.Id).Select(x => x.ServerId).ToListAsync());
            Assert.False(await db.DiscordImportChannels.AnyAsync(x => x.ServerId == serverId));
            Assert.False(await db.DiscordImportSections.AnyAsync(x => x.ServerId == serverId));
            Assert.False(await db.DiscordImportMessages.AnyAsync(x => x.ServerId == serverId));
            // Accounts are not removed with a server: the placeholder and what made it stay for an undo.
            var account = await db.DiscordImportAccounts.SingleAsync(x => x.DiscordId == "39");
            Assert.Equal(first.Import.Id, account.ImportId);
            Assert.True(await db.Users.AnyAsync(x => x.Id == account.UserId && x.Origin == AccountOrigin.Discord));
        }
    }

    [Fact]
    public async Task WhatHasNoShapeIsLeftOutAndCountedOnce()
    {
        var guild = new DiscordFakeGuild()
            .Channel("100", "general")
            .Channel("110", "Lounge", type: 2);
        guild.Message("1001", "100", "41", "", minute: 1, type: 7);
        guild.Message("1002", "100", "41", "", minute: 2, type: 7);
        guild.Message("1003", "100", "41", "", minute: 3, type: 6);
        guild.Message("1004", "100", "41", "", minute: 4, type: 8);
        guild.Message("1005", "100", "41", "", minute: 5, forwarded: "somebody else's words");
        guild.Message("1006", "100", "41", "", minute: 6, sticker: true);
        guild.Message("1007", "100", "41", "", minute: 7, embed: true, bot: true);
        guild.Message("1008", "100", "41", "   ", minute: 8);
        guild.Message("1009", "100", "41", "", minute: 9, attachments: 2);
        guild.Message("1010", "100", "41", "look at this", minute: 10, attachments: 1);
        guild.Message("1011", "100", "", "nobody", minute: 11);
        guild.Message("1012", "110", "41", "said in voice", minute: 12);
        var serverId = await NewServerAsync("ada");

        var first = await RunAsync(serverId, guild);
        var again = await RunAsync(serverId, guild);

        Assert.Equal((1, 1, 1), (first.Written, first.AttachmentsPending, first.AwaitingAttachments));
        Assert.Equal(
            new Dictionary<DiscordSkipReason, int>
            {
                [DiscordSkipReason.Notice] = 4,
                [DiscordSkipReason.Forward] = 1,
                [DiscordSkipReason.StickerOnly] = 1,
                [DiscordSkipReason.EmbedOnly] = 1,
                [DiscordSkipReason.Empty] = 1,
                [DiscordSkipReason.NoAuthor] = 1,
                [DiscordSkipReason.VoiceChannel] = 1,
            },
            first.Skipped);
        Assert.Equal(new Dictionary<int, int> { [7] = 2, [6] = 1, [8] = 1 }, first.Notices);
        Assert.Equal((0, 12), (again.Written, again.AlreadySeen));

        await using var scope = factory.OpenScope(out var db);
        var tally = await new DiscordImportWriter(db, new DiscordTestClock()).TallyAsync(serverId, CancellationToken.None);
        Assert.Equal((1, 1, 1, 1), (tally.Written, tally.AttachmentsPending, tally.AwaitingAttachments, tally.Placeholders));
        Assert.Equal(first.Skipped, tally.Skipped);
        Assert.Equal(first.Notices, tally.NoticesByType);
        var waiting = await db.DiscordImportMessages.AsNoTracking().SingleAsync(x => x.ServerId == serverId && x.DiscordId == "1009");
        Assert.Equal((DiscordMessageOutcome.AwaitingAttachments, (Guid?)null, true), (waiting.Outcome, waiting.MessageId, waiting.AttachmentsPending));
    }

    [Fact]
    public async Task ImportedHistoryIsNotUnreadWithOrWithoutAMarker()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general");
        for (var minute = 1; minute <= 30; minute++)
        {
            guild.Message((1000 + minute).ToString(System.Globalization.CultureInfo.InvariantCulture), "100", "51", $"old news {minute}", minute);
        }

        var serverId = await NewServerAsync("ada", "nils", "ilya");
        // nils opened the channel long before anything in it was written; ilya never did.
        var run = await RunAsync(serverId, guild, afterStructure: async db =>
        {
            var channelId = await ChannelIdAsync(db, serverId, "100");
            db.ChannelReads.Add(new ChannelRead
            {
                ChannelId = channelId,
                UserId = Accounts["nils"].Id,
                LastReadAt = new DateTime(2020, 1, 1, 0, 0, 0, DateTimeKind.Utc),
            });
            await db.SaveChangesAsync();
        });

        Assert.Equal(30, run.Written);
        var channel = await ChannelIdAsync(serverId, "100");
        foreach (var name in new[] { "nils", "ilya" })
        {
            using var member = await SignInAsync(name);
            var bootstrap = await GetAsync(member, serverId, "/api/bootstrap");
            var marker = bootstrap.GetProperty("channelReads").GetProperty(channel.ToString()).GetDateTime();
            var history = await GetAsync(member, serverId, $"/api/channels/{channel}/messages?limit=100");
            var items = history.GetProperty("messages").EnumerateArray().ToList();
            Assert.Equal(30, items.Count);
            Assert.DoesNotContain(items, x => x.GetProperty("createdAt").GetDateTime() > marker);
        }
    }

    [Fact]
    public async Task ImportedHistoryIsFoundBySearch()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general").Member("61", "jo", nick: "Joanna");
        guild.Message("1001", "100", "61", "the quokka migration is on friday", minute: 1);
        guild.Message("1002", "100", "61", "nothing to see", minute: 2);
        var serverId = await NewServerAsync("ada");

        await RunAsync(serverId, guild);

        using var ada = await SignInAsync("ada");
        var channel = await ChannelIdAsync(serverId, "100");
        var found = await GetAsync(ada, serverId, $"/api/channels/{channel}/messages/search?query=quokka");
        Assert.Equal(
            [DiscordImportNames.MessageId(serverId, "1001").ToString()],
            found.EnumerateArray().Select(x => x.GetProperty("id").GetString()));
        var byAuthor = await GetAsync(ada, serverId, $"/api/channels/{channel}/messages/search?query=Joanna");
        Assert.Equal(2, byAuthor.GetArrayLength());
    }

    [Fact]
    public async Task AClientReloadsTheChannelOnceInsteadOfReplayingEveryImportedMessage()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general");
        for (var index = 0; index < 600; index++)
        {
            guild.Message((10_000 + index).ToString(System.Globalization.CultureInfo.InvariantCulture), "100", "71", $"message {index}", index);
        }

        var serverId = await NewServerAsync("ada");
        using var ada = await SignInAsync("ada");
        string? before = null, between = null;
        var clock = new DiscordTestClock();
        var run = await RunAsync(serverId, guild, pageSize: 300, clock: clock,
            afterStructure: async _ => before = await SyncCursorAsync(ada, serverId, await ChannelIdAsync(serverId, "100")),
            afterPage: async page =>
            {
                if (page == 1)
                {
                    between = await SyncCursorAsync(ada, serverId, await ChannelIdAsync(serverId, "100"));
                }
            });

        Assert.Equal(600, run.Written);
        var channel = await ChannelIdAsync(serverId, "100");
        foreach (var cursor in new[] { before!, between! })
        {
            var sync = await GetAsync(ada, serverId, $"/api/channels/{channel}/messages/sync?after={cursor}");
            Assert.True(sync.GetProperty("resetRequired").GetBoolean());
            var change = Assert.Single(sync.GetProperty("changes").EnumerateArray());
            Assert.Equal(Guid.Empty.ToString(), change.GetProperty("messageId").GetString());
        }

        var after = await SyncCursorAsync(ada, serverId, channel);
        var quiet = await GetAsync(ada, serverId, $"/api/channels/{channel}/messages/sync?after={after}");
        Assert.Empty(quiet.GetProperty("changes").EnumerateArray());
        Assert.False(quiet.GetProperty("resetRequired").GetBoolean());

        await using (var scope = factory.OpenScope(out var db))
        {
            var rows = await db.MessageChanges.AsNoTracking().Where(x => x.ChannelId == channel).ToListAsync();
            Assert.Equal(MessageChangeKind.Reset, Assert.Single(rows).Kind);
        }

        // 600 messages in pages of 300 are four transactions, each after a pause.
        Assert.True(clock.Waits.Count(x => x == DiscordImportWriter.PauseBetweenTransactions) >= 3);

        // A live change afterwards is journalled as ever.
        var removed = await ada.SendAsync(new HttpRequestMessage(HttpMethod.Delete, $"/api/messages/{DiscordImportNames.MessageId(serverId, "10000")}")
        {
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        });
        Assert.Equal(HttpStatusCode.NoContent, removed.StatusCode);
        var live = await GetAsync(ada, serverId, $"/api/channels/{channel}/messages/sync?after={after}");
        Assert.Equal("delete", Assert.Single(live.GetProperty("changes").EnumerateArray()).GetProperty("kind").GetString());
    }

    [Fact]
    public async Task NothingASendDoesHappensAndADeletedMessageStaysDeleted()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general", slowMode: 600).Member("81", "jo");
        guild.Message("1001", "100", "81", "hey @jo and @everyone, read https://example.com/news", minute: 1);
        guild.Message("1002", "100", "81", "<@81> again", minute: 2);
        var serverId = await NewServerAsync("ada", "jo");
        int notifications, deliveries, postTimes, audit, embeds;
        await using (var scope = factory.OpenScope(out var db))
        {
            (notifications, deliveries, postTimes, audit, embeds) = (
                await db.UserNotifications.CountAsync(), await db.NotificationDeliveries.CountAsync(),
                await db.ChannelPostTimes.CountAsync(), await db.ServerAuditEntries.CountAsync(),
                await db.MessageEmbeds.CountAsync());
        }

        var run = await RunAsync(serverId, guild);

        Assert.Equal(2, run.Written);
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.Equal(
                (notifications, deliveries, postTimes, audit, embeds),
                (await db.UserNotifications.CountAsync(), await db.NotificationDeliveries.CountAsync(),
                    await db.ChannelPostTimes.CountAsync(), await db.ServerAuditEntries.CountAsync(),
                    await db.MessageEmbeds.CountAsync()));
            var first = await db.Messages.AsNoTracking().SingleAsync(x => x.Id == DiscordImportNames.MessageId(serverId, "1001"));
            Assert.Equal(@"hey \@jo and \@everyone, read https://example.com/news", first.Content);
        }

        using var ada = await SignInAsync("ada");
        var id = DiscordImportNames.MessageId(serverId, "1002");
        var removed = await ada.SendAsync(new HttpRequestMessage(HttpMethod.Delete, $"/api/messages/{id}")
        {
            Headers = { { ServerAccess.HeaderName, serverId.ToString() } },
        });
        Assert.Equal(HttpStatusCode.NoContent, removed.StatusCode);

        var again = await RunAsync(serverId, guild);

        Assert.Equal((0, 2), (again.Written, again.AlreadySeen));
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.False(await db.Messages.AnyAsync(x => x.Id == id));
            Assert.Null((await db.DiscordImportMessages.SingleAsync(x => x.ServerId == serverId && x.DiscordId == "1002")).MessageId);
        }
    }

    [Fact]
    public async Task ATemplateCannotBeImportedAndAPageMustBeOfItsChannel()
    {
        var guild = new DiscordFakeGuild().Channel("100", "general").Channel("101", "other");
        guild.Message("1001", "101", "91", "elsewhere", minute: 1);
        var serverId = await NewServerAsync("ada");
        var bot = guild.Bot();
        var snapshot = await bot.ReadServerAsync(DiscordFakeGuild.Id, CancellationToken.None);
        await using var scope = factory.OpenScope(out var db);
        var writer = new DiscordImportWriter(db, new DiscordTestClock());

        await Assert.ThrowsAsync<ArgumentException>(() => writer.StartAsync(
            serverId, Accounts["ada"].Id, snapshot with { Source = SnapshotSource.Template }, CancellationToken.None));
        var import = await writer.StartAsync(serverId, Accounts["ada"].Id, snapshot, CancellationToken.None);
        await writer.WriteStructureAsync(import, snapshot, CancellationToken.None);
        var page = await ToListAsync(bot.ReadMessagesAsync("101"));
        await Assert.ThrowsAsync<ArgumentException>(() =>
            writer.WriteMessagesAsync(import, snapshot, "100", page, null, CancellationToken.None));
        await Assert.ThrowsAsync<ArgumentException>(() =>
            writer.WriteMessagesAsync(import, snapshot, "999", [], null, CancellationToken.None));
    }

    /* ------------------------------------------------------------- the run */

    private sealed record Run(
        DiscordImport Import,
        DiscordStructureResult Structure,
        IReadOnlyList<DiscordPageResult> Pages,
        IReadOnlyList<DiscordLinkResult> Links)
    {
        public int Written => Pages.Sum(x => x.Written);
        public int AlreadySeen => Pages.Sum(x => x.AlreadySeen);
        public int PlaceholdersMade => Pages.Sum(x => x.PlaceholdersMade);
        public int AttachmentsPending => Pages.Sum(x => x.AttachmentsPending);
        public int AwaitingAttachments => Pages.Sum(x => x.AwaitingAttachments);

        public Dictionary<DiscordSkipReason, int> Skipped => Pages.SelectMany(x => x.Skipped)
            .GroupBy(x => x.Key).ToDictionary(x => x.Key, x => x.Sum(y => y.Value));

        public Dictionary<int, int> Notices => Pages.SelectMany(x => x.NoticesByType)
            .GroupBy(x => x.Key).ToDictionary(x => x.Key, x => x.Sum(y => y.Value));
    }

    /// <summary>
    /// Imports the fake server as the next stage will: the structure, then
    /// every channel and thread in pages as the reader reads them, each
    /// completed after its last page.
    /// </summary>
    private async Task<Run> RunAsync(
        Guid serverId,
        DiscordFakeGuild guild,
        int pageSize = 100,
        bool oldestFirst = false,
        bool threadsFirst = false,
        DiscordTestClock? clock = null,
        Func<AppDbContext, Task>? afterStructure = null,
        Func<int, Task>? afterPage = null)
    {
        var bot = guild.Bot();
        var snapshot = await bot.ReadServerAsync(DiscordFakeGuild.Id, CancellationToken.None);
        await using var scope = factory.OpenScope(out var db);
        var writer = new DiscordImportWriter(db, clock ?? new DiscordTestClock());
        var import = await writer.StartAsync(serverId, Accounts["ada"].Id, snapshot, CancellationToken.None);
        var structure = await writer.WriteStructureAsync(import, snapshot, CancellationToken.None);
        if (afterStructure is not null)
        {
            await using var other = factory.OpenScope(out var otherDb);
            await afterStructure(otherDb);
        }

        var channels = snapshot.Channels.Select(x => x.Id).ToList();
        var threads = snapshot.Threads.Select(x => x.Id).ToList();
        var pages = new List<DiscordPageResult>();
        var links = new List<DiscordLinkResult>();
        var written = 0;
        foreach (var source in threadsFirst ? threads.Concat(channels) : channels.Concat(threads))
        {
            var pins = new Dictionary<string, DateTimeOffset>(StringComparer.Ordinal);
            await foreach (var pin in bot.ReadPinsAsync(source))
            {
                pins[pin.Message.Id] = pin.PinnedAt;
            }

            var history = (await ToListAsync(bot.ReadMessagesAsync(source))).Chunk(pageSize).ToList();
            if (oldestFirst)
            {
                history.Reverse();
            }

            foreach (var page in history)
            {
                pages.Add(await writer.WriteMessagesAsync(import, snapshot, source, page, pins, CancellationToken.None));
                if (afterPage is not null)
                {
                    await afterPage(++written);
                }
            }

            links.Add(await writer.CompleteChannelAsync(import, source, CancellationToken.None));
        }

        return new Run(import, structure, pages, links);
    }

    private async Task CompleteAgainAsync(Guid serverId, DiscordImport import, params string[] sources)
    {
        await using var scope = factory.OpenScope(out var db);
        var writer = new DiscordImportWriter(db, new DiscordTestClock());
        foreach (var source in sources)
        {
            await writer.CompleteChannelAsync(import, source, CancellationToken.None);
        }
    }

    /* ---------------------------------------------------------- the server */

    /// <summary>A server of its own for one test, owned by the first person, with the others as members.</summary>
    private async Task<Guid> NewServerAsync(string owner, params string[] members)
    {
        using var client = await SignInAsync(owner);
        var created = await client.PostAsJsonAsync("/api/servers", new
        {
            name = DataModelSupport.UniqueName()[..12],
            description = (string?)null,
            textChannelName = "general",
            voiceChannelName = "voice",
        });
        created.EnsureSuccessStatusCode();
        var serverId = Guid.Parse((await created.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!);
        await using var scope = factory.OpenScope(out var db);
        var role = await ServerAccess.FindMemberRoleAsync(db, serverId, CancellationToken.None);
        foreach (var member in members)
        {
            MemberRoles.Admit(db, new ServerMember
            {
                ServerId = serverId,
                UserId = Accounts[member].Id,
                RoleId = role!.Id,
                JoinedAt = DateTime.UtcNow.AddMinutes(5),
            }, isBaseRole: true);
        }

        await db.SaveChangesAsync();
        return serverId;
    }

    private async Task<Guid> ChannelIdAsync(Guid serverId, string discordId)
    {
        await using var scope = factory.OpenScope(out var db);
        return await ChannelIdAsync(db, serverId, discordId);
    }

    private static async Task<Guid> ChannelIdAsync(AppDbContext db, Guid serverId, string discordId) =>
        (await db.DiscordImportChannels.AsNoTracking()
            .SingleAsync(x => x.ServerId == serverId && x.DiscordId == discordId)).ChannelId!.Value;

    private async Task<HttpClient> SignInAsync(string username)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password = Accounts[username].Password }))
            .EnsureSuccessStatusCode();
        return client;
    }

    private static async Task<JsonElement> GetAsync(HttpClient client, Guid serverId, string path)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, path);
        request.Headers.Add(ServerAccess.HeaderName, serverId.ToString());
        using var response = await client.SendAsync(request);
        response.EnsureSuccessStatusCode();
        return await response.Content.ReadFromJsonAsync<JsonElement>();
    }

    private static async Task<string> SyncCursorAsync(HttpClient client, Guid serverId, Guid channelId) =>
        (await GetAsync(client, serverId, $"/api/channels/{channelId}/messages?limit=1")).GetProperty("syncCursor").GetString()!;

    private static async Task<List<T>> ToListAsync<T>(IAsyncEnumerable<T> items)
    {
        var list = new List<T>();
        await foreach (var item in items)
        {
            list.Add(item);
        }

        return list;
    }
}
