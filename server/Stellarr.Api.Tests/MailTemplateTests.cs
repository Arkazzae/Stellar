using System.Text.RegularExpressions;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Mail;

namespace Stellarr.Api.Tests;

/// <summary>What the branded messages say and carry, apart from how a provider sends them.</summary>
public sealed partial class MailTemplateTests
{
    private const string From = "Stellarr <noreply@chat.test>";
    private const string To = "jo@example.test";
    private const string PublicUrl = "https://chat.test";

    private static readonly Messages Polish = new(new LanguageOptions { Language = LanguageOptions.Polish });
    private static readonly Messages English = new(new LanguageOptions { Language = LanguageOptions.English });

    public static TheoryData<string> Templates
    {
        get
        {
            var data = new TheoryData<string>();
            foreach (var (name, _) in AllTemplates(Polish))
            {
                data.Add(name);
            }

            return data;
        }
    }

    [Theory]
    [MemberData(nameof(Templates))]
    public void EveryImageTheHtmlShowsTravelsWithTheMessageAndNothingElse(string name)
    {
        var mail = AllTemplates(Polish).Single(x => x.Name == name).Mail;

        var shown = CidReference().Matches(mail.Html).Select(x => x.Groups[1].Value).Distinct().Order();
        var carried = mail.InlineImages.Select(x => x.ContentId).Order();

        Assert.Equal(carried, shown);
        Assert.Contains(MailImages.Lockup, mail.InlineImages);
        // Nothing is fetched when the mail is opened.
        Assert.DoesNotMatch(RemoteSource(), mail.Html);
    }

    [Theory]
    [MemberData(nameof(Templates))]
    public void EveryMessageIsInTheInstancesLanguageAndHasAPlainTextPart(string name)
    {
        var mail = AllTemplates(Polish).Single(x => x.Name == name).Mail;

        Assert.StartsWith("<!doctype html>\n<html lang=\"pl\"", mail.Html, StringComparison.Ordinal);
        Assert.NotEmpty(mail.Text);
        Assert.DoesNotContain("<", mail.Text, StringComparison.Ordinal);
    }

    [Fact]
    public void TheImagesAreEmbeddedPngs()
    {
        foreach (var image in new[] { MailImages.Lockup, MailImages.Mention, MailImages.DirectMessage })
        {
            Assert.Equal("image/png", image.ContentType);
            Assert.Equal(new byte[] { 0x89, 0x50, 0x4E, 0x47 }, image.Content[..4]);
        }
    }

    [Fact]
    public void ASecurityAlertNamesTheEventTheTimeInTheInstancesZoneAndTheDevice()
    {
        var mail = AccountEmail.SecurityAlert(
            From,
            To,
            "Jo",
            "Orbit",
            PublicUrl,
            Event(SecurityEventKinds.LoginSucceeded),
            TimeZoneInfo.FindSystemTimeZoneById("Europe/Warsaw"),
            Polish);

        Assert.Equal("Aktywność zabezpieczeń w Orbit", mail.Subject);
        Assert.Contains("Udane logowanie", mail.Html, StringComparison.Ordinal);
        // 13:05 UTC is 14:05 in Warsaw in winter; the month is in the genitive.
        Assert.Contains("20 listopada 2026, 14:05 (Europe/Warsaw)", mail.Html, StringComparison.Ordinal);
        Assert.Contains("Firefox na Linux", mail.Html, StringComparison.Ordinal);
        Assert.Contains("203.0.113.42", mail.Html, StringComparison.Ordinal);
        Assert.DoesNotContain("login-succeeded", mail.Html, StringComparison.Ordinal);
        Assert.Contains("Zdarzenie: Udane logowanie\nKiedy: 20 listopada 2026, 14:05 (Europe/Warsaw)\n", mail.Text, StringComparison.Ordinal);
        Assert.Contains("Urządzenie: Firefox na Linux", mail.Text, StringComparison.Ordinal);
    }

    [Fact]
    public void AnEventWithoutANameOfItsOwnIsShownByItsKind()
    {
        Assert.Equal("Event: something-new", AccountEmail.DescribeEvent("something-new", English));
        Assert.Equal("Password changed", AccountEmail.DescribeEvent(SecurityEventKinds.PasswordChanged, English));
    }

    [Fact]
    public void AnAlertLeavesOutTheDeviceAndAddressItDoesNotKnow()
    {
        var unknown = Event(SecurityEventKinds.PasswordChanged);
        unknown.Device = "curl/8.0";
        unknown.IpAddress = "";

        var mail = AccountEmail.SecurityAlert(From, To, "Jo", "Orbit", PublicUrl, unknown, TimeZoneInfo.Utc, English);

        Assert.Contains("Password changed", mail.Html, StringComparison.Ordinal);
        Assert.DoesNotContain("Device", mail.Html, StringComparison.Ordinal);
        Assert.DoesNotContain("IP address", mail.Html, StringComparison.Ordinal);
    }

    [Fact]
    public void ACodeMailShowsTheCodeAndHowLongItLasts()
    {
        var mail = AccountEmail.EmailCode(From, To, "Jo", "Orbit", PublicUrl, "482913", 8, Guid.NewGuid(), English);

        Assert.Equal("Your verification code for Orbit", mail.Subject);
        Assert.Contains(">482913</td>", mail.Html, StringComparison.Ordinal);
        Assert.Contains("valid for 8 min", mail.Html, StringComparison.Ordinal);
        Assert.Contains("\n482913\n", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Security activity", mail.Html, StringComparison.Ordinal);
    }

    [Fact]
    public void ADeliveryTestNamesTheWayItWent()
    {
        var smtp = new MailTransport(
            MailProvider.Smtp, new SmtpConnection("mail.chat.test", 587, SmtpSecurity.StartTls, "u", "secret"), true);
        var resend = new MailTransport(MailProvider.Resend, null, true, "re_key");

        var throughSmtp = AccountEmail.DeliveryTest(From, To, "Orbit", PublicUrl, smtp, English);
        var throughResend = AccountEmail.DeliveryTest(From, To, "Orbit", PublicUrl, resend, English);

        Assert.Contains("SMTP, mail.chat.test:587", throughSmtp.Html, StringComparison.Ordinal);
        Assert.DoesNotContain("secret", throughSmtp.Html, StringComparison.Ordinal);
        Assert.DoesNotContain("Resend", throughSmtp.Html, StringComparison.Ordinal);
        Assert.Contains("Resend", throughResend.Html, StringComparison.Ordinal);
        Assert.DoesNotContain("re_key", throughResend.Html, StringComparison.Ordinal);
    }

    [Fact]
    public void ActivityMailNamesNoInstanceAndSaysHowToStopIt()
    {
        var mail = ActivityEmail.DirectMessage(From, To, PublicUrl, Guid.NewGuid(), English);

        Assert.Contains(MailImages.DirectMessage, mail.InlineImages);
        Assert.Contains("Settings › Notifications", mail.Html, StringComparison.Ordinal);
        Assert.Contains("Settings › Notifications", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Sent by the", mail.Html, StringComparison.Ordinal);
    }

    [Fact]
    public void TheDefaultInstanceNameIsNotRepeatedBesideTheLockup()
    {
        var plain = AccountEmail.PasswordReset(From, To, "Jo", "Stellarr", "https://chat.test/reset", Guid.NewGuid(), English);
        var named = AccountEmail.PasswordReset(From, To, "Jo", "Orbit", "https://chat.test/reset", Guid.NewGuid(), English);

        Assert.DoesNotContain("class=\"instance\"", plain.Html, StringComparison.Ordinal);
        Assert.Contains("class=\"instance\"", named.Html, StringComparison.Ordinal);
        Assert.Contains(">Orbit</td>", named.Html, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0", "Firefox on Linux")]
    [InlineData("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0", "Edge on Windows")]
    [InlineData("Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1", "Safari on iOS")]
    [InlineData("Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36", "Chrome on Android")]
    [InlineData("curl/8.0", null)]
    [InlineData("", null)]
    public void ADeviceIsNamedByItsBrowserAndSystem(string userAgent, string? expected) =>
        Assert.Equal(expected, DeviceName.Describe(userAgent, English));

    private static SecurityEvent Event(string kind) => new()
    {
        Id = Guid.NewGuid(),
        Kind = kind,
        Detail = "password",
        IpAddress = "203.0.113.42",
        Device = "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Gecko/20100101 Firefox/146.0",
        CreatedAt = new DateTime(2026, 11, 20, 13, 5, 0, DateTimeKind.Unspecified),
    };

    private static (string Name, EmailMessage Mail)[] AllTemplates(Messages messages) =>
    [
        ("password-reset", AccountEmail.PasswordReset(From, To, "Jo", "Orbit", $"{PublicUrl}/reset?token=a", Guid.NewGuid(), messages)),
        ("email-change", AccountEmail.EmailChange(From, To, "Jo", "Orbit", $"{PublicUrl}/confirm?token=a", Guid.NewGuid(), messages)),
        ("email-code", AccountEmail.EmailCode(From, To, "Jo", "Orbit", PublicUrl, "123456", 8, Guid.NewGuid(), messages)),
        ("security-alert", AccountEmail.SecurityAlert(From, To, "Jo", "Orbit", PublicUrl, Event(SecurityEventKinds.LoginSucceeded), TimeZoneInfo.Utc, messages)),
        ("delivery-test", AccountEmail.DeliveryTest(From, To, "Orbit", PublicUrl, new MailTransport(MailProvider.Resend, null, true, "k"), messages)),
        ("mention", ActivityEmail.Mention(From, To, PublicUrl, Guid.NewGuid(), messages)),
        ("direct-message", ActivityEmail.DirectMessage(From, To, PublicUrl, Guid.NewGuid(), messages)),
    ];

    [GeneratedRegex("src=\"cid:([^\"]+)\"")]
    private static partial Regex CidReference();

    [GeneratedRegex("(src|href)=\"(?!cid:|https://chat\\.test)[a-z]+:", RegexOptions.IgnoreCase)]
    private static partial Regex RemoteSource();
}
