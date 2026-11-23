using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// Keeps what clients replay and what they count as unread as if the
/// imported history had always been there.
/// </summary>
/// <remarks>
/// <para>
/// The journal triggers write a row for every message inserted, and the sync
/// endpoint pages through rows and asks a client to reload only on a page that
/// holds a reset. Left alone, an import of a million messages would have every
/// client fetch five thousand pages of upserts after its next reconnect. So the
/// rows the import's own inserts and edits caused are removed in the same
/// transaction, and the channel is left with exactly one reset, written last:
/// the journal's sequence is AUTOINCREMENT, so it is newer than any position a
/// client can hold from before, and such a client reloads the channel once.
/// A client that loaded the channel after the reset holds its position and has
/// nothing to replay.
/// </para>
/// <para>
/// Read markers are times. A member's marker in an imported channel may be
/// older than history that Discord's people wrote while the import ran, so
/// every marker of the channel is raised to the newest imported message.
/// Without a marker, the first bootstrap sets one at that moment, which is
/// later than anything imported.
/// </para>
/// </remarks>
internal static class DiscordImportJournal
{
    /// <summary>
    /// Takes the database's write lock, so the rest of the transaction reads
    /// what nobody can change under it. SQLite refuses at once, without
    /// waiting, to turn a reading transaction into a writing one after
    /// another connection has committed, so a batch opens with this write.
    /// </summary>
    internal static Task<int> LockAsync(AppDbContext db, Guid serverId, string discordChannelId, CancellationToken cancellationToken) =>
        db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            UPDATE "DiscordImportChannels" SET "OldestMessageId" = "OldestMessageId"
            WHERE "ServerId" = {serverId} AND "DiscordId" = {discordChannelId};
            """,
            cancellationToken);

    /// <summary>
    /// Forgets the journal rows the import's own changes to these messages
    /// caused and leaves the channel with one reset, newer than any other row.
    /// </summary>
    internal static async Task ResetAsync(
        AppDbContext db,
        Guid channelId,
        IReadOnlyCollection<Guid> messageIds,
        DateTime now,
        CancellationToken cancellationToken)
    {
        foreach (var chunk in messageIds.Chunk(500))
        {
            await db.MessageChanges
                .Where(x => x.ChannelId == channelId && x.Kind == MessageChangeKind.Upsert && chunk.Contains(x.MessageId))
                .ExecuteDeleteAsync(cancellationToken);
        }

        await db.MessageChanges
            .Where(x => x.ChannelId == channelId && x.Kind == MessageChangeKind.Reset)
            .ExecuteDeleteAsync(cancellationToken);
        await db.Database.ExecuteSqlInterpolatedAsync(
            $"""
            INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
            VALUES ({channelId}, {Guid.Empty}, {(int)MessageChangeKind.Reset}, {now});
            """,
            cancellationToken);
    }

    /// <summary>
    /// Forgets the post times a trigger wrote for these placeholders. They are
    /// slow mode's memory of a person's last send, and a placeholder never sends.
    /// </summary>
    internal static async Task ForgetPostTimesAsync(
        AppDbContext db,
        Guid channelId,
        IReadOnlyCollection<Guid> placeholderIds,
        CancellationToken cancellationToken)
    {
        foreach (var chunk in placeholderIds.Chunk(500))
        {
            await db.ChannelPostTimes
                .Where(x => x.ChannelId == channelId && chunk.Contains(x.UserId))
                .ExecuteDeleteAsync(cancellationToken);
        }
    }

    /// <summary>Raises every member's marker in the channel to <paramref name="newest"/>, where it is earlier.</summary>
    internal static Task<int> MarkReadAsync(AppDbContext db, Guid channelId, DateTime newest, CancellationToken cancellationToken) =>
        db.ChannelReads
            .Where(x => x.ChannelId == channelId && x.LastReadAt < newest)
            .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.LastReadAt, newest), cancellationToken);
}
