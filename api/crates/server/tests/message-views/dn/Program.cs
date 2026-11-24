using System.Buffers.Binary;
using System.Buffers.Text;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;

public enum MessageIndexKind { All = 0, Media = 1, Links = 2 }

public static partial class P
{
    private const int MaxTerms = 16;
    private const char LowerStrokeL = 'ł';
    private const char UpperStrokeL = 'Ł';

    public static string? BuildQuery(string query)
    {
        var terms = Words().Matches(query)
            .Select(match => match.Value)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .Take(MaxTerms)
            .Select(PrefixTerm)
            .ToArray();
        return terms.Length == 0 ? null : string.Join(" AND ", terms);
    }
    private static string PrefixTerm(string word)
    {
        var prefix = $"\"{word}\"*";
        return ExchangeL(word) is { } other ? $"({prefix} OR \"{other}\"*)" : prefix;
    }
    private static string? ExchangeL(string word)
    {
        if (word.Contains(LowerStrokeL) || word.Contains(UpperStrokeL))
            return word.Replace(LowerStrokeL, 'l').Replace(UpperStrokeL, 'L');
        return word.Contains('l') || word.Contains('L')
            ? word.Replace('l', LowerStrokeL).Replace('L', UpperStrokeL) : null;
    }
    [GeneratedRegex(@"[\p{L}\p{N}]+", RegexOptions.CultureInvariant)]
    private static partial Regex Words();

    static ulong HashOf(string? query) =>
        BinaryPrimitives.ReadUInt64BigEndian(SHA256.HashData(Encoding.UTF8.GetBytes(query ?? "")));

    readonly record struct IndexCursor(Guid ChannelId, DateTime Snapshot, int Offset, MessageIndexKind Kind, ulong QueryHash)
    {
        private const int ChannelIdSize = 16;
        private const int SnapshotAt = ChannelIdSize;
        private const int OffsetAt = SnapshotAt + sizeof(long);
        private const int KindAt = OffsetAt + sizeof(int);
        private const int QueryHashAt = KindAt + sizeof(byte);
        private const int Size = QueryHashAt + sizeof(ulong);
        public static bool TryDecode(string encoded, out IndexCursor cursor)
        {
            cursor = default;
            if (!Base64Url.IsValid(encoded, out var decodedLength) || decodedLength != Size) return false;
            Span<byte> bytes = stackalloc byte[Size];
            Base64Url.DecodeFromChars(encoded, bytes);
            var ticks = BinaryPrimitives.ReadInt64BigEndian(bytes[SnapshotAt..]);
            var offset = BinaryPrimitives.ReadInt32BigEndian(bytes[OffsetAt..]);
            var kind = (MessageIndexKind)bytes[KindAt];
            if (ticks < 0 || ticks > DateTime.MaxValue.Ticks || offset < 0 || !Enum.IsDefined(kind)) return false;
            cursor = new IndexCursor(new Guid(bytes[..ChannelIdSize]), new DateTime(ticks, DateTimeKind.Utc), offset, kind,
                BinaryPrimitives.ReadUInt64BigEndian(bytes[QueryHashAt..]));
            return true;
        }
    }

    static string B(string s) => Convert.ToBase64String(Encoding.Unicode.GetBytes(s));
    static string FromB(string s) => Encoding.Unicode.GetString(Convert.FromBase64String(s));
    static string FromU8(string s) => Encoding.UTF8.GetString(Convert.FromBase64String(s));

    public static void Main()
    {
        string? line;
        while ((line = Console.ReadLine()) != null)
        {
            var parts = line.Split(' ');
            switch (parts[0])
            {
                case "Q": { var r = BuildQuery(FromU8(parts[1])); Console.WriteLine(r is null ? "-" : B(r)); break; }
                case "CF": {
                    var groups = new Dictionary<int, List<int>>();
                    for (int i = 0; i < 65536; i++) { if (i >= 0xD800 && i <= 0xDFFF) continue; var h = ((char)i).ToString().GetHashCode(); groups.TryAdd(h, new()); groups[h].Add(i); }
                    // use OrdinalIgnoreCase hash
                    groups.Clear();
                    for (int i = 0; i < 65536; i++) { if (i >= 0xD800 && i <= 0xDFFF) continue; var h = StringComparer.OrdinalIgnoreCase.GetHashCode(((char)i).ToString()); groups.TryAdd(h, new()); groups[h].Add(i); }
                    foreach (var g in groups.Values) {
                        var done = new List<List<int>>();
                        foreach (var i in g) {
                            var placed = false;
                            foreach (var cls in done) if (StringComparer.OrdinalIgnoreCase.Equals(((char)i).ToString(), ((char)cls[0]).ToString())) { cls.Add(i); placed = true; break; }
                            if (!placed) done.Add(new() { i });
                        }
                        foreach (var cls in done) if (cls.Count > 1) Console.WriteLine(string.Join(",", cls));
                    }
                    Console.WriteLine("END"); break; }
                case "LN": { var l = new List<int>(); for (int i = 0; i < 65536; i++) { var c = (char)i; var cat = char.GetUnicodeCategory(c); if (Regex.IsMatch(c.ToString(), @"^[\p{L}\p{N}]$")) l.Add(i); } Console.WriteLine(string.Join(",", l)); break; }
                case "WS": { var l = new List<int>(); for (int i = 0; i < 65536; i++) if (char.IsWhiteSpace((char)i)) l.Add(i); Console.WriteLine(string.Join(",", l)); break; }
                case "D": {
                    IndexCursor c = default; bool ok;
                    try { ok = IndexCursor.TryDecode(parts.Length > 1 ? parts[1] : "", out c); } catch (FormatException) { Console.WriteLine("crash"); break; }
                    Console.WriteLine(ok ? $"ok {c.ChannelId} {c.Snapshot.Ticks} {c.Offset} {(int)c.Kind} {c.QueryHash}" : "bad"); break; }
                case "H": Console.WriteLine(HashOf(FromU8(parts[1]))); break;
                case "L": Console.WriteLine(B(FromU8(parts[1]).ToLowerInvariant())); break;
                case "U": Console.WriteLine(B(FromU8(parts[1]).ToUpperInvariant())); break;
                case "G": { var ok = Guid.TryParse(FromU8(parts[1]), out var g); Console.WriteLine(ok ? g.ToString() : "bad"); break; }
                case "I": { var ok = int.TryParse(FromU8(parts[1]), System.Globalization.NumberStyles.Integer, System.Globalization.CultureInfo.InvariantCulture, out var g); Console.WriteLine(ok ? g.ToString() : "bad"); break; }
                case "R": { // reference preview
                    var content = FromU8(parts[1]);
                    var preview = string.Join(' ', content.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
                    if (preview.Length > 160) preview = $"{preview[..157]}…";
                    Console.WriteLine(B(preview)); break; }
                case "T": { var t = FromU8(parts[1]).Trim(); Console.WriteLine(B(t)); break; }
            }
        }
    }
}
