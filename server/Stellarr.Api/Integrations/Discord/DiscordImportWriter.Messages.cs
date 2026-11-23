using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Integrations.Discord;

// Writing a channel's history, a page at a time.
public sealed partial class DiscordImportWriter
{
    private ServerSnapshot? _namesOf;
    private Dictionary<string, string> _memberNames = new(StringComparer.Ordinal);
    private Dictionary<string, string> _roleNames = new(StringComparer.Ordinal);

    /// <summary>
    /// Writes one page of a channel's or a thread's history, as the reader
    /// gives it: newest first, and each page older than the one before.
    /// </summary>
    /// <param name="discordChannelId">The channel or thread the page was read from; the structure must hold it.</param>
    /// <param name="pinnedAt">
    /// When each pinned message was pinned, from the reader's pins, which a
    /// message itself does not say. A pinned message missing from it counts
    /// as pinned when it was sent.
    /// </param>
    /// <remarks>
    /// <para>
    /// Each message is written with its author's placeholder, made on first
    /// sight, and Discord's times. Replies and threads are only recorded here:
    /// the reader goes back in time, so a reply comes before what it answers,
    /// and <see cref="CompleteChannelAsync"/> links them once the history is
    /// whole.
    /// </para>
    /// <para>
    /// What Stellarr has no shape for is recorded and left out, so that it is
    /// counted once however often a page is read: notices, forwards, a sticker
    /// or embeds alone, an empty message. A message of nothing but attachments
    /// is recorded for the attachment stage, which writes it.
    /// </para>
    /// </remarks>
    public async Task<DiscordPageResult> WriteMessagesAsync(
        DiscordImport import,
        ServerSnapshot snapshot,
        string discordChannelId,
        IReadOnlyList<MessageSnapshot> page,
        IReadOnlyDictionary<string, DateTimeOffset>? pinnedAt,
        CancellationToken cancellationToken)
    {
        var serverId = RequireServer(import);
        RequireSameServer(import, snapshot);
        ArgumentNullException.ThrowIfNull(discordChannelId);
        ArgumentNullException.ThrowIfNull(page);
        if (page.FirstOrDefault(x => !string.Equals(x.ChannelId, discordChannelId, StringComparison.Ordinal)) is { } stray)
        {
            throw new ArgumentException($"Message {stray.Id} was read from another channel.", nameof(page));
        }

        var source = await db.DiscordImportChannels.AsNoTracking()
            .SingleOrDefaultAsync(x => x.ServerId == serverId && x.DiscordId == discordChannelId, cancellationToken)
            ?? throw new ArgumentException("The import has not recorded this channel or thread.", nameof(discordChannelId));
        var channel = source.ChannelId is { } channelId
            ? await db.Channels.AsNoTracking().SingleOrDefaultAsync(x => x.Id == channelId, cancellationToken)
            : null;
        LoadNames(snapshot);

        var tally = new PageTally();
        foreach (var chunk in page.Chunk(MessagesPerTransaction))
        {
            await PauseAsync(cancellationToken);
            await WriteChunkAsync(import, serverId, source, channel, chunk, pinnedAt, tally, cancellationToken);
        }

        return tally.Result();
    }

    private async Task WriteChunkAsync(
        DiscordImport import,
        Guid serverId,
        DiscordImportChannel source,
        Channel? channel,
        MessageSnapshot[] chunk,
        IReadOnlyDictionary<string, DateTimeOffset>? pinnedAt,
        PageTally tally,
        CancellationToken cancellationToken)
    {
        var now = Now();
        await using var transaction = await db.Database.BeginTransactionAsync(cancellationToken);
        await DiscordImportJournal.LockAsync(db, serverId, source.DiscordId, cancellationToken);

        var ids = chunk.Select(x => x.Id).Distinct(StringComparer.Ordinal).ToArray();
        var seen = (await db.DiscordImportMessages.AsNoTracking()
                .Where(x => x.ServerId == serverId && ids.Contains(x.DiscordId))
                .Select(x => x.DiscordId)
                .ToListAsync(cancellationToken))
            .ToHashSet(StringComparer.Ordinal);
        var fresh = chunk.DistinctBy(x => x.Id, StringComparer.Ordinal).Where(x => !seen.Contains(x.Id)).ToList();
        tally.AlreadySeen += chunk.Length - fresh.Count;

        // What can be written at all, before any text is read.
        var plans = fresh.Select(message => new Plan(message, Classify(message, channel))).ToList();
        var candidates = plans.Where(x => x.Reason == DiscordSkipReason.None).ToList();
        var derived = candidates.ToDictionary(x => x.Message.Id, x => DiscordImportNames.MessageId(serverId, x.Message.Id), StringComparer.Ordinal);
        var derivedIds = derived.Values.ToArray();
        var taken = (await db.Messages.AsNoTracking()
                .Where(x => derivedIds.Contains(x.Id))
                .Select(x => x.Id)
                .ToListAsync(cancellationToken))
            .ToHashSet();

        // The rewriter asks about the people and channels it meets; ask once
        // without answers to learn which, then load them.
        var recording = DiscordImportResolver.Recording(_roleNames);
        foreach (var plan in candidates)
        {
            DiscordContent.Rewrite(plan.Message.Content.Trim(), recording);
        }

        var authors = candidates
            .Select(x => x.Message.Author)
            .DistinctBy(AuthorKey, StringComparer.Ordinal)
            .ToDictionary(AuthorKey, StringComparer.Ordinal);
        var accounts = await AccountsAsync(authors.Keys.Concat(recording.AskedUsers), cancellationToken);
        var resolver = new DiscordImportResolver(
            UserNames(recording.AskedUsers, accounts, authors),
            await ChannelLabelsAsync(serverId, recording.AskedChannels, cancellationToken),
            _roleNames);

        var written = new List<(MessageSnapshot Source, Message Message)>();
        foreach (var plan in plans)
        {
            var message = plan.Message;
            var reason = plan.Reason;
            var outcome = DiscordMessageOutcome.Skipped;
            Guid? messageId = null;
            if (reason == DiscordSkipReason.None)
            {
                var content = DiscordContent.Rewrite(message.Content.Trim(), resolver);
                if (content.Length == 0)
                {
                    (outcome, reason) = message.Attachments.Count > 0
                        ? (DiscordMessageOutcome.AwaitingAttachments, DiscordSkipReason.None)
                        : (DiscordMessageOutcome.Skipped, message.Stickers.Count > 0 ? DiscordSkipReason.StickerOnly
                            : message.Embeds.Count > 0 ? DiscordSkipReason.EmbedOnly
                            : DiscordSkipReason.Empty);
                }
                else if (content.Length > MessageLimits.MaxContentLength)
                {
                    reason = DiscordSkipReason.TooLong;
                }
                else if (taken.Contains(derived[message.Id]))
                {
                    reason = DiscordSkipReason.IdTaken;
                }
                else
                {
                    outcome = DiscordMessageOutcome.Written;
                    messageId = derived[message.Id];
                    var pinned = pinnedAt?.TryGetValue(message.Id, out var at) == true ? at
                        : message.Pinned ? message.CreatedAt
                        : (DateTimeOffset?)null;
                    written.Add((message, new Message
                    {
                        Id = messageId.Value,
                        ChannelId = channel!.Id,
                        Content = content,
                        CreatedAt = message.CreatedAt.UtcDateTime,
                        EditedAt = message.EditedAt?.UtcDateTime,
                        PinnedAt = pinned?.UtcDateTime,
                    }));
                }
            }

            tally.Count(outcome, reason, message);
            db.DiscordImportMessages.Add(new DiscordImportMessage
            {
                ServerId = serverId,
                DiscordId = message.Id,
                ImportId = import.Id,
                ChannelDiscordId = source.DiscordId,
                Outcome = outcome,
                Reason = reason,
                DiscordType = message.Type,
                CreatedAt = message.CreatedAt.UtcDateTime,
                MessageId = messageId,
                ReplyToDiscordId = message.ReplyToMessageId,
                AttachmentsPending = message.Attachments.Count > 0,
            });
        }

        // Only an author of something written gets a placeholder.
        await MakePlaceholdersAsync(
            import,
            written.Select(x => authors[AuthorKey(x.Source.Author)]).DistinctBy(AuthorKey, StringComparer.Ordinal),
            accounts,
            now,
            tally,
            cancellationToken);
        foreach (var (read, message) in written)
        {
            message.AuthorId = accounts[AuthorKey(read.Author)].UserId;
        }

        db.Messages.AddRange(written.Select(x => x.Message));
        var record = await db.DiscordImportChannels
            .SingleAsync(x => x.ServerId == serverId && x.DiscordId == source.DiscordId, cancellationToken);
        record.OldestMessageId = ids.Aggregate(record.OldestMessageId, DiscordImportNames.Earlier);
        await db.SaveChangesAsync(cancellationToken);

        if (written.Count > 0)
        {
            await DiscordImportJournal.ResetAsync(
                db, channel!.Id, written.Select(x => x.Message.Id).ToArray(), now, cancellationToken);
            await DiscordImportJournal.ForgetPostTimesAsync(
                db, channel.Id, written.Select(x => x.Message.AuthorId).Distinct().ToArray(), cancellationToken);
            await DiscordImportJournal.MarkReadAsync(
                db, channel.Id, written.Max(x => x.Message.CreatedAt), cancellationToken);
        }

        await transaction.CommitAsync(cancellationToken);
        Committed();
        db.ChangeTracker.Clear();
    }

    private sealed record Plan(MessageSnapshot Message, DiscordSkipReason Reason);

    /// <summary>Why a message cannot be written whatever its text says, or <see cref="DiscordSkipReason.None"/>.</summary>
    private static DiscordSkipReason Classify(MessageSnapshot message, Channel? channel) =>
        channel is null ? DiscordSkipReason.ChannelRemoved
        : channel.Kind != ChannelKind.Text ? DiscordSkipReason.VoiceChannel
        : message.IsSystemNotice ? DiscordSkipReason.Notice
        : AuthorKey(message.Author).Length == 0 ? DiscordSkipReason.NoAuthor
        : message.Forwarded is not null ? DiscordSkipReason.Forward
        : DiscordSkipReason.None;

    /// <summary>Who the placeholder stands for: the person, or the webhook that posted under a name of its choice.</summary>
    private static string AuthorKey(AuthorSnapshot author) => author.WebhookId ?? author.Id;

    /// <summary>The placeholders that already exist among these Discord ids.</summary>
    private async Task<Dictionary<string, Placeholder>> AccountsAsync(
        IEnumerable<string> discordIds,
        CancellationToken cancellationToken)
    {
        var keys = discordIds.Distinct(StringComparer.Ordinal).ToArray();
        return (await db.DiscordImportAccounts.AsNoTracking()
                .Where(x => keys.Contains(x.DiscordId))
                .Join(db.Users, account => account.UserId, user => user.Id,
                    (account, user) => new { account.DiscordId, account.UserId, user.DisplayName })
                .ToListAsync(cancellationToken))
            .ToDictionary(x => x.DiscordId, x => new Placeholder(x.UserId, x.DisplayName), StringComparer.Ordinal);
    }

    /// <summary>
    /// Makes the placeholders these authors lack and adds them to
    /// <paramref name="accounts"/>. A mention makes none: only an author has one.
    /// </summary>
    private async Task MakePlaceholdersAsync(
        DiscordImport import,
        IEnumerable<AuthorSnapshot> authors,
        Dictionary<string, Placeholder> accounts,
        DateTime now,
        PageTally tally,
        CancellationToken cancellationToken)
    {
        var missing = authors.Where(x => !accounts.ContainsKey(AuthorKey(x))).ToList();
        if (missing.Count == 0)
        {
            return;
        }

        var usernames = missing.Select(x => DiscordImportNames.PlaceholderUsername(AuthorKey(x)).ToUpperInvariant()).ToArray();
        var existing = await db.Users.AsNoTracking()
            .Where(x => usernames.Contains(x.NormalizedUsername))
            .ToDictionaryAsync(x => x.NormalizedUsername, StringComparer.Ordinal, cancellationToken);
        foreach (var author in missing)
        {
            var key = AuthorKey(author);
            var username = DiscordImportNames.PlaceholderUsername(key);
            User user;
            if (existing.TryGetValue(username.ToUpperInvariant(), out var found))
            {
                // A placeholder whose record went missing is adopted; a person
                // is never one, and no form lets a person take such a name.
                user = found.IsPlaceholder
                    ? found
                    : throw new InvalidOperationException($"The account {username} is a person's, not a placeholder.");
            }
            else
            {
                var id = Guid.NewGuid();
                user = new User
                {
                    Id = id,
                    Username = username,
                    NormalizedUsername = username.ToUpperInvariant(),
                    DisplayName = PlaceholderName(author),
                    AvatarColor = $"oklch(0.58 0.14 {(uint)id.GetHashCode() % 360})",
                    IsDisabled = true,
                    Origin = AccountOrigin.Discord,
                    CreatedAt = now,
                };
                db.Users.Add(user);
                tally.PlaceholdersMade++;
            }

            db.DiscordImportAccounts.Add(new DiscordImportAccount
            {
                DiscordId = key,
                UserId = user.Id,
                ImportId = import.Id,
                IsWebhook = author.WebhookId is not null,
                IsBot = author.IsBot,
                DiscordUsername = DiscordImportNames.Clamp(author.Username, 100),
                DiscordAvatarUrl = author.AvatarUrl is { Length: <= 256 } avatar ? avatar : null,
                CreatedAt = now,
            });
            accounts[key] = new Placeholder(user.Id, user.DisplayName);
        }
    }

    private sealed record Placeholder(Guid UserId, string DisplayName);

    /// <summary>
    /// The name an author showed on the server: a member's nickname or name
    /// from the snapshot, else what the message itself says. A webhook shows
    /// the name it gave the message.
    /// </summary>
    private string PlaceholderName(AuthorSnapshot author) =>
        author.WebhookId is null && _memberNames.TryGetValue(author.Id, out var member)
            ? member
            : DiscordImportNames.DisplayName(AuthorKey(author), author.DisplayName, author.Username);

    /// <summary>
    /// How the people the text mentions are shown: by their placeholder, by the
    /// one this batch is about to make for them, else as members of the snapshot.
    /// </summary>
    private Dictionary<string, string> UserNames(
        IEnumerable<string> asked,
        IReadOnlyDictionary<string, Placeholder> accounts,
        Dictionary<string, AuthorSnapshot> authors)
    {
        var names = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var id in asked)
        {
            if (accounts.TryGetValue(id, out var account))
            {
                names[id] = account.DisplayName;
            }
            else if (authors.TryGetValue(id, out var author))
            {
                names[id] = PlaceholderName(author);
            }
            else if (_memberNames.TryGetValue(id, out var member))
            {
                names[id] = member;
            }
        }

        return names;
    }

    /// <summary>A channel as the import made it, <c>#name</c>; a thread by its own name.</summary>
    private async Task<Dictionary<string, string>> ChannelLabelsAsync(
        Guid serverId,
        HashSet<string> asked,
        CancellationToken cancellationToken)
    {
        if (asked.Count == 0)
        {
            return [];
        }

        var ids = asked.ToArray();
        var records = await db.DiscordImportChannels.AsNoTracking()
            .Where(x => x.ServerId == serverId && ids.Contains(x.DiscordId) && x.ChannelId != null)
            .Select(x => new { x.DiscordId, x.Kind, x.Name, x.ChannelId })
            .ToListAsync(cancellationToken);
        var channelIds = records.Select(x => x.ChannelId!.Value).Distinct().ToArray();
        var channelNames = await db.Channels.AsNoTracking()
            .Where(x => channelIds.Contains(x.Id))
            .ToDictionaryAsync(x => x.Id, x => x.Name, cancellationToken);
        var labels = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var record in records)
        {
            var name = record.Kind == DiscordImportChannelKind.Thread
                ? record.Name
                : channelNames.GetValueOrDefault(record.ChannelId!.Value);
            if (name is { Length: > 0 })
            {
                labels[record.DiscordId] = "#" + name;
            }
        }

        return labels;
    }

    /// <summary>The snapshot's member and role names, read once per snapshot.</summary>
    private void LoadNames(ServerSnapshot snapshot)
    {
        if (ReferenceEquals(_namesOf, snapshot))
        {
            return;
        }

        _memberNames = snapshot.Members
            .DistinctBy(x => x.Id, StringComparer.Ordinal)
            .ToDictionary(
                x => x.Id,
                x => DiscordImportNames.DisplayName(x.Id, x.Nickname, x.DisplayName, x.Username),
                StringComparer.Ordinal);
        _roleNames = snapshot.Roles
            .Where(x => x.Name.Length > 0)
            .DistinctBy(x => x.Id, StringComparer.Ordinal)
            .ToDictionary(x => x.Id, x => x.Name.StartsWith('@') ? x.Name : "@" + x.Name, StringComparer.Ordinal);
        _namesOf = snapshot;
    }

    /// <summary>The counts one page adds up to.</summary>
    private sealed class PageTally
    {
        private readonly Dictionary<DiscordSkipReason, int> _skipped = [];
        private readonly Dictionary<int, int> _notices = [];

        public int Written { get; private set; }
        public int AlreadySeen { get; set; }
        public int AttachmentsPending { get; private set; }
        public int AwaitingAttachments { get; private set; }
        public int PlaceholdersMade { get; set; }

        public void Count(DiscordMessageOutcome outcome, DiscordSkipReason reason, MessageSnapshot message)
        {
            switch (outcome)
            {
                case DiscordMessageOutcome.Written:
                    Written++;
                    AttachmentsPending += message.Attachments.Count > 0 ? 1 : 0;
                    break;
                case DiscordMessageOutcome.AwaitingAttachments:
                    AwaitingAttachments++;
                    break;
                default:
                    _skipped[reason] = _skipped.GetValueOrDefault(reason) + 1;
                    if (reason == DiscordSkipReason.Notice)
                    {
                        _notices[message.Type] = _notices.GetValueOrDefault(message.Type) + 1;
                    }

                    break;
            }
        }

        public DiscordPageResult Result() =>
            new(Written, AlreadySeen, AttachmentsPending, AwaitingAttachments, PlaceholdersMade, _skipped, _notices);
    }
}
