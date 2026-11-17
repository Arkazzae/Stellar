using System.Net;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;

namespace Stellarr.Api.Mail;

/// <summary>
/// The mail a mention or a direct message causes. It is deliberately generic:
/// no message body, sender name, server name or channel name leaves the
/// instance, and the only link is the public entry point. The app has no
/// stable channel or conversation permalink, so a deep link would be a guess.
/// </summary>
public static class ActivityEmail
{
    public static EmailMessage Mention(
        string from,
        string to,
        string publicUrl,
        Guid deliveryId,
        Messages messages) =>
        Build(
            from,
            to,
            publicUrl,
            deliveryId,
            messages.Get(MessageKey.NotificationMentionSubject),
            messages.Get(MessageKey.NotificationMentionBody),
            messages);

    public static EmailMessage DirectMessage(
        string from,
        string to,
        string publicUrl,
        Guid deliveryId,
        Messages messages) =>
        Build(
            from,
            to,
            publicUrl,
            deliveryId,
            messages.Get(MessageKey.NotificationDirectMessageSubject),
            messages.Get(MessageKey.NotificationDirectMessageBody),
            messages);

    private static EmailMessage Build(
        string from,
        string to,
        string publicUrl,
        Guid deliveryId,
        string subject,
        string body,
        Messages messages)
    {
        var url = $"{publicUrl.TrimEnd('/')}/";
        var safeUrl = WebUtility.HtmlEncode(url);
        var safeBody = WebUtility.HtmlEncode(body);
        var text = string.Join("\n", body, "", url);
        var html = $$"""
            <!doctype html>
            <html lang="{{messages.Language}}"><body style="margin:0;background:#171717;color:#f5f5f5;font-family:system-ui,-apple-system,Segoe UI,sans-serif">
              <div style="max-width:560px;margin:0 auto;padding:40px 24px">
                <p style="margin:0 0 24px;color:#d6a554;font-weight:700">Stellarr</p>
                <p style="margin:0 0 28px;color:#d4d4d4;line-height:1.6">{{safeBody}}</p>
                <p style="margin:0"><a href="{{safeUrl}}" style="display:inline-block;padding:12px 18px;background:#d6a554;color:#17130d;text-decoration:none;font-weight:700;border-radius:4px">{{messages.Get(MessageKey.NotificationOpenButton)}}</a></p>
              </div>
            </body></html>
            """;

        return new EmailMessage(
            from,
            to,
            subject,
            html,
            text,
            // Stable for the row, so a second attempt is the same message to
            // the provider even though SMTP itself promises no exactly-once.
            $"activity-notification/{deliveryId:N}");
    }
}
