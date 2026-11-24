using System.Net.Mail;
using System.Text;
using System.Text.Json;

// Reads one input a line, as a JSON string, and says what the first server's
// own address reader makes of it: `valid` and the address it read out (as a
// JSON string), or `invalid`. Every character outside printable ASCII is
// written as a \u escape of its UTF-16 unit, so the answers are plain ASCII
// and compare byte for byte with the ones of the second server.
//
// Build it outside the repository, so that nothing is written next to it:
//   cp Program.cs witness.csproj <dir> && dotnet build <dir> -c Release -o <dir>/out
//   dotnet <dir>/out/witness.dll < inputs.jsonl > answers.txt
// The answers are checked by the ignored test at the end of
// src/cockpit/address.rs.
var utf8 = new UTF8Encoding(false, true);
using var input = new StreamReader(Console.OpenStandardInput(), utf8);
using var output = new StreamWriter(Console.OpenStandardOutput(), utf8) { NewLine = "\n" };
string? line;
while ((line = input.ReadLine()) != null)
{
    var text = JsonSerializer.Deserialize<string>(line) ?? throw new InvalidDataException("null instead of a string");
    string answer;
    try
    {
        answer = MailAddress.TryCreate(text, out var parsed) ? "valid " + Quote(parsed.Address) : "invalid";
    }
    catch (Exception e)
    {
        // The reader is not meant to throw; say so rather than stop half way.
        answer = "throws " + e.GetType().Name;
    }
    output.WriteLine(answer);
}

static string Quote(string value)
{
    var quoted = new StringBuilder("\"");
    foreach (var c in value)
    {
        if (c == '"' || c == '\\') quoted.Append('\\').Append(c);
        else if (c < ' ' || c > '~') quoted.Append("\\u").Append(((int)c).ToString("x4", System.Globalization.CultureInfo.InvariantCulture));
        else quoted.Append(c);
    }
    return quoted.Append('"').ToString();
}
