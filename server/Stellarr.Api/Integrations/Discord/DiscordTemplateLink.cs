using System.Diagnostics.CodeAnalysis;
using System.Text.RegularExpressions;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>
/// The code of a Discord server template, checked and nothing else: the only
/// way to make one is <see cref="DiscordTemplateLink.TryParse"/>, so a value of
/// this type is always safe to put into the path of an API request.
/// </summary>
public sealed record DiscordTemplateCode
{
    internal DiscordTemplateCode(string value) => Value = value;

    public string Value { get; }

    public override string ToString() => Value;
}

/// <summary>Why a pasted text is not a template link.</summary>
public enum TemplateLinkRefusal
{
    Empty,

    /// <summary>An invite (discord.gg, discord.com/invite) joins a server; it does not describe one.</summary>
    InviteLink,

    /// <summary>A link to somewhere else, or with more in it than a template link has.</summary>
    NotATemplateLink,

    /// <summary>The right shape, but the code holds characters no template code has.</summary>
    InvalidCode,
}

/// <summary>
/// Reads what a person pastes as a Discord template: a discord.new link, a
/// discord.com or discordapp.com /template/ link, with or without the scheme
/// and www., or the bare code.
/// </summary>
/// <remarks>
/// Only the code is taken from the text. The request goes to an address
/// the client builds from the code, never to the text
/// itself, so a link that merely looks like Discord's cannot send the server
/// anywhere. That is also why a query, a fragment, a port or a login part is
/// refused rather than dropped: nothing Discord hands out has one, and a
/// parser that tolerates them has to decide what they mean.
/// </remarks>
public static partial class DiscordTemplateLink
{
    public static bool TryParse(
        string? input,
        [NotNullWhen(true)] out DiscordTemplateCode? code,
        out TemplateLinkRefusal refusal)
    {
        code = null;
        var text = input?.Trim() ?? "";
        if (text.Length == 0)
        {
            refusal = TemplateLinkRefusal.Empty;
            return false;
        }

        var scheme = text.IndexOf("://", StringComparison.Ordinal);
        if (scheme >= 0)
        {
            var name = text[..scheme];
            if (!name.Equals("https", StringComparison.OrdinalIgnoreCase) &&
                !name.Equals("http", StringComparison.OrdinalIgnoreCase))
            {
                refusal = TemplateLinkRefusal.NotATemplateLink;
                return false;
            }

            text = text[(scheme + 3)..];
        }

        if (text.IndexOfAny(['?', '#', '@', '\\', ' ', '\t']) >= 0)
        {
            refusal = TemplateLinkRefusal.NotATemplateLink;
            return false;
        }

        var slash = text.IndexOf('/', StringComparison.Ordinal);
        if (slash < 0)
        {
            // Without a path the whole text is the code, or a host on its own.
            if (scheme < 0 && CodePattern().IsMatch(text))
            {
                code = new DiscordTemplateCode(text);
                refusal = default;
                return true;
            }

            refusal = text.Contains('.', StringComparison.Ordinal) || scheme >= 0
                ? TemplateLinkRefusal.NotATemplateLink
                : TemplateLinkRefusal.InvalidCode;
            return false;
        }

        var host = text[..slash].ToLowerInvariant();
        if (host.StartsWith("www.", StringComparison.Ordinal))
        {
            host = host[4..];
        }

        var path = text[(slash + 1)..];
        if (path.EndsWith('/'))
        {
            path = path[..^1];
        }

        var candidate = host switch
        {
            "discord.new" => path,
            "discord.com" or "discordapp.com" when path.StartsWith("template/", StringComparison.OrdinalIgnoreCase) =>
                path["template/".Length..],
            _ => null,
        };

        if (candidate is null)
        {
            refusal = IsInvite(host, path) ? TemplateLinkRefusal.InviteLink : TemplateLinkRefusal.NotATemplateLink;
            return false;
        }

        if (candidate.Length == 0 || candidate.Contains('/', StringComparison.Ordinal))
        {
            refusal = TemplateLinkRefusal.NotATemplateLink;
            return false;
        }

        if (!CodePattern().IsMatch(candidate))
        {
            refusal = TemplateLinkRefusal.InvalidCode;
            return false;
        }

        code = new DiscordTemplateCode(candidate);
        refusal = default;
        return true;
    }

    private static bool IsInvite(string host, string path) =>
        host is "discord.gg" ||
        (host is "discord.com" or "discordapp.com" && path.StartsWith("invite/", StringComparison.OrdinalIgnoreCase));

    /// <summary>
    /// Codes Discord hands out are a dozen letters and digits; the class is a
    /// little wider so that a longer code still reads, and narrow enough that
    /// it can never hold a path separator or an escape.
    /// </summary>
    [GeneratedRegex("^[A-Za-z0-9_-]{2,64}$", RegexOptions.CultureInvariant)]
    private static partial Regex CodePattern();
}
