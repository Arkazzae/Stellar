using System.Globalization;
using System.Text;

// Reads one input a line, as hex of its UTF-8, and says what the first server's own code makes of it.
var inv = CultureInfo.InvariantCulture;
string line;
while ((line = Console.ReadLine()!) != null)
{
    var text = Encoding.UTF8.GetString(Convert.FromHexString(line));
    var d = double.TryParse(text, inv, out var dv) ? (double.IsNaN(dv) ? "nan" : BitConverter.DoubleToInt64Bits(dv).ToString("x")) : "x";
    var i = int.TryParse(text, inv, out var iv) ? iv.ToString() : "x";
    var b = bool.TryParse(text, out var bv) ? (bv ? "t" : "f") : "x";
    var p = IsPlaceholder(text) ? "1" : "0";
    var f = Hex(SafeFileName(text, ".pdf"));
    var r = ParseRange(text) is { } range ? $"{range.Item1}-{(range.Item2 is { } to ? to.ToString() : "")}" : "x";
    Console.WriteLine($"{d} {i} {b} {p} {f} {r}");
}

static string Hex(string s) => Convert.ToHexString(Encoding.UTF8.GetBytes(s));

static bool IsPlaceholder(string value)
{
    if (value.Length == 0 || value.Length % 4 != 0 || value.Any(char.IsWhiteSpace)) return false;
    Span<byte> decoded = stackalloc byte[32];
    return Convert.TryFromBase64String(value, decoded, out var written) && written > 0;
}

static string SafeFileName(string submittedName, string extension)
{
    var stem = Path.GetFileNameWithoutExtension(Path.GetFileName(submittedName));
    var safeStem = new string(stem
        .Where(value => char.IsLetterOrDigit(value) || value is ' ' or '-' or '_' or '.')
        .Take(200)
        .ToArray())
        .Trim(' ', '.');
    return $"{(safeStem.Length == 0 ? "attachment" : safeStem)}{extension}";
}

static (long, long?)? ParseRange(string value)
{
    const string BytesPrefix = "bytes=";
    if (!value.StartsWith(BytesPrefix, StringComparison.OrdinalIgnoreCase)) return null;
    var first = value.AsSpan(BytesPrefix.Length);
    var comma = first.IndexOf(',');
    if (comma >= 0) first = first[..comma];
    var dash = first.IndexOf('-');
    if (dash < 0 || !long.TryParse(first[..dash].Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var from)) return null;
    var end = first[(dash + 1)..].Trim();
    if (end.IsEmpty) return (from, null);
    return long.TryParse(end, NumberStyles.None, CultureInfo.InvariantCulture, out var to) && to >= from ? (from, to) : null;
}
