namespace Stellarr.Api.Mail;

/// <summary>
/// An image that travels inside the message and that the HTML shows through
/// <c>cid:</c> followed by <see cref="ContentId"/>. Nothing is fetched from
/// anywhere when the mail is opened.
/// </summary>
public sealed record InlineImage(string ContentId, string FileName, string ContentType, byte[] Content);

public sealed record EmailMessage(
    string From,
    string To,
    string Subject,
    string Html,
    string Text,
    string IdempotencyKey,
    IReadOnlyList<InlineImage>? Images = null)
{
    public IReadOnlyList<InlineImage> InlineImages => Images ?? [];
}

public interface IEmailDelivery
{
    /// <summary>Sends the message through <paramref name="transport"/> and returns the provider's id for it.</summary>
    Task<string> SendAsync(EmailMessage message, MailTransport transport, CancellationToken cancellationToken);
}
