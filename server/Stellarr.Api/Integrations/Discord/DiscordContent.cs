using System.Buffers;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Integrations.Discord;

/// <summary>What a Discord user mention becomes in a Stellarr message.</summary>
public sealed record MentionText
{
    private MentionText(string text, bool mentions)
    {
        Text = text;
        Mentions = mentions;
    }

    /// <summary>The username to mention, or the label to show.</summary>
    public string Text { get; }

    /// <summary>Whether <see cref="Text"/> is a username that the message mentions.</summary>
    public bool Mentions { get; }

    /// <summary>
    /// A mention of the Stellarr account with this username: it is written
    /// <c>@username</c>, and that account is notified when the message is sent.
    /// </summary>
    public static MentionText Account(string username) => new(username, mentions: true);

    /// <summary>Text shown as given, such as <c>@Ada (left)</c>; it mentions nobody.</summary>
    public static MentionText Label(string text) => new(text, mentions: false);
}

/// <summary>
/// What the Discord ids in a message stand for, as the import decided.
/// </summary>
/// <remarks>
/// The rewriter writes exactly what this returns, so this is where a mention
/// is made or not. <see cref="User"/> may answer
/// <see cref="MentionText.Account"/> only with the Stellarr account the import
/// made for that Discord user, or one its owner linked to that Discord user on
/// purpose. Anybody else, such as a user the import left out or an account
/// that only happens to share the name, gets a <see cref="MentionText.Label"/>
/// or null. Any other answer would notify a real person about a message they were
/// never part of. Roles, channels and unknown ids are always written as text.
/// </remarks>
public interface IDiscordReferenceResolver
{
    /// <summary>What a mention of this Discord user becomes, or null when the import knows nothing of them.</summary>
    MentionText? User(string userId);

    /// <summary>How to show a mention of this role, such as <c>@Moderators</c>, or null when unknown.</summary>
    string? Role(string roleId);

    /// <summary>How to show a link to this channel or thread, such as <c>#general</c>, or null when unknown.</summary>
    string? Channel(string channelId);

    /// <summary>The shortcode, without colons, of the Stellarr emoji this custom emoji became, or null.</summary>
    string? Emoji(string emojiId);
}

/// <summary>
/// Turns the text of a Discord message into the text of a Stellarr message.
/// </summary>
/// <remarks>
/// <para>
/// Discord writes references as tokens (<c>&lt;@id&gt;</c>, <c>&lt;@!id&gt;</c>,
/// <c>&lt;@&amp;id&gt;</c>, <c>&lt;#id&gt;</c>, <c>&lt;:name:id&gt;</c>,
/// <c>&lt;a:name:id&gt;</c>, <c>&lt;t:unix:style&gt;</c>); Stellarr has
/// <c>@username</c> and <c>:shortcode:</c> and nothing else. A user becomes what
/// the resolver says; a role, a channel and an id nobody knows become plain
/// text, never the raw token; a time becomes an absolute UTC time, since the
/// reader's own time zone is gone once the text is copied.
/// </para>
/// <para>
/// Code is read the way Stellarr reads it, fences first and then spans that
/// close at a run of backticks of the same length, and nothing inside is
/// touched. Discord's own escapes are kept, and they mean the same here.
/// </para>
/// <para>
/// Plain text that Stellarr would read as markup Discord did not have is
/// escaped: an <c>@word</c> (including <c>@everyone</c> and <c>@here</c>) that
/// was only text in Discord, and a <c>:word:</c> that would show one of the
/// server's emoji. The finished text is then read by
/// <see cref="MessageMentions.ExtractNormalizedUsernames"/>, the reader that
/// sends notifications; if it finds a name the resolver did not give, the
/// message is written again with every mention as text, and as a last resort
/// with a zero-width space after every at sign.
/// </para>
/// </remarks>
public static partial class DiscordContent
{
    private const string Fence = "```";
    private const string UnknownUser = "@unknown-user";
    private const string UnknownRole = "@deleted-role";
    private const string UnknownChannel = "#deleted-channel";

    // DateTimeOffset's range, in Unix seconds.
    private const long MinUnixSeconds = -62_135_596_800;
    private const long MaxUnixSeconds = 253_402_300_799;

    public static string Rewrite(string content, IDiscordReferenceResolver resolver)
    {
        ArgumentNullException.ThrowIfNull(content);
        ArgumentNullException.ThrowIfNull(resolver);
        if (content.Length == 0)
        {
            return "";
        }

        var pieces = Read(content, resolver);
        var intended = pieces
            .Where(piece => piece.Kind == PieceKind.Mention)
            .Select(piece => piece.Text[1..].ToUpperInvariant())
            .ToHashSet(StringComparer.Ordinal);

        var text = "";
        foreach (var mode in (ReadOnlySpan<Mode>)[Mode.Mentions, Mode.NoMentions, Mode.EveryAtEscaped])
        {
            text = Write(pieces, mode);
            var found = MessageMentions.ExtractNormalizedUsernames(text);
            if (found.IsSubsetOf(intended) && (mode == Mode.Mentions || found.Count == 0))
            {
                return text;
            }
        }

        // The escapes rest on reading the text as Stellarr does. Should the two
        // readings ever differ, no at sign is left before a name character at
        // all, code included: the zero-width space is not one, and shows as
        // nothing.
        text = text.Replace("@", "@\u200B", StringComparison.Ordinal);
        return MessageMentions.ExtractNormalizedUsernames(text).Count == 0
            ? text
            : throw new InvalidOperationException("The rewritten message still mentions somebody.");
    }

    /* ---------------------------------------------------------------- reading */

    private enum PieceKind
    {
        /// <summary>Written as it is: code, an escape, an emoji's shortcode.</summary>
        Raw,

        /// <summary>The author's own text: Discord's formatting stays, stray mentions and shortcodes are escaped.</summary>
        Prose,

        /// <summary>Text the import made up: every marker in it is escaped.</summary>
        Label,

        /// <summary><c>@username</c>, which is meant to mention that account.</summary>
        Mention,
    }

    private readonly record struct Piece(PieceKind Kind, string Text);

    private static List<Piece> Read(string content, IDiscordReferenceResolver resolver)
    {
        var pieces = new List<Piece>();
        var from = 0;
        foreach (var (start, end) in FencedRanges(content))
        {
            ReadInline(content, from, start, resolver, pieces);
            pieces.Add(new Piece(PieceKind.Raw, content[start..end]));
            from = end;
        }

        ReadInline(content, from, content.Length, resolver, pieces);
        return pieces;
    }

    /// <summary>Reads one stretch between fences: escapes, code spans, Discord's tokens and prose.</summary>
    private static void ReadInline(
        string content, int start, int end, IDiscordReferenceResolver resolver, List<Piece> pieces)
    {
        var prose = start;
        var index = start;
        while (index < end)
        {
            var current = content[index];
            if (current == '\\' && index + 1 < end && IsAsciiPunctuation(content[index + 1]))
            {
                Flush(content, prose, index, pieces);
                pieces.Add(new Piece(PieceKind.Raw, content.Substring(index, 2)));
                index += 2;
                prose = index;
                continue;
            }

            if (current == '`')
            {
                Flush(content, prose, index, pieces);
                var run = RunLength(content, index, end, '`');
                var close = NextRun(content, index + run, end, run);
                var spanEnd = close < 0 ? index + run : close + run;
                pieces.Add(new Piece(PieceKind.Raw, content[index..spanEnd]));
                index = spanEnd;
                prose = index;
                continue;
            }

            if (current == '<' && Token().Match(content, index) is { Success: true } token &&
                token.Index + token.Length <= end)
            {
                Flush(content, prose, index, pieces);
                pieces.Add(Resolve(token, resolver));
                index += token.Length;
                prose = index;
                continue;
            }

            index++;
        }

        Flush(content, prose, end, pieces);
    }

    private static void Flush(string content, int from, int to, List<Piece> pieces)
    {
        if (to > from)
        {
            pieces.Add(new Piece(PieceKind.Prose, content[from..to]));
        }
    }

    private static Piece Resolve(Match token, IDiscordReferenceResolver resolver)
    {
        var groups = token.Groups;
        if (groups["user"].Success)
        {
            return resolver.User(groups["user"].Value) switch
            {
                null => new Piece(PieceKind.Label, UnknownUser),
                { Mentions: true, Text: var username } when UsernamePattern().IsMatch(username) =>
                    new Piece(PieceKind.Mention, "@" + username),
                { Mentions: true, Text: var username } => new Piece(PieceKind.Label, "@" + username),
                { Text: var label } => new Piece(PieceKind.Label, label),
            };
        }

        if (groups["role"].Success)
        {
            return new Piece(PieceKind.Label, resolver.Role(groups["role"].Value) ?? UnknownRole);
        }

        if (groups["channel"].Success)
        {
            return new Piece(PieceKind.Label, resolver.Channel(groups["channel"].Value) ?? UnknownChannel);
        }

        if (groups["emoji"].Success)
        {
            var shortcode = resolver.Emoji(groups["emoji"].Value);
            return shortcode is not null && ShortcodePattern().IsMatch(shortcode)
                ? new Piece(PieceKind.Raw, $":{shortcode}:")
                : new Piece(PieceKind.Label, $":{groups["name"].Value}:");
        }

        return long.TryParse(groups["time"].Value, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var seconds) &&
            seconds is >= MinUnixSeconds and <= MaxUnixSeconds
            ? new Piece(PieceKind.Label, FormatTime(DateTimeOffset.FromUnixTimeSeconds(seconds), groups["style"].Value))
            : new Piece(PieceKind.Prose, token.Value);
    }

    /// <summary>
    /// A time in UTC, showing what the style showed: a date for the date
    /// styles, a time for the time styles, both otherwise. The relative style
    /// ("4 years ago") is written as the moment itself, since "ago" would only
    /// be true on the day of the import.
    /// </summary>
    private static string FormatTime(DateTimeOffset time, string style)
    {
        var utc = time.ToUniversalTime();
        var format = style switch
        {
            "d" or "D" => "yyyy-MM-dd",
            "t" => "HH:mm 'UTC'",
            "T" => "HH:mm:ss 'UTC'",
            "S" => "yyyy-MM-dd HH:mm:ss 'UTC'",
            _ => "yyyy-MM-dd HH:mm 'UTC'",
        };
        return utc.ToString(format, CultureInfo.InvariantCulture);
    }

    /* ---------------------------------------------------------------- writing */

    private enum Mode
    {
        /// <summary>Mentions as the resolver asked.</summary>
        Mentions,

        /// <summary>Every mention as text, for when the first attempt read as somebody else.</summary>
        NoMentions,

        /// <summary>No exceptions at all: every at sign before a name character is escaped.</summary>
        EveryAtEscaped,
    }

    private enum CharKind : byte
    {
        Text,
        Raw,
        Mention,
    }

    /// <summary>
    /// Joins the pieces and escapes what Stellarr would read differently.
    /// The escapes depend on the characters around them, which may come from
    /// another piece, so the pieces are joined first and read as one text.
    /// </summary>
    private static string Write(List<Piece> pieces, Mode mode)
    {
        var joined = new StringBuilder();
        var kinds = new List<CharKind>();
        foreach (var piece in pieces)
        {
            var text = piece.Kind == PieceKind.Label ? EscapeLabel(piece.Text) : piece.Text;
            var kind = piece.Kind switch
            {
                PieceKind.Raw => CharKind.Raw,
                PieceKind.Mention => CharKind.Mention,
                _ => CharKind.Text,
            };
            joined.Append(text);
            kinds.AddRange(Enumerable.Repeat(kind, text.Length));
        }

        var source = joined.ToString();
        var output = new Output(source.Length);
        var index = 0;
        var escapeNext = false;
        while (index < source.Length)
        {
            var kind = kinds[index];
            var end = index + 1;
            while (end < source.Length && kinds[end] == kind && (kind != CharKind.Mention || source[end] != '@'))
            {
                end++;
            }

            if (kind == CharKind.Raw)
            {
                output.Append(source.AsSpan(index, end - index));
                index = end;
                continue;
            }

            if (kind == CharKind.Mention)
            {
                Rune? next = end < source.Length &&
                    Rune.DecodeFromUtf16(source.AsSpan(end), out var rune, out _) == OperationStatus.Done
                        ? rune
                        : null;
                var asText = mode != Mode.Mentions ||
                    output.EndsWithLetterOrDigit ||
                    (next is { } letter && Rune.IsLetterOrDigit(letter));
                if (asText)
                {
                    output.Append('\\');
                }
                else
                {
                    // "@ada." would name "ada."; "@ada\." names ada.
                    escapeNext = next is { Value: '_' or '.' or '-' } && kinds[end] == CharKind.Text;
                }

                output.Append(source.AsSpan(index, end - index));
                index = end;
                continue;
            }

            WriteText(source, index, end, mode, output, ref escapeNext);
            index = end;
        }

        return output.ToString();
    }

    private static void WriteText(string source, int start, int end, Mode mode, Output output, ref bool escapeNext)
    {
        var index = start;
        while (index < end)
        {
            var current = source[index];
            if (escapeNext)
            {
                escapeNext = false;
                output.Append('\\');
                output.Append(current);
                index++;
                continue;
            }

            if (current == '\\' && index + 1 < end && IsAsciiPunctuation(source[index + 1]))
            {
                output.Append(source.AsSpan(index, 2));
                index += 2;
                continue;
            }

            if (current == 'h' && BareAddressEnd(source, index, end) is var addressEnd && addressEnd > index)
            {
                // Stellarr reads an address as a link and nothing in it as a mention.
                output.Append(source.AsSpan(index, addressEnd - index));
                index = addressEnd;
                continue;
            }

            if (current == '@' && StartsName(source, index + 1) &&
                (mode == Mode.EveryAtEscaped || !output.EndsWithLetterOrDigit))
            {
                output.Append('\\');
            }
            else if (current == ':' && ShortcodeAhead().IsMatch(source, index + 1))
            {
                output.Append('\\');
            }

            output.Append(current);
            index++;
        }
    }

    /// <summary>The text written so far, and whether it ends in a letter or digit.</summary>
    private sealed class Output(int capacity)
    {
        private readonly StringBuilder _text = new(capacity + 16);

        public bool EndsWithLetterOrDigit { get; private set; }

        public void Append(char value)
        {
            _text.Append(value);
            EndsWithLetterOrDigit = !char.IsSurrogate(value) && char.IsLetterOrDigit(value);
            if (char.IsLowSurrogate(value) && _text.Length >= 2 &&
                Rune.TryCreate(_text[^2], value, out var rune))
            {
                EndsWithLetterOrDigit = Rune.IsLetterOrDigit(rune);
            }
        }

        public void Append(ReadOnlySpan<char> value)
        {
            foreach (var character in value)
            {
                Append(character);
            }
        }

        public override string ToString() => _text.ToString();
    }

    /// <summary>
    /// Escapes text the import made up, as the web client's <c>escapeMarkup</c>
    /// does: every formatting marker, and the colon of an http(s) scheme so a
    /// name that only looks like an address does not become a link. At signs
    /// and shortcodes are left to <see cref="WriteText"/>, which sees the text
    /// around the label too.
    /// </summary>
    private static string EscapeLabel(string text)
    {
        var escaped = new StringBuilder(text.Length + 8);
        foreach (var character in text)
        {
            if (character is '\\' or '`' or '*' or '_' or '~' or '|' or '[' or ']')
            {
                escaped.Append('\\');
            }

            escaped.Append(character);
        }

        return Scheme().Replace(escaped.ToString(), @"$1\://");
    }

    /* ------------------------------------------------------------ characters */

    /// <summary>Whether a username character starts at <paramref name="index"/>.</summary>
    private static bool StartsName(string text, int index) =>
        index < text.Length &&
        Rune.DecodeFromUtf16(text.AsSpan(index), out var rune, out _) == OperationStatus.Done &&
        (Rune.IsLetterOrDigit(rune) || rune.Value is '_' or '.' or '-');

    /// <summary>
    /// Where a bare address that starts at <paramref name="index"/> ends, or
    /// <paramref name="index"/> itself when none does. As in Stellarr, only a
    /// lower-case scheme starts one, and it runs to the next character that is
    /// never raw in an address. It also ends at a <c>]</c>, where the web
    /// client ends it: what follows a bracket is read as text there, so it is
    /// escaped as text here.
    /// </summary>
    private static int BareAddressEnd(string text, int index, int end)
    {
        var span = text.AsSpan(index, end - index);
        var scheme = span.StartsWith("https://", StringComparison.Ordinal) ? 8
            : span.StartsWith("http://", StringComparison.Ordinal) ? 7
            : 0;
        if (scheme == 0)
        {
            return index;
        }

        var cursor = index + scheme;
        while (cursor < end && IsAddressCharacter(text[cursor]) && text[cursor] != ']')
        {
            cursor++;
        }

        return cursor > index + scheme ? cursor : index;
    }

    /// <summary>The characters a bare address may hold, exactly as <see cref="MessageMentions"/> reads them.</summary>
    private static bool IsAddressCharacter(char value) =>
        !IsJsWhitespace(value) && value is not ('<' or '>' or '"' or '\'' or '`');

    /// <summary>The characters JavaScript's <c>\s</c> matches, which end an address in both of Stellarr's readers.</summary>
    private static bool IsJsWhitespace(char value) =>
        value is '\t' or '\n' or '\v' or '\f' or '\r' or ' '
            or '\u00a0' or '\u1680' or (>= '\u2000' and <= '\u200a')
            or '\u2028' or '\u2029' or '\u202f' or '\u205f' or '\u3000' or '\ufeff';

    /// <summary>
    /// The fenced blocks, as <see cref="MessageMentions"/> finds them: ```
    /// at the start of a line whose line ends in a newline, up to the first
    /// later line that is ``` and nothing but spaces or tabs.
    /// </summary>
    private static List<(int Start, int End)> FencedRanges(string text)
    {
        var found = new List<(int, int)>();
        var line = 0;
        while (line < text.Length)
        {
            var infoEnd = LineEnd(text, line);
            if (!text.AsSpan(line).StartsWith(Fence, StringComparison.Ordinal) ||
                infoEnd >= text.Length || text[infoEnd] != '\n')
            {
                line = infoEnd + 1;
                continue;
            }

            var closing = infoEnd + 1;
            while (true)
            {
                var closingEnd = LineEnd(text, closing);
                if (text.AsSpan(closing).StartsWith(Fence, StringComparison.Ordinal) &&
                    text.AsSpan(closing + Fence.Length, closingEnd - closing - Fence.Length).Trim(" \t").IsEmpty)
                {
                    found.Add((line, closingEnd));
                    line = closingEnd + 1;
                    break;
                }

                if (closingEnd >= text.Length)
                {
                    return found;
                }

                closing = closingEnd + 1;
            }
        }

        return found;
    }

    private static int LineEnd(string text, int from)
    {
        var end = from;
        while (end < text.Length && text[end] is not ('\n' or '\r' or '\u2028' or '\u2029'))
        {
            end++;
        }

        return end;
    }

    private static int RunLength(string text, int from, int end, char character)
    {
        var cursor = from;
        while (cursor < end && text[cursor] == character)
        {
            cursor++;
        }

        return cursor - from;
    }

    /// <summary>Where the next run of exactly <paramref name="length"/> backticks starts, or -1.</summary>
    private static int NextRun(string text, int from, int end, int length)
    {
        var cursor = from;
        while (cursor < end)
        {
            var found = text.IndexOf('`', cursor, end - cursor);
            if (found < 0)
            {
                return -1;
            }

            var run = RunLength(text, found, end, '`');
            if (run == length)
            {
                return found;
            }

            cursor = found + run;
        }

        return -1;
    }

    private static bool IsAsciiPunctuation(char value) =>
        value is (>= '!' and <= '/') or (>= ':' and <= '@') or (>= '[' and <= '`') or (>= '{' and <= '~');

    /// <summary>Discord's reference tokens; the ids are ASCII digits only.</summary>
    [GeneratedRegex(
        @"\G<(?:@!?(?<user>[0-9]{1,20})|@&(?<role>[0-9]{1,20})|#(?<channel>[0-9]{1,20})|a?:(?<name>[^\s:<>]{1,64}):(?<emoji>[0-9]{1,20})|t:(?<time>-?[0-9]{1,20})(?::(?<style>[A-Za-z]))?)>",
        RegexOptions.CultureInvariant)]
    private static partial Regex Token();

    /// <summary>A name <c>@name</c> mentions: what Stellarr accepts as a username and reads after an at sign.</summary>
    [GeneratedRegex(@"^[\p{L}\p{N}_.-]{2,32}$", RegexOptions.CultureInvariant)]
    private static partial Regex UsernamePattern();

    /// <summary>The shortcodes a Stellarr server gives its emoji.</summary>
    [GeneratedRegex("^[a-z0-9_]{2,32}$", RegexOptions.CultureInvariant)]
    private static partial Regex ShortcodePattern();

    /// <summary>What follows the colon that opens a shortcode, as the web client reads one.</summary>
    [GeneratedRegex(@"\G[a-zA-Z0-9_-]{2,64}:", RegexOptions.CultureInvariant)]
    private static partial Regex ShortcodeAhead();

    [GeneratedRegex("(https?)://", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase)]
    private static partial Regex Scheme();
}
