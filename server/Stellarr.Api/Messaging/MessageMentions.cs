using System.Text;
using Microsoft.AspNetCore.SignalR;
using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Realtime;

namespace Stellarr.Api.Messaging;

/// <summary>
/// Turns <c>@username</c> in a message into entries in the inbox of the
/// members it names, so that a mention is more than styling. What the web
/// renderer does not show as a mention is not one here either: fenced code,
/// code spans, addresses, escaped characters and the labels of real http(s)
/// links all stay quiet.
/// </summary>
internal static class MessageMentions
{
    internal const string PackageId = "stellarr.mention";

    private const int MinNameLength = 2;
    private const int MaxNameLength = 32;
    private const int PreviewLength = 220;

    private const string Fence = "```";

    /// <summary>
    /// Adds an inbox entry for each member of the server the message names,
    /// except its author, to the context. The caller saves them with the
    /// message and announces them afterwards.
    /// </summary>
    internal static async Task<IReadOnlyList<UserNotification>> AddNotificationsAsync(
        AppDbContext db,
        Guid serverId,
        Message message,
        CancellationToken cancellationToken)
    {
        var names = ExtractNormalizedUsernames(message.Content);
        if (names.Count == 0)
        {
            return [];
        }

        var recipients = await db.ServerMembers.AsNoTracking()
            .Where(member => member.ServerId == serverId &&
                member.UserId != message.AuthorId &&
                names.Contains(member.User.NormalizedUsername))
            .Select(member => member.UserId)
            .Distinct()
            .ToArrayAsync(cancellationToken);
        if (recipients.Length == 0)
        {
            return [];
        }

        var author = message.Author.DisplayName;
        var preview = Preview(message.Content);
        var notifications = recipients.Select(userId => new UserNotification
        {
            Id = Guid.NewGuid(),
            ServerId = serverId,
            UserId = userId,
            Source = "system",
            SourcePackageId = PackageId,
            SourcePackageName = "Stellarr",
            SourceModuleId = "mention",
            SourceModuleName = author,
            Title = $"{author} mentioned you",
            Body = preview,
            Tone = "info",
            TargetChannelId = message.ChannelId,
            CreatedAt = message.CreatedAt,
        }).ToArray();
        db.UserNotifications.AddRange(notifications);
        return notifications;
    }

    /// <summary>Tells each recipient's connections that their inbox has a new entry.</summary>
    internal static Task FanOutAsync(
        IHubContext<AppHub> hub,
        IReadOnlyList<UserNotification> notifications,
        CancellationToken cancellationToken) =>
        Task.WhenAll(notifications.Select(notification => hub.Clients
            .Group(AppHub.UserGroup(notification.UserId))
            .SendAsync(
                "NotificationInboxChanged",
                new NotificationInboxChangedDto(notification.ServerId.ToString(), notification.Sequence, Global: false),
                cancellationToken)));

    /// <summary>
    /// The upper-cased names mentioned in <paramref name="content"/>. A name
    /// starts at an <c>@</c> that no name character precedes, so an e-mail
    /// address is not one, and is the whole word after it: a word longer than
    /// 32 characters names nobody, and its first 32 characters must not name a
    /// shorter member either.
    ///
    /// The body is read linearly: fences first, then the prose between them,
    /// then the names outside the ranges that prose marks. No construct is
    /// searched again from the start, so no part of a long message is read
    /// a number of times that grows with its length.
    /// </summary>
    internal static HashSet<string> ExtractNormalizedUsernames(string content)
    {
        var names = new HashSet<string>(StringComparer.Ordinal);
        if (content.Length == 0)
        {
            return names;
        }

        var excluded = ExcludedRanges(content);
        var index = 0;
        while (index < content.Length)
        {
            if (content[index] != '@' || excluded[index] ||
                (index > 0 && NameCharacterBefore(content, index, excluded)))
            {
                index++;
                continue;
            }

            var end = index + 1;
            while (end < content.Length && !excluded[end])
            {
                var width = NameCharacterWidth(content, end);
                if (width == 0)
                {
                    break;
                }

                end += width;
            }

            var length = end - index - 1;
            if (length is >= MinNameLength and <= MaxNameLength)
            {
                names.Add(content[(index + 1)..end].ToUpperInvariant());
            }

            index = end;
        }

        return names;
    }

    /* ---------------------------------------------------------- exclusions */

    /// <summary>
    /// What the renderer does not read as plain words: fenced blocks, code
    /// spans, escaped characters, bare addresses and the labels of valid
    /// http(s) links. A fenced block is cut out before anything inline is read,
    /// as it is in the transcript, so no inline construct can span one.
    /// </summary>
    private static bool[] ExcludedRanges(string content)
    {
        var text = content.AsSpan();
        var excluded = new bool[text.Length];
        var from = 0;
        foreach (var (start, end) in FencedRanges(text))
        {
            ExcludeInline(text[from..start], excluded, from);
            Exclude(excluded, start, end);
            from = end;
        }

        ExcludeInline(text[from..], excluded, from);
        return excluded;
    }

    /// <summary>Reads one stretch of prose, marking everything in it the renderer does not read as plain words.</summary>
    private static void ExcludeInline(ReadOnlySpan<char> text, bool[] excluded, int offset)
    {
        if (text.IsEmpty)
        {
            return;
        }

        var brackets = new List<Bracket>();
        var runs = new BacktickRuns(text);
        var parens = new ClosingParentheses(text);
        var delimiters = new FormattingDelimiters(text.ToString(), excluded, offset);
        var addresses = ProtectedAddresses(text);
        var address = 0;
        // The character after this offset was escaped and is not an image marker.
        var escapedUpTo = -1;
        // No link may start before this offset: an inner link wins over an outer one.
        var inactiveBefore = -1;

        var index = 0;
        while (index < text.Length)
        {
            while (address < addresses.Count && addresses[address].Start < index) address++;
            if (address < addresses.Count && addresses[address].Start == index)
            {
                index = addresses[address].End;
                continue;
            }

            var current = text[index];
            if (current == '\\' && index + 1 < text.Length && IsAsciiPunctuation(text[index + 1]))
            {
                excluded[offset + index + 1] = true;
                escapedUpTo = index + 2;
                index += 2;
                continue;
            }

            if (current == '`')
            {
                // A span closes only at a run of exactly the same length.
                var run = RunLength(text, index, '`');
                var close = runs.Next(run, index + run);
                if (close < 0)
                {
                    index += run;
                }
                else
                {
                    Exclude(excluded, offset + index, offset + close + run);
                    index = close + run;
                }

                continue;
            }

            if (current == '[')
            {
                brackets.Add(new Bracket(
                    index,
                    index > 0 && text[index - 1] == '!' && escapedUpTo != index,
                    delimiters.Top));
                index++;
                continue;
            }

            if (current == ']')
            {
                if (brackets.Count == 0)
                {
                    index++;
                    continue;
                }

                var opener = brackets[^1];
                brackets.RemoveAt(brackets.Count - 1);
                var targetEnd = 0;
                var link = !opener.Image && opener.Position >= inactiveBefore &&
                    TryLinkTarget(text, index + 1, parens, out targetEnd);
                if (link)
                {
                    delimiters.Process(opener.Floor);
                    Exclude(excluded, offset + opener.Position, offset + targetEnd);
                    inactiveBefore = opener.Position;
                    index = targetEnd;
                }
                else
                {
                    index++;
                }

                continue;
            }

            if (current is '*' or '_' or '~' or '|')
            {
                var length = RunLength(text, index, current);
                delimiters.Add(index, current, length);
                index += length;
                continue;
            }

            index++;
        }

        delimiters.Process(null);
        ExcludeAddresses(text, excluded, offset);
    }

    private readonly record struct Bracket(int Position, bool Image, FormattingDelimiters.Run? Floor);

    /// <summary>
    /// Addresses shield their markers from the grammar, but a closing bracket
    /// must still be read as the end of a possible label. Actual URL exclusions
    /// are applied after links and formatting have split the prose into spans.
    /// </summary>
    private static List<(int Start, int End)> ProtectedAddresses(ReadOnlySpan<char> text)
    {
        var found = new List<(int, int)>();
        for (var index = 0; index < text.Length; index++)
        {
            if (text[index] != 'h' || !TryBareAddressEnd(text, index, out _, out var matchEnd)) continue;
            var bracket = text[index..matchEnd].IndexOf(']');
            var end = bracket < 0 ? matchEnd : index + bracket;
            end = TrimAddress(text, index, end, formatting: true);
            if (end > index) found.Add((index, end));
            index = matchEnd - 1;
        }
        return found;
    }

    private static void ExcludeAddresses(ReadOnlySpan<char> text, bool[] excluded, int offset)
    {
        var index = 0;
        while (index < text.Length)
        {
            if (excluded[offset + index])
            {
                index++;
                continue;
            }
            var end = index;
            while (end < text.Length && !excluded[offset + end]) end++;
            var plain = text[index..end];
            for (var at = 0; at < plain.Length; at++)
            {
                if (plain[at] != 'h' || !TryBareAddressEnd(plain, at, out var addressEnd, out var matchEnd)) continue;
                Exclude(excluded, offset + index + at, offset + index + addressEnd);
                at = matchEnd - 1;
            }
            index = end;
        }
    }

    /// <summary>
    /// Only paired markers split the text a mention recogniser reads. In
    /// particular, the last underscore in _@ada_ closes emphasis, whereas the
    /// one in @ada_ belongs to that member's name. All marker kinds share the
    /// stack so crossing pairs discard the same inner openers as the renderer.
    /// </summary>
    private sealed class FormattingDelimiters(string source, bool[] excluded, int offset)
    {
        internal sealed class Run(int position, char character, int length, bool opens, bool closes)
        {
            internal readonly int Position = position;
            internal readonly char Character = character;
            internal readonly int Length = length;
            internal readonly bool Opens = opens;
            internal readonly bool Closes = closes;
            internal int Count = length;
            internal int Start = position;
            internal int End = position + length;
            internal Run? Previous;
            internal Run? Next;
        }

        private Run? bottom;
        internal Run? Top { get; private set; }
        private int[]? visible;

        internal void Add(int position, char character, int length)
        {
            var before = position == 0 ? (Rune?)null : RuneBefore(position);
            var after = position + length == source.Length ? (Rune?)null : RuneAt(position + length);
            var left = !Space(after) && (!Punctuation(after) || Space(before) || Punctuation(before));
            var right = !Space(before) && (!Punctuation(before) || Space(after) || Punctuation(after));
            var opens = character switch
            {
                '|' => length >= 2,
                '~' => left && length == 2,
                '_' => left && (!right || Punctuation(before)),
                _ => left,
            };
            var closes = character switch
            {
                '|' => length >= 2,
                '~' => right && length == 2,
                '_' => right && (!left || Punctuation(after)),
                _ => right,
            };
            if (!opens && !closes) return;
            var run = new Run(position, character, length, opens, closes) { Previous = Top };
            if (Top is null) bottom = run;
            else Top.Next = run;
            Top = run;
        }

        internal void Process(Run? floor)
        {
            // A failed search is the lower bound for later closers of that
            // kind, including the rule-of-three remainder. Each run is visited
            // or removed a bounded number of times, even on hostile input.
            var searched = new Dictionary<(char, bool, int), int>();
            var floorPosition = floor?.Position ?? -1;
            var closer = floor is null ? bottom : floor.Next;
            while (closer is not null)
            {
                if (!closer.Closes) { closer = closer.Next; continue; }
                var emphasis = closer.Character is '*' or '_';
                var kind = (closer.Character, emphasis && closer.Opens, emphasis ? closer.Length % 3 : 0);
                var lowest = Math.Max(floorPosition, searched.GetValueOrDefault(kind, -1));
                var opener = closer.Previous;
                while (opener is not null && opener.Position > lowest)
                {
                    if (opener.Character == closer.Character && opener.Opens)
                    {
                        if (Pairs(opener, closer)) break;
                        if (closer.Character == '|') { opener = null; break; }
                    }
                    opener = opener.Previous;
                }
                if (opener is not null && opener.Position <= lowest) opener = null;
                if (opener is null)
                {
                    searched[kind] = closer.Previous?.Position ?? -1;
                    var next = closer.Next;
                    if (!closer.Opens) Remove(closer);
                    closer = next;
                    continue;
                }

                var used = emphasis && (opener.Count < 2 || closer.Count < 2) ? 1 : 2;
                opener.Count -= used;
                opener.End -= used;
                Exclude(excluded, offset + opener.End, offset + opener.End + used);
                Exclude(excluded, offset + closer.Start, offset + closer.Start + used);
                closer.Count -= used;
                closer.Start += used;
                opener.Next = closer;
                closer.Previous = opener;
                if (Spent(opener)) Remove(opener);
                if (Spent(closer))
                {
                    var next = closer.Next;
                    Remove(closer);
                    closer = next;
                }
            }
            if (floor is null) bottom = Top = null;
            else { floor.Next = null; Top = floor; }
        }

        private bool Pairs(Run opener, Run closer)
        {
            if (closer.Character == '|')
            {
                visible ??= VisibleCounts();
                return visible[closer.Start] > visible[opener.End];
            }
            return closer.Character == '~' || !((opener.Closes || closer.Opens) &&
                (opener.Length + closer.Length) % 3 == 0 &&
                !(opener.Length % 3 == 0 && closer.Length % 3 == 0));
        }

        private int[] VisibleCounts()
        {
            var counts = new int[source.Length + 1];
            for (var index = 0; index < source.Length; index++)
                counts[index + 1] = counts[index] + (IsJsWhitespace(source[index]) ? 0 : 1);
            return counts;
        }

        private void Remove(Run run)
        {
            if (run.Previous is null) bottom = run.Next;
            else run.Previous.Next = run.Next;
            if (run.Next is null) Top = run.Previous;
            else run.Next.Previous = run.Previous;
        }

        private static bool Spent(Run run) => run.Count == 0 || (run.Character == '|' && run.Count < 2);
        private static bool Space(Rune? rune) => rune is null || (rune.Value.IsBmp && IsJsWhitespace((char)rune.Value.Value));
        private static bool Punctuation(Rune? rune) => rune is { } value && (Rune.IsPunctuation(value) || Rune.IsSymbol(value));
        private Rune RuneAt(int index) => Rune.DecodeFromUtf16(source.AsSpan(index), out var rune, out _) == System.Buffers.OperationStatus.Done ? rune : Rune.ReplacementChar;
        private Rune RuneBefore(int index) => Rune.DecodeLastFromUtf16(source.AsSpan(0, index), out var rune, out _) == System.Buffers.OperationStatus.Done ? rune : Rune.ReplacementChar;
    }

    /// <summary>
    /// The fenced blocks of a body, in order: ```<info> at the start of a line,
    /// with a line ending in a newline, up to the first later line that is ```
    /// and nothing but spaces or tabs. A fence that never closes is prose, and
    /// it ends the search: no later fence could close either.
    /// </summary>
    private static List<(int Start, int End)> FencedRanges(ReadOnlySpan<char> text)
    {
        var found = new List<(int Start, int End)>();
        var line = 0;
        while (line < text.Length)
        {
            var infoEnd = LineEnd(text, line);
            if (!text[line..].StartsWith(Fence, StringComparison.Ordinal) ||
                infoEnd >= text.Length || text[infoEnd] != '\n')
            {
                line = infoEnd + 1;
                continue;
            }

            var closing = infoEnd + 1;
            while (true)
            {
                var closingEnd = LineEnd(text, closing);
                if (text[closing..].StartsWith(Fence, StringComparison.Ordinal) &&
                    IsBlankOnly(text[(closing + Fence.Length)..closingEnd]))
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

    /// <summary>The runs of backticks, by length, with each length's cursor moving forward once.</summary>
    private sealed class BacktickRuns
    {
        private readonly Dictionary<int, List<int>> starts = [];
        private readonly Dictionary<int, int> cursors = [];

        internal BacktickRuns(ReadOnlySpan<char> text)
        {
            var index = 0;
            while (index < text.Length)
            {
                var found = text[index..].IndexOf('`');
                if (found < 0)
                {
                    break;
                }

                index += found;
                var run = RunLength(text, index, '`');
                if (!starts.TryGetValue(run, out var list))
                {
                    list = [];
                    starts[run] = list;
                }

                list.Add(index);
                index += run;
            }
        }

        internal int Next(int length, int from)
        {
            if (!starts.TryGetValue(length, out var list))
            {
                return -1;
            }

            cursors.TryGetValue(length, out var cursor);
            while (cursor < list.Count && list[cursor] < from)
            {
                cursor++;
            }

            cursors[length] = cursor;
            return cursor < list.Count ? list[cursor] : -1;
        }
    }

    /// <summary>
    /// The parenthesis that balances each <c>(</c>, with the count restarting
    /// at whitespace, which no address contains. Worked out once for the whole
    /// body so a malformed link cannot make the scan quadratic.
    /// </summary>
    private sealed class ClosingParentheses
    {
        private readonly int[] closing;

        internal ClosingParentheses(ReadOnlySpan<char> text)
        {
            closing = new int[text.Length];
            Array.Fill(closing, -1);
            var opened = new List<int>();
            for (var index = 0; index < text.Length; index++)
            {
                var current = text[index];
                if (current == '(')
                {
                    opened.Add(index);
                }
                else if (current == ')')
                {
                    if (opened.Count > 0)
                    {
                        closing[opened[^1]] = index;
                        opened.RemoveAt(opened.Count - 1);
                    }
                }
                else if (IsBlankUnit(current))
                {
                    opened.Clear();
                }
            }
        }

        internal int Of(int open) => open >= 0 && open < closing.Length ? closing[open] : -1;
    }

    /// <summary>
    /// <c>(http://…)</c> right after a <c>]</c>: a valid link target with a
    /// balanced parenthesis, no whitespace, and something after the scheme.
    /// </summary>
    private static bool TryLinkTarget(
        ReadOnlySpan<char> text,
        int open,
        ClosingParentheses parens,
        out int end)
    {
        end = open;
        if (open >= text.Length || text[open] != '(')
        {
            return false;
        }

        var scheme = text[(open + 1)..];
        var schemeLength = scheme.StartsWith("http://", StringComparison.OrdinalIgnoreCase) ? 7
            : scheme.StartsWith("https://", StringComparison.OrdinalIgnoreCase) ? 8
            : 0;
        if (schemeLength == 0)
        {
            return false;
        }

        var close = parens.Of(open);
        if (close <= open + 1 + schemeLength)
        {
            return false;
        }

        end = close + 1;
        return true;
    }

    /// <summary>
    /// Where a bare address starting at <paramref name="index"/> ends, or false
    /// when there is no address there. Only lower-case schemes are addresses
    /// here, exactly as in the transcript's linkify pass. The end is trimmed the
    /// same way: sentence punctuation and a closing bracket that was not opened
    /// inside the address are not part of it.
    /// </summary>
    private static bool TryBareAddressEnd(ReadOnlySpan<char> text, int index, out int end, out int matchEnd)
    {
        end = matchEnd = index;
        var schemeLength = text[index..].StartsWith("http://", StringComparison.Ordinal) ? 7
            : text[index..].StartsWith("https://", StringComparison.Ordinal) ? 8
            : 0;
        if (schemeLength == 0 || index + schemeLength >= text.Length ||
            !IsAddressCharacter(text[index + schemeLength]))
        {
            return false;
        }

        matchEnd = index + schemeLength;
        while (matchEnd < text.Length && IsAddressCharacter(text[matchEnd])) matchEnd++;

        end = TrimAddress(text, index, matchEnd, formatting: false);
        return end > index;
    }

    private static int TrimAddress(ReadOnlySpan<char> text, int index, int matchEnd, bool formatting)
    {
        var openParentheses = 0;
        var closeParentheses = 0;
        var openBrackets = 0;
        var closeBrackets = 0;
        for (var cursor = index; cursor < matchEnd; cursor++)
        {
            switch (text[cursor])
            {
                case '(':
                    openParentheses++;
                    break;
                case ')':
                    closeParentheses++;
                    break;
                case '[':
                    openBrackets++;
                    break;
                case ']':
                    closeBrackets++;
                    break;
            }
        }

        var trimmed = matchEnd;
        while (trimmed > index)
        {
            var last = text[trimmed - 1];
            if (IsTrailingPunctuation(last) || (formatting && last is '*' or '_' or '~' or '|'))
            {
                trimmed--;
                continue;
            }

            if (last == ')' && closeParentheses > openParentheses)
            {
                closeParentheses--;
                trimmed--;
                continue;
            }

            if (last == ']' && closeBrackets > openBrackets)
            {
                closeBrackets--;
                trimmed--;
                continue;
            }

            break;
        }

        return trimmed;
    }

    /* ------------------------------------------------------------- characters */

    private static int LineEnd(ReadOnlySpan<char> text, int from)
    {
        var end = from;
        while (end < text.Length && text[end] is not ('\n' or '\r' or '\u2028' or '\u2029')) end++;
        return end;
    }

    private static bool IsBlankOnly(ReadOnlySpan<char> text)
    {
        foreach (var value in text)
        {
            if (value is not (' ' or '\t'))
            {
                return false;
            }
        }

        return true;
    }

    private static int RunLength(ReadOnlySpan<char> text, int from, char character)
    {
        var end = from;
        while (end < text.Length && text[end] == character) end++;
        return end - from;
    }

    private static void Exclude(bool[] excluded, int from, int to)
    {
        var start = Math.Max(0, from);
        var end = Math.Min(to, excluded.Length);
        for (var index = start; index < end; index++) excluded[index] = true;
    }

    private static bool IsAsciiPunctuation(char value) =>
        value is (>= '!' and <= '/') or (>= ':' and <= '@') or (>= '[' and <= '`') or (>= '{' and <= '~');

    /// <summary>The characters a bare address may hold between its scheme and the next space.</summary>
    private static bool IsAddressCharacter(char value) =>
        !IsJsWhitespace(value) && value is not ('<' or '>' or '"' or '\'' or '`');

    private static bool IsTrailingPunctuation(char value) =>
        value is '.' or ',' or ';' or ':' or '!' or '?' or '…' or '\'' or '"';

    /// <summary>Whitespace or a control character, which no address and no valid link target holds.</summary>
    private static bool IsBlankUnit(char value) =>
        value <= ' ' || value == '\u007f' || IsJsWhitespace(value);

    /// <summary>The characters JavaScript's <c>\s</c> matches, which bound a bare address.</summary>
    private static bool IsJsWhitespace(char value) =>
        value is '\t' or '\n' or '\v' or '\f' or '\r' or ' '
            or '\u00a0' or '\u1680' or (>= '\u2000' and <= '\u200a')
            or '\u2028' or '\u2029' or '\u202f' or '\u205f' or '\u3000' or '\ufeff';

    /// <summary>How many UTF-16 units the name character at <paramref name="index"/> takes, or zero.</summary>
    private static int NameCharacterWidth(string content, int index)
    {
        if (!Rune.TryGetRuneAt(content, index, out var rune))
        {
            return 0;
        }

        return IsNameCharacter(rune) ? rune.Utf16SequenceLength : 0;
    }

    /// <summary>Whether the code point before <paramref name="index"/> is a name character in visible text.</summary>
    private static bool NameCharacterBefore(string content, int index, bool[] excluded)
    {
        var start = index - 1;
        if (start > 0 && char.IsLowSurrogate(content[start]) && char.IsHighSurrogate(content[start - 1]))
        {
            start--;
        }

        return !excluded[start] && NameCharacterWidth(content, start) > 0;
    }

    private static bool IsNameCharacter(Rune value) =>
        Rune.IsLetterOrDigit(value) ||
        (value.IsBmp && (char)value.Value is '_' or '.' or '-');

    private static string Preview(string content)
    {
        var compact = string.Join(' ', content.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
        return compact.Length <= PreviewLength ? compact : $"{compact[..(PreviewLength - 3)]}…";
    }
}
