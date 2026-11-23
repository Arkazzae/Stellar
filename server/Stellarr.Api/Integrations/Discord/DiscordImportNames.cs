using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>How Discord's names, ids and numbers fit Stellarr's rules.</summary>
internal static class DiscordImportNames
{
    /// <summary>What Stellarr's endpoints allow; the import keeps to the same.</summary>
    internal const int MaxChannelName = 60;
    internal const int MaxSectionName = 60;
    internal const int MaxTopic = 1024;
    internal const int MaxSlowModeSeconds = 6 * 60 * 60;
    internal const int MaxUserLimit = 99;
    internal const int MaxDisplayName = 64;

    /// <summary>Discord's permission bit to see a channel.</summary>
    private const ulong ViewChannel = 1 << 10;

    /// <summary>The username of a placeholder, which registration cannot produce: a colon is not a name character.</summary>
    internal static string PlaceholderUsername(string discordId) => $"discord:{discordId}";

    /// <summary>
    /// The Stellarr id of an imported message: derived from the server and
    /// Discord's id, so the same message written twice is the same row, and a
    /// page read again inserts nothing.
    /// </summary>
    internal static Guid MessageId(Guid serverId, string discordMessageId)
    {
        var input = Encoding.UTF8.GetBytes($"stellarr.discord.message/{serverId:N}/{discordMessageId}");
        Span<byte> hash = stackalloc byte[32];
        SHA256.HashData(input, hash);
        var bytes = hash[..16];
        // RFC 9562's version 8, for an id made by a rule of one's own.
        bytes[6] = (byte)((bytes[6] & 0x0F) | 0x80);
        bytes[8] = (byte)((bytes[8] & 0x3F) | 0x80);
        return new Guid(bytes, bigEndian: true);
    }

    /// <summary>
    /// A channel name Stellarr accepts: lower case, letters, digits and dashes,
    /// at most 60 characters. Anything else becomes a dash, as Discord itself
    /// writes spaces in a text channel's name.
    /// </summary>
    internal static string ChannelName(string discordName)
    {
        var name = new StringBuilder(discordName.Length);
        var dash = false;
        foreach (var rune in discordName.Trim().ToLowerInvariant().EnumerateRunes())
        {
            if (Rune.IsLetterOrDigit(rune) && !(rune.Utf16SequenceLength > 1))
            {
                name.Append(rune.ToString());
                dash = false;
            }
            else if (!dash && name.Length > 0)
            {
                name.Append('-');
                dash = true;
            }
        }

        var text = name.ToString().TrimEnd('-');
        if (text.Length > MaxChannelName)
        {
            text = text[..MaxChannelName].TrimEnd('-');
        }

        return text.Length == 0 ? "channel" : text;
    }

    /// <summary>The name, made unique among <paramref name="taken"/> with a number, as a second "general" becomes "general-2".</summary>
    internal static string Unique(string name, ISet<string> taken)
    {
        if (taken.Add(name))
        {
            return name;
        }

        for (var number = 2; ; number++)
        {
            var suffix = $"-{number.ToString(CultureInfo.InvariantCulture)}";
            var candidate = (name.Length + suffix.Length > MaxChannelName ? name[..(MaxChannelName - suffix.Length)] : name) + suffix;
            if (taken.Add(candidate))
            {
                return candidate;
            }
        }
    }

    internal static string SectionName(string discordName) => Clamp(discordName.Trim(), MaxSectionName) is { Length: > 0 } name
        ? name
        : "Category";

    internal static string Topic(string? topic) => Clamp(topic?.Trim() ?? "", MaxTopic);

    internal static int SlowMode(int seconds) => Math.Clamp(seconds, 0, MaxSlowModeSeconds);

    /// <summary>
    /// A voice channel's limit. A stage allows thousands, which in Stellarr
    /// is no limit at all rather than the 99 it allows at most.
    /// </summary>
    internal static int UserLimit(int limit) => limit is > 0 and <= MaxUserLimit ? limit : 0;

    /// <summary>
    /// The name a placeholder shows: the nickname on the server, else the name
    /// shown everywhere on Discord, else the username, else the id itself.
    /// </summary>
    internal static string DisplayName(string id, params string?[] candidates) =>
        candidates.Select(name => Clamp(name?.Trim() ?? "", MaxDisplayName)).FirstOrDefault(name => name.Length > 0) ?? id;

    /// <summary>
    /// Whether everyone is kept out of a channel: the role every member has
    /// cannot see it, by the server's grant or by the channel's own overwrite.
    /// </summary>
    internal static bool HiddenFromEveryone(ChannelSnapshot channel, RoleSnapshot? everyone)
    {
        if (everyone is null)
        {
            return false;
        }

        var visible = (Bits(everyone.Permissions) & ViewChannel) != 0;
        if (channel.Overwrites.FirstOrDefault(x => x.Target == OverwriteTarget.Role && x.TargetId == everyone.Id) is { } overwrite)
        {
            if ((Bits(overwrite.Deny) & ViewChannel) != 0)
            {
                visible = false;
            }

            if ((Bits(overwrite.Allow) & ViewChannel) != 0)
            {
                visible = true;
            }
        }

        return !visible;
    }

    /// <summary>Orders snowflakes as the numbers they are; text order would put 9 after 10.</summary>
    internal static int Compare(string left, string right) =>
        ulong.TryParse(left, NumberStyles.None, CultureInfo.InvariantCulture, out var a) &&
        ulong.TryParse(right, NumberStyles.None, CultureInfo.InvariantCulture, out var b)
            ? a.CompareTo(b)
            : left.Length != right.Length ? left.Length.CompareTo(right.Length) : string.CompareOrdinal(left, right);

    internal static string Earlier(string? current, string id) => current is null || Compare(id, current) < 0 ? id : current;

    internal static readonly IComparer<string> Snowflakes = Comparer<string>.Create(Compare);

    private static ulong Bits(string value) =>
        ulong.TryParse(value, NumberStyles.None, CultureInfo.InvariantCulture, out var bits) ? bits : 0;

    /// <summary>At most <paramref name="length"/> characters, never cutting a surrogate pair in two.</summary>
    internal static string Clamp(string text, int length)
    {
        if (text.Length <= length)
        {
            return text;
        }

        var end = char.IsLowSurrogate(text[length]) ? length - 1 : length;
        return text[..end];
    }
}
