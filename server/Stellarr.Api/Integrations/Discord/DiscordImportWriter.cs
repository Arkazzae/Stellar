using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// Writes what the Discord reader read into a Stellarr server that already
/// exists: its sections and channels, placeholder accounts for its authors,
/// and its history, page by page as the reader gives it.
/// </summary>
/// <remarks>
/// <para>
/// Everything written is recorded under Discord's id in the bookkeeping, and
/// a message's Stellarr id is derived from the server and Discord's id, so a
/// second run, or a page read twice, writes nothing twice. What somebody
/// deleted since stays deleted.
/// </para>
/// <para>
/// History is not posted, it is restored: nothing a send does happens here.
/// No slow mode, no mention notification or outbox row, no broadcast, no
/// link preview, no post time and no audit line. The journal, the post times
/// and the read markers, which triggers and bootstraps keep, are left as if
/// the history had always been there; see <see cref="DiscordImportJournal"/>.
/// </para>
/// <para>
/// A placeholder is one per Discord id on the whole instance, so a person
/// met by an earlier import keeps the name that import gave them.
/// </para>
/// <para>
/// The writer holds no lock of its own: one import of a server runs at a
/// time, which whoever drives it has to see to. It keeps the time of its last
/// commit, so one instance should drive one import.
/// </para>
/// </remarks>
public sealed partial class DiscordImportWriter(AppDbContext db, TimeProvider time)
{
    /// <summary>
    /// How many messages one transaction writes. SQLite has one writer, and
    /// a message costs its own row, one of bookkeeping, and the rows the
    /// triggers add to the journal and the search index. A few hundred keep a
    /// transaction to tens of milliseconds, which a live send waits out
    /// without anybody noticing; a whole page of a large import would not.
    /// </summary>
    public const int MessagesPerTransaction = 250;

    /// <summary>
    /// The pause before a transaction that follows another. A writer that found
    /// the database busy tries again every 150 ms (Microsoft.Data.Sqlite's own
    /// loop, see <see cref="DatabaseConnection"/>), so any shorter pause could
    /// take the lock back before a waiting send looked again.
    /// </summary>
    public static readonly TimeSpan PauseBetweenTransactions = TimeSpan.FromMilliseconds(200);

    private DateTimeOffset? _lastCommit;

    /// <summary>Records the start of an import of <paramref name="snapshot"/>'s server into <paramref name="serverId"/>.</summary>
    public async Task<DiscordImport> StartAsync(
        Guid serverId,
        Guid startedById,
        ServerSnapshot snapshot,
        CancellationToken cancellationToken)
    {
        RequireBot(snapshot);
        if (!await db.Servers.AsNoTracking().AnyAsync(x => x.Id == serverId, cancellationToken))
        {
            throw new ArgumentException("There is no such server to import into.", nameof(serverId));
        }

        var import = new DiscordImport
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            DiscordServerId = snapshot.SourceId,
            DiscordServerName = DiscordImportNames.Clamp(snapshot.Name, 100),
            StartedById = startedById,
            StartedAt = Now(),
        };
        db.DiscordImports.Add(import);
        await db.SaveChangesAsync(cancellationToken);
        db.ChangeTracker.Clear();
        return import;
    }

    /// <summary>
    /// Makes the snapshot's sections and channels on the server, below what it
    /// has, in Discord's order, and records its threads. Roles, overwrites,
    /// emoji and attachments are not imported here.
    /// </summary>
    /// <remarks>
    /// A forum, a media or an announcement channel becomes a text channel and
    /// a stage a voice channel; what it was stays in the bookkeeping. A name is
    /// made one Stellarr accepts and unique on the server. What an earlier run
    /// made is left as it is, and what somebody deleted since is not made again.
    /// </remarks>
    public async Task<DiscordStructureResult> WriteStructureAsync(
        DiscordImport import,
        ServerSnapshot snapshot,
        CancellationToken cancellationToken)
    {
        var serverId = RequireServer(import);
        RequireSameServer(import, snapshot);
        var everyone = snapshot.Roles.FirstOrDefault(x => x.IsEveryone);
        var simplified = new Dictionary<string, DiscordChannelType>(StringComparer.Ordinal);
        var hidden = new List<string>();
        int sectionsMade = 0, channelsMade = 0, already = 0;
        Dictionary<string, DiscordImportChannel> channels;

        await PauseAsync(cancellationToken);
        await using (var transaction = await db.Database.BeginTransactionAsync(cancellationToken))
        {
            await LockAsync(import, cancellationToken);
            var sections = await db.DiscordImportSections.AsNoTracking()
                .Where(x => x.ServerId == serverId)
                .ToDictionaryAsync(x => x.DiscordId, StringComparer.Ordinal, cancellationToken);
            channels = await db.DiscordImportChannels.AsNoTracking()
                .Where(x => x.ServerId == serverId && x.Kind == DiscordImportChannelKind.Channel)
                .ToDictionaryAsync(x => x.DiscordId, StringComparer.Ordinal, cancellationToken);
            var sectionPosition = (await db.Sections.Where(x => x.ServerId == serverId)
                .MaxAsync(x => (int?)x.Position, cancellationToken) ?? -1) + 1;
            var channelPosition = (await db.Channels.Where(x => x.ServerId == serverId)
                .MaxAsync(x => (int?)x.Position, cancellationToken) ?? -1) + 1;
            var names = (await db.Channels.Where(x => x.ServerId == serverId)
                .Select(x => x.Name).ToListAsync(cancellationToken)).ToHashSet(StringComparer.Ordinal);

            var orderedSections = snapshot.Sections
                .OrderBy(x => x.Position).ThenBy(x => x.Id, DiscordImportNames.Snowflakes).ToList();
            foreach (var section in orderedSections)
            {
                if (sections.ContainsKey(section.Id))
                {
                    already++;
                    continue;
                }

                var made = new Section
                {
                    Id = Guid.NewGuid(),
                    ServerId = serverId,
                    Name = DiscordImportNames.SectionName(section.Name),
                    Position = sectionPosition++,
                };
                var record = new DiscordImportSection
                {
                    ServerId = serverId,
                    DiscordId = section.Id,
                    ImportId = import.Id,
                    SectionId = made.Id,
                };
                db.Sections.Add(made);
                db.DiscordImportSections.Add(record);
                sections[section.Id] = record;
                sectionsMade++;
            }

            // Discord's client shows the channels outside any category first,
            // then each category's; within one, text before voice, each by
            // position and then by id.
            var sectionOrder = orderedSections
                .Select((section, index) => (section.Id, index))
                .ToDictionary(x => x.Id, x => x.index, StringComparer.Ordinal);
            var orderedChannels = snapshot.Channels
                .OrderBy(x => x.SectionId is { } section ? sectionOrder.GetValueOrDefault(section, int.MaxValue) : -1)
                .ThenBy(x => x.Kind == ChannelKind.Voice ? 1 : 0)
                .ThenBy(x => x.Position)
                .ThenBy(x => x.Id, DiscordImportNames.Snowflakes);
            foreach (var channel in orderedChannels)
            {
                if (channels.ContainsKey(channel.Id))
                {
                    already++;
                    continue;
                }

                var text = channel.Kind == ChannelKind.Text;
                var made = new Channel
                {
                    Id = Guid.NewGuid(),
                    ServerId = serverId,
                    Name = DiscordImportNames.Unique(DiscordImportNames.ChannelName(channel.Name), names),
                    Kind = channel.Kind,
                    Position = channelPosition++,
                    Topic = text ? DiscordImportNames.Topic(channel.Topic) : "",
                    SlowModeSeconds = text ? DiscordImportNames.SlowMode(channel.SlowModeSeconds) : 0,
                    UserLimit = text ? 0 : DiscordImportNames.UserLimit(channel.UserLimit),
                    SectionId = channel.SectionId is { } section ? sections.GetValueOrDefault(section)?.SectionId : null,
                };
                var record = new DiscordImportChannel
                {
                    ServerId = serverId,
                    DiscordId = channel.Id,
                    ImportId = import.Id,
                    Kind = DiscordImportChannelKind.Channel,
                    ChannelId = made.Id,
                    DiscordType = channel.DiscordType,
                    Name = DiscordImportNames.Clamp(channel.Name, 100),
                    IsPrivate = DiscordImportNames.HiddenFromEveryone(channel, everyone),
                };
                db.Channels.Add(made);
                db.DiscordImportChannels.Add(record);
                channels[channel.Id] = record;
                channelsMade++;
            }

            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
            Committed();
        }

        db.ChangeTracker.Clear();
        foreach (var channel in snapshot.Channels)
        {
            if (channel.DiscordType != DiscordChannelType.Conversation)
            {
                simplified[channel.Id] = channel.DiscordType;
            }

            if (channels.TryGetValue(channel.Id, out var record) && record.IsPrivate)
            {
                hidden.Add(channel.Id);
            }
        }

        var threads = await WriteThreadsAsync(import, snapshot, channels, cancellationToken);
        return new DiscordStructureResult(
            sectionsMade, channelsMade, threads.Recorded, already + threads.Already, simplified, hidden);
    }

    /// <summary>
    /// Records the threads, whose messages become threads in their parent's
    /// channel. There can be tens of thousands in a busy forum, so they go in
    /// bounded transactions too.
    /// </summary>
    private async Task<(int Recorded, int Already)> WriteThreadsAsync(
        DiscordImport import,
        ServerSnapshot snapshot,
        Dictionary<string, DiscordImportChannel> parents,
        CancellationToken cancellationToken)
    {
        var serverId = RequireServer(import);
        var known = (await db.DiscordImportChannels.AsNoTracking()
                .Where(x => x.ServerId == serverId && x.Kind == DiscordImportChannelKind.Thread)
                .Select(x => x.DiscordId)
                .ToListAsync(cancellationToken))
            .ToHashSet(StringComparer.Ordinal);
        var already = snapshot.Threads.Count(x => known.Contains(x.Id));
        var fresh = snapshot.Threads
            .Where(x => !known.Contains(x.Id) && parents.ContainsKey(x.ParentChannelId))
            .DistinctBy(x => x.Id)
            .ToList();
        foreach (var chunk in fresh.Chunk(MessagesPerTransaction))
        {
            await PauseAsync(cancellationToken);
            await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
            await LockAsync(import, cancellationToken);
            foreach (var thread in chunk)
            {
                var parent = parents[thread.ParentChannelId];
                db.DiscordImportChannels.Add(new DiscordImportChannel
                {
                    ServerId = serverId,
                    DiscordId = thread.Id,
                    ImportId = import.Id,
                    Kind = DiscordImportChannelKind.Thread,
                    ChannelId = parent.ChannelId,
                    DiscordType = parent.DiscordType,
                    ParentDiscordId = parent.DiscordId,
                    Name = DiscordImportNames.Clamp(thread.Name, 100),
                    StarterMessageId = thread.StarterMessageId,
                    IsPrivate = thread.Private || parent.IsPrivate,
                });
            }

            await db.SaveChangesAsync(cancellationToken);
            await transaction.CommitAsync(cancellationToken);
            Committed();
            db.ChangeTracker.Clear();
        }

        return (fresh.Count, already);
    }

    /// <summary>Counts everything the imports into this server have met, from the bookkeeping.</summary>
    public async Task<DiscordImportTally> TallyAsync(Guid serverId, CancellationToken cancellationToken)
    {
        var groups = await db.DiscordImportMessages.AsNoTracking()
            .Where(x => x.ServerId == serverId)
            .GroupBy(x => new { x.Outcome, x.Reason, x.DiscordType, x.AttachmentsPending })
            .Select(x => new { x.Key.Outcome, x.Key.Reason, x.Key.DiscordType, x.Key.AttachmentsPending, Count = x.Count() })
            .ToListAsync(cancellationToken);
        var placeholders = await db.DiscordImportAccounts.AsNoTracking()
            .CountAsync(x => db.DiscordImports.Any(i => i.Id == x.ImportId && i.ServerId == serverId), cancellationToken);
        return new DiscordImportTally(
            groups.Where(x => x.Outcome == DiscordMessageOutcome.Written).Sum(x => x.Count),
            groups.Where(x => x.Outcome == DiscordMessageOutcome.Written && x.AttachmentsPending).Sum(x => x.Count),
            groups.Where(x => x.Outcome == DiscordMessageOutcome.AwaitingAttachments).Sum(x => x.Count),
            groups.Where(x => x.Outcome == DiscordMessageOutcome.Skipped)
                .GroupBy(x => x.Reason).ToDictionary(x => x.Key, x => x.Sum(y => y.Count)),
            groups.Where(x => x.Outcome == DiscordMessageOutcome.Skipped && x.Reason == DiscordSkipReason.Notice)
                .GroupBy(x => x.DiscordType).ToDictionary(x => x.Key, x => x.Sum(y => y.Count)),
            placeholders);
    }

    /// <summary>
    /// Waits out <see cref="PauseBetweenTransactions"/> since the last commit,
    /// so that a write that queued behind the last transaction gets its turn.
    /// </summary>
    private async Task PauseAsync(CancellationToken cancellationToken)
    {
        if (_lastCommit is not { } last)
        {
            return;
        }

        var left = PauseBetweenTransactions - (time.GetUtcNow() - last);
        if (left > TimeSpan.Zero)
        {
            await Task.Delay(left, time, cancellationToken);
        }
    }

    private void Committed() => _lastCommit = time.GetUtcNow();

    /// <summary>Opens a structure transaction with a write; see <see cref="DiscordImportJournal.LockAsync"/>.</summary>
    private Task<int> LockAsync(DiscordImport import, CancellationToken cancellationToken) =>
        db.Database.ExecuteSqlInterpolatedAsync(
            $"""UPDATE "DiscordImports" SET "StartedAt" = "StartedAt" WHERE "Id" = {import.Id};""",
            cancellationToken);

    private DateTime Now() => time.GetUtcNow().UtcDateTime;

    private static Guid RequireServer(DiscordImport import)
    {
        ArgumentNullException.ThrowIfNull(import);
        return import.ServerId ?? throw new InvalidOperationException("The server this import wrote into is gone.");
    }

    private static void RequireBot(ServerSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        if (snapshot.Source != SnapshotSource.Bot)
        {
            // A template numbers its channels 0, 1, 2...: those are not ids
            // anything can be recorded under, and it has no history anyway.
            throw new ArgumentException("An import is read by a bot; a template has no ids to record.", nameof(snapshot));
        }
    }

    private static void RequireSameServer(DiscordImport import, ServerSnapshot snapshot)
    {
        RequireBot(snapshot);
        if (!string.Equals(snapshot.SourceId, import.DiscordServerId, StringComparison.Ordinal))
        {
            throw new ArgumentException("The snapshot is of another Discord server than the import's.", nameof(snapshot));
        }
    }
}
