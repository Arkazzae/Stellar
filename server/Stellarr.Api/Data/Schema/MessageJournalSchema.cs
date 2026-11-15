using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Records changes to messages in the journal that clients replay after a
/// dropped connection. Triggers rather than the endpoints write it, so that
/// every writer is covered, and a reaction or a link preview added or removed
/// counts as a change of its message.
/// </summary>
internal static class MessageJournalSchema
{
    private const string TriggersSql =
        """
        CREATE TRIGGER IF NOT EXISTS "Messages_Changes_AfterInsert"
        AFTER INSERT ON "Messages"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            VALUES (new."ChannelId", new."Id", 0, strftime('%Y-%m-%d %H:%M:%f', 'now'));
        END;

        CREATE TRIGGER IF NOT EXISTS "Messages_Changes_AfterUpdate"
        AFTER UPDATE OF "Content", "EditedAt", "PinnedAt" ON "Messages"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            VALUES (new."ChannelId", new."Id", 0, strftime('%Y-%m-%d %H:%M:%f', 'now'));
        END;

        CREATE TRIGGER IF NOT EXISTS "Messages_Changes_AfterDelete"
        AFTER DELETE ON "Messages"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            VALUES (old."ChannelId", old."Id", 1, strftime('%Y-%m-%d %H:%M:%f', 'now'));
        END;

        CREATE TRIGGER IF NOT EXISTS "Channels_Changes_AfterDelete"
        AFTER DELETE ON "Channels"
        BEGIN
            DELETE FROM "MessageChanges" WHERE "ChannelId" = old."Id";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageReactions_Changes_AfterInsert"
        AFTER INSERT ON "MessageReactions"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            SELECT "ChannelId", new."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
            FROM "Messages" WHERE "Id" = new."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageReactions_Changes_AfterDelete"
        AFTER DELETE ON "MessageReactions"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            SELECT "ChannelId", old."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
            FROM "Messages" WHERE "Id" = old."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageEmbeds_Changes_AfterInsert"
        AFTER INSERT ON "MessageEmbeds"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            SELECT "ChannelId", new."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
            FROM "Messages" WHERE "Id" = new."MessageId";
        END;

        CREATE TRIGGER IF NOT EXISTS "MessageEmbeds_Changes_AfterDelete"
        AFTER DELETE ON "MessageEmbeds"
        BEGIN
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            SELECT "ChannelId", old."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
            FROM "Messages" WHERE "Id" = old."MessageId";
        END;
        """;

    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken) =>
        await db.Database.ExecuteSqlRawAsync(TriggersSql, cancellationToken);
}
