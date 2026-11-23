namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// What the Discord ids in one batch of messages stand for, from the import's
/// bookkeeping. Nobody is ever mentioned: a placeholder belongs to no server
/// and must not be notified, so a mention of a Discord user is a label with
/// the name they showed. Emoji are not imported yet, so a custom emoji keeps
/// its name as text.
/// </summary>
/// <remarks>
/// The rewriter asks as it reads, and the answers have to be at hand by then.
/// So a batch is rewritten twice: first with an empty resolver that only notes
/// which users and channels it was asked about, then, once those are loaded,
/// for real.
/// </remarks>
internal sealed class DiscordImportResolver(
    IReadOnlyDictionary<string, string> userNames,
    IReadOnlyDictionary<string, string> channelLabels,
    IReadOnlyDictionary<string, string> roleNames) : IDiscordReferenceResolver
{
    public HashSet<string> AskedUsers { get; } = new(StringComparer.Ordinal);
    public HashSet<string> AskedChannels { get; } = new(StringComparer.Ordinal);

    public static DiscordImportResolver Recording(IReadOnlyDictionary<string, string> roleNames) =>
        new(new Dictionary<string, string>(), new Dictionary<string, string>(), roleNames);

    public MentionText? User(string userId)
    {
        AskedUsers.Add(userId);
        return userNames.TryGetValue(userId, out var name) ? MentionText.Label("@" + name) : null;
    }

    public string? Role(string roleId) => roleNames.TryGetValue(roleId, out var name) ? name : null;

    public string? Channel(string channelId)
    {
        AskedChannels.Add(channelId);
        return channelLabels.GetValueOrDefault(channelId);
    }

    public string? Emoji(string emojiId) => null;
}
