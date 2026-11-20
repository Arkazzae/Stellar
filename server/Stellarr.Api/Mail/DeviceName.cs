using System.Text.RegularExpressions;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Mail;

/// <summary>
/// The browser and system of a user-agent string, named for a person reading
/// a security alert. The same patterns as <c>describeUserAgent</c> in
/// web/src/stellarr/lib/platform/userAgent.ts, so the mail names a device as
/// the account's activity list does.
/// </summary>
public static partial class DeviceName
{
    // Order matters: Edge and Opera also claim to be Chrome, and Chrome also
    // claims to be Safari.
    private static readonly (Regex Pattern, string Name)[] Browsers =
    [
        (EdgePattern(), "Edge"),
        (OperaPattern(), "Opera"),
        (FirefoxPattern(), "Firefox"),
        (ChromePattern(), "Chrome"),
        (SafariPattern(), "Safari"),
    ];

    private static readonly (Regex Pattern, string Name)[] Systems =
    [
        (IosPattern(), "iOS"),
        (AndroidPattern(), "Android"),
        (WindowsPattern(), "Windows"),
        (MacPattern(), "macOS"),
        (ChromeOsPattern(), "ChromeOS"),
        (LinuxPattern(), "Linux"),
    ];

    /// <summary>"Firefox on Linux", one of the two, or null when neither is recognised.</summary>
    public static string? Describe(string userAgent, Messages messages)
    {
        var browser = Browsers.FirstOrDefault(x => x.Pattern.IsMatch(userAgent)).Name;
        var system = Systems.FirstOrDefault(x => x.Pattern.IsMatch(userAgent)).Name;
        return browser is not null && system is not null
            ? messages.Get(MessageKey.SecurityAlertDeviceOn, ("browser", browser), ("system", system))
            : browser ?? system;
    }

    [GeneratedRegex(@"\bEdg(?:e|A|iOS)?/")]
    private static partial Regex EdgePattern();

    [GeneratedRegex(@"\bOPR/|\bOpera\b")]
    private static partial Regex OperaPattern();

    [GeneratedRegex(@"\bFirefox/|\bFxiOS/")]
    private static partial Regex FirefoxPattern();

    [GeneratedRegex(@"\bChrome/|\bCriOS/")]
    private static partial Regex ChromePattern();

    [GeneratedRegex(@"\bVersion/[\d.]+.*\bSafari/")]
    private static partial Regex SafariPattern();

    [GeneratedRegex(@"\biPhone\b|\biPad\b|\biPod\b")]
    private static partial Regex IosPattern();

    [GeneratedRegex(@"\bAndroid\b")]
    private static partial Regex AndroidPattern();

    [GeneratedRegex(@"\bWindows\b")]
    private static partial Regex WindowsPattern();

    [GeneratedRegex(@"\bMac OS X\b|\bMacintosh\b")]
    private static partial Regex MacPattern();

    [GeneratedRegex(@"\bCrOS\b")]
    private static partial Regex ChromeOsPattern();

    [GeneratedRegex(@"\bLinux\b")]
    private static partial Regex LinuxPattern();
}
