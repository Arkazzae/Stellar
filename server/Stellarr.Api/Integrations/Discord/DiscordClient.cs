namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// A Discord bot token, which prints as nothing but its type: it opens every
/// server the bot is in, so no log line, exception or debugger view may show it.
/// </summary>
public sealed class DiscordBotToken
{
    public DiscordBotToken(string token)
    {
        var value = token?.Trim() ?? "";
        // The Developer Portal shows the bare token; a pasted header is forgiven.
        if (value.StartsWith("Bot ", StringComparison.OrdinalIgnoreCase))
        {
            value = value[4..].Trim();
        }
        else if (value.Equals("Bot", StringComparison.OrdinalIgnoreCase))
        {
            value = "";
        }

        if (value.Length == 0 || value.Any(character => char.IsWhiteSpace(character) || char.IsControl(character)))
        {
            throw new ArgumentException("A Discord bot token is one word of printable characters.", nameof(token));
        }

        Value = value;
    }

    internal string Value { get; }

    public override string ToString() => nameof(DiscordBotToken);
}

/// <summary>The bot a token belongs to.</summary>
public sealed record DiscordBotIdentity(string Id, string Username, bool IsBot);

/// <summary>
/// The app behind a bot, and whether the two privileged intents the import
/// needs are on. Without Server Members the member list cannot be read;
/// without Message Content every message reads as empty, with no error.
/// </summary>
public sealed record DiscordApplication(string Id, string Name, bool ServerMembersIntent, bool MessageContentIntent);

/// <summary>A server the bot is in, as the list of them gives it.</summary>
public sealed record DiscordServerSummary(string Id, string Name, string? IconUrl, int? ApproximateMemberCount);

/// <summary>One server's own fields, with Discord's estimate of how many people are in it.</summary>
public sealed record DiscordServerInfo(
    string Id,
    string Name,
    string? Description,
    string? IconUrl,
    string? OwnerId,
    int? ApproximateMemberCount,
    int? ApproximatePresenceCount);

/// <summary>
/// Reads Discord servers through version 10 of Discord's REST API: a template
/// by its code without signing in, or everything a bot may see with its token.
/// </summary>
/// <remarks>
/// Every address is built here from ids and codes, on discord.com, and the
/// client registered for this logs nothing and follows no redirect, so
/// neither a pasted link nor an answer can send the token anywhere else.
/// </remarks>
public sealed class DiscordClient(HttpClient http, TimeProvider time, ILogger<DiscordClient> logger)
{
    /// <summary>How long requests wait out rate limits and failures; tests shorten it.</summary>
    internal DiscordRequestPolicy Policy { get; init; } = DiscordRequestPolicy.Default;

    /// <summary>A template's structure: its server's name, roles, sections and channels. No token is needed.</summary>
    public async Task<ServerSnapshot> ReadTemplateAsync(DiscordTemplateCode code, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(code);
        var transport = new DiscordTransport(http, time, logger, Policy, token: null);
        var template = await transport.GetAsync<WireTemplate>(
            new DiscordRoute($"guilds/templates/{Uri.EscapeDataString(code.Value)}", "guilds/templates/{code}", ""),
            cancellationToken);
        return DiscordSnapshotMapper.FromTemplate(template);
    }

    /// <summary>
    /// A reader for one bot. Keep it for the whole import: it remembers
    /// Discord's rate limits, which are counted per bot.
    /// </summary>
    public DiscordBotReader ForBot(DiscordBotToken token)
    {
        ArgumentNullException.ThrowIfNull(token);
        return new DiscordBotReader(new DiscordTransport(http, time, logger, Policy, token));
    }
}
