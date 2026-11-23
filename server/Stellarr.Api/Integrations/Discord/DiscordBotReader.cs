using System.Globalization;
using System.Runtime.CompilerServices;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>A server's channel list, split into Discord's categories and the channels in them.</summary>
public sealed record ChannelList(IReadOnlyList<SectionSnapshot> Sections, IReadOnlyList<ChannelSnapshot> Channels);

/// <summary>
/// Everything an import reads with a bot's token, one request at a time.
/// </summary>
/// <remarks>
/// <para>
/// Lists Discord pages are read to the end, each page asked for with the
/// largest size the documentation allows and the cursor it documents, and a
/// page that does not move the cursor on ends the read rather than repeating
/// it. Long lists come as streams, so the import can store as it reads and
/// stop at any point.
/// </para>
/// <para>
/// Ids go into paths, so they must be snowflakes: digits only. Anything else
/// is refused before a request is made.
/// </para>
/// </remarks>
public sealed class DiscordBotReader
{
    private const long GatewayGuildMembers = 1 << 14;
    private const long GatewayGuildMembersLimited = 1 << 15;
    private const long GatewayMessageContent = 1 << 18;
    private const long GatewayMessageContentLimited = 1 << 19;

    /// <summary>Discord's code for a read the bot has no access to, which is also its answer to a missing intent.</summary>
    private const int MissingAccessCode = 50001;

    private const int ServerPage = 200;
    private const int MemberPage = 1000;
    private const int MessagePage = 100;
    private const int PinPage = 50;
    private const int ReactionPage = 100;

    private readonly DiscordTransport _transport;

    internal DiscordBotReader(DiscordTransport transport) => _transport = transport;

    /// <summary>Who the token belongs to; the cheapest way to check that a token works.</summary>
    public async Task<DiscordBotIdentity> GetCurrentUserAsync(CancellationToken cancellationToken)
    {
        var user = await GetAsync<WireUser>("users/@me", "users/@me", "", cancellationToken);
        return new DiscordBotIdentity(user.Id ?? "", user.Username ?? "", user.Bot ?? false);
    }

    /// <summary>
    /// The bot's app and its privileged intents, read from the app's flags:
    /// Discord answers a read that lacks one with an empty field rather than an
    /// error, so this is the only place to learn of it beforehand.
    /// </summary>
    public async Task<DiscordApplication> GetCurrentApplicationAsync(CancellationToken cancellationToken)
    {
        var application = await GetAsync<WireApplication>("applications/@me", "applications/@me", "", cancellationToken);
        var flags = application.Flags ?? 0;
        return new DiscordApplication(
            application.Id ?? "",
            application.Name ?? "",
            ServerMembersIntent: (flags & (GatewayGuildMembers | GatewayGuildMembersLimited)) != 0,
            MessageContentIntent: (flags & (GatewayMessageContent | GatewayMessageContentLimited)) != 0);
    }

    /// <summary>The servers the bot is in, 200 to a page.</summary>
    public async Task<IReadOnlyList<DiscordServerSummary>> ListServersAsync(CancellationToken cancellationToken)
    {
        var servers = new List<DiscordServerSummary>();
        string? after = null;
        while (true)
        {
            var page = await GetAsync<List<WireGuild?>>(
                $"users/@me/guilds?limit={ServerPage}&with_counts=true" + (after is null ? "" : $"&after={after}"),
                "users/@me/guilds", "", cancellationToken);
            string? highest = null;
            foreach (var guild in page)
            {
                if (guild?.Id is not { } id)
                {
                    continue;
                }

                highest = Later(highest, id);
                servers.Add(new DiscordServerSummary(
                    id, guild.Name ?? "", DiscordCdn.ServerIcon(id, guild.Icon), guild.ApproximateMemberCount));
            }

            if (!Moved(page.Count, ServerPage, highest, after, forward: true))
            {
                return servers;
            }

            after = highest;
        }
    }

    /// <summary>A server's own fields and Discord's estimate of its size.</summary>
    public async Task<DiscordServerInfo> GetServerAsync(string serverId, CancellationToken cancellationToken)
    {
        var id = Snowflake(serverId);
        var guild = await GetAsync<WireGuild>($"guilds/{id}?with_counts=true", "guilds/{guild}", id, cancellationToken);
        return new DiscordServerInfo(
            id,
            guild.Name ?? "",
            guild.Description,
            DiscordCdn.ServerIcon(id, guild.Icon),
            guild.OwnerId,
            guild.ApproximateMemberCount,
            guild.ApproximatePresenceCount);
    }

    /// <summary>The server's categories and channels, without threads; only those the bot may see.</summary>
    public async Task<ChannelList> GetChannelsAsync(string serverId, CancellationToken cancellationToken)
    {
        var id = Snowflake(serverId);
        var channels = await GetAsync<List<WireChannel?>>($"guilds/{id}/channels", "guilds/{guild}/channels", id, cancellationToken);
        var (sections, mapped) = DiscordSnapshotMapper.Structure(channels);
        return new ChannelList(sections, mapped);
    }

    public async Task<IReadOnlyList<RoleSnapshot>> GetRolesAsync(string serverId, CancellationToken cancellationToken)
    {
        var id = Snowflake(serverId);
        var roles = await GetAsync<List<WireRole?>>($"guilds/{id}/roles", "guilds/{guild}/roles", id, cancellationToken);
        // The role every member has carries the server's own id.
        return DiscordSnapshotMapper.Roles(roles, everyoneId: id);
    }

    public async Task<IReadOnlyList<EmojiSnapshot>> GetEmojiAsync(string serverId, CancellationToken cancellationToken)
    {
        var id = Snowflake(serverId);
        var emoji = await GetAsync<List<WireEmoji?>>($"guilds/{id}/emojis", "guilds/{guild}/emojis", id, cancellationToken);
        return emoji.OfType<WireEmoji>().Select(DiscordSnapshotMapper.Emoji).OfType<EmojiSnapshot>().ToList();
    }

    public async Task<IReadOnlyList<StickerSnapshot>> GetStickersAsync(string serverId, CancellationToken cancellationToken)
    {
        var id = Snowflake(serverId);
        var stickers = await GetAsync<List<WireSticker?>>($"guilds/{id}/stickers", "guilds/{guild}/stickers", id, cancellationToken);
        return stickers.OfType<WireSticker>().Select(DiscordSnapshotMapper.Sticker).OfType<StickerSnapshot>().ToList();
    }

    /// <summary>
    /// Every member, a thousand to a page in order of id. It needs the Server
    /// Members intent: without it Discord refuses with the same code as for a
    /// server the bot cannot see, so that refusal here reads as
    /// <see cref="DiscordFailureKind.MissingIntent"/>.
    /// </summary>
    public async IAsyncEnumerable<MemberSnapshot> ReadMembersAsync(
        string serverId, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var id = Snowflake(serverId);
        string? after = null;
        while (true)
        {
            var page = await _transport.GetAsync<List<WireMember?>>(
                new DiscordRoute(
                    $"guilds/{id}/members?limit={MemberPage}" + (after is null ? "" : $"&after={after}"),
                    "guilds/{guild}/members",
                    id),
                missingIntent: code => code == MissingAccessCode,
                cancellationToken);
            string? highest = null;
            foreach (var wire in page)
            {
                if (wire is not null && DiscordSnapshotMapper.Member(wire, id) is { } member)
                {
                    highest = Later(highest, member.Id);
                    yield return member;
                }
            }

            if (!Moved(page.Count, MemberPage, highest, after, forward: true))
            {
                yield break;
            }

            after = highest;
        }
    }

    /// <summary>The server's active threads, public and private, that the bot may see.</summary>
    public async Task<IReadOnlyList<ThreadSnapshot>> GetActiveThreadsAsync(string serverId, CancellationToken cancellationToken)
    {
        var id = Snowflake(serverId);
        var list = await GetAsync<WireThreadList>($"guilds/{id}/threads/active", "guilds/{guild}/threads/active", id, cancellationToken);
        return (list.Threads ?? []).OfType<WireChannel>().Select(DiscordSnapshotMapper.Thread).OfType<ThreadSnapshot>().ToList();
    }

    /// <summary>
    /// One channel's archived threads, newest archived first. Public ones need
    /// Read Message History in the channel; private ones Manage Threads as well.
    /// </summary>
    public async IAsyncEnumerable<ThreadSnapshot> ReadArchivedThreadsAsync(
        string channelId, bool privateThreads, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var id = Snowflake(channelId);
        var kind = privateThreads ? "private" : "public";
        string? before = null;
        while (true)
        {
            // The documentation names no range for limit, so Discord's own page size is used.
            var page = await GetAsync<WireThreadList>(
                $"channels/{id}/threads/archived/{kind}" + (before is null ? "" : $"?before={Uri.EscapeDataString(before)}"),
                $"channels/{{channel}}/threads/archived/{kind}", id, cancellationToken);
            string? oldest = null;
            foreach (var wire in page.Threads ?? [])
            {
                if (wire is null)
                {
                    continue;
                }

                oldest = wire.ThreadMetadata?.ArchiveTimestamp ?? oldest;
                if (DiscordSnapshotMapper.Thread(wire) is { } thread)
                {
                    yield return thread;
                }
            }

            if (page.HasMore != true || oldest is null || oldest == before)
            {
                yield break;
            }

            before = oldest;
        }
    }

    /// <summary>
    /// A channel's or thread's messages, newest first, a hundred to a page.
    /// Start from <paramref name="before"/> to go on from where an earlier
    /// read stopped. Without Read Message History the channel reads as empty.
    /// </summary>
    public async IAsyncEnumerable<MessageSnapshot> ReadMessagesAsync(
        string channelId, string? before = null, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var id = Snowflake(channelId);
        var cursor = before is null ? null : Snowflake(before);
        while (true)
        {
            var page = await GetAsync<List<WireMessage?>>(
                $"channels/{id}/messages?limit={MessagePage}" + (cursor is null ? "" : $"&before={cursor}"),
                "channels/{channel}/messages", id, cancellationToken);
            string? oldest = null;
            foreach (var wire in page)
            {
                if (wire is not null && DiscordSnapshotMapper.Message(wire) is { } message)
                {
                    oldest = Earlier(oldest, message.Id);
                    yield return message;
                }
            }

            if (!Moved(page.Count, MessagePage, oldest, cursor, forward: false))
            {
                yield break;
            }

            cursor = oldest;
        }
    }

    /// <summary>A channel's pinned messages with when each was pinned, latest pin first.</summary>
    public async IAsyncEnumerable<PinSnapshot> ReadPinsAsync(
        string channelId, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var id = Snowflake(channelId);
        string? before = null;
        while (true)
        {
            var page = await GetAsync<WirePinList>(
                $"channels/{id}/messages/pins?limit={PinPage}" + (before is null ? "" : $"&before={Uri.EscapeDataString(before)}"),
                "channels/{channel}/messages/pins", id, cancellationToken);
            string? oldest = null;
            foreach (var pin in page.Items ?? [])
            {
                if (pin?.Message is null || pin.PinnedAt is null)
                {
                    continue;
                }

                oldest = pin.PinnedAt;
                if (DiscordSnapshotMapper.Message(pin.Message) is { } message &&
                    DiscordSnapshotMapper.Time(pin.PinnedAt) is { } pinnedAt)
                {
                    yield return new PinSnapshot(message, pinnedAt);
                }
            }

            if (page.HasMore != true || oldest is null || oldest == before)
            {
                yield break;
            }

            before = oldest;
        }
    }

    /// <summary>
    /// Who reacted to a message with one emoji, a hundred to a page. Super
    /// reactions are a list of their own: ask again with
    /// <paramref name="burst"/> when <see cref="ReactionSnapshot.BurstCount"/>
    /// is above zero.
    /// </summary>
    public async IAsyncEnumerable<ReactionUser> ReadReactionUsersAsync(
        string channelId,
        string messageId,
        ReactionEmoji emoji,
        bool burst = false,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(emoji);
        var channel = Snowflake(channelId);
        var message = Snowflake(messageId);
        // A custom emoji is named:id; a deleted one may have lost its name, and the id is what counts.
        var segment = Uri.EscapeDataString(emoji.Id is null
            ? emoji.Name
            : $"{(emoji.Name.Length > 0 ? emoji.Name : "_")}:{Snowflake(emoji.Id)}");
        string? after = null;
        while (true)
        {
            var page = await GetAsync<List<WireUser?>>(
                $"channels/{channel}/messages/{message}/reactions/{segment}?type={(burst ? 1 : 0)}&limit={ReactionPage}" +
                    (after is null ? "" : $"&after={after}"),
                "channels/{channel}/messages/{message}/reactions/{emoji}", channel, cancellationToken);
            string? highest = null;
            foreach (var wire in page)
            {
                if (wire is not null && DiscordSnapshotMapper.ReactionUser(wire) is { } user)
                {
                    highest = Later(highest, user.Id);
                    yield return user;
                }
            }

            if (!Moved(page.Count, ReactionPage, highest, after, forward: true))
            {
                yield break;
            }

            after = highest;
        }
    }

    /// <summary>
    /// A server's whole structure and its people, in one snapshot: what a
    /// template has, plus emoji, stickers, members and threads.
    /// </summary>
    /// <remarks>
    /// A part the bot is refused (the members without the intent, one
    /// channel's archived threads without the permission) is listed in
    /// <see cref="ServerSnapshot.Gaps"/> and the rest is still read. Anything
    /// else that fails, a bad token or Discord out of reach, fails the read.
    /// It costs seven requests, one more per thousand members, and for each
    /// text channel one request per page of archived threads, public and, under
    /// an ordinary text channel, private.
    /// </remarks>
    public async Task<ServerSnapshot> ReadServerAsync(string serverId, CancellationToken cancellationToken)
    {
        var info = await GetServerAsync(serverId, cancellationToken);
        var structure = await GetChannelsAsync(info.Id, cancellationToken);
        var roles = await GetRolesAsync(info.Id, cancellationToken);
        var emoji = await GetEmojiAsync(info.Id, cancellationToken);
        var stickers = await GetStickersAsync(info.Id, cancellationToken);
        var gaps = new List<SnapshotGap>();

        var members = new List<MemberSnapshot>();
        await Refusable(gaps, "members", null, async () =>
        {
            await foreach (var member in ReadMembersAsync(info.Id, cancellationToken))
            {
                members.Add(member);
            }
        });

        var threads = new Dictionary<string, ThreadSnapshot>(StringComparer.Ordinal);
        await Refusable(gaps, "active threads", null, async () =>
        {
            foreach (var thread in await GetActiveThreadsAsync(info.Id, cancellationToken))
            {
                threads.TryAdd(thread.Id, thread);
            }
        });

        foreach (var channel in structure.Channels.Where(channel => channel.Kind == ChannelKind.Text))
        {
            // Private threads exist only under ordinary text channels.
            var lists = channel.DiscordType == DiscordChannelType.Conversation ? new[] { false, true } : [false];
            foreach (var privateThreads in lists)
            {
                await Refusable(gaps, privateThreads ? "private archived threads" : "archived threads", channel.Id, async () =>
                {
                    await foreach (var thread in ReadArchivedThreadsAsync(channel.Id, privateThreads, cancellationToken))
                    {
                        threads.TryAdd(thread.Id, thread);
                    }
                });
            }
        }

        return new ServerSnapshot
        {
            Source = SnapshotSource.Bot,
            SourceId = info.Id,
            Name = info.Name,
            Description = info.Description,
            IconUrl = info.IconUrl,
            Sections = structure.Sections,
            Channels = structure.Channels,
            Roles = roles,
            Emoji = emoji,
            Stickers = stickers,
            Members = members,
            Threads = [.. threads.Values],
            Gaps = gaps,
        };
    }

    /// <summary>Runs a read whose refusal is a gap in the snapshot, not a failure of it.</summary>
    private static async Task Refusable(List<SnapshotGap> gaps, string what, string? channelId, Func<Task> read)
    {
        try
        {
            await read();
        }
        catch (DiscordApiException exception) when (exception.Kind is DiscordFailureKind.MissingAccess or DiscordFailureKind.MissingIntent)
        {
            gaps.Add(new SnapshotGap(what, channelId, exception.Kind, exception.DiscordCode));
        }
    }

    private Task<T> GetAsync<T>(string path, string shape, string major, CancellationToken cancellationToken) =>
        _transport.GetAsync<T>(new DiscordRoute(path, shape, major), missingIntent: null, cancellationToken);

    /// <summary>
    /// Whether to ask for another page: the last one was full, and its cursor
    /// moved past the one it was asked with, up for ids read upwards and down
    /// for history read backwards.
    /// </summary>
    private static bool Moved(int count, int pageSize, string? next, string? previous, bool forward) =>
        count >= pageSize && next is not null &&
        (previous is null || (forward ? Compare(next, previous) > 0 : Compare(next, previous) < 0));

    private static string Later(string? current, string id) => current is null || Compare(id, current) > 0 ? id : current;

    private static string Earlier(string? current, string id) => current is null || Compare(id, current) < 0 ? id : current;

    /// <summary>Orders snowflakes as the numbers they are; text order would put 9 after 10.</summary>
    private static int Compare(string left, string right) =>
        ulong.TryParse(left, NumberStyles.None, CultureInfo.InvariantCulture, out var a) &&
        ulong.TryParse(right, NumberStyles.None, CultureInfo.InvariantCulture, out var b)
            ? a.CompareTo(b)
            : left.Length != right.Length ? left.Length.CompareTo(right.Length) : string.CompareOrdinal(left, right);

    private static string Snowflake(string id, [CallerArgumentExpression(nameof(id))] string? name = null)
    {
        ArgumentNullException.ThrowIfNull(id, name);
        if (id.Length is 0 or > 20 || !id.All(char.IsAsciiDigit))
        {
            throw new ArgumentException("A Discord id is a snowflake: up to 20 digits.", name);
        }

        return id;
    }
}
