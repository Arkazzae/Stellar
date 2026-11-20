using System.Globalization;
using System.Net;
using System.Text;

namespace Stellarr.Api.Mail;

/// <summary>
/// The look every message shares: the Stellarr lockup above
/// one card on the Midnight surfaces, the indigo accent only on the main
/// action, and small print below. Each method takes plain text and encodes it.
/// </summary>
/// <remarks>
/// Mail clients keep little of CSS, so the page is tables with inline styles,
/// and it loads nothing: the lockup and the stickers travel inside the message
/// as <see cref="InlineImage"/>s, and the type falls back from Outfit and
/// JetBrains Mono, which only a few readers have installed, to the system's.
/// The page declares itself dark so that clients which repaint light mail for
/// a dark mode leave it alone. Classic Outlook ignores max-width, so a table
/// only it reads holds the column at 560 px there.
/// </remarks>
public sealed class MailLayout(string language)
{
    // Midnight, from branding/README.md.
    private const string Darkest = "#09090b";
    private const string Surface = "#18181b";
    private const string Border = "#292e39";
    private const string Text = "#f3f4f6";
    private const string Muted = "#b9bec9";
    private const string Faint = "#9aa0ad";
    private const string Accent = "#6f65d1";

    private const string Sans = "'Outfit',ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";
    private const string Mono = "'JetBrains Mono',ui-monospace,SFMono-Regular,Menlo,Consolas,'Liberation Mono',monospace";

    private readonly StringBuilder _content = new();
    private readonly List<InlineImage> _images = [MailImages.Lockup];
    private readonly List<string> _footer = [];
    private string _preheader = "";
    private string? _instance;

    // The bottom margin of the last block, which the card's own padding
    // gives back so the space under a closing button matches the top.
    private int _trailing;

    /// <summary>Names the instance beside the lockup. Activity mail leaves it out on purpose.</summary>
    public MailLayout Instance(string name)
    {
        // An instance that kept the default name would read "Stellarr" twice.
        _instance = string.Equals(name.Trim(), "Stellarr", StringComparison.OrdinalIgnoreCase) ? null : name;
        return this;
    }

    /// <summary>The line a mail client shows next to the subject in the list; it is not visible in the message.</summary>
    public MailLayout Preheader(string text)
    {
        _preheader = text;
        return this;
    }

    public MailLayout Illustration(InlineImage image)
    {
        _images.Add(image);
        // Decorative: the heading below says what the mascot shows.
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<img src="cid:{Encode(image.ContentId)}" width="112" height="112" alt="" style="display:block;width:112px;height:112px;border:0;margin:0 0 20px">""");
        _trailing = 20;
        return this;
    }

    public MailLayout Heading(string text)
    {
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<h1 style="margin:0 0 12px;font-family:{Sans};font-size:24px;line-height:32px;font-weight:600;letter-spacing:-0.01em;color:{Text}">{Encode(text)}</h1>""");
        _trailing = 12;
        return this;
    }

    public MailLayout Paragraph(string text)
    {
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<p style="margin:0 0 16px;font-family:{Sans};font-size:15px;line-height:24px;color:{Muted}">{Encode(text)}</p>""");
        _trailing = 16;
        return this;
    }

    /// <summary>The main action. A table cell carries the colour, so clients that drop padding on links still show a button.</summary>
    public MailLayout Button(string url, string label)
    {
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px"><tr><td bgcolor="{Accent}" style="background:{Accent};border-radius:8px"><a href="{Encode(url)}" target="_blank" style="display:inline-block;padding:12px 22px;font-family:{Sans};font-size:15px;line-height:20px;font-weight:600;color:#ffffff;text-decoration:none;border-radius:8px">{Encode(label)}</a></td></tr></table>""");
        _trailing = 24;
        return this;
    }

    /// <summary>The address of the button spelled out, for clients that do not open it.</summary>
    public MailLayout LinkFallback(string url, string label)
    {
        var link = Encode(url);
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<p style="margin:0 0 4px;font-family:{Sans};font-size:13px;line-height:20px;color:{Faint}">{Encode(label)}</p>""");
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<p style="margin:0 0 24px;font-family:{Mono};font-size:12px;line-height:18px;word-break:break-all"><a href="{link}" target="_blank" style="color:{Muted};text-decoration:underline">{link}</a></p>""");
        _trailing = 24;
        return this;
    }

    /// <summary>A one-time code, large and spaced so it reads at a glance and copies as one word.</summary>
    public MailLayout Code(string code)
    {
        // letter-spacing also follows the last digit, so the left padding is
        // larger by that much to keep the code centred.
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px"><tr><td align="center" bgcolor="{Darkest}" class="code" style="background:{Darkest};border:1px solid {Border};border-radius:12px;padding:20px 12px 20px 22px;font-family:{Mono};font-size:32px;line-height:40px;font-weight:600;letter-spacing:0.3em;color:{Text}">{Encode(code)}</td></tr></table>""");
        _trailing = 24;
        return this;
    }

    /// <summary>Labelled facts, one under another, so they hold on a narrow screen.</summary>
    public MailLayout Details(IReadOnlyList<(string Label, string Value)> rows)
    {
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{Darkest}" style="margin:4px 0 24px;background:{Darkest};border:1px solid {Border};border-radius:12px;border-collapse:separate">""");
        for (var index = 0; index < rows.Count; index++)
        {
            var (label, value) = rows[index];
            var rule = index < rows.Count - 1 ? $"border-bottom:1px solid {Border};" : "";
            _content.Append(
                CultureInfo.InvariantCulture,
                $"""<tr><td style="{rule}padding:12px 16px"><div style="font-family:{Sans};font-size:12px;line-height:16px;color:{Faint}">{Encode(label)}</div><div style="padding-top:2px;font-family:{Sans};font-size:15px;line-height:22px;font-weight:500;color:{Text}">{Encode(value)}</div></td></tr>""");
        }

        _content.Append("</table>");
        _trailing = 24;
        return this;
    }

    public MailLayout Divider()
    {
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 20px"><tr><td style="border-top:1px solid {Border};font-size:0;line-height:0;height:1px">&nbsp;</td></tr></table>""");
        _trailing = 20;
        return this;
    }

    /// <summary>Small print inside the card: what to do if the mail was not expected.</summary>
    public MailLayout Note(string text)
    {
        _content.Append(
            CultureInfo.InvariantCulture,
            $"""<p style="margin:0;font-family:{Sans};font-size:13px;line-height:20px;color:{Faint}">{Encode(text)}</p>""");
        _trailing = 0;
        return this;
    }

    /// <summary>A line under the card.</summary>
    public MailLayout Footer(string text)
    {
        _footer.Add(Encode(text));
        return this;
    }

    /// <summary>The instance's address under the card, as the host alone.</summary>
    public MailLayout FooterLink(string url)
    {
        if (url.Length == 0)
        {
            return this;
        }

        var host = Uri.TryCreate(url, UriKind.Absolute, out var parsed) ? parsed.Authority : url;
        _footer.Add($"""<a href="{Encode(url)}" target="_blank" style="color:{Faint};text-decoration:underline">{Encode(host)}</a>""");
        return this;
    }

    public EmailMessage ToMessage(string from, string to, string subject, string text, string idempotencyKey) =>
        new(from, to, subject, Render(subject), text, idempotencyKey, _images.ToArray());

    private string Render(string subject)
    {
        var instance = _instance is null
            ? ""
            : $"""<td class="instance" align="right" style="vertical-align:middle;padding-left:16px;font-family:{Sans};font-size:13px;line-height:18px;color:{Faint}">{Encode(_instance)}</td>""";
        var footer = _footer.Count == 0
            ? ""
            : $"""<tr><td style="padding:20px 8px 0;font-family:{Sans};font-size:12px;line-height:18px;color:{Faint}">{string.Join("<br>", _footer)}</td></tr>""";
        // Fillers after the preheader keep clients from running the body's
        // first words into the preview line.
        var filler = string.Concat(Enumerable.Repeat("&#847;&zwnj;&nbsp;", 60));

        return $$"""
            <!doctype html>
            <html lang="{{Encode(language)}}">
            <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1">
            <meta name="x-apple-disable-message-reformatting">
            <meta name="color-scheme" content="dark">
            <meta name="supported-color-schemes" content="dark">
            <title>{{Encode(subject)}}</title>
            <style>
            :root { color-scheme: dark; supported-color-schemes: dark; }
            body { margin: 0; padding: 0; -webkit-text-size-adjust: 100%; }
            @media (max-width: 600px) {
              .shell { padding: 24px 12px !important; }
              .card { padding-top: 28px !important; padding-left: 20px !important; padding-right: 20px !important; }
              .code { font-size: 28px !important; }
            }
            </style>
            </head>
            <body bgcolor="{{Darkest}}" style="margin:0;padding:0;background:{{Darkest}}">
            <div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;mso-hide:all">{{Encode(_preheader)}}{{filler}}</div>
            <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{{Darkest}}" style="background:{{Darkest}}">
            <tr><td align="center" class="shell" style="padding:40px 16px">
            <!--[if mso]><table role="presentation" width="560" align="center" cellpadding="0" cellspacing="0" border="0"><tr><td><![endif]-->
            <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:560px">
            <tr><td style="padding:0 8px 24px">
            <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0"><tr>
            <td style="vertical-align:middle"><img src="cid:{{Encode(MailImages.Lockup.ContentId)}}" width="141" height="36" alt="Stellarr" style="display:block;width:141px;height:36px;border:0;font-family:{{Sans}};font-size:20px;font-weight:600;color:{{Text}}"></td>
            {{instance}}
            </tr></table>
            </td></tr>
            <tr><td class="card" bgcolor="{{Surface}}" style="background:{{Surface}};border:1px solid {{Border}};border-radius:16px;padding:36px 32px {{Math.Max(12, 36 - _trailing).ToString(CultureInfo.InvariantCulture)}}px">
            {{_content}}
            </td></tr>
            {{footer}}
            </table>
            <!--[if mso]></td></tr></table><![endif]-->
            </td></tr>
            </table>
            </body>
            </html>
            """;
    }

    private static string Encode(string value) => WebUtility.HtmlEncode(value);
}
