using Stellarr.Api.Localization;

namespace Stellarr.Api.Mail;

/// <summary>
/// The mail a mention or a direct message causes. It is deliberately generic:
/// no message body, sender name, server name or channel name leaves the
/// instance, and the only link is the public entry point. The app has no
/// stable channel or conversation permalink, so a deep link would be a guess.
/// For the same reason the header shows the wordmark without the instance's
/// name, which is the name of its first server.
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
            messages.Get(MessageKey.NotificationMentionPushTitle),
            messages.Get(MessageKey.NotificationMentionBody),
            MailImages.Mention,
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
            messages.Get(MessageKey.NotificationDirectMessagePushTitle),
            messages.Get(MessageKey.NotificationDirectMessageBody),
            MailImages.DirectMessage,
            messages);

    private static EmailMessage Build(
        string from,
        string to,
        string publicUrl,
        Guid deliveryId,
        string subject,
        string heading,
        string body,
        InlineImage sticker,
        Messages messages)
    {
        var url = $"{publicUrl.TrimEnd('/')}/";
        var footer = messages.Get(MessageKey.NotificationEmailFooter);
        var text = string.Join("\n", body, "", url, "", "-- ", footer);

        return new MailLayout(messages.Language)
            .Preheader(body)
            .Illustration(sticker)
            .Heading(heading)
            .Paragraph(body)
            .Button(url, messages.Get(MessageKey.NotificationOpenButton))
            .Footer(footer)
            .ToMessage(
                from,
                to,
                subject,
                text,
                // Stable for the row, so a second attempt is the same message to
                // the provider even though SMTP itself promises no exactly-once.
                $"activity-notification/{deliveryId:N}");
    }
}
