using Microsoft.EntityFrameworkCore;

namespace Stellarr.Api.Data.Schema;

/// <summary>
/// Deletes what servers and channels deleted before <see cref="ServerContentSchema"/>
/// left behind: channels whose server is gone, messages whose channel is
/// gone, and the journal entries of channels that are gone, with whatever
/// hangs off those messages.
/// </summary>
/// <remarks>
/// <para>
/// Such a row is a leftover and nothing else. A channel is only ever made on a
/// server that exists and a message only in a channel that exists; direct
/// messages are kept in tables of their own. The journal is written for the
/// messages of a channel, and read only for a channel that exists. The parent
/// is looked for without regard to case, so that an id spelt in other letters
/// keeps its rows rather than losing them.
/// </para>
/// <para>
/// The channels go first, and their trigger takes their messages and journal.
/// The messages go next, through the cascades and triggers every deleted
/// message goes through, which write a journal entry for each of them; the
/// journal goes last, so those entries go too.
/// </para>
/// <para>
/// It runs on every start and finds nothing in a database without leftovers,
/// so it comes after every step that creates a trigger it relies on. The
/// channels that messages and journal entries name are walked through the
/// index that starts with the channel, one seek for each channel, so a start
/// does not read the whole transcript to find that nothing is left over.
/// </para>
/// </remarks>
internal static class OrphanContentSchema
{
    private const string RemoveSql =
        """
        DELETE FROM "Channels"
        WHERE NOT EXISTS (
            SELECT 1 FROM "Servers" AS s WHERE s."Id" = "Channels"."ServerId" COLLATE NOCASE);

        WITH RECURSIVE "Named"("ChannelId") AS (
            SELECT (SELECT "ChannelId" FROM "Messages" ORDER BY "ChannelId" LIMIT 1)
            UNION ALL
            SELECT (
                SELECT m."ChannelId" FROM "Messages" AS m
                WHERE m."ChannelId" > n."ChannelId"
                ORDER BY m."ChannelId" LIMIT 1)
            FROM "Named" AS n
            WHERE n."ChannelId" IS NOT NULL
        )
        DELETE FROM "Messages"
        WHERE "ChannelId" IN (
            SELECT n."ChannelId" FROM "Named" AS n
            WHERE n."ChannelId" IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM "Channels" AS c WHERE c."Id" = n."ChannelId" COLLATE NOCASE));

        WITH RECURSIVE "Named"("ChannelId") AS (
            SELECT (SELECT "ChannelId" FROM "MessageChanges" ORDER BY "ChannelId" LIMIT 1)
            UNION ALL
            SELECT (
                SELECT j."ChannelId" FROM "MessageChanges" AS j
                WHERE j."ChannelId" > n."ChannelId"
                ORDER BY j."ChannelId" LIMIT 1)
            FROM "Named" AS n
            WHERE n."ChannelId" IS NOT NULL
        )
        DELETE FROM "MessageChanges"
        WHERE "ChannelId" IN (
            SELECT n."ChannelId" FROM "Named" AS n
            WHERE n."ChannelId" IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM "Channels" AS c WHERE c."Id" = n."ChannelId" COLLATE NOCASE));
        """;

    internal static async Task RemoveAsync(AppDbContext db, CancellationToken cancellationToken) =>
        await db.Database.ExecuteSqlRawAsync(RemoveSql, cancellationToken);
}
