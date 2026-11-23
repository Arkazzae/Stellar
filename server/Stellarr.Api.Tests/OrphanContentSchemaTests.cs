using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// A database in which a server and a channel were deleted before their
/// content went with them: the next start removes what they left behind, and
/// leaves what is still in use as it was, direct messages included.
/// </summary>
public sealed class OrphanContentSchemaTests
{
    [Fact]
    public async Task TheNextStartRemovesWhatADeletedServerOrChannelLeftAndNothingElse()
    {
        await using var connection = new SqliteConnection(DatabaseConnection.From("Data Source=:memory:"));
        await connection.OpenAsync();
        await using var db = new AppDbContext(
            new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        var author = User("autorka");
        var reader = User("czytelnik");
        db.Users.AddRange(author, reader);
        var live = Server("Zostaje");
        var doomed = Server("Usunięty dawniej");
        db.Servers.AddRange(live, doomed);
        var kept = Channel(live);
        var removedEarlier = Channel(live);
        var ofDoomed = Channel(doomed);
        db.Channels.AddRange(kept, removedEarlier, ofDoomed);
        await db.SaveChangesAsync();
        var keptContent = await WriteAsync(db, kept, author, reader);
        var removedContent = await WriteAsync(db, removedEarlier, author, reader);
        var doomedContent = await WriteAsync(db, ofDoomed, author, reader);
        var direct = await WriteDirectAsync(db, author, reader);

        // Rows whose parent is spelt in other letters: nothing to remove.
        var otherCase = Guid.NewGuid();
        await db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            INSERT INTO "Channels" ("Id", "ServerId", "Name", "Kind", "Position", "Topic", "SlowModeSeconds", "HideAfterInactivityHours", "UserLimit")
            VALUES ({otherCase.ToString().ToUpperInvariant()}, {live.Id.ToString().ToLowerInvariant()}, 'inna-pisownia', 0, 9, '', 0, 0, 0);
            INSERT INTO "Messages" ("Id", "ChannelId", "AuthorId", "IsSystem", "Content", "CreatedAt")
            VALUES ({Guid.NewGuid().ToString().ToUpperInvariant()}, {kept.Id.ToString().ToLowerInvariant()}, {author.Id.ToString().ToUpperInvariant()}, 1, 'inna pisownia', {DateTime.UtcNow});
            """);

        // As it was before: the server and the channel went alone.
        await db.Database.ExecuteSqlRawAsync(
            """
            DROP TRIGGER "Servers_Channels_AfterDelete";
            DROP TRIGGER "Channels_Messages_AfterDelete";
            """);
        await db.Servers.Where(x => x.Id == doomed.Id).ExecuteDeleteAsync();
        await db.Channels.Where(x => x.Id == removedEarlier.Id).ExecuteDeleteAsync();
        // A link card that was built after its channel was gone wrote a
        // journal entry for that channel.
        db.MessageEmbeds.Add(Card(removedContent.MessageIds[1], removedContent.Token));
        await db.SaveChangesAsync();
        db.ChangeTracker.Clear();

        var leftovers = new[] { removedContent, doomedContent };
        Assert.All(await CountAsync(db, leftovers), table => Assert.True(table.Value > 0, table.Key));
        var keptBefore = await CountAsync(db, [keptContent]);
        var everythingElse = await EverythingElseAsync(db, direct, live.Id);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        var left = (await CountAsync(db, leftovers)).Where(x => x.Value > 0).Select(x => $"{x.Key}: {x.Value}");
        Assert.Empty(left);
        Assert.Equal(keptBefore, await CountAsync(db, [keptContent]));
        Assert.Equal(everythingElse, await EverythingElseAsync(db, direct, live.Id));
    }

    [Fact]
    public async Task ADatabaseWithoutLeftoversLosesNothing()
    {
        await using var connection = new SqliteConnection(DatabaseConnection.From("Data Source=:memory:"));
        await connection.OpenAsync();
        await using var db = new AppDbContext(
            new DbContextOptionsBuilder<AppDbContext>().UseSqlite(connection).Options);
        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
        var author = User("autorka");
        var reader = User("czytelnik");
        db.Users.AddRange(author, reader);
        var server = Server("Orbita");
        db.Servers.Add(server);
        var channels = new[] { Channel(server), Channel(server) };
        db.Channels.AddRange(channels);
        await db.SaveChangesAsync();
        var written = new List<Written>();
        foreach (var channel in channels)
        {
            written.Add(await WriteAsync(db, channel, author, reader));
        }

        var direct = await WriteDirectAsync(db, author, reader);
        var before = await CountAsync(db, written);
        var rest = await EverythingElseAsync(db, direct, server.Id);

        await DatabaseSchema.PrepareAsync(db, CancellationToken.None);

        Assert.Equal(before, await CountAsync(db, written));
        Assert.Equal(rest, await EverythingElseAsync(db, direct, server.Id));
    }

    /// <summary>What was written in the channels, table by table.</summary>
    private static async Task<SortedDictionary<string, int>> CountAsync(AppDbContext db, IReadOnlyList<Written> written)
    {
        var channels = written.Select(x => x.ChannelId).ToArray();
        var messages = written.SelectMany(x => x.MessageIds).ToArray();
        var search = 0;
        foreach (var token in written.Select(x => x.Token))
        {
            search += await db.Database
                .SqlQuery<int>($"""SELECT count(*) AS "Value" FROM "MessageSearch" WHERE "MessageSearch" MATCH {token}""")
                .SingleAsync();
        }

        return new SortedDictionary<string, int>(StringComparer.Ordinal)
        {
            ["Messages"] = await db.Messages.CountAsync(x => channels.Contains(x.ChannelId)),
            ["MessageAttachments"] = await db.MessageAttachments.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageReactions"] = await db.MessageReactions.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageEmbeds"] = await db.MessageEmbeds.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageIndexEntries"] = await db.MessageIndexEntries.CountAsync(x => messages.Contains(x.MessageId)),
            ["MessageSearch"] = search,
            ["MessageChanges"] = await db.MessageChanges.CountAsync(x => channels.Contains(x.ChannelId)),
            // The channel deleted earlier took its read markers and post times
            // by foreign key; the channels of the deleted server still hold theirs.
            ["Channels, read markers and post times"] =
                await db.Channels.CountAsync(x => channels.Contains(x.Id)) +
                await db.ChannelReads.CountAsync(x => channels.Contains(x.ChannelId)) +
                await db.ChannelPostTimes.CountAsync(x => channels.Contains(x.ChannelId)),
        };
    }

    /// <summary>
    /// Whatever the step must not touch: the server, every upload, the direct
    /// conversation, and the rows whose parent's id is spelt in other letters.
    /// </summary>
    /// <remarks>Uploads go only by the maintenance sweep, once nothing refers to them.</remarks>
    private static async Task<string> EverythingElseAsync(AppDbContext db, Guid conversationId, Guid serverId) =>
        string.Join(
            ", ",
            $"servers {await db.Servers.CountAsync(x => x.Id == serverId)}",
            $"uploads {await db.ImageAttachments.CountAsync()}",
            $"users {await db.Users.CountAsync()}",
            $"conversations {await db.DirectConversations.CountAsync(x => x.Id == conversationId)}",
            $"direct messages {await db.DirectMessages.CountAsync(x => x.ConversationId == conversationId)}",
            $"direct attachments {await db.DirectMessageAttachments.CountAsync()}",
            $"direct reads {await db.DirectConversationReads.CountAsync(x => x.ConversationId == conversationId)}",
            $"other-case channels {await RawCountAsync(db, """SELECT count(*) AS "Value" FROM "Channels" WHERE "Name" = 'inna-pisownia'""")}",
            $"other-case messages {await RawCountAsync(db, """SELECT count(*) AS "Value" FROM "Messages" WHERE "Content" = 'inna pisownia'""")}",
            $"other-case journal {await RawCountAsync(db, """SELECT count(*) AS "Value" FROM "MessageChanges" WHERE "ChannelId" IN (SELECT "ChannelId" FROM "Messages" WHERE "Content" = 'inna pisownia')""")}");

    private static Task<int> RawCountAsync(AppDbContext db, string sql) =>
        db.Database.SqlQueryRaw<int>(sql).SingleAsync();

    /// <summary>
    /// A message with an upload, a reaction and a link card, and a reply to
    /// it, under a word of their own; and a read marker for the reader.
    /// </summary>
    private static async Task<Written> WriteAsync(AppDbContext db, Channel channel, User author, User reader)
    {
        var token = $"zz{Guid.NewGuid():N}";
        var now = DateTime.UtcNow;
        var upload = Upload(author);
        var first = new Message
        {
            Id = Guid.NewGuid(),
            ChannelId = channel.Id,
            AuthorId = author.Id,
            Content = $"{token} pierwsza",
            CreatedAt = now,
            PinnedAt = now,
            Attachments = [new MessageAttachment { AttachmentId = upload.Id }],
        };
        var reply = new Message
        {
            Id = Guid.NewGuid(),
            ChannelId = channel.Id,
            AuthorId = reader.Id,
            Content = $"{token} odpowiedź",
            CreatedAt = now.AddSeconds(1),
            ReplyToMessageId = first.Id,
        };
        db.ImageAttachments.Add(upload);
        db.Messages.AddRange(first, reply);
        db.MessageReactions.Add(new MessageReaction { MessageId = first.Id, UserId = reader.Id, Emoji = "👍", CreatedAt = now });
        db.MessageEmbeds.Add(Card(first.Id, token));
        db.ChannelReads.Add(new ChannelRead { ChannelId = channel.Id, UserId = reader.Id, LastReadAt = now });
        await db.SaveChangesAsync();
        return new Written(channel.Id, token, [first.Id, reply.Id]);
    }

    /// <returns>The conversation's id.</returns>
    private static async Task<Guid> WriteDirectAsync(AppDbContext db, User author, User reader)
    {
        var now = DateTime.UtcNow;
        var conversation = new DirectConversation
        {
            Id = Guid.NewGuid(),
            UserAId = author.Id,
            UserBId = reader.Id,
            CreatedAt = now,
        };
        var upload = Upload(author);
        var message = new DirectMessage
        {
            Id = Guid.NewGuid(),
            ConversationId = conversation.Id,
            AuthorId = author.Id,
            Content = "prywatnie",
            CreatedAt = now,
            Attachments = [new DirectMessageAttachment { AttachmentId = upload.Id }],
        };
        db.DirectConversations.Add(conversation);
        db.ImageAttachments.Add(upload);
        db.DirectMessages.Add(message);
        db.DirectConversationReads.Add(new DirectConversationRead
        {
            ConversationId = conversation.Id,
            UserId = reader.Id,
            LastReadAt = now,
        });
        await db.SaveChangesAsync();
        return conversation.Id;
    }

    private static MessageEmbed Card(Guid messageId, string token) => new()
    {
        Id = Guid.NewGuid(),
        MessageId = messageId,
        Url = $"https://cards.example/{token}",
        Kind = EmbedKind.Link,
        Provider = "cards.example",
        Domain = "cards.example",
        Title = $"Karta {token}",
        Position = 0,
        CreatedAt = DateTime.UtcNow,
    };

    private static User User(string name) => new()
    {
        Id = Guid.NewGuid(),
        Username = name,
        NormalizedUsername = name.ToUpperInvariant(),
        DisplayName = name,
        PasswordHash = "hash",
        AvatarColor = "#777777",
        CreatedAt = DateTime.UtcNow,
    };

    private static StellarrServer Server(string name) =>
        new() { Id = Guid.NewGuid(), Name = name, CreatedAt = DateTime.UtcNow };

    private static Channel Channel(StellarrServer server) => new()
    {
        Id = Guid.NewGuid(),
        ServerId = server.Id,
        Name = $"k{Guid.NewGuid():N}"[..20],
        Kind = ChannelKind.Text,
    };

    private static ImageAttachment Upload(User uploader) => new()
    {
        Id = Guid.NewGuid(),
        UploadedById = uploader.Id,
        ObjectKey = $"uploads/{Guid.NewGuid():N}",
        FileName = "zdjęcie.png",
        ContentType = "image/png",
        SizeBytes = 4,
        CreatedAt = DateTime.UtcNow,
    };

    private sealed record Written(Guid ChannelId, string Token, Guid[] MessageIds);
}
