using System.Globalization;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Mail;

/// <summary>The messages the instance sends about an account or about itself.</summary>
public static class AccountEmail
{
    /// <param name="messages">The catalog of the instance.</param>
    public static EmailMessage PasswordReset(
        string from,
        string to,
        string displayName,
        string instanceName,
        string resetUrl,
        Guid tokenId,
        Messages messages)
    {
        var intro = messages.Get(MessageKey.PasswordResetIntro, ("name", displayName));
        var ignore = messages.Get(MessageKey.PasswordResetIgnore);
        var text = Lines(
            messages.Get(MessageKey.PasswordResetGreeting, ("name", displayName)),
            "",
            messages.Get(MessageKey.PasswordResetInstruction, ("instance", instanceName)),
            resetUrl,
            "",
            messages.Get(MessageKey.PasswordResetValidity),
            Signature(instanceName, OriginOf(resetUrl), messages));

        return new MailLayout(messages.Language)
            .Instance(instanceName)
            .Preheader(intro)
            .Heading(messages.Get(MessageKey.PasswordResetHeading))
            .Paragraph(intro)
            .Button(resetUrl, messages.Get(MessageKey.PasswordResetButton))
            .LinkFallback(resetUrl, messages.Get(MessageKey.MailLinkFallback))
            .Divider()
            .Note(ignore)
            .Footer(messages.Get(MessageKey.MailSentFromInstance, ("instance", instanceName)))
            .FooterLink(OriginOf(resetUrl))
            .ToMessage(
                from,
                to,
                messages.Get(MessageKey.PasswordResetSubject, ("instance", instanceName)),
                text,
                $"password-reset/{tokenId:N}");
    }

    /// <summary>Sent to the new address of an account, which proves it receives mail by opening the link.</summary>
    public static EmailMessage EmailChange(
        string from,
        string to,
        string displayName,
        string instanceName,
        string confirmUrl,
        Guid requestId,
        Messages messages)
    {
        var intro = messages.Get(MessageKey.EmailChangeIntro, ("name", displayName));
        var text = Lines(
            messages.Get(MessageKey.EmailChangeGreeting, ("name", displayName)),
            "",
            messages.Get(MessageKey.EmailChangeInstruction, ("instance", instanceName)),
            confirmUrl,
            "",
            messages.Get(MessageKey.EmailChangeValidity),
            Signature(instanceName, OriginOf(confirmUrl), messages));

        return new MailLayout(messages.Language)
            .Instance(instanceName)
            .Preheader(intro)
            .Heading(messages.Get(MessageKey.EmailChangeHeading))
            .Paragraph(intro)
            .Button(confirmUrl, messages.Get(MessageKey.EmailChangeButton))
            .LinkFallback(confirmUrl, messages.Get(MessageKey.MailLinkFallback))
            .Divider()
            .Note(messages.Get(MessageKey.EmailChangeIgnore))
            .Footer(messages.Get(MessageKey.MailSentFromInstance, ("instance", instanceName)))
            .FooterLink(OriginOf(confirmUrl))
            .ToMessage(
                from,
                to,
                messages.Get(MessageKey.EmailChangeSubject, ("instance", instanceName)),
                text,
                $"email-change/{requestId:N}");
    }

    /// <summary>
    /// What the operator panel sends to check that the provider accepts mail.
    /// It names the way it went, so a test that arrives also says which
    /// settings sent it.
    /// </summary>
    public static EmailMessage DeliveryTest(
        string from,
        string to,
        string instanceName,
        string publicUrl,
        MailTransport transport,
        Messages messages)
    {
        var body = messages.Get(MessageKey.DeliveryTestBody, ("instance", instanceName));
        var route = transport is { Provider: MailProvider.Smtp, Smtp: { } smtp }
            ? messages.Get(MessageKey.DeliveryTestSmtp, ("server", $"{smtp.Host}:{smtp.Port.ToString(CultureInfo.InvariantCulture)}"))
            : messages.Get(MessageKey.DeliveryTestResend);
        var routeLabel = messages.Get(MessageKey.DeliveryTestTransportLabel);
        var text = Lines(body, "", $"{routeLabel}: {route}", Signature(instanceName, publicUrl, messages));

        return new MailLayout(messages.Language)
            .Instance(instanceName)
            .Preheader(body)
            .Heading(messages.Get(MessageKey.DeliveryTestHeading))
            .Paragraph(body)
            .Details([(routeLabel, route)])
            .Footer(messages.Get(MessageKey.MailSentFromInstance, ("instance", instanceName)))
            .FooterLink(publicUrl)
            .ToMessage(
            from,
            to,
            messages.Get(MessageKey.DeliveryTestSubject, ("instance", instanceName)),
            text,
            $"delivery-test/{Guid.NewGuid():N}");
    }

    /// <summary>A one-time code for the second step or for setting up mail as one.</summary>
    public static EmailMessage EmailCode(
        string from,
        string to,
        string displayName,
        string instanceName,
        string publicUrl,
        string code,
        int validMinutes,
        Guid codeId,
        Messages messages)
    {
        var intro = messages.Get(MessageKey.EmailCodeIntro, ("name", displayName), ("minutes", validMinutes));
        var ignore = messages.Get(MessageKey.SecurityAlertIgnore);
        var text = Lines(
            messages.Get(MessageKey.SecurityAlertGreeting, ("name", displayName)),
            "",
            messages.Get(MessageKey.EmailCodeInstruction, ("minutes", validMinutes)),
            code,
            "",
            ignore,
            Signature(instanceName, publicUrl, messages));

        return new MailLayout(messages.Language)
            .Instance(instanceName)
            .Preheader(intro)
            .Heading(messages.Get(MessageKey.EmailCodeHeading))
            .Paragraph(intro)
            .Code(code)
            .Note(ignore)
            .Footer(messages.Get(MessageKey.MailSentFromInstance, ("instance", instanceName)))
            .FooterLink(publicUrl)
            .ToMessage(
                from,
                to,
                messages.Get(MessageKey.EmailCodeSubject, ("instance", instanceName)),
                text,
                $"email-code/{codeId:N}");
    }

    /// <summary>
    /// Tells the owner of an account what just happened to it, when and from
    /// where, in the instance's time zone.
    /// </summary>
    public static EmailMessage SecurityAlert(
        string from,
        string to,
        string displayName,
        string instanceName,
        string publicUrl,
        SecurityEvent securityEvent,
        TimeZoneInfo timeZone,
        Messages messages)
    {
        var lead = messages.Get(MessageKey.SecurityAlertLead, ("name", displayName), ("instance", instanceName));
        var activity = DescribeEvent(securityEvent.Kind, messages);
        var time = FormatTime(securityEvent.CreatedAt, timeZone, messages);
        var ignore = messages.Get(MessageKey.SecurityAlertIgnore);

        List<(string Label, string Value)> facts =
        [
            (messages.Get(MessageKey.SecurityAlertActivityLabel), activity),
            (messages.Get(MessageKey.SecurityAlertTimeLabel), time),
        ];
        if (DeviceName.Describe(securityEvent.Device, messages) is { } device)
        {
            facts.Add((messages.Get(MessageKey.SecurityAlertDeviceLabel), device));
        }

        if (securityEvent.IpAddress.Length > 0)
        {
            facts.Add((messages.Get(MessageKey.SecurityAlertAddressLabel), securityEvent.IpAddress));
        }

        var text = Lines(
            [
                lead,
                "",
                .. facts.Select(x => $"{x.Label}: {x.Value}"),
                "",
                ignore,
                Signature(instanceName, publicUrl, messages),
            ]);

        return new MailLayout(messages.Language)
            .Instance(instanceName)
            .Preheader($"{activity} · {time}")
            .Heading(messages.Get(MessageKey.SecurityAlertHeading))
            .Paragraph(lead)
            .Details(facts)
            .Paragraph(ignore)
            .Button(publicUrl.TrimEnd('/') + "/", messages.Get(MessageKey.NotificationOpenButton))
            .Footer(messages.Get(MessageKey.MailSentFromInstance, ("instance", instanceName)))
            .FooterLink(publicUrl)
            .ToMessage(
                from,
                to,
                messages.Get(MessageKey.SecurityAlertSubject, ("instance", instanceName)),
                text,
                $"security-event/{securityEvent.Id:N}");
    }

    /// <summary>The name of an event as the account's activity list shows it.</summary>
    public static string DescribeEvent(string kind, Messages messages) => kind switch
    {
        SecurityEventKinds.LoginSucceeded => messages.Get(MessageKey.SecurityEventLoginSucceeded),
        SecurityEventKinds.LoginFailed => messages.Get(MessageKey.SecurityEventLoginFailed),
        SecurityEventKinds.MfaSucceeded => messages.Get(MessageKey.SecurityEventMfaSucceeded),
        SecurityEventKinds.MfaFailed => messages.Get(MessageKey.SecurityEventMfaFailed),
        SecurityEventKinds.MethodAdded => messages.Get(MessageKey.SecurityEventMethodAdded),
        SecurityEventKinds.MethodRemoved => messages.Get(MessageKey.SecurityEventMethodRemoved),
        SecurityEventKinds.RecoveryCodesRegenerated => messages.Get(MessageKey.SecurityEventRecoveryCodesRegenerated),
        SecurityEventKinds.RecoveryCodeUsed => messages.Get(MessageKey.SecurityEventRecoveryCodeUsed),
        SecurityEventKinds.TrustedDeviceAdded => messages.Get(MessageKey.SecurityEventTrustedDeviceAdded),
        SecurityEventKinds.TrustedDeviceRemoved => messages.Get(MessageKey.SecurityEventTrustedDeviceRemoved),
        SecurityEventKinds.SessionRevoked => messages.Get(MessageKey.SecurityEventSessionRevoked),
        SecurityEventKinds.PasswordChanged => messages.Get(MessageKey.SecurityEventPasswordChanged),
        SecurityEventKinds.CliRecovery => messages.Get(MessageKey.SecurityEventCliRecovery),
        SecurityEventKinds.PolicyChanged => messages.Get(MessageKey.SecurityEventPolicyChanged),
        SecurityEventKinds.DesktopApproved => messages.Get(MessageKey.SecurityEventDesktopApproved),
        SecurityEventKinds.IdentityLinked => messages.Get(MessageKey.SecurityEventIdentityLinked),
        SecurityEventKinds.IdentityUnlinked => messages.Get(MessageKey.SecurityEventIdentityUnlinked),
        SecurityEventKinds.EmailChanged => messages.Get(MessageKey.SecurityEventEmailChanged),
        _ => messages.Get(MessageKey.SecurityEventOther, ("kind", kind)),
    };

    /// <summary>"20 listopada 2026, 14:03 (Europe/Warsaw)": the date in the instance's language, the zone named.</summary>
    private static string FormatTime(DateTime instant, TimeZoneInfo zone, Messages messages)
    {
        // SQLite hands timestamps back without a kind; they are stored in UTC.
        var local = TimeZoneInfo.ConvertTimeFromUtc(DateTime.SpecifyKind(instant, DateTimeKind.Utc), zone);
        var culture = CultureInfo.GetCultureInfo(messages.Language == LanguageOptions.Polish ? "pl-PL" : "en-GB");
        return $"{local.ToString("d MMMM yyyy, HH:mm", culture)} ({zone.Id})";
    }

    /// <summary>The scheme, host and port of a link into the instance.</summary>
    private static string OriginOf(string url) =>
        Uri.TryCreate(url, UriKind.Absolute, out var parsed) ? parsed.GetLeftPart(UriPartial.Authority) : "";

    /// <summary>The plain-text part's sign-off, after the conventional "-- " separator.</summary>
    private static string Signature(string instanceName, string publicUrl, Messages messages)
    {
        var sentBy = messages.Get(MessageKey.MailSentFromInstance, ("instance", instanceName));
        return publicUrl.Length == 0 ? $"\n-- \n{sentBy}" : $"\n-- \n{sentBy}\n{publicUrl}";
    }

    private static string Lines(params string[] lines) => string.Join("\n", lines);
}
