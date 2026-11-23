using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Deletes what a server and its channels hold together with them, in the
/// statement that deletes them. A channel has no foreign key to its server and
/// a message none to its channel, and a database file cannot be given one
/// afterwards, so triggers do what a cascade would: whichever code deletes a
/// server or a channel, nothing written in it stays behind.
/// </summary>
/// <remarks>
/// <para>
/// A server takes its channels, and a channel takes its messages, which take
/// their attachment links, reactions, link cards and catalogue rows by cascade,
/// and the full-text rows through the catalogue's triggers. The channel's
/// journal goes last: deleting a message writes an entry for it, and the
/// channel trigger removes those itself rather than counting on running before
/// <c>Channels_Changes_AfterDelete</c>, since SQLite promises no order between
/// two triggers on one event.
/// </para>
/// <para>
/// The uploads those messages used stay as rows nobody refers to, and the
/// maintenance sweep removes them with their files, as it does for a single
/// deleted message. An emoji's file is no upload, so the sweep never sees it;
/// its key is queued for deletion when the emoji row goes, by its own delete
/// or with its server.
/// </para>
/// <para>
/// <c>IF NOT EXISTS</c> keeps an existing trigger as it is, so a change to a
/// body here needs a trigger of a new name.
/// </para>
/// </remarks>
internal static class ServerContentSchema
{
    private const string TriggersSql =
        """
        CREATE TRIGGER IF NOT EXISTS "Servers_Channels_AfterDelete"
        AFTER DELETE ON "Servers"
        BEGIN
            DELETE FROM "Channels" WHERE "ServerId" = old."Id";
        END;

        CREATE TRIGGER IF NOT EXISTS "Channels_Messages_AfterDelete"
        AFTER DELETE ON "Channels"
        BEGIN
            DELETE FROM "Messages" WHERE "ChannelId" = old."Id";
            DELETE FROM "MessageChanges" WHERE "ChannelId" = old."Id";
        END;

        CREATE TRIGGER IF NOT EXISTS "ServerEmoji_Objects_AfterDelete"
        AFTER DELETE ON "ServerEmoji"
        BEGIN
            INSERT OR IGNORE INTO "ObjectDeletions" ("ObjectKey", "QueuedAt")
            VALUES (old."ObjectKey", strftime('%Y-%m-%d %H:%M:%f', 'now'));
        END;
        """;

    internal static async Task CreateAsync(AppDbContext db, CancellationToken cancellationToken) =>
        await db.Database.ExecuteSqlRawAsync(TriggersSql, cancellationToken);
}
