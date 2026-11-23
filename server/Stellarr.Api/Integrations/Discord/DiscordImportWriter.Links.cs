using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Integrations.Discord;

// Linking replies and threads once a history is whole.
public sealed partial class DiscordImportWriter
{
    /// <summary>How many ids one lookup of the bookkeeping names, well below SQLite's limit on parameters.</summary>
    private const int LookupSize = 500;

    /// <summary>
    /// Says that a channel's or a thread's history has been read to its first
    /// message, and links what waited for that.
    /// </summary>
    /// <remarks>
    /// <para>
    /// In a channel, a reply answers what it answered on Discord, flattened to
    /// Stellarr's rule that a reply answers a thread's root: a reply to a reply
    /// answers the first message of the chain that is still there. A reply
    /// whose target is gone, or is not written yet because it waits for its
    /// attachments, stays a plain message.
    /// </para>
    /// <para>
    /// A thread started from a message of its channel becomes replies to that
    /// message (to its root, if it is a reply itself). One with no such
    /// message (a forum post, a private thread, a thread whose message was
    /// deleted) takes its first message as the root, which gets the thread's
    /// name as a bold first line. Whether the message is there, and what it
    /// answers, is only known for good once the parent channel is complete, so
    /// such a thread completed before it waits, and completing the parent
    /// links it, and links again the ones it linked before.
    /// </para>
    /// <para>
    /// It can be called again, as the attachment stage will once it has
    /// written what waited for it: links are worked out afresh, and a thread's
    /// name is put on its root once.
    /// </para>
    /// </remarks>
    public async Task<DiscordLinkResult> CompleteChannelAsync(
        DiscordImport import,
        string discordChannelId,
        CancellationToken cancellationToken)
    {
        var serverId = RequireServer(import);
        ArgumentNullException.ThrowIfNull(discordChannelId);
        var record = await db.DiscordImportChannels.AsNoTracking()
            .SingleOrDefaultAsync(x => x.ServerId == serverId && x.DiscordId == discordChannelId, cancellationToken)
            ?? throw new ArgumentException("The import has not recorded this channel or thread.", nameof(discordChannelId));
        if (record.CompletedAt is null)
        {
            var now = Now();
            await db.DiscordImportChannels
                .Where(x => x.ServerId == serverId && x.DiscordId == discordChannelId)
                .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.CompletedAt, now), cancellationToken);
            record.CompletedAt = now;
        }

        var result = new LinkTally();
        if (record.Kind == DiscordImportChannelKind.Thread)
        {
            await LinkThreadAsync(serverId, record, result, cancellationToken);
            return result.Result();
        }

        await LinkRepliesAsync(serverId, record, result, cancellationToken);
        // The threads that waited for this channel, and those that grew from
        // one of its messages, whose root a new completion may have moved.
        var forum = record.DiscordType is DiscordChannelType.Forum or DiscordChannelType.Media;
        var threads = await db.DiscordImportChannels.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.ParentDiscordId == record.DiscordId &&
                x.Kind == DiscordImportChannelKind.Thread && x.CompletedAt != null &&
                (x.LinkedAt == null || (!forum && x.StarterMessageId != null)))
            .ToListAsync(cancellationToken);
        foreach (var thread in threads.OrderBy(x => x.DiscordId, DiscordImportNames.Snowflakes))
        {
            await LinkThreadAsync(serverId, thread, result, cancellationToken);
        }

        return result.Result();
    }

    /// <summary>One message of the bookkeeping as linking sees it.</summary>
    private sealed record Node(string DiscordId, Guid? MessageId, string? ReplyTo, DateTime CreatedAt);

    private async Task LinkRepliesAsync(Guid serverId, DiscordImportChannel record, LinkTally result, CancellationToken cancellationToken)
    {
        if (record.ChannelId is not { } channelId)
        {
            return;
        }

        var replies = await db.DiscordImportMessages.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.ChannelDiscordId == record.DiscordId &&
                x.Outcome == DiscordMessageOutcome.Written && x.MessageId != null && x.ReplyToDiscordId != null)
            .Select(x => new Node(x.DiscordId, x.MessageId, x.ReplyToDiscordId, x.CreatedAt))
            .ToListAsync(cancellationToken);
        if (replies.Count == 0)
        {
            return;
        }

        var nodes = replies.ToDictionary(x => x.DiscordId, StringComparer.Ordinal);
        await LoadChainsAsync(serverId, record.DiscordId, nodes, replies.Select(x => x.ReplyTo!), cancellationToken);

        var links = new List<(Guid Message, Guid? Root)>(replies.Count);
        foreach (var reply in replies)
        {
            var root = RootOf(reply, nodes);
            links.Add((reply.MessageId!.Value, root));
            if (root is null)
            {
                result.LeftPlain++;
            }
            else
            {
                result.Linked++;
            }
        }

        await SetRepliesAsync(serverId, record.DiscordId, channelId, links, cancellationToken);
    }

    /// <summary>
    /// Adds to <paramref name="nodes"/> every message the chains starting at
    /// <paramref name="targets"/> pass through in this channel, a lookup at a
    /// time; one that is not there at all is added as gone.
    /// </summary>
    private async Task LoadChainsAsync(
        Guid serverId,
        string channelDiscordId,
        Dictionary<string, Node> nodes,
        IEnumerable<string> targets,
        CancellationToken cancellationToken)
    {
        var pending = targets.Where(x => !nodes.ContainsKey(x)).ToHashSet(StringComparer.Ordinal);
        while (pending.Count > 0)
        {
            var next = new HashSet<string>(StringComparer.Ordinal);
            foreach (var chunk in pending.Chunk(LookupSize))
            {
                var found = await db.DiscordImportMessages.AsNoTracking()
                    .Where(x => x.ServerId == serverId && x.ChannelDiscordId == channelDiscordId && chunk.Contains(x.DiscordId))
                    .Select(x => new Node(
                        x.DiscordId,
                        x.Outcome == DiscordMessageOutcome.Written ? x.MessageId : null,
                        x.ReplyToDiscordId,
                        x.CreatedAt))
                    .ToListAsync(cancellationToken);
                foreach (var node in found)
                {
                    nodes[node.DiscordId] = node;
                    if (node.ReplyTo is { } further && !nodes.ContainsKey(further))
                    {
                        next.Add(further);
                    }
                }

                foreach (var id in chunk.Where(id => !nodes.ContainsKey(id)))
                {
                    nodes[id] = new Node(id, null, null, default);
                }
            }

            pending = next;
        }
    }

    /// <summary>
    /// The root a message answers: the earliest message of its chain that is
    /// written, walking back until a target is gone. Null when even the first
    /// target is gone.
    /// </summary>
    private static Guid? RootOf(Node reply, IReadOnlyDictionary<string, Node> nodes)
    {
        Guid? root = null;
        var visited = new HashSet<string>(StringComparer.Ordinal) { reply.DiscordId };
        var target = reply.ReplyTo;
        while (target is not null && visited.Add(target) &&
               nodes.TryGetValue(target, out var node) && node.MessageId is { } written)
        {
            root = written;
            target = node.ReplyTo;
        }

        return root;
    }

    private async Task LinkThreadAsync(Guid serverId, DiscordImportChannel thread, LinkTally result, CancellationToken cancellationToken)
    {
        if (thread.ChannelId is not { } channelId)
        {
            return;
        }

        var parent = thread.ParentDiscordId is { } parentId
            ? await db.DiscordImportChannels.AsNoTracking()
                .SingleOrDefaultAsync(x => x.ServerId == serverId && x.DiscordId == parentId, cancellationToken)
            : null;
        var forum = thread.DiscordType is DiscordChannelType.Forum or DiscordChannelType.Media;

        // A thread under a text channel grew from one of its messages, when
        // that message is there; what it answers is only certain once the
        // whole channel is read. In a forum, the post's own first message
        // opens it, inside the thread.
        Guid? root = null;
        if (!forum && parent is not null && thread.StarterMessageId is { } starterId)
        {
            if (parent.CompletedAt is null)
            {
                result.Waiting = true;
                return;
            }

            var nodes = new Dictionary<string, Node>(StringComparer.Ordinal);
            await LoadChainsAsync(serverId, parent.DiscordId, nodes, [starterId], cancellationToken);
            var starter = nodes[starterId];
            if (starter.MessageId is { } written)
            {
                root = RootOf(starter, nodes) ?? written;
            }
        }

        var messages = await db.DiscordImportMessages.AsNoTracking()
            .Where(x => x.ServerId == serverId && x.ChannelDiscordId == thread.DiscordId &&
                x.Outcome == DiscordMessageOutcome.Written && x.MessageId != null)
            .Select(x => new Node(x.DiscordId, x.MessageId, x.ReplyToDiscordId, x.CreatedAt))
            .ToListAsync(cancellationToken);
        var titled = root is null;
        if (titled)
        {
            var first = messages
                .OrderBy(x => x.CreatedAt)
                .ThenBy(x => x.DiscordId, DiscordImportNames.Snowflakes)
                .FirstOrDefault();
            root = first?.MessageId;
        }

        var titleRoot = titled && root is not null && thread.RootMessageId != root ? root : null;
        var links = messages
            .Where(x => x.MessageId != root)
            .Select(x => (x.MessageId!.Value, root))
            .ToList();
        if (titled && root is { } own)
        {
            // The root starts the thread and answers nothing itself.
            links.Add((own, null));
        }

        await SetRepliesAsync(serverId, thread.DiscordId, channelId, links, cancellationToken);
        if (titleRoot is { } titledRoot)
        {
            await TitleAsync(serverId, thread, channelId, titledRoot, cancellationToken);
        }

        var now = Now();
        await db.DiscordImportChannels
            .Where(x => x.ServerId == serverId && x.DiscordId == thread.DiscordId)
            .ExecuteUpdateAsync(setters => setters
                .SetProperty(x => x.RootMessageId, root)
                .SetProperty(x => x.LinkedAt, now), cancellationToken);
        result.Linked += links.Count(x => x.Item2 is not null);
        result.Threads++;
    }

    /// <summary>
    /// Puts the thread's name on its root as a bold first line, escaped, so
    /// that the name shows as written and mentions nobody. The root is
    /// recorded in the same transaction, which is what keeps a second
    /// completion from putting the name on twice.
    /// </summary>
    private async Task TitleAsync(
        Guid serverId,
        DiscordImportChannel thread,
        Guid channelId,
        Guid rootId,
        CancellationToken cancellationToken)
    {
        await PauseAsync(cancellationToken);
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await DiscordImportJournal.LockAsync(db, serverId, thread.DiscordId, cancellationToken);
        await db.DiscordImportChannels
            .Where(x => x.ServerId == serverId && x.DiscordId == thread.DiscordId)
            .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.RootMessageId, rootId), cancellationToken);
        var root = await db.Messages.SingleOrDefaultAsync(x => x.Id == rootId, cancellationToken);
        if (root is not null && thread.Name is { Length: > 0 } name)
        {
            var titled = $"**{DiscordContent.Escape(name)}**\n{root.Content}";
            if (titled.Length <= MessageLimits.MaxContentLength &&
                MessageMentions.ExtractNormalizedUsernames(titled).Count == 0)
            {
                root.Content = titled;
                await db.SaveChangesAsync(cancellationToken);
                await DiscordImportJournal.ResetAsync(db, channelId, [rootId], Now(), cancellationToken);
            }
        }

        await transaction.CommitAsync(cancellationToken);
        Committed();
        db.ChangeTracker.Clear();
    }

    /// <summary>Sets what each message answers, a bounded transaction at a time, and resets the channel's journal.</summary>
    private async Task SetRepliesAsync(
        Guid serverId,
        string discordChannelId,
        Guid channelId,
        IReadOnlyList<(Guid Message, Guid? Root)> links,
        CancellationToken cancellationToken)
    {
        foreach (var chunk in links.Chunk(MessagesPerTransaction))
        {
            await PauseAsync(cancellationToken);
            await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
            await DiscordImportJournal.LockAsync(db, serverId, discordChannelId, cancellationToken);
            var changed = 0;
            foreach (var (message, root) in chunk)
            {
                changed += await db.Database.ExecuteSqlInterpolatedAsync(
                    $"""
                    UPDATE "Messages" SET "ReplyToMessageId" = {root}
                    WHERE "Id" = {message} AND "ReplyToMessageId" IS NOT {root};
                    """,
                    cancellationToken);
            }

            // What a message answers is not in the journal; a client that
            // loaded the channel before has to read it again.
            if (changed > 0)
            {
                await DiscordImportJournal.ResetAsync(db, channelId, [], Now(), cancellationToken);
            }

            await transaction.CommitAsync(cancellationToken);
            Committed();
        }
    }

    private sealed class LinkTally
    {
        public int Linked { get; set; }
        public int LeftPlain { get; set; }
        public int Threads { get; set; }
        public bool Waiting { get; set; }

        public DiscordLinkResult Result() => new(Linked, LeftPlain, Threads, Waiting);
    }
}
