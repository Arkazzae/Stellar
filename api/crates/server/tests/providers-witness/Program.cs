// Asks the first server's own code what its pure rules make of an input, for
// the tests of the identity provider group.
//
//   dotnet run -- table           the letters and digits (and separators) each
//                                 non-ASCII character becomes once it is
//                                 lower-cased and decomposed, as Rust source
//   dotnet run -- check < lines   one JSON string per line in; one JSON object
//                                 per line out: what SafeReturnUrl and
//                                 PlainLogin make of it
//   dotnet run -- configured < lines
//                                 one JSON string per line (an authority); out:
//                                 whether a provider with it counts as set up
using System.Globalization;
using System.Text;
using System.Text.Json;
using Stellarr.Api.Auth;

namespace Stellarr.Api.Auth.Security
{
    // The one thing the copied options file asks of the rest of the server.
    internal static class MfaPolicy
    {
        public static readonly TimeSpan FreshMfaWindow = TimeSpan.FromMinutes(5);
    }
}

internal static class Witness
{
    private static int Main(string[] args)
    {
        switch (args.FirstOrDefault())
        {
            case "table":
                Table();
                return 0;
            case "check":
                Check();
                return 0;
            case "configured":
                Configured();
                return 0;
            default:
                Console.Error.WriteLine("table, check or configured");
                return 2;
        }
    }

    // What the loop of PlainLogin sees of one character after the text was
    // lower-cased and decomposed: only the characters it keeps.
    private static string Kept(string decomposed)
    {
        var kept = new StringBuilder();
        foreach (var c in decomposed)
        {
            if (CharUnicodeInfo.GetUnicodeCategory(c) == UnicodeCategory.NonSpacingMark)
            {
                continue;
            }

            if (c is (>= 'a' and <= 'z') or (>= '0' and <= '9') or '.' or '_' or '-')
            {
                kept.Append(c);
            }
            else if (char.IsWhiteSpace(c))
            {
                kept.Append(' ');
            }
        }

        return kept.ToString();
    }

    private static void Table()
    {
        for (var code = 0x80; code <= 0xFFFF; code++)
        {
            if (code is >= 0xD800 and <= 0xDFFF)
            {
                continue;
            }

            string kept;
            try
            {
                kept = Kept(((char)code).ToString().Normalize(NormalizationForm.FormD));
            }
            catch (ArgumentException)
            {
                // Not text the runtime can decompose; PlainLogin throws on it.
                Console.Error.WriteLine($"refused {code:X4}");
                continue;
            }

            if (kept.Length > 0)
            {
                Console.WriteLine($"{code:X4}\t{kept}");
            }
        }
    }

    private static void Check()
    {
        string? line;
        while ((line = Console.ReadLine()) is not null)
        {
            var text = JsonSerializer.Deserialize<string>(line);
            Console.WriteLine(JsonSerializer.Serialize(new
            {
                safe = Stellarr.Api.Auth.ExternalOidcOptions.SafeReturnUrl(text),
                login = PlainLogin(text),
            }));
        }
    }

    private static void Configured()
    {
        string? line;
        while ((line = Console.ReadLine()) is not null)
        {
            var authority = JsonSerializer.Deserialize<string>(line) ?? "";
            var options = new ExternalOidcOptions
            {
                Enabled = true, Authority = authority, ClientId = "c", ClientSecret = "s",
            };
            Console.WriteLine(JsonSerializer.Serialize(options.IsConfigured));
        }
    }

    // A copy of the rule as the first server has it.
    private static string PlainLogin(string? text)
    {
        var decomposed = (text ?? "").ToLowerInvariant().Replace('ł', 'l').Normalize(NormalizationForm.FormD);
        var login = new StringBuilder();
        char? separator = null;
        foreach (var c in decomposed.Trim())
        {
            if (CharUnicodeInfo.GetUnicodeCategory(c) == UnicodeCategory.NonSpacingMark)
            {
                continue;
            }

            if (c is (>= 'a' and <= 'z') or (>= '0' and <= '9'))
            {
                if (separator is { } between && login.Length > 0)
                {
                    login.Append(between);
                }

                separator = null;
                login.Append(c);
            }
            else if (char.IsWhiteSpace(c) || c is '.' or '_' or '-')
            {
                separator ??= char.IsWhiteSpace(c) ? '.' : c;
            }
        }

        var result = login.ToString();
        return result.Length <= 32 ? result : result[..32].TrimEnd('.', '_', '-');
    }
}
