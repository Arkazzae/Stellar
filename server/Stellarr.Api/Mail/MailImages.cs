namespace Stellarr.Api.Mail;

/// <summary>
/// The images the messages carry, embedded in the assembly from
/// <c>Mail/Assets</c>. They are rendered from <c>branding/</c>: the lockup
/// from <c>logo/lockup-small.svg</c> at three times the 36 px it is shown at,
/// the stickers from <c>mascot/stickers/</c> at twice their 112 px.
/// </summary>
public static class MailImages
{
    public static readonly InlineImage Lockup = Load("lockup.png", "lockup@stellarr");
    public static readonly InlineImage Mention = Load("mention.png", "mention@stellarr");
    public static readonly InlineImage DirectMessage = Load("direct-message.png", "direct-message@stellarr");

    private static InlineImage Load(string fileName, string contentId)
    {
        var name = $"Stellarr.Api.Mail.Assets.{fileName}";
        using var stream = typeof(MailImages).Assembly.GetManifestResourceStream(name)
            ?? throw new InvalidOperationException($"The mail image {name} is not embedded in the assembly.");
        using var copy = new MemoryStream();
        stream.CopyTo(copy);
        return new InlineImage(contentId, fileName, "image/png", copy.ToArray());
    }
}
