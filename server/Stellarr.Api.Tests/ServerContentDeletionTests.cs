using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Maintenance;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Tests;

/// <summary>
/// What a server or a channel leaves behind when it is deleted through the
/// routes people use: nothing of what was written in it, in any table that
/// holds the transcript, its search or its journal, and none of its files once
/// maintenance has run. What was written beside it comes through untouched.
/// </summary>
/// <remarks>
/// Every test runs a host of its own, because it deletes what it seeds. The
/// content is written straight into the database, so that every table holds
/// rows before the delete; the triggers fill the derived ones as they do for
/// the endpoints.
/// </remarks>
public sealed class ServerContentDeletionTests
{
    private static readonly Guid MajaId = Guid.Parse("30000000-0000-0000-0000-000000000001");
    private static readonly Guid JoId = Guid.Parse("30000000-0000-0000-0000-000000000005");

    [Theory]
    [InlineData("owner")]
    [InlineData("cockpit")]
    public async Task DeletingAServerLeavesNothingOfWhatWasWrittenInIt(string route)
    {
        using var factory = new StellarrWebApplicationFactory();
        using var jo = await SignInAsync(factory, "jo", "Corona!2026");
        var serverId = await CreateServerAsync(jo);
        var doomed = await SeedAsync(factory, serverId, extraTextChannels: 2);
        var kept = await SeedAsync(factory, TestData.ServerId, extraTextChannels: 0);
        var keptBefore = await CountAsync(factory, kept.Channels);
        Assert.All(await CountAsync(factory, doomed.Channels), table => Assert.True(table.Value > 0, table.Key));

        await DeleteServerAsync(factory, jo, serverId, route);

        AssertNoneLeft(await CountAsync(factory, doomed.Channels));
        Assert.Equal(keptBefore, await CountAsync(factory, kept.Channels));
    }

    [Fact]
    public async Task DeletingAChannelLeavesNothingOfWhatWasWrittenInIt()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var jo = await SignInAsync(factory, "jo", "Corona!2026");
        var serverId = await CreateServerAsync(jo);
        var seeded = await SeedAsync(factory, serverId, extraTextChannels: 1);
        SeededChannel[] doomed = [seeded.Channels[^1]];
        SeededChannel[] kept = [seeded.Channels[0]];
        var keptBefore = await CountAsync(factory, kept);
        Assert.All(await CountAsync(factory, doomed), table => Assert.True(table.Value > 0, table.Key));

        using var deleted = await jo.SendAsync(
            WithServer(HttpMethod.Delete, $"/api/admin/channels/{doomed[0].Id}", serverId));

        Assert.Equal(HttpStatusCode.NoContent, deleted.StatusCode);
        AssertNoneLeft(await CountAsync(factory, doomed));
        Assert.Equal(keptBefore, await CountAsync(factory, kept));
        // A notification belongs to the inbox of the person it went to: it
        // stays there, and only stops pointing at the channel.
        await using var scope = factory.OpenScope(out var db);
        Assert.True(await db.UserNotifications.AnyAsync(x => x.Body == doomed[0].Token && x.TargetChannelId == null));
    }

    [Theory]
    [InlineData("owner")]
    [InlineData("cockpit")]
    [InlineData("channel")]
    public async Task TheStoredFilesOfDeletedContentGoWithTheNextMaintenance(string route)
    {
        using var factory = new StellarrWebApplicationFactory();
        using var jo = await SignInAsync(factory, "jo", "Corona!2026");
        var serverId = await CreateServerAsync(jo);
        var seeded = await SeedAsync(factory, serverId, extraTextChannels: 1, serverFiles: route != "channel");
        var (doomed, kept) = route == "channel"
            ? (Files.Of(seeded.Channels[^1]), Files.Of(seeded.Channels[0]))
            : (Files.Of(seeded), Files.Of(await SeedAsync(factory, TestData.ServerId, extraTextChannels: 0)));
        var storage = (FakeObjectStorage)factory.Services.GetRequiredService<IObjectStorage>();
        var before = await StoredKeysAsync(storage);
        Assert.All(doomed.Objects.Keys.Concat(kept.Objects.Keys), key => Assert.Contains(key, before));

        if (route == "channel")
        {
            using var deleted = await jo.SendAsync(
                WithServer(HttpMethod.Delete, $"/api/admin/channels/{seeded.Channels[^1].Id}", serverId));
            Assert.Equal(HttpStatusCode.NoContent, deleted.StatusCode);
        }
        else
        {
            await DeleteServerAsync(factory, jo, serverId, route);
        }

        // Past the retention of an unused upload, which the sweep waits out.
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            await scope.ServiceProvider.GetRequiredService<InstanceMaintenance>()
                .RunAsync(DateTime.UtcNow.AddDays(2), CancellationToken.None);
        }

        var after = await StoredKeysAsync(storage);
        await using (var scope = factory.OpenScope(out var db))
        {
            var doomedKeys = doomed.Objects.Keys.ToArray();
            var rows = await db.ImageAttachments.AsNoTracking()
                .Where(x => doomed.UploadIds.Contains(x.Id)).Select(x => x.ObjectKey).ToListAsync();
            var queued = await db.ObjectDeletions.AsNoTracking()
                .Where(x => doomedKeys.Contains(x.ObjectKey)).Select(x => x.ObjectKey).ToListAsync();
            AssertNoneLeft(rows.Select(key => $"ImageAttachments row of {doomed.Objects[key]}")
                .Concat(queued.Select(key => $"ObjectDeletions entry of {doomed.Objects[key]}"))
                .Concat(doomedKeys.Where(after.Contains).Select(key => $"stored {doomed.Objects[key]}"))
                .GroupBy(x => x, StringComparer.Ordinal)
                .ToDictionary(x => x.Key, x => x.Count()));
            Assert.Equal(
                kept.UploadIds.Length,
                await db.ImageAttachments.CountAsync(x => kept.UploadIds.Contains(x.Id)));
        }

        Assert.All(kept.Objects.Keys, key => Assert.Contains(key, after));
    }

    /// <summary>Fails with every table that still holds rows, so that one run names them all.</summary>
    private static void AssertNoneLeft(IDictionary<string, int> tables)
    {
        var left = tables.Where(x => x.Value > 0).Select(x => $"{x.Key}: {x.Value}").ToArray();
        if (left.Length > 0)
        {
            Assert.Fail($"Left behind: {string.Join("; ", left)}");
        }
    }

    /// <summary>
    /// The rows left of what was written in the channels, table by table. A
    /// derived table is counted by the message or channel it belongs to, and
    /// the full-text index by the word that only each channel's seed contains.
    /// </summary>
    private static async Task<SortedDictionary<string, int>> CountAsync(
        StellarrWebApplicationFactory factory,
        IReadOnlyList<SeededChannel> seeded)
    {
        await using var scope = factory.OpenScope(out var db);
        var channels = seeded.Select(x => x.Id).ToArray();
        var messages = seeded.SelectMany(x => x.MessageIds).ToArray();
        return new SortedDictionary<string, int>(StringComparer.Ordinal)
        {
            ["Channels"] = await db.Channels.CountAsync(x => channels.Contains(x.Id)),
            ["Messages"] = await db.Messages.CountAsync(x => channels.Contains(x.ChannelId)),
            ["Messages pinned"] = await db.Messages.CountAsync(x => channels.Contains(x.ChannelId) && x.PinnedAt != null),
            ["MessageAttachments"] = await db.MessageAttachments.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageReactions"] = await db.MessageReactions.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageEmbeds"] = await db.MessageEmbeds.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageIndexEntries"] = await db.MessageIndexEntries.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageSearch"] = await SearchHitsAsync(db, seeded),
            ["MessageChanges"] = await db.MessageChanges.CountAsync(x => channels.Contains(x.ChannelId)),
            ["ChannelReads"] = await db.ChannelReads.CountAsync(x => channels.Contains(x.ChannelId)),
            ["ChannelPostTimes"] = await db.ChannelPostTimes.CountAsync(x => channels.Contains(x.ChannelId)),
            ["UserNotifications"] = await db.UserNotifications.CountAsync(x =>
                x.TargetChannelId != null && channels.Contains(x.TargetChannelId.Value)),
        };
    }

    private static async Task<int> SearchHitsAsync(AppDbContext db, IEnumerable<SeededChannel> seeded)
    {
        var hits = 0;
        foreach (var channel in seeded)
        {
            hits += await db.Database
                .SqlQuery<int>($"""SELECT count(*) AS "Value" FROM "MessageSearch" WHERE "MessageSearch" MATCH {channel.Token}""")
                .SingleAsync();
        }

        return hits;
    }

    /// <summary>
    /// Writes into every text channel of the server, after adding the ones
    /// asked for: a pinned message with an image, a reaction and a link card
    /// with its preview, a reply to it, both authors' read markers and a
    /// notification that points at the channel. On the instance server, whose
    /// channels other tests use, it writes into one channel of its own. With
    /// <paramref name="serverFiles"/> the server also gets an icon, an emoji
    /// and a soundboard clip.
    /// </summary>
    private static async Task<SeededServer> SeedAsync(
        StellarrWebApplicationFactory factory,
        Guid serverId,
        int extraTextChannels,
        bool serverFiles = false)
    {
        var now = DateTime.UtcNow;
        await using var scope = factory.OpenScope(out var db);
        var added = Enumerable.Range(0, serverId == TestData.ServerId ? 1 : extraTextChannels)
            .Select(i => new Channel
            {
                Id = Guid.NewGuid(),
                ServerId = serverId,
                Name = $"k{Guid.NewGuid():N}"[..20],
                Kind = ChannelKind.Text,
                Position = 10 + i,
            })
            .ToArray();
        db.Channels.AddRange(added);
        await db.SaveChangesAsync();
        var channelIds = serverId == TestData.ServerId
            ? added.Select(x => x.Id).ToArray()
            : await db.Channels.Where(x => x.ServerId == serverId && x.Kind == ChannelKind.Text)
                .OrderBy(x => x.Position).Select(x => x.Id).ToArrayAsync();

        var channels = new List<SeededChannel>();
        foreach (var channelId in channelIds)
        {
            var token = $"zz{Guid.NewGuid():N}";
            var image = Upload(JoId, "zdjęcie.png", withPreview: true);
            var cardImage = Upload(Guid.Empty, "karta.jpg", withPreview: false);
            var first = new Message
            {
                Id = Guid.NewGuid(),
                ChannelId = channelId,
                AuthorId = JoId,
                Content = $"{token} pierwsza https://cards.example/{token}",
                CreatedAt = now,
                PinnedAt = now,
                Attachments = [new MessageAttachment { AttachmentId = image.Id }],
            };
            var reply = new Message
            {
                Id = Guid.NewGuid(),
                ChannelId = channelId,
                AuthorId = MajaId,
                Content = $"{token} odpowiedź",
                CreatedAt = now.AddSeconds(1),
                ReplyToMessageId = first.Id,
            };
            db.ImageAttachments.AddRange(image, cardImage);
            db.Messages.AddRange(first, reply);
            db.MessageReactions.Add(new MessageReaction
            {
                MessageId = first.Id,
                UserId = MajaId,
                Emoji = "👍",
                CreatedAt = now,
            });
            db.MessageEmbeds.Add(new MessageEmbed
            {
                Id = Guid.NewGuid(),
                MessageId = first.Id,
                Url = $"https://cards.example/{token}",
                Kind = EmbedKind.Link,
                Provider = "cards.example",
                Domain = "cards.example",
                Title = $"Karta {token}",
                ThumbnailAttachmentId = cardImage.Id,
                Position = 0,
                CreatedAt = now,
            });
            db.ChannelReads.AddRange(
                new ChannelRead { ChannelId = channelId, UserId = JoId, LastReadAt = now },
                new ChannelRead { ChannelId = channelId, UserId = MajaId, LastReadAt = now });
            db.UserNotifications.Add(Mention(serverId, channelId, token, now));
            channels.Add(new SeededChannel(channelId, token, [first.Id, reply.Id], [image, cardImage]));
        }

        var serverUploads = new List<ImageAttachment>();
        var emojiKeys = new List<string>();
        if (serverFiles)
        {
            var icon = Upload(JoId, "ikona.png", withPreview: false);
            var clip = Upload(JoId, "klakson.ogg", withPreview: false);
            serverUploads.AddRange([icon, clip]);
            db.ImageAttachments.AddRange(icon, clip);
            db.SoundboardSounds.Add(new SoundboardSound
            {
                Id = Guid.NewGuid(),
                ServerId = serverId,
                AttachmentId = clip.Id,
                CreatedById = JoId,
                Name = "klakson",
                Glyph = "x",
                TrimEndSeconds = 1,
                CreatedAt = now,
            });
            var emoji = new ServerEmoji
            {
                Id = Guid.NewGuid(),
                ServerId = serverId,
                Shortcode = $"e{Guid.NewGuid():N}"[..20],
                ObjectKey = $"emoji/{Guid.NewGuid():N}",
                ContentType = "image/png",
                SizeBytes = 4,
                UploadedById = JoId,
                CreatedAt = now,
            };
            emojiKeys.Add(emoji.ObjectKey);
            db.ServerEmoji.Add(emoji);
            await db.SaveChangesAsync();
            await db.Servers.Where(x => x.Id == serverId)
                .ExecuteUpdateAsync(x => x.SetProperty(server => server.IconUrl, $"/assets/{icon.Id}"));
        }

        await db.SaveChangesAsync();
        var seeded = new SeededServer(serverId, channels, serverUploads, emojiKeys);
        var storage = (FakeObjectStorage)factory.Services.GetRequiredService<IObjectStorage>();
        foreach (var key in Files.Of(seeded).Objects.Keys)
        {
            await storage.PutAsync(key, new MemoryStream([1, 2, 3, 4]), CancellationToken.None);
        }

        return seeded;
    }

    private static UserNotification Mention(Guid serverId, Guid channelId, string token, DateTime now) => new()
    {
        Id = Guid.NewGuid(),
        ServerId = serverId,
        UserId = MajaId,
        Source = "system",
        SourcePackageId = "stellarr",
        SourcePackageName = "Stellarr",
        SourceModuleId = "mention",
        SourceModuleName = "Jo",
        Title = "Jo mentioned you",
        Body = token,
        Tone = "info",
        TargetChannelId = channelId,
        CreatedAt = now,
    };

    /// <summary>An upload from a few days ago, so that only a reference to it keeps it.</summary>
    private static ImageAttachment Upload(Guid uploaderId, string fileName, bool withPreview) => new()
    {
        Id = Guid.NewGuid(),
        UploadedById = uploaderId,
        ObjectKey = $"uploads/{Guid.NewGuid():N}",
        FileName = fileName,
        ContentType = "application/octet-stream",
        SizeBytes = 4,
        PreviewObjectKey = withPreview ? $"previews/{Guid.NewGuid():N}" : null,
        CreatedAt = DateTime.UtcNow.AddDays(-3),
    };

    private static async Task<HashSet<string>> StoredKeysAsync(FakeObjectStorage storage)
    {
        var keys = new HashSet<string>(StringComparer.Ordinal);
        await foreach (var stored in storage.ListAsync(CancellationToken.None))
        {
            keys.Add(stored.Key);
        }

        return keys;
    }

    private static async Task DeleteServerAsync(
        StellarrWebApplicationFactory factory,
        HttpClient owner,
        Guid serverId,
        string route)
    {
        if (route == "owner")
        {
            using var response = await owner.SendAsync(WithServer(HttpMethod.Delete, "/api/admin/server", serverId));
            Assert.Equal(HttpStatusCode.NoContent, response.StatusCode);
            return;
        }

        using var instanceOwner = await SignInAsync(factory, "maja", "Aurora!2026");
        using var deleted = await instanceOwner.DeleteAsync($"/api/cockpit/servers/{serverId}");
        Assert.Equal(HttpStatusCode.NoContent, deleted.StatusCode);
    }

    private static async Task<Guid> CreateServerAsync(HttpClient client)
    {
        using var response = await client.PostAsJsonAsync("/api/servers", new { name = "Do usunięcia" });
        response.EnsureSuccessStatusCode();
        var created = await response.Content.ReadFromJsonAsync<JsonElement>();
        return Guid.Parse(created.GetProperty("id").GetString()!);
    }

    private static HttpRequestMessage WithServer(HttpMethod method, string path, Guid serverId)
    {
        var request = new HttpRequestMessage(method, path);
        request.Headers.Add("X-Stellarr-Server", serverId.ToString());
        return request;
    }

    private static async Task<HttpClient> SignInAsync(
        StellarrWebApplicationFactory factory,
        string username,
        string password)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password })).EnsureSuccessStatusCode();
        return client;
    }

    /// <param name="Uploads">The message's image and the link card's preview, in that order.</param>
    private sealed record SeededChannel(Guid Id, string Token, Guid[] MessageIds, ImageAttachment[] Uploads);

    /// <param name="Uploads">What the server itself uses: its icon and its soundboard clip.</param>
    private sealed record SeededServer(
        Guid Id,
        IReadOnlyList<SeededChannel> Channels,
        IReadOnlyList<ImageAttachment> Uploads,
        IReadOnlyList<string> EmojiKeys);

    /// <summary>The uploads of a seed and every stored object behind them.</summary>
    /// <param name="Objects">Each object key, with what it holds.</param>
    private sealed record Files(Guid[] UploadIds, IReadOnlyDictionary<string, string> Objects)
    {
        internal static Files Of(SeededChannel channel) => Of(channel.Uploads, []);

        internal static Files Of(SeededServer server) =>
            Of(server.Channels.SelectMany(x => x.Uploads).Concat(server.Uploads), server.EmojiKeys);

        private static Files Of(IEnumerable<ImageAttachment> uploads, IEnumerable<string> emojiKeys)
        {
            var all = uploads.ToArray();
            var objects = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (var upload in all)
            {
                objects[upload.ObjectKey] = upload.FileName;
                if (upload.PreviewObjectKey is { } preview)
                {
                    objects[preview] = $"{upload.FileName} preview";
                }
            }

            foreach (var key in emojiKeys)
            {
                objects[key] = "emoji";
            }

            return new Files(all.Select(x => x.Id).ToArray(), objects);
        }
    }
}
